//! Interface between the agent engine and tool execution.
//!
//! The engine owns the loop, approvals and engine tools (`ask_user`,
//! `update_plan`). Everything that touches a target goes through a
//! [`ToolRuntime`].

use futures::future::BoxFuture;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::model::ToolSpec;
use crate::policy::AgentRiskLevel;
use crate::tools::AgentToolCancellation;
use crate::transcript::{ToolDisplay, ToolResultStatus};

use super::ToolClass;

/// One validated tool call requested by the model.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolInvocation {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}

/// Risk assessment of an invocation, computed before execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolAssessment {
    pub class: ToolClass,
    /// Execution target name the call runs on (empty for local/engine).
    pub target: String,
    pub risk: AgentRiskLevel,
    /// One-line reason shown on the approval card / denial.
    pub reason: String,
    /// Prefix used for session grants (`systemctl restart`).
    pub grant_prefix: Option<String>,
    /// Short human summary (`shell @prod: systemctl restart nginx`).
    pub summary: String,
    /// Editable payload for "edit then approve" (command text or patch).
    pub editable: Option<String>,
}

/// Final result of an invocation.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolOutcome {
    pub status: ToolResultStatus,
    /// Shaped text the model sees.
    pub model_text: String,
    /// Structured payload for the UI.
    pub display: ToolDisplay,
}

impl ToolOutcome {
    pub fn ok(model_text: impl Into<String>, display: ToolDisplay) -> Self {
        Self {
            status: ToolResultStatus::Ok,
            model_text: model_text.into(),
            display,
        }
    }

    pub fn failed(model_text: impl Into<String>) -> Self {
        Self {
            status: ToolResultStatus::Failed,
            model_text: model_text.into(),
            display: ToolDisplay::None,
        }
    }

    pub fn cancelled() -> Self {
        Self {
            status: ToolResultStatus::Cancelled,
            model_text: "Cancelled: the user interrupted this call before it finished.".into(),
            display: ToolDisplay::None,
        }
    }
}

/// Incremental output while a tool runs (for the UI only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolProgress {
    pub call_id: String,
    pub text: String,
}

pub type ProgressSender = mpsc::UnboundedSender<ToolProgress>;

pub trait ToolRuntime: Send + Sync {
    /// Tool specs offered to the model for the given options.
    fn specs(&self, options: &super::ToolSetOptions) -> Vec<ToolSpec>;

    /// Assess an invocation (risk, target, grant prefix). Must not have
    /// side effects. An `Err` string becomes a `Failed` result.
    fn assess<'a>(
        &'a self,
        call: &'a ToolInvocation,
    ) -> BoxFuture<'a, Result<ToolAssessment, String>>;

    /// Replace the editable payload (command/patch) after the user edited
    /// it on the approval card.
    fn apply_edit(&self, call: &ToolInvocation, edited: &str) -> Value;

    /// Execute an approved invocation. Implementations must honour
    /// `cancel` and return [`ToolOutcome::cancelled`] when it fires.
    fn execute(
        &self,
        call: ToolInvocation,
        cancel: AgentToolCancellation,
        progress: ProgressSender,
    ) -> BoxFuture<'static, ToolOutcome>;

    /// Probe the execution targets (OS, shell, cwd, available tools) for the
    /// `<environment>` block. `None` keeps the configured targets.
    fn environment(&self) -> BoxFuture<'_, Option<Vec<crate::context::TargetInfo>>> {
        Box::pin(async { None })
    }
}
