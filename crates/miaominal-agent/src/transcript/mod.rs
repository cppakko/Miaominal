//! Conversation transcript: the single source of truth for what the model saw.
//!
//! Every item is persisted. [`project`] converts the transcript into the
//! provider-neutral wire messages for the next request.

mod codec;
mod legacy;
mod project;

pub use codec::{item_to_record, items_from_legacy_records, items_from_records};
pub use legacy::{LegacyMessage, LegacyRole, items_from_legacy};
pub use project::{ProjectionBudget, project, repair_dangling_tool_calls};

use crate::model::{ReasoningReplay, TokenUsage};
use miaominal_core::chat_attachment::ChatAttachment;
use serde::{Deserialize, Serialize};

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    /// All items produced by one user submission share a turn id.
    pub turn_id: String,
    pub created_at: i64,
    pub kind: ItemKind,
}

impl Item {
    pub fn new(turn_id: &str, kind: ItemKind) -> Self {
        Self {
            id: new_id(),
            turn_id: turn_id.to_string(),
            created_at: now_unix(),
            kind,
        }
    }

    /// Stable short tag used as the storage `kind` column.
    pub fn kind_tag(&self) -> &'static str {
        self.kind.tag()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemKind {
    UserMessage {
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<ChatAttachment>,
        /// Rendered `<context>` block captured at submission time.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context: Option<String>,
        /// Execution targets mentioned by the user (`@name`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        targets: Vec<String>,
    },
    AssistantText {
        text: String,
        completion_id: String,
    },
    Reasoning {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        redacted: Option<String>,
        completion_id: String,
        replay: ReasoningReplay,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
    },
    ToolCall {
        call_id: String,
        name: String,
        arguments: String,
        completion_id: String,
    },
    ToolResult {
        call_id: String,
        name: String,
        status: ToolResultStatus,
        /// Text the model sees.
        model_text: String,
        /// Structured payload for the UI.
        #[serde(default)]
        display: ToolDisplay,
        /// Model text was replaced by a stub during context compaction.
        #[serde(default)]
        elided: bool,
    },
    /// Replaces the items from `first_item_id` through `last_item_id`
    /// (inclusive) when projecting for the model.
    Summary {
        text: String,
        first_item_id: String,
        last_item_id: String,
    },
    /// UI-only item. Never sent to the model.
    Notice {
        level: NoticeLevel,
        text: String,
    },
    /// Per-completion usage record (UI/statistics only).
    Usage {
        usage: TokenUsage,
        context_used: u64,
    },
}

impl ItemKind {
    pub fn tag(&self) -> &'static str {
        match self {
            Self::UserMessage { .. } => "user",
            Self::AssistantText { .. } => "assistant",
            Self::Reasoning { .. } => "reasoning",
            Self::ToolCall { .. } => "tool_call",
            Self::ToolResult { .. } => "tool_result",
            Self::Summary { .. } => "summary",
            Self::Notice { .. } => "notice",
            Self::Usage { .. } => "usage",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolResultStatus {
    Ok,
    Failed,
    Denied {
        by_user: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Cancelled,
    TimedOut,
}

impl ToolResultStatus {
    pub fn is_error(&self) -> bool {
        !matches!(self, Self::Ok)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
    /// Converted from a pre-v2 record that cannot be replayed to the model.
    Legacy,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolDisplay {
    #[default]
    None,
    Shell {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
        timed_out: bool,
        truncated: bool,
        duration_ms: u64,
    },
    File {
        path: String,
        content: String,
        #[serde(default)]
        start_line: usize,
        truncated: bool,
    },
    Diff {
        path: String,
        diff: String,
    },
    Entries {
        title: String,
        entries: Vec<String>,
        truncated: bool,
    },
    Plan {
        items: Vec<PlanItem>,
    },
    Answer {
        question: String,
        answer: String,
    },
    Text {
        title: String,
        body: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanItem {
    pub step: String,
    pub status: PlanStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}
