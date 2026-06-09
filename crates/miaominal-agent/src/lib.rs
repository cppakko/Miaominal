mod backend;
mod capabilities;
mod channel;
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
pub use error::{AgentError, AgentResult};
pub use jobs::{AgentJobId, AgentJobRegistry, JobPollResult, JobStatus};
pub use policy::{AgentPolicy, AgentPolicyDecision};
pub use tools::{AgentToolSet, ListEntry, ListEntryType, TOOL_NAMES};
pub use web::{DisabledWebSearchProvider, WebFetchConfig, WebSearchProvider};
