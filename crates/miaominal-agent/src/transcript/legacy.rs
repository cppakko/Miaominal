//! Conversion of pre-v2 chat records into transcript items.
//!
//! Old records lack tool call ids, arguments and results, so tool calls cannot
//! be replayed to the model; they become UI-only legacy notices.

use super::{Item, ItemKind, NoticeLevel, new_id};
use crate::model::ReasoningReplay;
use miaominal_core::chat_attachment::ChatAttachment;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyRole {
    User,
    Assistant,
    ToolCall,
    Thinking,
    Error,
}

#[derive(Clone, Debug)]
pub struct LegacyMessage {
    pub role: LegacyRole,
    pub content: String,
    pub tool_name: Option<String>,
    pub tool_summary: Option<String>,
    pub tool_status: Option<String>,
    pub attachments: Vec<ChatAttachment>,
    pub created_at: i64,
}

pub fn items_from_legacy(messages: Vec<LegacyMessage>) -> Vec<Item> {
    let mut items = Vec::with_capacity(messages.len());
    let mut turn_id = new_id();
    let mut completion_id = new_id();
    for message in messages {
        let kind = match message.role {
            LegacyRole::User => {
                turn_id = new_id();
                completion_id = new_id();
                ItemKind::UserMessage {
                    text: message.content,
                    attachments: message.attachments,
                    context: None,
                    targets: Vec::new(),
                }
            }
            LegacyRole::Assistant => {
                let kind = ItemKind::AssistantText {
                    text: message.content,
                    completion_id: completion_id.clone(),
                };
                completion_id = new_id();
                kind
            }
            LegacyRole::Thinking => ItemKind::Reasoning {
                text: message.content,
                signature: None,
                redacted: None,
                completion_id: completion_id.clone(),
                replay: ReasoningReplay::Never,
                duration_ms: None,
            },
            LegacyRole::ToolCall => {
                let name = message.tool_name.unwrap_or_else(|| "tool".into());
                let summary = message
                    .tool_summary
                    .filter(|summary| !summary.trim().is_empty())
                    .unwrap_or(message.content);
                let status = message
                    .tool_status
                    .map(|status| format!(" ({status})"))
                    .unwrap_or_default();
                ItemKind::Notice {
                    level: NoticeLevel::Legacy,
                    text: format!("{name}{status}: {summary}"),
                }
            }
            LegacyRole::Error => ItemKind::Notice {
                level: NoticeLevel::Error,
                text: message.content,
            },
        };
        items.push(Item {
            id: new_id(),
            turn_id: turn_id.clone(),
            created_at: message.created_at,
            kind,
        });
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: LegacyRole, content: &str) -> LegacyMessage {
        LegacyMessage {
            role,
            content: content.into(),
            tool_name: Some("run_shell".into()),
            tool_summary: None,
            tool_status: Some("completed".into()),
            attachments: Vec::new(),
            created_at: 1,
        }
    }

    #[test]
    fn legacy_records_map_to_items() {
        let items = items_from_legacy(vec![
            message(LegacyRole::User, "hi"),
            message(LegacyRole::Thinking, "hmm"),
            message(LegacyRole::ToolCall, "ls"),
            message(LegacyRole::Assistant, "done"),
            message(LegacyRole::Error, "boom"),
        ]);
        assert_eq!(items.len(), 5);
        assert!(items.iter().all(|item| item.turn_id == items[0].turn_id));
        assert!(matches!(
            &items[2].kind,
            ItemKind::Notice { level: NoticeLevel::Legacy, text } if text == "run_shell (completed): ls"
        ));
        assert!(matches!(
            &items[1].kind,
            ItemKind::Reasoning {
                replay: ReasoningReplay::Never,
                ..
            }
        ));
    }
}
