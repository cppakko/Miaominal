use std::fmt;

pub type AgentResult<T> = Result<T, AgentError>;

#[derive(Debug)]
pub enum AgentError {
    ApprovalRequired { tool_name: String },
    Denied { tool_name: String, reason: String },
    InvalidArguments(String),
    InvalidPath(String),
    JobNotFound(String),
    PosixOnly(String),
    ProfileNotFound(String),
    UnsupportedProvider(String),
    UnsupportedRoute(String),
    UnknownTool(String),
    Backend(anyhow::Error),
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApprovalRequired { tool_name } => {
                write!(f, "tool `{tool_name}` requires user approval")
            }
            Self::Denied { tool_name, reason } => {
                write!(f, "tool `{tool_name}` was denied: {reason}")
            }
            Self::InvalidArguments(message) => write!(f, "invalid tool arguments: {message}"),
            Self::InvalidPath(message) => write!(f, "invalid workspace path: {message}"),
            Self::JobNotFound(job_id) => write!(f, "agent job `{job_id}` was not found"),
            Self::PosixOnly(message) => write!(f, "{message}"),
            Self::ProfileNotFound(profile_id) => {
                write!(f, "profile `{profile_id}` was not found")
            }
            Self::UnsupportedProvider(message) => write!(f, "{message}"),
            Self::UnsupportedRoute(route) => write!(f, "backend route `{route}` is not supported"),
            Self::UnknownTool(tool_name) => write!(f, "unknown agent tool `{tool_name}`"),
            Self::Backend(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for AgentError {}

impl From<anyhow::Error> for AgentError {
    fn from(error: anyhow::Error) -> Self {
        Self::Backend(error)
    }
}
