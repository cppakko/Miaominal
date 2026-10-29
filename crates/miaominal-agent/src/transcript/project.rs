use super::{Item, ItemKind, ToolResultStatus};
use crate::model::{AssistantPart, ModelCapabilities, ReasoningReplay, UserPart, WireMessage};
use miaominal_core::chat_attachment::{ChatAttachment, ChatAttachmentContent};
use std::collections::{HashMap, HashSet};

/// Controls how much of the transcript is sent verbatim.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProjectionBudget {
    /// Tool results that belong to user turns with index lower than this value
    /// (0-based, counted over `UserMessage` items) are replaced by a stub.
    pub elide_tool_results_before_turn: Option<usize>,
    /// Images on user turns with index lower than this value are dropped.
    pub drop_images_before_turn: Option<usize>,
}

const ELIDED_TOOL_RESULT: &str =
    "[output elided to save context — re-run the tool if you need it again]";

/// Project the transcript into wire messages.
pub fn project(
    items: &[Item],
    capabilities: &ModelCapabilities,
    budget: ProjectionBudget,
) -> Vec<WireMessage> {
    let summaries: HashMap<&str, (&str, &str)> = items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::Summary {
                text,
                first_item_id,
                last_item_id,
            } => Some((
                first_item_id.as_str(),
                (text.as_str(), last_item_id.as_str()),
            )),
            _ => None,
        })
        .collect();
    let answered: HashSet<&str> = items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::ToolResult { call_id, .. } => Some(call_id.as_str()),
            _ => None,
        })
        .collect();
    let called: HashSet<&str> = items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::ToolCall { call_id, .. } => Some(call_id.as_str()),
            _ => None,
        })
        .collect();
    let last_user_index = items
        .iter()
        .rposition(|item| matches!(item.kind, ItemKind::UserMessage { .. }));

    let mut messages: Vec<WireMessage> = Vec::new();
    let mut pending_assistant: Option<(String, Vec<AssistantPart>)> = None;
    let mut skip_until: Option<&str> = None;
    let mut user_turn: usize = 0;

    let flush = |pending: &mut Option<(String, Vec<AssistantPart>)>,
                 messages: &mut Vec<WireMessage>| {
        if let Some((_, parts)) = pending.take()
            && !parts.is_empty()
        {
            messages.push(WireMessage::Assistant(parts));
        }
    };

    for (index, item) in items.iter().enumerate() {
        if let Some(last) = skip_until {
            if item.id == last {
                skip_until = None;
            }
            if matches!(item.kind, ItemKind::UserMessage { .. }) {
                user_turn += 1;
            }
            continue;
        }
        if let Some((text, last)) = summaries.get(item.id.as_str()) {
            flush(&mut pending_assistant, &mut messages);
            messages.push(WireMessage::User(vec![UserPart::Text(format!(
                "<conversation_summary>\nSummary of the earlier part of this conversation:\n{text}\n</conversation_summary>"
            ))]));
            if item.id != *last {
                skip_until = Some(last);
            }
            if matches!(item.kind, ItemKind::UserMessage { .. }) {
                user_turn += 1;
            }
            continue;
        }
        let in_current_turn = last_user_index.is_some_and(|last| index > last);
        match &item.kind {
            ItemKind::UserMessage {
                text,
                attachments,
                context,
                ..
            } => {
                flush(&mut pending_assistant, &mut messages);
                let keep_images = budget
                    .drop_images_before_turn
                    .is_none_or(|limit| user_turn >= limit);
                messages.push(WireMessage::User(user_parts(
                    text,
                    attachments,
                    context.as_deref(),
                    capabilities.vision && keep_images,
                    keep_images,
                )));
                user_turn += 1;
            }
            ItemKind::AssistantText {
                text,
                completion_id,
            } => {
                if text.is_empty() {
                    continue;
                }
                assistant_parts(&mut pending_assistant, &mut messages, completion_id)
                    .push(AssistantPart::Text(text.clone()));
            }
            ItemKind::Reasoning {
                text,
                signature,
                redacted,
                completion_id,
                replay,
                ..
            } => {
                let replay_now = *replay == ReasoningReplay::CurrentTurn
                    && capabilities.reasoning_replay == ReasoningReplay::CurrentTurn
                    && in_current_turn;
                if !replay_now {
                    continue;
                }
                let parts = assistant_parts(&mut pending_assistant, &mut messages, completion_id);
                if let Some(data) = redacted {
                    parts.push(AssistantPart::RedactedReasoning(data.clone()));
                } else {
                    parts.push(AssistantPart::Reasoning {
                        text: text.clone(),
                        signature: signature.clone(),
                    });
                }
            }
            ItemKind::ToolCall {
                call_id,
                name,
                arguments,
                completion_id,
            } => {
                if !answered.contains(call_id.as_str()) {
                    // A dangling call would make the request invalid. The engine
                    // repairs these before projecting; skip defensively.
                    continue;
                }
                assistant_parts(&mut pending_assistant, &mut messages, completion_id).push(
                    AssistantPart::ToolCall {
                        id: call_id.clone(),
                        name: name.clone(),
                        arguments: if arguments.trim().is_empty() {
                            "{}".to_string()
                        } else {
                            arguments.clone()
                        },
                    },
                );
            }
            ItemKind::ToolResult {
                call_id,
                status,
                model_text,
                elided,
                ..
            } => {
                if !called.contains(call_id.as_str()) {
                    continue;
                }
                flush(&mut pending_assistant, &mut messages);
                let elide = *elided
                    || budget
                        .elide_tool_results_before_turn
                        .is_some_and(|limit| user_turn <= limit && !in_current_turn);
                messages.push(WireMessage::ToolResult {
                    call_id: call_id.clone(),
                    content: if elide {
                        ELIDED_TOOL_RESULT.to_string()
                    } else {
                        model_text.clone()
                    },
                    is_error: !matches!(status, ToolResultStatus::Ok),
                });
            }
            ItemKind::Summary { .. } | ItemKind::Notice { .. } | ItemKind::Usage { .. } => {}
        }
    }
    flush(&mut pending_assistant, &mut messages);
    messages
}

fn assistant_parts<'a>(
    pending: &'a mut Option<(String, Vec<AssistantPart>)>,
    messages: &mut Vec<WireMessage>,
    completion_id: &str,
) -> &'a mut Vec<AssistantPart> {
    let same = pending
        .as_ref()
        .is_some_and(|(current, _)| current == completion_id);
    if !same {
        if let Some((_, parts)) = pending.take()
            && !parts.is_empty()
        {
            messages.push(WireMessage::Assistant(parts));
        }
        *pending = Some((completion_id.to_string(), Vec::new()));
    }
    &mut pending.as_mut().expect("pending assistant was just set").1
}

fn user_parts(
    text: &str,
    attachments: &[ChatAttachment],
    context: Option<&str>,
    include_images: bool,
    images_retained: bool,
) -> Vec<UserPart> {
    let mut body = String::new();
    if let Some(context) = context.filter(|context| !context.trim().is_empty()) {
        body.push_str(context.trim_end());
        body.push_str("\n\n");
    }
    body.push_str(text);
    let mut image_count = 0usize;
    for attachment in attachments {
        match &attachment.content {
            ChatAttachmentContent::TextFile(file) => {
                let language = file.language.as_deref().unwrap_or("");
                body.push_str(&format!(
                    "\n\n<attachment name=\"{}\">\n```{language}\n{}\n```\n</attachment>",
                    attachment.filename, file.text
                ));
            }
            ChatAttachmentContent::Image(_) => image_count += 1,
        }
    }
    if image_count > 0 && !include_images {
        let note = if images_retained {
            format!("[{image_count} image(s) attached, but the current model cannot view images]")
        } else {
            format!("[{image_count} image(s) were attached earlier and removed to save context]")
        };
        body.push_str("\n\n");
        body.push_str(&note);
    }
    let mut parts = vec![UserPart::Text(body)];
    if include_images {
        for attachment in attachments {
            if let ChatAttachmentContent::Image(image) = &attachment.content {
                parts.push(UserPart::Image {
                    mime_type: image.mime_type.clone(),
                    data_base64: image.data_base64.clone(),
                });
            }
        }
    }
    parts
}

/// Returns `Cancelled` results for tool calls that have no result, so the
/// transcript satisfies the call/result pairing invariant.
pub fn repair_dangling_tool_calls(items: &[Item]) -> Vec<Item> {
    let answered: HashSet<&str> = items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::ToolResult { call_id, .. } => Some(call_id.as_str()),
            _ => None,
        })
        .collect();
    items
        .iter()
        .filter_map(|item| match &item.kind {
            ItemKind::ToolCall { call_id, name, .. } if !answered.contains(call_id.as_str()) => {
                Some(Item::new(
                    &item.turn_id,
                    ItemKind::ToolResult {
                        call_id: call_id.clone(),
                        name: name.clone(),
                        status: ToolResultStatus::Cancelled,
                        model_text:
                            "Cancelled: this call was interrupted before it produced a result."
                                .into(),
                        display: Default::default(),
                        elided: false,
                    },
                ))
            }
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ReasoningReplay;

    fn caps(replay: ReasoningReplay) -> ModelCapabilities {
        ModelCapabilities {
            vision: false,
            parallel_tool_calls: true,
            reasoning_replay: replay,
            context_window: 128_000,
        }
    }

    fn user(turn: &str, text: &str) -> Item {
        Item::new(
            turn,
            ItemKind::UserMessage {
                text: text.into(),
                attachments: Vec::new(),
                context: None,
                targets: Vec::new(),
            },
        )
    }

    fn call(turn: &str, completion: &str, id: &str) -> Item {
        Item::new(
            turn,
            ItemKind::ToolCall {
                call_id: id.into(),
                name: "shell".into(),
                arguments: "{\"command\":\"ls\"}".into(),
                completion_id: completion.into(),
            },
        )
    }

    fn result(turn: &str, id: &str, text: &str) -> Item {
        Item::new(
            turn,
            ItemKind::ToolResult {
                call_id: id.into(),
                name: "shell".into(),
                status: ToolResultStatus::Ok,
                model_text: text.into(),
                display: Default::default(),
                elided: false,
            },
        )
    }

    fn reasoning(turn: &str, completion: &str) -> Item {
        Item::new(
            turn,
            ItemKind::Reasoning {
                text: "think".into(),
                signature: Some("sig".into()),
                redacted: None,
                completion_id: completion.into(),
                replay: ReasoningReplay::CurrentTurn,
                duration_ms: None,
            },
        )
    }

    #[test]
    fn previous_turn_tool_results_are_kept() {
        let items = vec![
            user("t1", "check disk"),
            call("t1", "c1", "a"),
            result("t1", "a", "50% used"),
            Item::new(
                "t1",
                ItemKind::AssistantText {
                    text: "Disk is fine".into(),
                    completion_id: "c2".into(),
                },
            ),
            user("t2", "and memory?"),
        ];
        let messages = project(&items, &caps(ReasoningReplay::Never), Default::default());
        assert_eq!(messages.len(), 5);
        assert!(matches!(
            &messages[2],
            WireMessage::ToolResult { content, .. } if content == "50% used"
        ));
    }

    #[test]
    fn parallel_calls_share_one_assistant_message() {
        let items = vec![
            user("t1", "x"),
            call("t1", "c1", "a"),
            call("t1", "c1", "b"),
            result("t1", "a", "1"),
            result("t1", "b", "2"),
        ];
        let messages = project(&items, &caps(ReasoningReplay::Never), Default::default());
        assert_eq!(messages.len(), 4);
        let WireMessage::Assistant(parts) = &messages[1] else {
            panic!("expected assistant");
        };
        assert_eq!(parts.len(), 2);
    }

    #[test]
    fn reasoning_replayed_only_in_current_turn() {
        let items = vec![
            user("t1", "x"),
            reasoning("t1", "c1"),
            call("t1", "c1", "a"),
            result("t1", "a", "1"),
            user("t2", "y"),
            reasoning("t2", "c2"),
            call("t2", "c2", "b"),
            result("t2", "b", "2"),
        ];
        let messages = project(
            &items,
            &caps(ReasoningReplay::CurrentTurn),
            Default::default(),
        );
        let WireMessage::Assistant(first) = &messages[1] else {
            panic!()
        };
        assert_eq!(first.len(), 1, "old turn reasoning dropped");
        let WireMessage::Assistant(second) = &messages[4] else {
            panic!()
        };
        assert!(matches!(second[0], AssistantPart::Reasoning { .. }));
        let none = project(&items, &caps(ReasoningReplay::Never), Default::default());
        let WireMessage::Assistant(second) = &none[4] else {
            panic!()
        };
        assert_eq!(second.len(), 1);
    }

    #[test]
    fn dangling_calls_are_skipped_and_repairable() {
        let items = vec![user("t1", "x"), call("t1", "c1", "a")];
        let messages = project(&items, &caps(ReasoningReplay::Never), Default::default());
        assert_eq!(messages.len(), 1);
        let repairs = repair_dangling_tool_calls(&items);
        assert_eq!(repairs.len(), 1);
        let mut repaired = items.clone();
        repaired.extend(repairs);
        let messages = project(&repaired, &caps(ReasoningReplay::Never), Default::default());
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn notices_never_reach_the_model() {
        let items = vec![
            user("t1", "x"),
            Item::new(
                "t1",
                ItemKind::Notice {
                    level: super::super::NoticeLevel::Error,
                    text: "boom".into(),
                },
            ),
        ];
        let messages = project(&items, &caps(ReasoningReplay::Never), Default::default());
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn summary_replaces_range_and_old_results_can_be_elided() {
        let first = user("t1", "a");
        let first_id = first.id.clone();
        let res = result("t1", "a1", "big output");
        let last_id = res.id.clone();
        let items = vec![
            first,
            call("t1", "c1", "a1"),
            res,
            user("t2", "b"),
            call("t2", "c2", "b1"),
            result("t2", "b1", "old output"),
            user("t3", "c"),
            Item::new(
                "t3",
                ItemKind::Summary {
                    text: "did a".into(),
                    first_item_id: first_id,
                    last_item_id: last_id,
                },
            ),
        ];
        let messages = project(
            &items,
            &caps(ReasoningReplay::Never),
            ProjectionBudget {
                elide_tool_results_before_turn: Some(2),
                drop_images_before_turn: None,
            },
        );
        assert!(matches!(&messages[0], WireMessage::User(parts)
            if matches!(&parts[0], UserPart::Text(text) if text.contains("did a"))));
        assert!(
            matches!(&messages[3], WireMessage::ToolResult { content, .. }
            if content.starts_with("[output elided"))
        );
        assert_eq!(messages.len(), 5);
    }
}
