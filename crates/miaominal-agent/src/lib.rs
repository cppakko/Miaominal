mod backend;
mod capabilities;
mod channel;
mod chat;
mod error;
mod jobs;
mod path_guard;
mod policy;
mod tools;
mod web;

pub use backend::{BackendRoute, BackendRouter, SshExecBackend, SshExecRequest};
pub use capabilities::{CapabilityProbe, CapabilityProbeResult, RemoteCapabilities};
pub use channel::{
    AgentExecChannel, AgentToolCallRequest, AgentToolCallResponse, ShellCommandResult, ToolOutput,
};
pub use chat::{
    AgentChatEvent, AgentChatMessage, AgentChatProvider, AgentChatProviderKind, AgentChatRequest,
    AgentChatRole, AgentChatToolEvent, send_chat, stream_chat,
};
pub use error::{AgentError, AgentResult};
pub use jobs::{AgentJobId, AgentJobRegistry, JobPollResult, JobStatus};
pub use policy::{AgentPolicy, AgentPolicyDecision};
pub use tools::{AgentToolSet, ListEntry, ListEntryType, TOOL_NAMES};
pub use web::{DisabledWebSearchProvider, WebFetchConfig, WebSearchProvider};
