#[path = "ssh/session.rs"]
mod session;

#[path = "ssh/auth.rs"]
mod auth;

#[path = "ssh/forwarding.rs"]
mod forwarding;

#[path = "ssh/monitor.rs"]
mod monitor;

pub(crate) use crate::domain::forwarding::{
    AgentIdentitySummary, HostKeyDecision, HostKeyPrompt, KbiChallenge, SessionMonitorSnapshot,
};
pub(crate) use auth::{authenticate, hydrate_profile_from_secrets, list_local_agent_identities};
pub(crate) use forwarding::start_port_forward_session;
#[allow(unused_imports)]
pub(crate) use session::SessionConnection;
pub(crate) use session::{
    SessionCommandSender, SessionEvent, connection, execute_profile_command, start_session,
};
