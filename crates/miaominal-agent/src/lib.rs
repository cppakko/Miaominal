mod backend;
mod capabilities;
mod channel;
mod chat;
mod error;
mod jobs;
mod path_guard;
mod policy;
mod runtime;
mod tools;
mod web;

pub use backend::{BackendRoute, BackendRouter, ExecMode, SshBackend, SshExecRequest};
pub use capabilities::{CapabilityProbe, CapabilityProbeResult, RemoteCapabilities};
pub use channel::{
    AgentExecChannel, AgentToolCallRequest, AgentToolCallResponse,
    ShellCommandResult, TerminalExecHandle, ToolOutput,
};
pub use chat::{
    AgentChatEvent, AgentChatMessage, AgentMode, AgentChatProvider, AgentChatProviderKind,
    AgentChatRequest, AgentChatRole, AgentChatToolEvent, AgentToolResultContinuationRequest,
    generate_title, send_chat, stream_chat, stream_chat_after_tool_result,
};
pub use error::{AgentError, AgentResult};
pub use runtime::agent_runtime;
pub use jobs::{AgentJobId, AgentJobRegistry, AgentJobSummary, JobPollResult, JobStatus};
pub use policy::{AgentPolicy, AgentPolicyDecision};
pub use tools::{AgentToolSet, ListEntry, ListEntryType, TOOL_NAMES};
pub use web::{
    ConfiguredWebSearchProvider, DisabledWebSearchProvider, WebFetchConfig, WebSearchProvider,
};
