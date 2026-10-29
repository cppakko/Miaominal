//! Conversion between transcript items and storage rows.

use super::{Item, ItemKind, NoticeLevel, legacy, repair_dangling_tool_calls};
use miaominal_core::chat_attachment::ChatAttachment;
use miaominal_storage::chat_store::{
    ChatItemRecord, ChatMessageRecord, ChatMessageRole, LoadedChatItem,
};

pub fn item_to_record(item: &Item, seq: i64) -> ChatItemRecord {
    ChatItemRecord {
        id: item.id.clone(),
        seq,
        turn_id: item.turn_id.clone(),
        kind: item.kind_tag().to_string(),
        payload: serde_json::to_string(&item.kind).unwrap_or_else(|_| "{}".into()),
        created_at: item.created_at,
    }
}

/// Decode stored rows. Unreadable rows become error notices; dangling tool
/// calls (e.g. after a crash) receive `Cancelled` results so the transcript
/// stays valid. Returns the items plus the repair items that should be
/// appended to storage.
pub fn items_from_records(rows: Vec<LoadedChatItem>) -> (Vec<Item>, Vec<Item>) {
    let mut items: Vec<Item> = rows
        .into_iter()
        .map(|row| {
            let kind = row
                .payload
                .as_deref()
                .and_then(|payload| serde_json::from_str::<ItemKind>(payload).ok())
                .unwrap_or_else(|| ItemKind::Notice {
                    level: NoticeLevel::Error,
                    text: "[无法解密或解析的记录]".into(),
                });
            Item {
                id: row.id,
                turn_id: row.turn_id,
                created_at: row.created_at,
                kind,
            }
        })
        .collect();
    let repairs = repair_dangling_tool_calls(&items);
    items.extend(repairs.iter().cloned());
    (items, repairs)
}

/// Convert pre-v2 message rows.
pub fn items_from_legacy_records(records: Vec<ChatMessageRecord>) -> Vec<Item> {
    legacy::items_from_legacy(
        records
            .into_iter()
            .map(|record| legacy::LegacyMessage {
                role: match record.role {
                    ChatMessageRole::User => legacy::LegacyRole::User,
                    ChatMessageRole::Assistant => legacy::LegacyRole::Assistant,
                    ChatMessageRole::ToolCall => legacy::LegacyRole::ToolCall,
                    ChatMessageRole::Thinking => legacy::LegacyRole::Thinking,
                    ChatMessageRole::Error => legacy::LegacyRole::Error,
                },
                content: record.content,
                tool_name: record.tool_name,
                tool_summary: record.tool_summary,
                tool_status: record.tool_status,
                attachments: record
                    .attachments
                    .as_deref()
                    .and_then(|json| serde_json::from_str::<Vec<ChatAttachment>>(json).ok())
                    .unwrap_or_default(),
                created_at: record.created_at,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_repair() {
        let call = Item::new(
            "t",
            ItemKind::ToolCall {
                call_id: "c".into(),
                name: "shell".into(),
                arguments: "{}".into(),
                completion_id: "x".into(),
            },
        );
        let record = item_to_record(&call, 3);
        assert_eq!(record.kind, "tool_call");
        let rows = vec![
            LoadedChatItem {
                id: record.id.clone(),
                seq: 3,
                turn_id: record.turn_id.clone(),
                kind: record.kind.clone(),
                payload: Some(record.payload.clone()),
                created_at: record.created_at,
            },
            LoadedChatItem {
                id: "bad".into(),
                seq: 4,
                turn_id: "t".into(),
                kind: "user".into(),
                payload: None,
                created_at: 0,
            },
        ];
        let (items, repairs) = items_from_records(rows);
        assert_eq!(items[0], call);
        assert!(matches!(items[1].kind, ItemKind::Notice { .. }));
        assert_eq!(repairs.len(), 1);
        assert_eq!(items.len(), 3);
    }
}
