//! The agent engine (design §5): a pure-Rust session that owns the whole
//! model/tool loop. Approvals and questions are awaited *inside* the loop.
//!
//! The UI only translates user actions into [`AgentCommand`]s and projects
//! [`AgentEvent`]s into views.

mod compact;
mod session;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use miaominal_core::chat_attachment::ChatAttachment;
use tokio::sync::mpsc;

use crate::context::TargetInfo;
use crate::model::{ModelCapabilities, ModelClient};
use crate::policy::{AgentRiskLevel, ApprovalSettings};
use crate::toolkit::ToolRuntime;
use crate::transcript::{Item, NoticeLevel};

pub use session::spawn_session;

/// Loop limits for one turn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurnLimits {
    /// Model requests per user turn.
    pub max_steps: usize,
    /// Retries for transient model errors (429/5xx/network).
    pub max_retries: usize,
    /// The same invalid tool call in a row this many times aborts the turn.
    pub max_repeated_invalid_calls: usize,
    /// Concurrent read-only tool calls.
    pub max_parallel_reads: usize,
    /// Context budget as a fraction of the context window.
    pub context_budget_ratio: f64,
}

impl Default for TurnLimits {
    fn default() -> Self {
        Self {
            max_steps: 60,
            max_retries: 3,
            max_repeated_invalid_calls: 3,
            max_parallel_reads: 4,
            context_budget_ratio: 0.7,
        }
    }
}

/// Session configuration that can change between steps.
#[derive(Clone)]
pub struct AgentSessionConfig {
    pub session_id: String,
    pub approval: ApprovalSettings,
    pub limits: TurnLimits,
    /// Execution targets rendered into `<environment>`.
    pub targets: Vec<TargetInfo>,
    pub web_search: bool,
    pub terminal_available: bool,
}

/// Persists committed items (append-only from the engine's view).
pub trait TranscriptSink: Send + Sync {
    fn append(&self, items: &[Item]) -> Result<(), String>;
}

/// A sink that drops everything (tests, ephemeral sessions).
pub struct NullSink;

impl TranscriptSink for NullSink {
    fn append(&self, _items: &[Item]) -> Result<(), String> {
        Ok(())
    }
}

pub struct AgentDeps {
    pub model: Arc<dyn ModelClient>,
    pub tools: Arc<dyn ToolRuntime>,
    pub store: Arc<dyn TranscriptSink>,
}

/// Fields of [`AgentSessionConfig`]/[`AgentDeps`] to replace; applied before
/// the next model request.
#[derive(Default)]
pub struct AgentConfigPatch {
    pub model: Option<Arc<dyn ModelClient>>,
    pub tools: Option<Arc<dyn ToolRuntime>>,
    pub approval: Option<ApprovalSettings>,
    pub targets: Option<Vec<TargetInfo>>,
    pub web_search: Option<bool>,
    pub terminal_available: Option<bool>,
}

pub enum AgentCommand {
    Submit {
        text: String,
        attachments: Vec<ChatAttachment>,
        /// Targets mentioned with `@name`.
        targets: Vec<String>,
        /// Rendered `<context>` block (terminal selection/tail, cwd).
        context: Option<String>,
    },
    Decide {
        call_id: String,
        decision: ApprovalDecision,
    },
    Answer {
        call_id: String,
        answer: String,
    },
    /// Stop the current step, keep finished results, cancel the rest.
    Interrupt,
    /// Extra user instruction injected before the next model request.
    Steer {
        text: String,
    },
    /// Summarize older history now.
    Compact,
    UpdateConfig(AgentConfigPatch),
    /// Stop the session task.
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approve,
    /// Approve and auto-approve calls with the same grant prefix on the same
    /// target for the rest of this session.
    ApproveAndGrant,
    /// Approve with an edited command/patch.
    ApproveEdited {
        edited: String,
    },
    Deny {
        reason: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub call_id: String,
    pub tool: String,
    pub target: String,
    pub risk: AgentRiskLevel,
    pub reason: String,
    pub summary: String,
    pub editable: Option<String>,
    /// Shown as "auto-approve `<prefix>` on `@target` for this session".
    pub grant_prefix: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemHint {
    AssistantText,
    Reasoning,
    ToolCall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnEndReason {
    Completed,
    Interrupted,
    LimitReached,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionStatus {
    Idle,
    Streaming,
    RunningTools,
    WaitingForUser,
    Compacting,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
    ItemStarted {
        item_id: String,
        hint: ItemHint,
        /// Tool name for `ToolCall`.
        name: Option<String>,
    },
    TextDelta {
        item_id: String,
        delta: String,
    },
    ReasoningDelta {
        item_id: String,
        delta: String,
    },
    ToolArgumentsDelta {
        item_id: String,
        delta: String,
    },
    /// A streamed item was abandoned (retry after a transient error).
    ItemDiscarded {
        item_id: String,
    },
    /// Persisted item; the UI should replace any streaming placeholder with
    /// the same id.
    ItemCommitted(Item),
    ApprovalRequested(ApprovalRequest),
    QuestionAsked {
        call_id: String,
        question: String,
        options: Vec<String>,
    },
    ToolProgress {
        call_id: String,
        text: String,
    },
    Usage {
        input: u64,
        output: u64,
        cached: u64,
        context_used: u64,
        context_window: u64,
    },
    TurnFinished {
        turn_id: String,
        reason: TurnEndReason,
    },
    Status(SessionStatus),
    /// Transient information (retrying, compacting) that is not persisted.
    Info {
        level: NoticeLevel,
        text: String,
    },
}

/// Handle to a running session.
#[derive(Clone)]
pub struct AgentHandle {
    commands: mpsc::UnboundedSender<AgentCommand>,
}

impl AgentHandle {
    /// Returns false when the session task has stopped.
    pub fn send(&self, command: AgentCommand) -> bool {
        self.commands.send(command).is_ok()
    }

    pub fn is_closed(&self) -> bool {
        self.commands.is_closed()
    }
}

pub(crate) fn context_window(capabilities: &ModelCapabilities) -> u64 {
    capabilities.context_window.max(4_096)
}

/// Ask the model for a short conversation title (in the user's language).
pub async fn generate_title(
    model: &dyn ModelClient,
    first_message: &str,
) -> crate::error::AgentResult<String> {
    use futures::StreamExt;

    let excerpt: String = first_message.chars().take(1000).collect();
    let request = crate::model::ModelRequest {
        system: "Write a short title (at most 20 characters) for a conversation that starts with \
the user's message. Use the user's language. Reply with the title only, no quotes."
            .into(),
        messages: vec![crate::model::WireMessage::User(vec![
            crate::model::UserPart::Text(excerpt),
        ])],
        tools: Vec::new(),
    };
    let mut stream = model.stream(request).await?;
    let mut title = String::new();
    while let Some(event) = stream.next().await {
        if let crate::model::ModelEvent::TextDelta(delta) = event? {
            title.push_str(&delta);
        }
    }
    let title = title
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .trim()
        .trim_matches(['"', '\'', '“', '”', '《', '》', '#', '*'])
        .trim();
    Ok(title.chars().take(40).collect())
}
