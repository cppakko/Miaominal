//! History summarization (design §7.4 step 2).

use futures::StreamExt;

use super::session::Worker;
use super::{AgentEvent, SessionStatus};
use crate::context::SUMMARY_INSTRUCTIONS;
use crate::model::{ModelEvent, ModelRequest, UserPart, WireMessage};
use crate::tools::AgentToolCancellation;
use crate::transcript::{Item, ItemKind, NoticeLevel, ProjectionBudget, project};

/// Choose the transcript prefix to summarize: whole user turns, the oldest
/// half of the turns not yet covered by a summary, never the latest turn.
/// Returns the index of the last item to include.
pub(super) fn summary_cut(items: &[Item]) -> Option<usize> {
    let covered_until = items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::Summary { last_item_id, .. } => items
                .iter()
                .position(|candidate| &candidate.id == last_item_id),
            _ => None,
        })
        .max();
    let start = covered_until.map(|index| index + 1).unwrap_or(0);
    let turn_starts: Vec<usize> = items
        .iter()
        .enumerate()
        .skip(start)
        .filter(|(_, item)| matches!(item.kind, ItemKind::UserMessage { .. }))
        .map(|(index, _)| index)
        .collect();
    // Need at least one complete turn before the latest one.
    if turn_starts.len() < 2 {
        return None;
    }
    let uncovered_turns = turn_starts.len() - 1;
    let take = uncovered_turns.div_ceil(2).max(1);
    // The cut ends just before the first turn we keep.
    let keep_from = turn_starts[take];
    let mut cut = keep_from.checked_sub(1)?;
    // Do not end on a Summary item itself.
    while cut > 0 && matches!(items[cut].kind, ItemKind::Summary { .. }) {
        cut -= 1;
    }
    Some(cut)
}

/// Summarize the oldest half of the conversation with the session model.
/// Returns true when a summary was committed.
pub(super) async fn summarize_oldest(
    worker: &mut Worker,
    turn_id: &str,
    cancel: &AgentToolCancellation,
) -> bool {
    let Some(cut) = summary_cut(&worker.items) else {
        return false;
    };
    let first_item_id = worker.items[0].id.clone();
    let last_item_id = worker.items[cut].id.clone();
    worker.emit(AgentEvent::Status(SessionStatus::Compacting));
    worker.emit(AgentEvent::Info {
        level: NoticeLevel::Info,
        text: "正在压缩较早的对话…".into(),
    });

    let capabilities = worker.model.capabilities();
    // Earlier summaries are stored after the range they replace; include
    // them so the new summary builds on the old one.
    let scope: Vec<Item> = worker
        .items
        .iter()
        .enumerate()
        .filter(|(index, item)| *index <= cut || matches!(item.kind, ItemKind::Summary { .. }))
        .map(|(_, item)| item.clone())
        .collect();
    let user_turns = scope
        .iter()
        .filter(|item| matches!(item.kind, ItemKind::UserMessage { .. }))
        .count();
    let mut messages = project(
        &scope,
        &capabilities,
        ProjectionBudget {
            elide_tool_results_before_turn: None,
            drop_images_before_turn: Some(user_turns),
        },
    );
    messages.push(WireMessage::User(vec![UserPart::Text(
        SUMMARY_INSTRUCTIONS.to_string(),
    )]));
    let request = ModelRequest {
        system: "You compress conversation history for a server operations assistant. \
Reply with the summary only."
            .to_string(),
        messages,
        tools: Vec::new(),
    };

    let model = worker.model.clone();
    let stream = tokio::select! {
        stream = model.stream(request) => stream,
        _ = cancel.cancelled() => return false,
    };
    let mut stream = match stream {
        Ok(stream) => stream,
        Err(error) => {
            worker.emit(AgentEvent::Info {
                level: NoticeLevel::Warning,
                text: format!("压缩对话失败：{error}"),
            });
            return false;
        }
    };
    let mut text = String::new();
    loop {
        let event = tokio::select! {
            event = stream.next() => event,
            _ = cancel.cancelled() => return false,
        };
        match event {
            Some(Ok(ModelEvent::TextDelta(delta))) => text.push_str(&delta),
            Some(Ok(_)) => {}
            Some(Err(error)) => {
                worker.emit(AgentEvent::Info {
                    level: NoticeLevel::Warning,
                    text: format!("压缩对话失败：{error}"),
                });
                return false;
            }
            None => break,
        }
    }
    let text = text.trim().to_string();
    if text.is_empty() {
        return false;
    }
    worker.commit(vec![Item::new(
        turn_id,
        ItemKind::Summary {
            text,
            first_item_id,
            last_item_id,
        },
    )]);
    worker.budget = ProjectionBudget::default();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(turn: &str) -> Item {
        Item::new(
            turn,
            ItemKind::UserMessage {
                text: turn.into(),
                attachments: vec![],
                context: None,
                targets: vec![],
            },
        )
    }

    fn reply(turn: &str) -> Item {
        Item::new(
            turn,
            ItemKind::AssistantText {
                text: "ok".into(),
                completion_id: "c".into(),
            },
        )
    }

    #[test]
    fn cut_keeps_latest_turn_and_takes_oldest_half() {
        let items = vec![
            user("1"),
            reply("1"),
            user("2"),
            reply("2"),
            user("3"),
            reply("3"),
            user("4"),
        ];
        // 3 complete turns before the latest; summarize ceil(3/2)=2 turns.
        assert_eq!(summary_cut(&items), Some(3));
        assert_eq!(summary_cut(&items[..2]), None);
    }

    #[test]
    fn cut_skips_already_summarized_turns() {
        let mut items = vec![user("1"), reply("1"), user("2"), reply("2")];
        let summary = Item::new(
            "s",
            ItemKind::Summary {
                text: "s".into(),
                first_item_id: items[0].id.clone(),
                last_item_id: items[1].id.clone(),
            },
        );
        items.push(summary);
        items.push(user("3"));
        // Uncovered turns: 2 (complete) and 3 (latest) → summarize turn 2.
        assert_eq!(summary_cut(&items), Some(4 - 1));
    }
}
