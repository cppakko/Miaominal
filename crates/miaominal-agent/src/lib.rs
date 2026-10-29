//! Miaominal AI assistant backend: engine loop, in-house model clients,
//! tool set v2, policy v2, context construction and the persisted
//! transcript. Pure Rust; no desktop or GPUI dependencies.

mod backend;
mod capabilities;
mod channel;
pub mod context;
pub mod engine;
mod error;
mod jobs;
pub mod model;
mod path_guard;
pub mod policy;
mod runtime;
pub mod toolkit;
mod tools;
pub mod transcript;
mod web;

pub use backend::{BackendRoute, BackendRouter, ExecMode, SshBackend, SshExecRequest};
pub use capabilities::{CapabilityProbe, CapabilityProbeResult, RemoteCapabilities};
pub use channel::{
    AgentExecChannel, AgentShellRegistry, AgentToolCallRequest, AgentToolCallResponse,
    ShellCommandResult, TERMINAL_INTERRUPT_SETTLE_TIMEOUT, TerminalExecHandle,
    TerminalExecLeaseState, TerminalOutputReceiver, TerminalOutputTap, TerminalOutputTapError,
    ToolOutput,
};
pub use error::{AgentError, AgentResult, ModelError};
pub use jobs::{AgentJobId, AgentJobRegistry, AgentJobSummary, JobPollResult, JobStatus};
pub use model::{AgentReasoningSupport, agent_reasoning_support};
pub use policy::{AgentPolicy, AgentPolicyDecision};
pub use runtime::agent_runtime;
pub use tools::{AgentToolCancellation, ListEntry, ListEntryType};
pub use web::{
    ConfiguredWebSearchProvider, DisabledWebSearchProvider, WebFetchConfig, WebSearchProvider,
};
