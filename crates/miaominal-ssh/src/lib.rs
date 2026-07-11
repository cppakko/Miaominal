#[path = "ssh/session.rs"]
mod session;

#[path = "ssh/auth.rs"]
mod auth;

#[path = "ssh/forwarding.rs"]
mod forwarding;

#[path = "ssh/monitor.rs"]
mod monitor;

pub use auth::{authenticate, hydrate_profile_from_secrets, list_local_agent_identities};
pub use forwarding::start_port_forward_session;
pub use miaominal_core::forwarding::{
    AgentIdentitySummary, HostKeyDecision, HostKeyPrompt, KbiChallenge, SessionMonitorSnapshot,
};
#[allow(unused_imports)]
pub use session::SessionConnection;
pub use session::{
    SessionCommandSender, SessionEvent, SessionEventReceiver, connection, execute_profile_command,
    execute_profile_pty_command, start_session,
};
