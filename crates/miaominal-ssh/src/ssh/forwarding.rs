use super::session::{
    CancelToken, ClientHandler, ConnectedSession, ConnectionCancelled, SessionCommand,
    SessionCommandSender, SessionConnection, SessionEvent, SessionEventSender,
    connect_authenticated_session_internal, session_event_channel,
};
use anyhow::{Context, Result, anyhow, bail};
use miaominal_core::profile::{PortForwardKind, PortForwardRule, SessionProfile};
use miaominal_core::proxy::ProxyProfile;
use miaominal_secrets::SecretStore;
use miaominal_storage::KnownHostsStore;
use russh::client;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::copy_bidirectional;
use tokio::net::TcpListener;
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};

const PORT_FORWARD_HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const PORT_FORWARD_MAX_RECONNECT_ATTEMPTS: u32 = 10;
const PORT_FORWARD_RECONNECT_DELAYS_SECS: &[u64] = &[1, 2, 4, 8, 16, 30];

/// A forwarding rule that became active: a local listener is bound, or the
/// server accepted a remote listening port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortForwardBinding {
    /// Identifies one local listener lifetime; zero for remote forwards.
    pub binding_id: u64,
    pub rule_id: String,
    pub label: String,
    pub kind: PortForwardKind,
    pub listen_host: String,
    /// Actual bound port; the requested port unless the server assigned one.
    pub bound_port: u16,
}

/// Typed forwarding failure so hosts never parse English log strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortForwardFailureKind {
    /// The local listener could not bind (for example the port is in use).
    LocalBind,
    /// The server refused to bind the remote listening port.
    RemoteBind,
    /// A forwarded connection could not reach its target.
    TargetUnreachable,
    /// The SSH route failed (authentication, transport, or channel setup).
    Transport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortForwardFailure {
    pub rule_id: String,
    pub label: String,
    pub kind: PortForwardFailureKind,
    pub message: String,
}

#[derive(Debug, Clone)]
pub(super) struct RemoteForwardTarget {
    pub rule_id: String,
    pub label: String,
    pub target_host: String,
    pub target_port: u16,
}

pub(super) type RemoteForwardTargets = Arc<Mutex<HashMap<(String, u16), RemoteForwardTarget>>>;

#[derive(Debug, Clone)]
pub(super) struct ActiveRemoteForward {
    pub rule: PortForwardRule,
    pub label: String,
    pub listen_host: String,
    pub bound_port: u16,
}

pub(super) struct ActiveLocalForward {
    pub bound_port: u16,
    pub rule: PortForwardRule,
    pub task: JoinHandle<Result<()>>,
}

impl Drop for ActiveLocalForward {
    fn drop(&mut self) {
        // A session can be cancelled while the whole forwarding future is being
        // dropped (CancelToken), in which case `sync_port_forward_rules` never
        // runs: the listener task must not outlive the session and keep the
        // port bound.
        self.task.abort();
    }
}

#[derive(Debug, Default)]
pub(super) struct PortForwardSyncReport {
    start_failures: HashMap<String, String>,
}

impl PortForwardSyncReport {
    fn record_start_failure(&mut self, rule_id: String, error: impl Into<String>) {
        self.start_failures.insert(rule_id, error.into());
    }

    fn start_failure(&self, rule_id: &str) -> Option<&str> {
        self.start_failures.get(rule_id).map(String::as_str)
    }
}

pub(super) async fn emit_port_forward_notice(
    event_sender: &SessionEventSender,
    message: impl Into<String>,
) {
    let _ = event_sender
        .send(SessionEvent::PortForwardNotice(message.into()))
        .await;
}

pub(super) async fn emit_port_forward_active(
    event_sender: &SessionEventSender,
    binding: PortForwardBinding,
) {
    let _ = event_sender
        .send(SessionEvent::PortForwardActive(binding))
        .await;
}

pub(super) async fn emit_port_forward_failure(
    event_sender: &SessionEventSender,
    failure: PortForwardFailure,
) {
    let _ = event_sender
        .send(SessionEvent::PortForwardFailed(failure))
        .await;
}

/// Local listener binding policy. Existing forwarding keeps strict binding semantics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LocalForwardBindingPolicy {
    #[default]
    Strict,
    /// For dedicated, runtime-only listeners (one rule per SSH connection). Retries an
    /// unavailable preferred port with port zero and retains the actual port on reconnect.
    /// The listener owns its relays and reports `SessionEvent::PortForwardConnections`.
    PreferAvailable,
}

impl LocalForwardBindingPolicy {
    fn is_dedicated(self) -> bool {
        self == Self::PreferAvailable
    }
}

/// Bind errors that mean "this particular port cannot be used here" rather than
/// "local listening is impossible". Windows reports ports inside Hyper-V/WinNAT
/// excluded ranges as `PermissionDenied` (WSAEACCES) instead of `AddrInUse`.
fn is_port_unavailable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
    )
}

#[derive(Clone, Copy, Debug, Default)]
struct ConnectionCount {
    sequence: u64,
    active: usize,
}
struct ConnectionGuard(watch::Sender<ConnectionCount>);
impl ConnectionGuard {
    fn new(counts: watch::Sender<ConnectionCount>) -> Self {
        counts.send_modify(|state| {
            state.sequence += 1;
            state.active += 1;
        });
        Self(counts)
    }
}
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.send_modify(|state| {
            state.sequence += 1;
            state.active = state.active.saturating_sub(1);
        });
    }
}

async fn start_local_forward(
    session: Arc<client::Handle<ClientHandler>>,
    rule: PortForwardRule,
    event_sender: SessionEventSender,
    policy: LocalForwardBindingPolicy,
) -> Result<ActiveLocalForward> {
    let (listener, bind_address) =
        match bind_local_forward_listener_with_policy(&rule, policy).await {
            Ok(listener) => listener,
            Err(error) => {
                let message = format!("{error:#}");
                emit_port_forward_failure(
                    &event_sender,
                    PortForwardFailure {
                        rule_id: rule.id.clone(),
                        label: rule.label.clone(),
                        kind: PortForwardFailureKind::LocalBind,
                        message: message.clone(),
                    },
                )
                .await;
                return Err(anyhow!(message));
            }
        };
    let local_address = listener.local_addr()?;
    let bound_port = local_address.port();
    static NEXT_BINDING: AtomicU64 = AtomicU64::new(1);
    let binding_id = NEXT_BINDING.fetch_add(1, Ordering::Relaxed);
    let listening_on = if bind_address.ends_with(":0") || policy.is_dedicated() {
        local_address.to_string()
    } else {
        bind_address
    };
    emit_port_forward_notice(
        &event_sender,
        format!("Forward {} is listening on {}", rule.label, listening_on),
    )
    .await;
    emit_port_forward_active(
        &event_sender,
        PortForwardBinding {
            binding_id,
            rule_id: rule.id.clone(),
            label: rule.label.clone(),
            kind: PortForwardKind::Local,
            listen_host: rule.listen_host.clone(),
            bound_port,
        },
    )
    .await;
    let active_rule = rule.clone();
    let task = if policy.is_dedicated() {
        tokio::spawn(run_dedicated_local_listener(
            listener,
            session,
            rule,
            event_sender,
            binding_id,
        ))
    } else {
        tokio::spawn(run_shared_local_listener(
            listener,
            session,
            rule,
            event_sender,
        ))
    };
    Ok(ActiveLocalForward {
        rule: active_rule,
        task,
        bound_port,
    })
}

/// Ordinary forwards keep their historical semantics: relays are detached from the
/// listener, so restarting or editing a rule never drops connections already in use,
/// and the listener never waits on the shared session event channel before accepting.
async fn run_shared_local_listener(
    listener: TcpListener,
    session: Arc<client::Handle<ClientHandler>>,
    rule: PortForwardRule,
    event_sender: SessionEventSender,
) -> Result<()> {
    loop {
        let (stream, originator) = match listener.accept().await {
            Ok(connection) => connection,
            Err(error) => {
                let message = format!(
                    "Forward {} stopped accepting connections: {}",
                    rule.label, error
                );
                emit_port_forward_notice(&event_sender, message.clone()).await;
                bail!(message);
            }
        };
        let session = session.clone();
        let event_sender = event_sender.clone();
        let rule = rule.clone();
        tokio::spawn(async move {
            relay_local_connection(session, rule, stream, originator, event_sender, true).await;
        });
    }
}

/// Dedicated listeners (one runtime-only rule per SSH connection, e.g. remote desktop)
/// own their relays and report live connection counts so the owner can reclaim them.
async fn run_dedicated_local_listener(
    listener: TcpListener,
    session: Arc<client::Handle<ClientHandler>>,
    rule: PortForwardRule,
    event_sender: SessionEventSender,
    binding_id: u64,
) -> Result<()> {
    let (counts, mut count_updates) = watch::channel(ConnectionCount::default());
    // Counts are reported from a separate task so accepting never waits on the event
    // channel. The watch channel coalesces intermediate values; only the latest matters.
    let reporter = {
        let event_sender = event_sender.clone();
        let rule_id = rule.id.clone();
        AbortOnDrop(tokio::spawn(async move {
            while count_updates.changed().await.is_ok() {
                let state = *count_updates.borrow_and_update();
                if event_sender
                    .send(SessionEvent::PortForwardConnections {
                        rule_id: rule_id.clone(),
                        binding_id,
                        sequence: state.sequence,
                        active: state.active,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }))
    };
    // JoinSet owns every relay. Cancelling the listener also cancels open channels.
    let mut relays = JoinSet::new();
    let result = loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, originator) = match accepted {
                    Ok(connection) => connection,
                    Err(error) => {
                        let message = format!(
                            "Forward {} stopped accepting connections: {}",
                            rule.label, error
                        );
                        emit_port_forward_notice(&event_sender, message.clone()).await;
                        break Err(anyhow!(message));
                    }
                };
                let guard = ConnectionGuard::new(counts.clone());
                let session = session.clone();
                let event_sender = event_sender.clone();
                let rule = rule.clone();
                relays.spawn(async move {
                    let _guard = guard;
                    relay_local_connection(session, rule, stream, originator, event_sender, false)
                        .await;
                });
            }
            _ = relays.join_next(), if !relays.is_empty() => {}
        }
    };
    drop(reporter);
    result
}

async fn relay_local_connection(
    session: Arc<client::Handle<ClientHandler>>,
    rule: PortForwardRule,
    mut stream: tokio::net::TcpStream,
    originator: std::net::SocketAddr,
    event_sender: SessionEventSender,
    notify: bool,
) {
    let message = match session
        .channel_open_direct_tcpip(
            rule.target_host.clone(),
            u32::from(rule.target_port),
            originator.ip().to_string(),
            u32::from(originator.port()),
        )
        .await
    {
        Ok(channel) => {
            let mut forwarded = channel.into_stream();
            match copy_bidirectional(&mut forwarded, &mut stream).await {
                Ok(_) => return,
                Err(error) => format!("Forward {} relay failed: {}", rule.label, error),
            }
        }
        Err(error) => format!(
            "Forward {} could not open SSH channel: {}",
            rule.label, error
        ),
    };
    emit_port_forward_failure(
        &event_sender,
        PortForwardFailure {
            rule_id: rule.id,
            label: rule.label,
            kind: PortForwardFailureKind::TargetUnreachable,
            message: message.clone(),
        },
    )
    .await;
    if notify {
        emit_port_forward_notice(&event_sender, message).await;
    }
}

struct AbortOnDrop(JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn bind_local_forward_listener_with_policy(
    rule: &PortForwardRule,
    policy: LocalForwardBindingPolicy,
) -> Result<(TcpListener, String)> {
    match bind_local_forward_listener(rule).await {
        Ok(result) => Ok(result),
        Err(error)
            if policy == LocalForwardBindingPolicy::PreferAvailable
                && rule.listen_port != 0
                && error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(is_port_unavailable) =>
        {
            let mut automatic = rule.clone();
            automatic.listen_port = 0;
            bind_local_forward_listener(&automatic).await
        }
        Err(error) => Err(error),
    }
}

async fn bind_local_forward_listener(rule: &PortForwardRule) -> Result<(TcpListener, String)> {
    let bind_address = format!("{}:{}", rule.listen_host, rule.listen_port);
    let listener = TcpListener::bind(&bind_address)
        .await
        .with_context(|| format!("Forward {} failed to bind on {}", rule.label, bind_address))?;
    Ok((listener, bind_address))
}

pub(super) async fn sync_port_forward_rules(
    session: &Arc<client::Handle<ClientHandler>>,
    desired_rules: &[PortForwardRule],
    active_local_forwards: &mut HashMap<String, ActiveLocalForward>,
    active_remote_forwards: &mut HashMap<String, ActiveRemoteForward>,
    remote_forward_targets: &RemoteForwardTargets,
    event_sender: &SessionEventSender,
) -> PortForwardSyncReport {
    sync_port_forward_rules_with_policy(
        session,
        desired_rules,
        active_local_forwards,
        active_remote_forwards,
        remote_forward_targets,
        event_sender,
        LocalForwardBindingPolicy::Strict,
    )
    .await
}

pub(super) async fn sync_port_forward_rules_with_policy(
    session: &Arc<client::Handle<ClientHandler>>,
    desired_rules: &[PortForwardRule],
    active_local_forwards: &mut HashMap<String, ActiveLocalForward>,
    active_remote_forwards: &mut HashMap<String, ActiveRemoteForward>,
    remote_forward_targets: &RemoteForwardTargets,
    event_sender: &SessionEventSender,
    policy: LocalForwardBindingPolicy,
) -> PortForwardSyncReport {
    let mut report = PortForwardSyncReport::default();
    let desired_local: HashMap<_, _> = desired_rules
        .iter()
        .filter(|rule| rule.enabled && rule.kind == PortForwardKind::Local)
        .map(|rule| (rule.id.clone(), rule.clone()))
        .collect();
    let desired_remote: HashMap<_, _> = desired_rules
        .iter()
        .filter(|rule| rule.enabled && rule.kind == PortForwardKind::Remote)
        .map(|rule| (rule.id.clone(), rule.clone()))
        .collect();

    let local_to_stop: Vec<_> = active_local_forwards
        .iter()
        .filter_map(|(rule_id, active)| match desired_local.get(rule_id) {
            Some(desired_rule) if active.rule == *desired_rule => None,
            Some(_) | None => Some(rule_id.clone()),
        })
        .collect();
    for rule_id in local_to_stop {
        if let Some(mut active) = active_local_forwards.remove(&rule_id) {
            active.task.abort();
            let _ = (&mut active.task).await;
            emit_port_forward_notice(event_sender, format!("Stopped local forward {}", rule_id))
                .await;
        }
    }

    let remote_to_stop: Vec<_> = active_remote_forwards
        .iter()
        .filter_map(|(rule_id, active)| match desired_remote.get(rule_id) {
            Some(desired_rule) if active.rule == *desired_rule => None,
            Some(_) | None => Some(rule_id.clone()),
        })
        .collect();
    for rule_id in remote_to_stop {
        if let Some(active) = active_remote_forwards.remove(&rule_id) {
            match session
                .cancel_tcpip_forward(active.listen_host.clone(), u32::from(active.bound_port))
                .await
            {
                Ok(()) => {
                    emit_port_forward_notice(
                        event_sender,
                        format!("Stopped remote forward {}", active.label),
                    )
                    .await;
                }
                Err(error) => {
                    emit_port_forward_notice(
                        event_sender,
                        format!("Failed to stop remote forward {}: {}", active.label, error),
                    )
                    .await;
                }
            }
            if let Ok(mut targets) = remote_forward_targets.lock() {
                targets.remove(&(active.listen_host, active.bound_port));
            }
        }
    }

    for (rule_id, rule) in desired_local {
        if active_local_forwards.contains_key(&rule_id) {
            continue;
        }

        match start_local_forward(session.clone(), rule.clone(), event_sender.clone(), policy).await
        {
            Ok(active) => {
                active_local_forwards.insert(rule_id, active);
            }
            Err(error) => {
                let message = format!("{error:#}");
                emit_port_forward_notice(event_sender, message.clone()).await;
                report.record_start_failure(rule_id, message);
            }
        }
    }

    for (rule_id, rule) in desired_remote {
        if active_remote_forwards.contains_key(&rule_id) {
            continue;
        }

        match session
            .tcpip_forward(rule.listen_host.clone(), u32::from(rule.listen_port))
            .await
        {
            Ok(bound_port) => {
                let Ok(bound_port) = u16::try_from(bound_port) else {
                    let message = format!(
                        "Remote forward {} returned unsupported port {}",
                        rule.label, bound_port
                    );
                    emit_port_forward_failure(
                        event_sender,
                        PortForwardFailure {
                            rule_id: rule_id.clone(),
                            label: rule.label.clone(),
                            kind: PortForwardFailureKind::RemoteBind,
                            message: message.clone(),
                        },
                    )
                    .await;
                    emit_port_forward_notice(event_sender, message.clone()).await;
                    report.record_start_failure(rule_id, message);
                    continue;
                };

                let target_registered = remote_forward_targets.lock().map(|mut targets| {
                    targets.insert(
                        (rule.listen_host.clone(), bound_port),
                        RemoteForwardTarget {
                            rule_id: rule.id.clone(),
                            label: rule.label.clone(),
                            target_host: rule.target_host.clone(),
                            target_port: rule.target_port,
                        },
                    );
                });
                if target_registered.is_err() {
                    let message = format!(
                        "Remote forward {} target registry is unavailable",
                        rule.label
                    );
                    let _ = session
                        .cancel_tcpip_forward(rule.listen_host.clone(), u32::from(bound_port))
                        .await;
                    emit_port_forward_notice(event_sender, message.clone()).await;
                    report.record_start_failure(rule_id, message);
                    continue;
                }
                active_remote_forwards.insert(
                    rule_id,
                    ActiveRemoteForward {
                        rule: rule.clone(),
                        label: rule.label.clone(),
                        listen_host: rule.listen_host.clone(),
                        bound_port,
                    },
                );
                emit_port_forward_notice(
                    event_sender,
                    format!(
                        "Remote forward {} is listening on {}:{}",
                        rule.label, rule.listen_host, bound_port
                    ),
                )
                .await;
                emit_port_forward_active(
                    event_sender,
                    PortForwardBinding {
                        binding_id: 0,
                        rule_id: rule.id.clone(),
                        label: rule.label.clone(),
                        kind: PortForwardKind::Remote,
                        listen_host: rule.listen_host.clone(),
                        bound_port,
                    },
                )
                .await;
            }
            Err(error) => {
                let message = format!("Failed to start remote forward {}: {}", rule.label, error);
                emit_port_forward_failure(
                    event_sender,
                    PortForwardFailure {
                        rule_id: rule_id.clone(),
                        label: rule.label.clone(),
                        kind: PortForwardFailureKind::RemoteBind,
                        message: message.clone(),
                    },
                )
                .await;
                emit_port_forward_notice(event_sender, message.clone()).await;
                report.record_start_failure(rule_id, message);
            }
        }
    }

    report
}

pub fn start_port_forward_session(
    runtime: &TokioHandle,
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
) -> SessionConnection {
    start_port_forward_session_with_policy(
        runtime,
        profile,
        all_profiles,
        all_proxies,
        secrets,
        known_hosts,
        LocalForwardBindingPolicy::Strict,
    )
}

pub fn start_port_forward_session_with_policy(
    runtime: &TokioHandle,
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    policy: LocalForwardBindingPolicy,
) -> SessionConnection {
    let (event_sender, event_receiver) = session_event_channel();
    let (command_sender, command_receiver) = unbounded_channel();
    let runtime = runtime.clone();
    let cancel = CancelToken::default();
    let worker_cancel = cancel.clone();

    let thread = std::thread::Builder::new()
        .name(format!("ssh-forward-{}", profile.id))
        .spawn(move || {
            runtime.block_on(async move {
                let result = tokio::select! {
                    result = run_port_forward_session(
                        profile,
                        all_profiles,
                        all_proxies,
                        secrets,
                        known_hosts,
                        command_receiver,
                        event_sender.clone(),
                        policy,
                    ) => result,
                    () = worker_cancel.cancelled() => Err(ConnectionCancelled.into()),
                };

                match result {
                    Ok(()) => {}
                    Err(error) if error.downcast_ref::<ConnectionCancelled>().is_some() => {
                        let _ = event_sender.send(SessionEvent::Closed).await;
                    }
                    Err(error) => {
                        if event_sender
                            .send(SessionEvent::Error(error.to_string()))
                            .await
                            .is_err()
                        {
                            return;
                        }

                        let _ = event_sender.send(SessionEvent::Closed).await;
                    }
                }
            });
        })
        .expect("failed to spawn SSH port forwarding thread");

    SessionConnection::new(
        SessionCommandSender::new(command_sender, cancel),
        event_receiver,
        Some(thread),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReconnectWaitOutcome {
    Retry,
    Closed,
}

#[derive(Debug)]
enum ConnectedForwardOutcome {
    Closed,
    Lost(String),
    Fatal(anyhow::Error),
}

fn reconnect_delay_secs(attempt: u32) -> u64 {
    PORT_FORWARD_RECONNECT_DELAYS_SECS
        .get(attempt.saturating_sub(1) as usize)
        .copied()
        .unwrap_or(30)
}

fn next_reconnect_attempt(current_attempt: u32) -> Option<(u32, u64)> {
    let attempt = current_attempt.checked_add(1)?;
    (attempt <= PORT_FORWARD_MAX_RECONNECT_ATTEMPTS)
        .then(|| (attempt, reconnect_delay_secs(attempt)))
}

fn connection_was_cancelled(error: &anyhow::Error) -> bool {
    error.downcast_ref::<ConnectionCancelled>().is_some()
}

fn ensure_dedicated_forward_active(
    configured_rules: &[PortForwardRule],
    active_local_forwards: &HashMap<String, ActiveLocalForward>,
    active_remote_forwards: &HashMap<String, ActiveRemoteForward>,
    report: &PortForwardSyncReport,
) -> Result<()> {
    let mut enabled_rules = configured_rules.iter().filter(|rule| rule.enabled);
    let rule = enabled_rules
        .next()
        .ok_or_else(|| anyhow!("dedicated port forwarding session has no enabled rule"))?;
    if enabled_rules.next().is_some() {
        bail!("dedicated port forwarding session has more than one enabled rule");
    }

    let active = match rule.kind {
        PortForwardKind::Local => active_local_forwards.contains_key(&rule.id),
        PortForwardKind::Remote => active_remote_forwards.contains_key(&rule.id),
    };
    if active {
        return Ok(());
    }

    if let Some(error) = report.start_failure(&rule.id) {
        bail!("failed to start port forward {}: {}", rule.label, error);
    }
    bail!("port forward {} did not become active", rule.label)
}

async fn wait_for_reconnect_delay(
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    configured_rules: &mut Vec<PortForwardRule>,
    event_sender: &SessionEventSender,
    error: String,
    attempt: u32,
    retry_after_secs: u64,
    delay: Duration,
) -> ReconnectWaitOutcome {
    if event_sender
        .send(SessionEvent::PortForwardReconnecting {
            error,
            attempt,
            max_attempts: PORT_FORWARD_MAX_RECONNECT_ATTEMPTS,
            retry_after_secs,
        })
        .await
        .is_err()
    {
        return ReconnectWaitOutcome::Closed;
    }

    let delay = tokio::time::sleep(delay);
    tokio::pin!(delay);
    loop {
        tokio::select! {
            _ = &mut delay => return ReconnectWaitOutcome::Retry,
            command = command_receiver.recv() => {
                match command {
                    Some(SessionCommand::SyncPortForwardRules(rules)) => {
                        *configured_rules = rules;
                    }
                    Some(SessionCommand::Close) | None => {
                        return ReconnectWaitOutcome::Closed;
                    }
                    Some(
                        SessionCommand::HostKeyDecision { .. }
                        | SessionCommand::KeyboardInteractiveResponse { .. }
                        | SessionCommand::SetMonitoringEnabled(_)
                        | SessionCommand::Send(_)
                        | SessionCommand::Resize { .. },
                    ) => {}
                }
            }
        }
    }
}

async fn supervise_connected_forward(
    session: &Arc<client::Handle<ClientHandler>>,
    configured_rules: &mut Vec<PortForwardRule>,
    active_local_forwards: &mut HashMap<String, ActiveLocalForward>,
    active_remote_forwards: &mut HashMap<String, ActiveRemoteForward>,
    remote_forward_targets: &RemoteForwardTargets,
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    event_sender: &SessionEventSender,
    policy: LocalForwardBindingPolicy,
) -> ConnectedForwardOutcome {
    let mut health_check = tokio::time::interval(PORT_FORWARD_HEALTH_CHECK_INTERVAL);
    health_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            command = command_receiver.recv() => {
                match command {
                    Some(SessionCommand::SyncPortForwardRules(rules)) => {
                        *configured_rules = rules;
                        let report = sync_port_forward_rules_with_policy(
                            session,
                            configured_rules,
                            active_local_forwards,
                            active_remote_forwards,
                            remote_forward_targets,
                            event_sender,
                            policy,
                        )
                        .await;
                        if let Err(error) = ensure_dedicated_forward_active(
                            configured_rules,
                            active_local_forwards,
                            active_remote_forwards,
                            &report,
                        ) {
                            return ConnectedForwardOutcome::Fatal(error);
                        }
                    }
                    Some(SessionCommand::Close) | None => {
                        return ConnectedForwardOutcome::Closed;
                    }
                    Some(
                        SessionCommand::HostKeyDecision { .. }
                        | SessionCommand::KeyboardInteractiveResponse { .. }
                        | SessionCommand::SetMonitoringEnabled(_)
                        | SessionCommand::Send(_)
                        | SessionCommand::Resize { .. },
                    ) => {}
                }
            }
            _ = health_check.tick() => {
                if session.is_closed() {
                    return ConnectedForwardOutcome::Lost(
                        "SSH transport closed while port forwarding was active".into(),
                    );
                }

                let failed_rule_id = active_local_forwards
                    .iter()
                    .find_map(|(rule_id, active)| active.task.is_finished().then(|| rule_id.clone()));
                if let Some(rule_id) = failed_rule_id
                    && let Some(mut active) = active_local_forwards.remove(&rule_id)
                {
                    let label = active.rule.label.clone();
                    let error = match (&mut active.task).await {
                        Ok(Ok(())) => format!("Forward {label} listener stopped unexpectedly"),
                        Ok(Err(error)) => format!("{error:#}"),
                        Err(error) => format!("Forward {label} listener task failed: {error}"),
                    };
                    return ConnectedForwardOutcome::Lost(error);
                }
            }
        }
    }
}

async fn stop_active_forwards(
    route: &ConnectedSession,
    active_local_forwards: &mut HashMap<String, ActiveLocalForward>,
    active_remote_forwards: &mut HashMap<String, ActiveRemoteForward>,
    event_sender: &SessionEventSender,
) {
    let _ = sync_port_forward_rules(
        &route.session,
        &[],
        active_local_forwards,
        active_remote_forwards,
        &route.remote_forward_targets,
        event_sender,
    )
    .await;
    route.disconnect().await;
}

async fn run_port_forward_session(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    mut command_receiver: UnboundedReceiver<SessionCommand>,
    event_sender: SessionEventSender,
    policy: LocalForwardBindingPolicy,
) -> Result<()> {
    let remote_label = profile.connection_label();
    let mut configured_port_forward_rules = profile.port_forwarding_rules.clone();
    let mut connected_once = false;
    let mut reconnect_attempt = 0;
    let mut reconnect_error: Option<String> = None;

    loop {
        if let Some(error) = reconnect_error.take() {
            let Some((next_attempt, retry_after_secs)) = next_reconnect_attempt(reconnect_attempt)
            else {
                bail!(
                    "port forwarding reconnect failed after {} attempts: {}",
                    PORT_FORWARD_MAX_RECONNECT_ATTEMPTS,
                    error
                );
            };
            reconnect_attempt = next_attempt;
            if wait_for_reconnect_delay(
                &mut command_receiver,
                &mut configured_port_forward_rules,
                &event_sender,
                error,
                reconnect_attempt,
                retry_after_secs,
                Duration::from_secs(retry_after_secs),
            )
            .await
                == ReconnectWaitOutcome::Closed
            {
                let _ = event_sender.send(SessionEvent::Closed).await;
                return Ok(());
            }
        }

        let mut connection_profile = profile.clone();
        connection_profile.port_forwarding_rules = configured_port_forward_rules.clone();
        let route = match connect_authenticated_session_internal(
            connection_profile,
            all_profiles.clone(),
            all_proxies.clone(),
            secrets.clone(),
            known_hosts.clone(),
            &mut command_receiver,
            &event_sender,
        )
        .await
        {
            Ok(route) => route,
            Err(error) if connection_was_cancelled(&error) => {
                let _ = event_sender.send(SessionEvent::Closed).await;
                return Ok(());
            }
            Err(error) if connected_once => {
                reconnect_error = Some(format!("{error:#}"));
                continue;
            }
            Err(error) => return Err(error),
        };

        configured_port_forward_rules = route.configured_port_forward_rules.clone();
        let mut active_local_forwards = HashMap::new();
        let mut active_remote_forwards = HashMap::new();
        let report = sync_port_forward_rules_with_policy(
            &route.session,
            &configured_port_forward_rules,
            &mut active_local_forwards,
            &mut active_remote_forwards,
            &route.remote_forward_targets,
            &event_sender,
            policy,
        )
        .await;
        if policy == LocalForwardBindingPolicy::PreferAvailable {
            for rule in &mut configured_port_forward_rules {
                if let Some(active) = active_local_forwards.get_mut(&rule.id) {
                    rule.listen_port = active.bound_port;
                    active.rule.listen_port = active.bound_port;
                }
            }
        }
        if let Err(error) = ensure_dedicated_forward_active(
            &configured_port_forward_rules,
            &active_local_forwards,
            &active_remote_forwards,
            &report,
        ) {
            stop_active_forwards(
                &route,
                &mut active_local_forwards,
                &mut active_remote_forwards,
                &event_sender,
            )
            .await;
            return Err(error);
        }

        if route.session.is_closed() {
            stop_active_forwards(
                &route,
                &mut active_local_forwards,
                &mut active_remote_forwards,
                &event_sender,
            )
            .await;
            let error = "SSH transport closed before port forwarding became ready".to_string();
            if connected_once {
                reconnect_error = Some(error);
                continue;
            }
            bail!(error);
        }

        if event_sender
            .send(SessionEvent::Connected(remote_label.clone()))
            .await
            .is_err()
        {
            stop_active_forwards(
                &route,
                &mut active_local_forwards,
                &mut active_remote_forwards,
                &event_sender,
            )
            .await;
            return Ok(());
        }
        connected_once = true;
        reconnect_attempt = 0;

        let outcome = supervise_connected_forward(
            &route.session,
            &mut configured_port_forward_rules,
            &mut active_local_forwards,
            &mut active_remote_forwards,
            &route.remote_forward_targets,
            &mut command_receiver,
            &event_sender,
            policy,
        )
        .await;
        stop_active_forwards(
            &route,
            &mut active_local_forwards,
            &mut active_remote_forwards,
            &event_sender,
        )
        .await;

        match outcome {
            ConnectedForwardOutcome::Closed => {
                let _ = event_sender.send(SessionEvent::Closed).await;
                return Ok(());
            }
            ConnectedForwardOutcome::Lost(error) => {
                reconnect_error = Some(error);
            }
            ConnectedForwardOutcome::Fatal(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::profile::PortForwardRule;
    #[tokio::test]
    async fn coalesced_connection_count_keeps_final_zero_after_cancellation() {
        let (counts, mut receiver) = watch::channel(ConnectionCount::default());
        let first = ConnectionGuard::new(counts.clone());
        let second = ConnectionGuard::new(counts.clone());
        let task = tokio::spawn(async move {
            let _guard = second;
            std::future::pending::<()>().await;
        });
        drop(first);
        task.abort();
        let _ = task.await;
        receiver.changed().await.unwrap();
        let state = *receiver.borrow_and_update();
        assert_eq!(state.active, 0);
        assert_eq!(state.sequence, 4);
    }

    fn local_rule(listen_port: u16) -> PortForwardRule {
        PortForwardRule {
            id: "forward-a".to_string(),
            label: "Forward A".to_string(),
            kind: PortForwardKind::Local,
            listen_host: "127.0.0.1".to_string(),
            listen_port,
            target_host: "127.0.0.1".to_string(),
            target_port: 8080,
            enabled: true,
        }
    }

    #[test]
    fn reconnect_delays_follow_bounded_backoff() {
        let delays = (1..=PORT_FORWARD_MAX_RECONNECT_ATTEMPTS)
            .map(reconnect_delay_secs)
            .collect::<Vec<_>>();

        assert_eq!(delays, vec![1, 2, 4, 8, 16, 30, 30, 30, 30, 30]);
        assert_eq!(next_reconnect_attempt(0), Some((1, 1)));
        assert_eq!(next_reconnect_attempt(9), Some((10, 30)));
        assert_eq!(next_reconnect_attempt(10), None);
    }

    #[test]
    fn connection_cancellation_detection_uses_the_error_type() {
        let cancelled = anyhow::Error::new(ConnectionCancelled).context("connect failed");
        assert!(connection_was_cancelled(&cancelled));

        let same_message = anyhow!("connection cancelled");
        assert!(!connection_was_cancelled(&same_message));
    }

    #[test]
    fn dedicated_forward_requires_exactly_one_active_rule() {
        let rule = local_rule(8081);
        let rules = vec![rule.clone()];
        let active_local = HashMap::new();
        let active_remote = HashMap::new();
        let mut report = PortForwardSyncReport::default();
        report.record_start_failure(rule.id.clone(), "address already in use");

        let error = ensure_dedicated_forward_active(&rules, &active_local, &active_remote, &report)
            .expect_err("inactive dedicated rule should fail");

        assert!(error.to_string().contains("address already in use"));
    }

    #[tokio::test]
    async fn occupied_local_port_is_rejected_before_listener_task_starts() {
        let occupied = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let port = occupied
            .local_addr()
            .expect("test listener should have an address")
            .port();
        let rule = local_rule(port);

        let error = bind_local_forward_listener(&rule)
            .await
            .expect_err("occupied forwarding port should not bind");

        assert!(error.to_string().contains("failed to bind"));
        assert!(error.to_string().contains(&port.to_string()));
    }

    #[tokio::test]
    async fn reconnect_wait_emits_state_and_stops_immediately_on_close() {
        let (event_sender, mut events) = session_event_channel();
        let (commands, mut command_receiver) = unbounded_channel();
        let mut configured_rules = vec![local_rule(8082)];
        commands
            .send(SessionCommand::Close)
            .expect("close command should send");

        let outcome = wait_for_reconnect_delay(
            &mut command_receiver,
            &mut configured_rules,
            &event_sender,
            "transport closed".to_string(),
            2,
            4,
            Duration::from_secs(60),
        )
        .await;

        assert_eq!(outcome, ReconnectWaitOutcome::Closed);
        assert!(matches!(
            events.recv().await,
            Some(SessionEvent::PortForwardReconnecting {
                attempt: 2,
                max_attempts: PORT_FORWARD_MAX_RECONNECT_ATTEMPTS,
                retry_after_secs: 4,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn reconnect_wait_keeps_latest_synced_rule() {
        let (event_sender, _events) = session_event_channel();
        let (commands, mut command_receiver) = unbounded_channel();
        let mut configured_rules = vec![local_rule(8083)];
        let updated = local_rule(8084);
        commands
            .send(SessionCommand::SyncPortForwardRules(vec![updated.clone()]))
            .expect("sync command should send");
        commands
            .send(SessionCommand::Close)
            .expect("close command should send");

        let outcome = wait_for_reconnect_delay(
            &mut command_receiver,
            &mut configured_rules,
            &event_sender,
            "transport closed".to_string(),
            1,
            1,
            Duration::from_secs(60),
        )
        .await;

        assert_eq!(outcome, ReconnectWaitOutcome::Closed);
        assert_eq!(configured_rules, vec![updated]);
    }
}
