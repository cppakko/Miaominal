#[path = "ssh/session.rs"]
mod session;

#[path = "ssh/auth.rs"]
mod auth;

#[path = "ssh/forwarding.rs"]
mod forwarding;

#[path = "ssh/monitor.rs"]
mod monitor;

#[path = "ssh/transport.rs"]
pub mod transport;

#[path = "ssh/profile_connector.rs"]
mod profile_connector;

#[path = "ssh/bridge.rs"]
pub mod bridge;

#[path = "ssh/bridge_ipc.rs"]
mod bridge_ipc;

#[path = "ssh/bridge_server.rs"]
mod bridge_server;

pub use auth::{authenticate, hydrate_profile_from_secrets, list_local_agent_identities};
pub use bridge::{SshBridgeEndpoint, SshBridgeRoute, SshBridgeStatus, SshBridgeSyncResult};
pub use bridge_ipc::{
    SshBridgeConnection, SshBridgeHelperArgs, SshBridgeListener, SshBridgeRouteRequest,
    SshBridgeRouteResponse, SshBridgeRouteTable, SshBridgeStream, accept_route_request,
    accept_route_request_with, connect_endpoint, parse_ssh_bridge_helper_args, read_control_frame,
    request_route, run_ssh_bridge_helper, write_control_frame,
};
pub use bridge_server::{
    SshBridgeServerIdentity, run_ssh_bridge_server, run_ssh_bridge_server_with_shutdown,
};
pub use forwarding::start_port_forward_session;
pub use miaominal_core::forwarding::{
    AgentIdentitySummary, HostKeyDecision, HostKeyPrompt, KbiChallenge, SessionMonitorSnapshot,
};
pub use profile_connector::{
    BridgeCredentialReadiness, ConnectedSshRoute, ProfileConnector, is_bridge_vault_locked_error,
};
#[allow(unused_imports)]
pub use session::SessionConnection;
pub use session::{
    SessionCommandSender, SessionEvent, SessionEventReceiver, connection, execute_profile_command,
    execute_profile_pty_command, start_session,
};
