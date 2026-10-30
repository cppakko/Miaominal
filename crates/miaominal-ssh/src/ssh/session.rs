use super::auth::connect_local_agent_stream;
use super::forwarding::{
    ActiveLocalForward, ActiveRemoteForward, PortForwardFailure, PortForwardFailureKind,
    RemoteForwardTargets, emit_port_forward_failure, emit_port_forward_notice,
    sync_port_forward_rules,
};
use super::monitor::{run_exec_command, run_exec_pty_command, run_monitor_loop};
use super::profile_connector::{ConnectedSshRoute, ProfileConnector};
use anyhow::{Context, Result, anyhow, bail};
use miaominal_core::forwarding::{
    HostKeyDecision, HostKeyPrompt, KbiChallenge, SessionMonitorSnapshot,
};
use miaominal_core::known_host::HostKeyCheck;
use miaominal_core::profile::{
    AuthMethod, PortForwardRule, SessionEnvironmentVariable, SessionProfile, ShellType,
};
use miaominal_core::proxy::ProxyProfile;
use miaominal_core::terminal::MIN_TERMINAL_COLUMNS;
use miaominal_secrets::SecretStore;
use miaominal_storage::KnownHostsStore;
use russh::keys::{HashAlg, PublicKey};
use russh::{Channel, ChannelMsg, Disconnect, client};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;
use tokio::io::copy_bidirectional;
use tokio::net::TcpStream;
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::mpsc::{
    Receiver, Sender, UnboundedReceiver, UnboundedSender, channel, unbounded_channel,
};
use tokio::sync::oneshot;
use tokio::sync::watch;

#[derive(Debug)]
pub(super) struct ConnectionCancelled;

impl std::fmt::Display for ConnectionCancelled {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("connection cancelled")
    }
}

impl std::error::Error for ConnectionCancelled {}

pub const SESSION_EVENT_QUEUE_CAPACITY: usize = 64;
const SSH_CHANNEL_BUFFER_SIZE: usize = 64;
const SSH_MAXIMUM_PACKET_SIZE: u32 = 32 * 1024;
/// Upper bound for a server reply to a channel request. OpenSSH always answers
/// `want_reply` requests; a server that stays silent must not hang the session.
const CHANNEL_REPLY_TIMEOUT: Duration = Duration::from_secs(15);

/// Ordered connection phases for an interactive session.
///
/// A session can fail or be cancelled from any phase; `Failed` and `Closed`
/// are terminal. Only a successful PTY and shell request move it to `Ready`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    Connecting,
    WaitingForTrust,
    WaitingForAuthentication,
    OpeningShell,
    Ready,
    Closing,
    Closed,
    Failed,
}

/// Identifies which hop of a route a request belongs to (jump hosts included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHop {
    /// Human-readable hop label; the target uses the profile connection label.
    pub label: String,
    /// One-based hop index; the target is the last hop.
    pub index: u32,
    pub total: u32,
}

impl SessionHop {
    pub fn new(label: impl Into<String>, index: u32, total: u32) -> Self {
        Self {
            label: label.into(),
            index,
            total: total.max(1),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionPhaseChange {
    pub phase: SessionPhase,
    /// Hop the phase refers to; `None` before the route is resolved.
    pub hop: Option<SessionHop>,
}

/// A pending host-key decision, bound to one connection attempt and request id.
#[derive(Debug, Clone)]
pub struct HostKeyRequest {
    pub request_id: u64,
    pub hop: SessionHop,
    pub prompt: HostKeyPrompt,
}

/// A pending keyboard-interactive challenge, bound to one request id.
#[derive(Debug, Clone)]
pub struct AuthenticationRequest {
    pub request_id: u64,
    pub hop: SessionHop,
    pub challenge: KbiChallenge,
}

/// Process-wide monotonic request id. Unique within a process is enough to make
/// stale answers detectable per connection.
pub(super) fn next_request_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

struct AbortTaskOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> AbortTaskOnDrop<T> {
    fn new(handle: tokio::task::JoinHandle<T>) -> Self {
        Self(handle)
    }

    fn abort(&self) {
        self.0.abort();
    }
}

impl<T> Drop for AbortTaskOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) type SessionEventSender = Sender<SessionEvent>;

pub struct SessionEventReceiver {
    receiver: Receiver<SessionEvent>,
}

impl SessionEventReceiver {
    fn new(receiver: Receiver<SessionEvent>) -> Self {
        Self { receiver }
    }

    pub async fn recv(&mut self) -> Option<SessionEvent> {
        self.receiver.recv().await
    }

    pub fn try_recv(
        &mut self,
    ) -> std::result::Result<SessionEvent, tokio::sync::mpsc::error::TryRecvError> {
        self.receiver.try_recv()
    }
}

impl From<Receiver<SessionEvent>> for SessionEventReceiver {
    fn from(receiver: Receiver<SessionEvent>) -> Self {
        Self::new(receiver)
    }
}

impl futures::Stream for SessionEventReceiver {
    type Item = SessionEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().receiver.poll_recv(cx)
    }
}

pub(super) fn session_event_channel() -> (SessionEventSender, SessionEventReceiver) {
    let (sender, receiver) = channel(SESSION_EVENT_QUEUE_CAPACITY);
    (sender, SessionEventReceiver::new(receiver))
}

/// Publish one typed event, reporting a closed receiver to the caller.
pub(super) async fn emit_session_event(
    event_sender: &SessionEventSender,
    event: SessionEvent,
) -> Result<()> {
    if event_sender.send(event).await.is_err() {
        bail!("session event receiver is closed");
    }
    Ok(())
}

pub(super) async fn emit_phase(
    event_sender: &SessionEventSender,
    phase: SessionPhase,
    hop: Option<SessionHop>,
) -> Result<()> {
    emit_session_event(
        event_sender,
        SessionEvent::Phase(SessionPhaseChange { phase, hop }),
    )
    .await
}

pub(super) fn clear_pending_host_key(
    pending: &Arc<Mutex<Option<PendingHostKeyDecision>>>,
) -> Result<()> {
    let mut guard = pending
        .lock()
        .map_err(|_| anyhow!("host key decision mutex poisoned"))?;
    *guard = None;
    Ok(())
}

fn fingerprint_of(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

fn algorithm_of(key: &PublicKey) -> String {
    key.algorithm().to_string()
}

pub mod connection {
    use anyhow::{Context, Result, anyhow, bail};
    use miaominal_core::profile::SessionProfile;
    use russh::client;
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::time::Duration;

    pub fn default_client_config() -> Arc<client::Config> {
        Arc::new(client::Config {
            maximum_packet_size: super::SSH_MAXIMUM_PACKET_SIZE,
            channel_buffer_size: super::SSH_CHANNEL_BUFFER_SIZE,
            inactivity_timeout: None,
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 3,
            ..Default::default()
        })
    }

    pub fn resolve_proxy_jump_profiles(
        profile: &SessionProfile,
        all_profiles: &[SessionProfile],
    ) -> Result<Vec<SessionProfile>> {
        let mut seen = HashSet::new();
        let mut resolved = Vec::new();

        for profile_id in &profile.proxy_jump_profile_ids {
            if profile_id == &profile.id {
                bail!("host chaining cannot reference the host being connected");
            }

            if !seen.insert(profile_id.clone()) {
                bail!("host chaining cannot include the same saved host more than once");
            }

            let resolved_profile = all_profiles
                .iter()
                .find(|candidate| candidate.id == *profile_id)
                .cloned()
                .ok_or_else(|| anyhow!("jump host {profile_id} is no longer available"))?;
            resolved.push(resolved_profile);
        }

        Ok(resolved)
    }

    pub async fn connect_profile_session<H>(
        profile: &SessionProfile,
        config: Arc<client::Config>,
        handler: H,
    ) -> Result<client::Handle<H>>
    where
        H: client::Handler<Error = anyhow::Error> + Send + 'static,
    {
        let host = profile.host.clone();
        let port = profile.port;

        let transport = crate::transport::connect_direct(&host, port).await?;
        client::connect_stream(config, transport, handler)
            .await
            .with_context(|| format!("failed to connect to {}:{}", host, port))
    }

    pub async fn connect_profile_stream<H, R>(
        profile: &SessionProfile,
        transport: R,
        config: Arc<client::Config>,
        handler: H,
    ) -> Result<client::Handle<H>>
    where
        H: client::Handler<Error = anyhow::Error> + Send + 'static,
        R: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let host = profile.host.clone();
        let port = profile.port;

        client::connect_stream(config, transport, handler)
            .await
            .with_context(|| format!("failed to connect to {}:{} through transport", host, port))
    }
}

#[derive(Debug)]
pub enum SessionEvent {
    /// Latest accepted TCP connection count for one listener lifetime.
    PortForwardConnections {
        rule_id: String,
        binding_id: u64,
        sequence: u64,
        active: usize,
    },
    Connected(String),
    /// Typed connection phase transition; the interactive flow emits these so
    /// hosts never parse the English status strings.
    Phase(SessionPhaseChange),
    PortForwardReconnecting {
        error: String,
        attempt: u32,
        max_attempts: u32,
        retry_after_secs: u64,
    },
    /// A forwarding rule became active (local listener bound or remote port
    /// accepted by the server).
    PortForwardActive(super::forwarding::PortForwardBinding),
    /// A forwarding rule failed, classified by kind so hosts do not parse logs.
    PortForwardFailed(super::forwarding::PortForwardFailure),
    Output(Vec<u8>),
    Status(String),
    Error(String),
    MonitorUpdated(SessionMonitorSnapshot),
    MonitorFailed(String),
    PortForwardNotice(String),
    HostKeyPrompt(HostKeyRequest),
    KeyboardInteractivePrompt(AuthenticationRequest),
    /// A queued answer arrived after its request was superseded or cancelled.
    RequestExpired {
        request_id: u64,
    },
    Exited {
        exit_code: u32,
        signal: Option<String>,
    },
    Closed,
}

#[derive(Debug)]
pub enum SessionCommand {
    Send(Vec<u8>),
    Resize {
        columns: usize,
        lines: usize,
    },
    SetMonitoringEnabled(bool),
    HostKeyDecision {
        request_id: u64,
        decision: HostKeyDecision,
    },
    KeyboardInteractiveResponse {
        request_id: u64,
        answers: Vec<String>,
    },
    SyncPortForwardRules(Vec<PortForwardRule>),
    Close,
}

/// Cancellation token shared by a session's command sender and its worker
/// thread. `close()` must also interrupt a connection that is still inside a
/// non-cancellable await (TCP connect, SSH handshake, password authentication),
/// which the command channel alone cannot reach.
#[derive(Clone, Debug, Default)]
pub(super) struct CancelToken {
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    notify: Arc<tokio::sync::Notify>,
}

impl CancelToken {
    pub(super) fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.notify.notify_one();
    }

    pub(super) async fn cancelled(&self) {
        loop {
            if self.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            // `notify_one` stores a permit, so a cancel between the check and
            // the await cannot be lost.
            self.notify.notified().await;
        }
    }
}

/// Latest requested geometry survives connection/authentication command routing,
/// which may consume resize commands before the interactive PTY exists.
#[derive(Clone, Debug)]
struct InitialPtySize {
    dimensions: Arc<Mutex<(usize, usize)>>,
}

impl InitialPtySize {
    fn new(columns: usize, lines: usize) -> Self {
        Self {
            dimensions: Arc::new(Mutex::new((
                columns.max(MIN_TERMINAL_COLUMNS),
                lines.max(1),
            ))),
        }
    }

    fn latest(&self) -> Result<(usize, usize)> {
        self.dimensions
            .lock()
            .map(|dimensions| *dimensions)
            .map_err(|_| anyhow!("PTY size mutex poisoned"))
    }
}

#[derive(Clone, Debug)]
pub struct SessionCommandSender {
    sender: UnboundedSender<SessionCommand>,
    cancel: CancelToken,
    initial_pty_size: Option<InitialPtySize>,
}

impl SessionCommandSender {
    pub(super) fn new(sender: UnboundedSender<SessionCommand>, cancel: CancelToken) -> Self {
        Self {
            sender,
            cancel,
            initial_pty_size: None,
        }
    }

    /// Cancellation signal shared with [`SessionCommandSender::close`].
    ///
    /// A host that drives a connect future directly (for example the SFTP
    /// subsystem setup) can race this signal to interrupt stages that never
    /// read the command channel, such as a server that does not answer
    /// authentication or a subsystem request.
    pub fn cancellation(&self) -> SessionCancellation {
        SessionCancellation {
            cancel: self.cancel.clone(),
        }
    }

    pub fn send_bytes(&self, bytes: Vec<u8>) -> Result<()> {
        self.sender
            .send(SessionCommand::Send(bytes))
            .map_err(|_| anyhow!("session is no longer available"))
    }

    pub fn resize(&self, columns: usize, lines: usize) -> Result<()> {
        // Record before enqueueing: connect/trust and keyboard-interactive
        // routers may consume this command before request_pty can use it.
        // Keep the guard through send so cloned senders preserve the same
        // geometry order in both the startup snapshot and the command queue.
        let mut initial_pty_size = match self.initial_pty_size.as_ref() {
            Some(size) => Some(
                size.dimensions
                    .lock()
                    .map_err(|_| anyhow!("PTY size mutex poisoned"))?,
            ),
            None => None,
        };
        if let Some(dimensions) = initial_pty_size.as_mut() {
            **dimensions = (columns.max(MIN_TERMINAL_COLUMNS), lines.max(1));
        }
        self.sender
            .send(SessionCommand::Resize { columns, lines })
            .map_err(|_| anyhow!("session is no longer available"))
    }

    pub fn set_monitoring_enabled(&self, enabled: bool) -> Result<()> {
        self.sender
            .send(SessionCommand::SetMonitoringEnabled(enabled))
            .map_err(|_| anyhow!("session is no longer available"))
    }

    pub fn respond_host_key(&self, request_id: u64, decision: HostKeyDecision) -> Result<()> {
        self.sender
            .send(SessionCommand::HostKeyDecision {
                request_id,
                decision,
            })
            .map_err(|_| anyhow!("session is no longer available"))
    }

    pub fn respond_keyboard_interactive(
        &self,
        request_id: u64,
        responses: Vec<String>,
    ) -> Result<()> {
        self.sender
            .send(SessionCommand::KeyboardInteractiveResponse {
                request_id,
                answers: responses,
            })
            .map_err(|_| anyhow!("session is no longer available"))
    }

    pub fn sync_port_forward_rules(&self, rules: Vec<PortForwardRule>) -> Result<()> {
        self.sender
            .send(SessionCommand::SyncPortForwardRules(rules))
            .map_err(|_| anyhow!("session is no longer available"))
    }

    /// Explicit close: interrupts the connection worker even when it is stuck
    /// in an await that does not poll the command channel, then asks the
    /// running session loop to wind down.
    pub fn close(&self) -> Result<()> {
        self.cancel.cancel();
        self.sender
            .send(SessionCommand::Close)
            .map_err(|_| anyhow!("session is no longer available"))
    }
}

/// Cloneable cancellation signal for one session connection.
#[derive(Clone, Debug, Default)]
pub struct SessionCancellation {
    cancel: CancelToken,
}

impl SessionCancellation {
    /// Resolve when the session is cancelled; returns immediately when it was
    /// already cancelled.
    pub async fn cancelled(&self) {
        self.cancel.cancelled().await;
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel
            .cancelled
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

pub struct SessionConnection {
    pub commands: SessionCommandSender,
    pub events: SessionEventReceiver,
    /// Worker thread for SSH-backed sessions; `None` for host-provided
    /// backends. Callers may join it after `commands.close()` to prove that no
    /// connection task is left behind.
    pub thread: Option<std::thread::JoinHandle<()>>,
}

impl SessionConnection {
    pub(super) fn new(
        commands: SessionCommandSender,
        events: SessionEventReceiver,
        thread: Option<std::thread::JoinHandle<()>>,
    ) -> Self {
        Self {
            commands,
            events,
            thread,
        }
    }

    /// Take the worker thread handle (if any) so one waiter can join it.
    pub fn take_thread(&mut self) -> Option<std::thread::JoinHandle<()>> {
        self.thread.take()
    }

    /// Take the event receiver so a non-terminal consumer (for example an SFTP
    /// subsystem driver) can forward typed session events itself. The original
    /// field keeps a placeholder receiver that never yields events.
    pub fn take_events(&mut self) -> SessionEventReceiver {
        let placeholder = SessionEventReceiver::from(tokio::sync::mpsc::channel(1).1);
        std::mem::replace(&mut self.events, placeholder)
    }
}

/// Transport channels for a session backend implemented outside this crate.
///
/// The SSH backend owns its own threads and channel wiring. A desktop-side
/// backend such as a local PTY uses this helper to speak the same session
/// protocol without reaching into SSH internals: it drives `commands` and
/// `events` on its own threads and hands `connection` to the UI, exactly like
/// [`start_session`] does.
pub struct SessionChannels {
    pub connection: SessionConnection,
    pub commands: UnboundedReceiver<SessionCommand>,
    pub events: Sender<SessionEvent>,
}

impl SessionChannels {
    pub fn new() -> Self {
        session_channels()
    }
}

impl Default for SessionChannels {
    fn default() -> Self {
        Self::new()
    }
}

pub fn session_channels() -> SessionChannels {
    let (event_sender, event_receiver) = session_event_channel();
    let (command_sender, command_receiver) = unbounded_channel();
    SessionChannels {
        connection: SessionConnection::new(
            SessionCommandSender::new(command_sender, CancelToken::default()),
            event_receiver,
            None,
        ),
        commands: command_receiver,
        events: event_sender,
    }
}

pub(super) type ConnectedSession = ConnectedSshRoute;

pub(super) struct ConnectedClient {
    pub(super) handle: client::Handle<ClientHandler>,
    pub(super) remote_forward_targets: RemoteForwardTargets,
}

#[allow(clippy::too_many_arguments)]
pub fn start_session(
    runtime: &TokioHandle,
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    columns: usize,
    lines: usize,
    monitoring_enabled: bool,
) -> SessionConnection {
    let (event_sender, event_receiver) = session_event_channel();
    let (command_sender, command_receiver) = unbounded_channel();
    let runtime = runtime.clone();
    let cancel = CancelToken::default();
    let worker_cancel = cancel.clone();

    let initial_pty_size = InitialPtySize::new(columns, lines);
    let worker_pty_size = initial_pty_size.clone();

    let thread = std::thread::Builder::new()
        .name(format!("ssh-session-{}", profile.id))
        .spawn(move || {
            runtime.block_on(async move {
                // Racing the whole session against the cancel token means a
                // `close()` also interrupts TCP connect, the SSH handshake and
                // authentication, which never poll the command channel.
                let result = tokio::select! {
                    result = run_session(
                        profile,
                        all_profiles,
                        all_proxies,
                        secrets,
                        known_hosts,
                        command_receiver,
                        event_sender.clone(),
                        worker_pty_size,
                        monitoring_enabled,
                    ) => result,
                    () = worker_cancel.cancelled() => Err(ConnectionCancelled.into()),
                };

                match result {
                    Ok(()) => {}
                    Err(error) if error.downcast_ref::<ConnectionCancelled>().is_some() => {
                        // An explicit close is orderly: report Closed, not Error.
                        let _ = event_sender.send(SessionEvent::Closed).await;
                    }
                    Err(error) => {
                        if event_sender
                            .send(SessionEvent::Error(format!("{error:#}")))
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
        .expect("failed to spawn SSH session thread");

    let mut commands = SessionCommandSender::new(command_sender, cancel);
    commands.initial_pty_size = Some(initial_pty_size);
    SessionConnection::new(commands, event_receiver, Some(thread))
}

pub async fn execute_profile_command(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    command: String,
) -> Result<String> {
    if matches!(
        profile.effective_auth_method(),
        AuthMethod::KeyboardInteractive
    ) {
        bail!("keyboard-interactive authentication is not supported for key deployment");
    }

    let (event_sender, mut event_receiver) = session_event_channel();
    let (command_sender, mut command_receiver) = unbounded_channel();
    let non_interactive_error = Arc::new(Mutex::new(None::<String>));
    let non_interactive_error_for_events = non_interactive_error.clone();
    let event_command_sender = command_sender.clone();
    let event_task = AbortTaskOnDrop::new(tokio::spawn(async move {
        while let Some(event) = event_receiver.recv().await {
            match event {
                SessionEvent::HostKeyPrompt(request) => {
                    let prompt = &request.prompt;
                    let message = if prompt.previous_fingerprint.is_some() {
                        format!(
                            "host key for {}:{} has changed ({}); connect or test this profile manually before deploying the key",
                            prompt.host, prompt.port, prompt.fingerprint
                        )
                    } else {
                        format!(
                            "host key for {}:{} is not trusted yet ({}); connect or test this profile once before deploying the key",
                            prompt.host, prompt.port, prompt.fingerprint
                        )
                    };
                    if let Ok(mut guard) = non_interactive_error_for_events.lock() {
                        *guard = Some(message);
                    }
                    let _ = event_command_sender.send(SessionCommand::HostKeyDecision {
                        request_id: request.request_id,
                        decision: HostKeyDecision::Reject,
                    });
                }
                SessionEvent::KeyboardInteractivePrompt(_) => {
                    if let Ok(mut guard) = non_interactive_error_for_events.lock() {
                        *guard = Some(
                            "keyboard-interactive authentication is not supported for key deployment"
                                .into(),
                        );
                    }
                    let _ = event_command_sender.send(SessionCommand::Close);
                }
                _ => {}
            }
        }
    }));

    let result = async {
        let ConnectedSession {
            session,
            jump_sessions,
            cleanup: _route_cleanup,
            ..
        } = connect_authenticated_session_internal(
            profile,
            all_profiles,
            all_proxies,
            secrets,
            known_hosts,
            &mut command_receiver,
            &event_sender,
        )
        .await?;

        let output = run_exec_command(&session, &command).await;

        if let Err(error) = session
            .disconnect(Disconnect::ByApplication, "", "English")
            .await
        {
            log::debug!("failed to disconnect exec session cleanly: {error:?}");
        }

        for jump_session in jump_sessions.into_iter().rev() {
            if let Err(error) = jump_session
                .disconnect(Disconnect::ByApplication, "", "English")
                .await
            {
                log::debug!("failed to disconnect exec ProxyJump session cleanly: {error:?}");
            }
        }

        output
    }
    .await;

    drop(command_sender);
    drop(event_sender);
    event_task.abort();

    match result {
        Ok(output) => Ok(output),
        Err(error) => {
            if let Some(message) = take_non_interactive_exec_error(&non_interactive_error) {
                Err(error.context(message))
            } else {
                Err(error)
            }
        }
    }
}

fn take_non_interactive_exec_error(error: &Arc<Mutex<Option<String>>>) -> Option<String> {
    error.lock().ok().and_then(|mut guard| guard.take())
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_profile_pty_command(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    command: String,
    columns: u32,
    lines: u32,
) -> Result<String> {
    if matches!(
        profile.effective_auth_method(),
        AuthMethod::KeyboardInteractive
    ) {
        bail!("keyboard-interactive authentication is not supported for PTY exec");
    }

    let (event_sender, mut event_receiver) = session_event_channel();
    let (command_sender, mut command_receiver) = unbounded_channel();
    let non_interactive_error = Arc::new(Mutex::new(None::<String>));
    let non_interactive_error_for_events = non_interactive_error.clone();
    let event_command_sender = command_sender.clone();
    let event_task = AbortTaskOnDrop::new(tokio::spawn(async move {
        while let Some(event) = event_receiver.recv().await {
            match event {
                SessionEvent::HostKeyPrompt(request) => {
                    let prompt = &request.prompt;
                    let message = if prompt.previous_fingerprint.is_some() {
                        format!(
                            "host key for {}:{} has changed ({}); connect or test this profile manually before using the agent",
                            prompt.host, prompt.port, prompt.fingerprint
                        )
                    } else {
                        format!(
                            "host key for {}:{} is not trusted yet ({}); connect or test this profile once before using the agent",
                            prompt.host, prompt.port, prompt.fingerprint
                        )
                    };
                    if let Ok(mut guard) = non_interactive_error_for_events.lock() {
                        *guard = Some(message);
                    }
                    let _ = event_command_sender.send(SessionCommand::HostKeyDecision {
                        request_id: request.request_id,
                        decision: HostKeyDecision::Reject,
                    });
                }
                SessionEvent::KeyboardInteractivePrompt(_) => {
                    if let Ok(mut guard) = non_interactive_error_for_events.lock() {
                        *guard = Some(
                            "keyboard-interactive authentication is not supported for PTY exec"
                                .into(),
                        );
                    }
                    let _ = event_command_sender.send(SessionCommand::Close);
                }
                _ => {}
            }
        }
    }));

    let result = async {
        let ConnectedSession {
            session,
            jump_sessions,
            cleanup: _route_cleanup,
            ..
        } = connect_authenticated_session_internal(
            profile,
            all_profiles,
            all_proxies,
            secrets,
            known_hosts,
            &mut command_receiver,
            &event_sender,
        )
        .await?;

        let output = run_exec_pty_command(&session, &command, columns, lines).await;

        if let Err(error) = session
            .disconnect(Disconnect::ByApplication, "", "English")
            .await
        {
            log::debug!("failed to disconnect PTY exec session cleanly: {error:?}");
        }

        for jump_session in jump_sessions.into_iter().rev() {
            if let Err(error) = jump_session
                .disconnect(Disconnect::ByApplication, "", "English")
                .await
            {
                log::debug!("failed to disconnect exec ProxyJump session cleanly: {error:?}");
            }
        }

        output
    }
    .await;

    drop(command_sender);
    drop(event_sender);
    event_task.abort();

    match result {
        Ok(output) => Ok(output),
        Err(error) => {
            if let Some(message) = take_non_interactive_exec_error(&non_interactive_error) {
                Err(error.context(message))
            } else {
                Err(error)
            }
        }
    }
}

pub(super) struct PendingHostKeyDecision {
    pub(super) request_id: u64,
    pub(super) decision: oneshot::Sender<HostKeyDecision>,
}

pub(super) struct ClientHandler {
    pub(super) known_hosts: KnownHostsStore,
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) event_sender: SessionEventSender,
    pub(super) pending_host_key: Arc<Mutex<Option<PendingHostKeyDecision>>>,
    pub(super) hop: SessionHop,
    pub(super) remote_forward_targets: RemoteForwardTargets,
    pub(super) agent_forwarding_allowed: bool,
    pub(super) strict_host_keys: bool,
}

async fn connect_agent_if_authorized<C, F, T, E>(
    authorized: bool,
    connector: C,
) -> std::result::Result<Option<T>, E>
where
    C: FnOnce() -> F,
    F: Future<Output = std::result::Result<T, E>>,
{
    if !authorized {
        return Ok(None);
    }

    connector().await.map(Some)
}

async fn spawn_forwarding_relay<F>(relay: F) -> Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    drop(tokio::spawn(relay));
    Ok(())
}

impl client::Handler for ClientHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        let check = self
            .known_hosts
            .check(&self.host, self.port, server_public_key)?;

        if self.strict_host_keys {
            return match check {
                HostKeyCheck::Match => Ok(true),
                HostKeyCheck::Unknown => bail!(
                    "host key for {}:{} is not trusted; connect once inside Miaominal to review and save it before using SSH Bridge",
                    self.host,
                    self.port
                ),
                HostKeyCheck::Mismatch { line } => bail!(
                    "host key for {}:{} has changed (known_hosts line {}); SSH Bridge refuses to override a mismatched key",
                    self.host,
                    self.port,
                    line
                ),
            };
        }

        let prompt = match check {
            HostKeyCheck::Match => return Ok(true),
            HostKeyCheck::Unknown => HostKeyPrompt {
                host: self.host.clone(),
                port: self.port,
                algorithm: algorithm_of(server_public_key),
                fingerprint: fingerprint_of(server_public_key),
                previous_fingerprint: None,
            },
            HostKeyCheck::Mismatch { line } => {
                let previous = self.known_hosts.list().ok().and_then(|entries| {
                    entries
                        .into_iter()
                        .find(|entry| entry.host == self.host && entry.port == self.port)
                        .map(|entry| entry.fingerprint)
                });
                log::warn!(
                    "host key mismatch for {}:{} at known_hosts line {line}",
                    self.host,
                    self.port
                );
                HostKeyPrompt {
                    host: self.host.clone(),
                    port: self.port,
                    algorithm: algorithm_of(server_public_key),
                    fingerprint: fingerprint_of(server_public_key),
                    previous_fingerprint: previous,
                }
            }
        };

        let request_id = next_request_id();
        let (decision_sender, decision_receiver) = oneshot::channel();
        {
            let mut guard = self
                .pending_host_key
                .lock()
                .map_err(|_| anyhow!("host key decision mutex poisoned"))?;
            *guard = Some(PendingHostKeyDecision {
                request_id,
                decision: decision_sender,
            });
        }

        let _ = emit_phase(
            &self.event_sender,
            SessionPhase::WaitingForTrust,
            Some(self.hop.clone()),
        )
        .await;
        if self
            .event_sender
            .send(SessionEvent::HostKeyPrompt(HostKeyRequest {
                request_id,
                hop: self.hop.clone(),
                prompt,
            }))
            .await
            .is_err()
        {
            clear_pending_host_key(&self.pending_host_key)?;
            return Ok(false);
        }

        let decision = match decision_receiver.await {
            Ok(decision) => decision,
            // The session was cancelled or the router rejected the request.
            Err(_) => return Ok(false),
        };

        match decision {
            HostKeyDecision::AcceptOnce => {
                let _ = emit_phase(
                    &self.event_sender,
                    SessionPhase::Connecting,
                    Some(self.hop.clone()),
                )
                .await;
                Ok(true)
            }
            HostKeyDecision::AcceptAndSave => {
                if let Err(error) = self
                    .known_hosts
                    .learn(&self.host, self.port, server_public_key)
                {
                    log::warn!("failed to record host key: {error:?}");
                    let _ = self
                        .event_sender
                        .send(SessionEvent::Status(format!(
                            "Could not save host key: {error}"
                        )))
                        .await;
                }
                let _ = emit_phase(
                    &self.event_sender,
                    SessionPhase::Connecting,
                    Some(self.hop.clone()),
                )
                .await;
                Ok(true)
            }
            HostKeyDecision::Reject => Ok(false),
        }
    }

    fn server_channel_open_agent_forward(
        &mut self,
        channel: Channel<client::Msg>,
        _session: &mut client::Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let event_sender = self.event_sender.clone();
        let agent_forwarding_allowed = self.agent_forwarding_allowed;
        spawn_forwarding_relay(async move {
            match connect_agent_if_authorized(agent_forwarding_allowed, connect_local_agent_stream)
                .await
            {
                Ok(Some(mut agent_stream)) => {
                    let mut forwarded = channel.into_stream();
                    if let Err(error) = copy_bidirectional(&mut forwarded, &mut agent_stream).await
                    {
                        log::warn!("agent forwarding relay ended with error: {error:?}");
                    }
                }
                Ok(None) => {
                    log::warn!(
                        "server attempted to open an unauthorized SSH agent forwarding channel"
                    );
                    if let Err(error) = channel.close().await {
                        log::debug!(
                            "failed to close unauthorized agent forwarding channel: {error:?}"
                        );
                    }
                }
                Err(error) => {
                    let _ = event_sender
                        .send(SessionEvent::Status(format!(
                            "Agent forwarding unavailable: {error}"
                        )))
                        .await;
                    if let Err(close_error) = channel.close().await {
                        log::debug!("failed to close forwarded agent channel: {close_error:?}");
                    }
                }
            }
        })
    }

    fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        originator_address: &str,
        originator_port: u32,
        _session: &mut client::Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let event_sender = self.event_sender.clone();
        let remote_forward_targets = self.remote_forward_targets.clone();
        let connected_address = connected_address.to_string();
        let originator_address = originator_address.to_string();

        spawn_forwarding_relay(async move {
            let connected_port = match u16::try_from(connected_port) {
                Ok(port) => port,
                Err(_) => {
                    emit_port_forward_notice(
                        &event_sender,
                        format!(
                            "Remote forwarding used an unsupported port value for {}",
                            connected_address
                        ),
                    )
                    .await;
                    if let Err(error) = channel.close().await {
                        log::debug!("failed to close forwarded tcpip channel: {error:?}");
                    }
                    return;
                }
            };

            let (target, registry_unavailable) = {
                match remote_forward_targets.lock() {
                    Ok(targets) => (
                        targets
                            .get(&(connected_address.clone(), connected_port))
                            .cloned(),
                        false,
                    ),
                    Err(_) => (None, true),
                }
            };
            if registry_unavailable {
                emit_port_forward_notice(
                    &event_sender,
                    "Remote forwarding target registry is unavailable",
                )
                .await;
            }

            let Some(target) = target else {
                emit_port_forward_notice(
                    &event_sender,
                    format!(
                        "No local target is registered for remote forward {}:{}",
                        connected_address, connected_port
                    ),
                )
                .await;
                if let Err(error) = channel.close().await {
                    log::debug!("failed to close forwarded tcpip channel: {error:?}");
                }
                return;
            };

            let local_target = format!("{}:{}", target.target_host, target.target_port);
            match TcpStream::connect(&local_target).await {
                Ok(mut stream) => {
                    let mut forwarded = channel.into_stream();
                    if let Err(error) = copy_bidirectional(&mut forwarded, &mut stream).await {
                        let message = format!(
                            "Remote forward {} relay failed from {}:{}: {}",
                            target.label, originator_address, originator_port, error
                        );
                        emit_port_forward_failure(
                            &event_sender,
                            PortForwardFailure {
                                rule_id: target.rule_id.clone(),
                                label: target.label.clone(),
                                kind: PortForwardFailureKind::TargetUnreachable,
                                message: message.clone(),
                            },
                        )
                        .await;
                        emit_port_forward_notice(&event_sender, message).await;
                    }
                }
                Err(error) => {
                    let message = format!(
                        "Remote forward {} could not reach local target {}: {}",
                        target.label, local_target, error
                    );
                    emit_port_forward_failure(
                        &event_sender,
                        PortForwardFailure {
                            rule_id: target.rule_id.clone(),
                            label: target.label.clone(),
                            kind: PortForwardFailureKind::TargetUnreachable,
                            message: message.clone(),
                        },
                    )
                    .await;
                    emit_port_forward_notice(&event_sender, message).await;
                    if let Err(close_error) = channel.close().await {
                        log::debug!(
                            "failed to close forwarded tcpip channel after connect error: {close_error:?}"
                        );
                    }
                }
            }
        })
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_session(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    mut command_receiver: UnboundedReceiver<SessionCommand>,
    event_sender: SessionEventSender,
    initial_pty_size: InitialPtySize,
    monitoring_enabled: bool,
) -> Result<()> {
    let remote_label = profile.connection_label();
    let agent_forwarding = profile.agent_forwarding;
    let startup_command = profile.startup_command.clone();
    let environment_variables = profile.environment_variables.clone();
    let shell_type = profile.shell_type;
    let ConnectedSession {
        session,
        mut configured_port_forward_rules,
        remote_forward_targets,
        jump_sessions,
        target_hop,
        cleanup: _route_cleanup,
    } = connect_authenticated_session_internal(
        profile.clone(),
        all_profiles,
        all_proxies,
        secrets,
        known_hosts,
        &mut command_receiver,
        &event_sender,
    )
    .await?;

    configured_port_forward_rules.clear();

    let mut active_local_forwards: HashMap<String, ActiveLocalForward> = HashMap::new();
    let mut active_remote_forwards: HashMap<String, ActiveRemoteForward> = HashMap::new();
    sync_port_forward_rules(
        &session,
        &configured_port_forward_rules,
        &mut active_local_forwards,
        &mut active_remote_forwards,
        &remote_forward_targets,
        &event_sender,
    )
    .await;

    // Only a confirmed PTY and shell request may report the session as usable:
    // russh's request methods only send the request, the server's reply arrives
    // later as `Success`/`Failure` on the channel.
    emit_phase(
        &event_sender,
        SessionPhase::OpeningShell,
        Some(target_hop.clone()),
    )
    .await?;

    let mut channel = session
        .channel_open_session()
        .await
        .context("failed to open SSH session channel")?;

    if agent_forwarding {
        channel
            .agent_forward(true)
            .await
            .context("failed to enable SSH agent forwarding")?;
        // The agent-forwarding request also asks for a reply; it must be
        // consumed before the PTY/shell replies or the whole sequence shifts.
        // A refusal is a warning: the shell itself can still work.
        if await_channel_reply(&mut channel, "agent forwarding", &event_sender).await? {
            let _ = event_sender
                .send(SessionEvent::Status("SSH agent forwarding enabled".into()))
                .await;
        } else {
            let _ = event_sender
                .send(SessionEvent::Status(
                    "SSH agent forwarding was refused by the server".into(),
                ))
                .await;
        }
    }

    // Snapshot only after every hop's connection/authentication routers have
    // finished. Resizes arriving after this point stay queued for the live loop.
    let (columns, lines) = initial_pty_size.latest()?;
    log::debug!("requesting interactive SSH PTY: {columns}x{lines}");
    channel
        .request_pty(
            true,
            "xterm-256color",
            columns as u32,
            lines as u32,
            0,
            0,
            &[],
        )
        .await
        .context("failed to request PTY")?;
    if !await_channel_reply(&mut channel, "PTY", &event_sender).await? {
        bail!("server rejected the PTY request");
    }

    for variable in &environment_variables {
        // SSH protocol-level env request (RFC 4254 §6.4). The server may silently
        // ignore variables not listed in AcceptEnv, so we don't treat failure as fatal.
        let _ = channel
            .set_env(false, variable.name.clone(), variable.value.clone())
            .await;
    }

    channel
        .request_shell(true)
        .await
        .context("failed to request interactive shell")?;
    if !await_channel_reply(&mut channel, "interactive shell", &event_sender).await? {
        bail!("server rejected the interactive shell request");
    }

    if let Some(bootstrap_commands) =
        shell_bootstrap_commands(&environment_variables, shell_type, startup_command.trim())
    {
        channel
            .data(bootstrap_commands.as_bytes())
            .await
            .context("failed to send startup commands to the remote shell")?;
    }

    let _ = emit_phase(&event_sender, SessionPhase::Ready, Some(target_hop.clone())).await;
    if event_sender
        .send(SessionEvent::Connected(remote_label))
        .await
        .is_err()
    {
        return Ok(());
    }

    let (monitoring_sender, monitoring_receiver) = watch::channel(monitoring_enabled);
    let monitor_task = tokio::spawn(run_monitor_loop(
        session.clone(),
        event_sender.clone(),
        monitoring_receiver,
    ));
    let mut pending_resize = None;
    let resize_delay = Duration::from_millis(120);
    let resize_timer = tokio::time::sleep(Duration::from_secs(60 * 60 * 24));
    tokio::pin!(resize_timer);

    loop {
        tokio::select! {
            command = command_receiver.recv() => {
                match command {
                    Some(SessionCommand::Send(bytes)) => {
                        flush_pending_resize(&mut channel, &mut pending_resize).await?;
                        channel
                            .data(bytes.as_slice())
                            .await
                            .context("failed to send input to remote shell")?;
                    }
                    Some(SessionCommand::Resize { columns, lines }) => {
                        pending_resize = Some((columns.max(MIN_TERMINAL_COLUMNS), lines.max(1)));
                        resize_timer.as_mut().reset(tokio::time::Instant::now() + resize_delay);
                    }
                    Some(SessionCommand::SetMonitoringEnabled(enabled)) => {
                        let _ = monitoring_sender.send(enabled);
                    }
                    Some(SessionCommand::HostKeyDecision { .. }) => {}
                    Some(SessionCommand::KeyboardInteractiveResponse { .. }) => {}
                    Some(SessionCommand::SyncPortForwardRules(rules)) => {
                        configured_port_forward_rules = rules;
                        sync_port_forward_rules(
                            &session,
                            &configured_port_forward_rules,
                            &mut active_local_forwards,
                            &mut active_remote_forwards,
                            &remote_forward_targets,
                            &event_sender,
                        )
                        .await;
                    }
                    Some(SessionCommand::Close) | None => {
                        let _ = emit_phase(
                            &event_sender,
                            SessionPhase::Closing,
                            Some(target_hop.clone()),
                        )
                        .await;
                        drop(monitoring_sender);
                        monitor_task.abort();
                        if let Err(error) = channel.eof().await {
                            log::debug!("failed to send EOF: {error:?}");
                        }
                        if let Err(error) = channel.close().await {
                            log::debug!("failed to close channel: {error:?}");
                        }
                        break;
                    }
                }
            }
            _ = &mut resize_timer, if pending_resize.is_some() => {
                flush_pending_resize(&mut channel, &mut pending_resize).await?;
            }
            message = channel.wait() => {
                match message {
                    Some(ChannelMsg::Data { data }) => {
                        if event_sender.send(SessionEvent::Output(data.to_vec())).await.is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        if event_sender.send(SessionEvent::Output(data.to_vec())).await.is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        if event_sender.send(SessionEvent::Status(format!("Remote exit status: {exit_status}"))).await.is_err() {
                            break;
                        }
                    }
                    Some(_) => {}
                    None => break,
                }
            }
        }
    }

    sync_port_forward_rules(
        &session,
        &[],
        &mut active_local_forwards,
        &mut active_remote_forwards,
        &remote_forward_targets,
        &event_sender,
    )
    .await;

    monitor_task.abort();

    if let Err(error) = session
        .disconnect(Disconnect::ByApplication, "", "English")
        .await
    {
        log::debug!("failed to disconnect session cleanly: {error:?}");
    }

    for jump_session in jump_sessions.into_iter().rev() {
        if let Err(error) = jump_session
            .disconnect(Disconnect::ByApplication, "", "English")
            .await
        {
            log::debug!("failed to disconnect ProxyJump session cleanly: {error:?}");
        }
    }

    if event_sender.send(SessionEvent::Closed).await.is_err() {
        return Ok(());
    }

    Ok(())
}

async fn flush_pending_resize(
    channel: &mut Channel<client::Msg>,
    pending_resize: &mut Option<(usize, usize)>,
) -> Result<()> {
    let Some((columns, lines)) = pending_resize.take() else {
        return Ok(());
    };

    channel
        .window_change(columns as u32, lines as u32, 0, 0)
        .await
        .context("failed to resize remote PTY")
}

/// Wait for the server's reply to a channel request.
///
/// Returns `true` for `Success` and `false` for `Failure`. Data that arrives
/// while waiting is forwarded to the session event stream so an early banner
/// is not lost. Replies must be consumed in request order: an unconsumed reply
/// would be misread as the answer to the next request.
async fn await_channel_reply(
    channel: &mut Channel<client::Msg>,
    request: &str,
    event_sender: &SessionEventSender,
) -> Result<bool> {
    match tokio::time::timeout(
        CHANNEL_REPLY_TIMEOUT,
        wait_channel_reply(channel, request, event_sender),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => bail!("server did not answer the {request} request in time"),
    }
}

async fn wait_channel_reply(
    channel: &mut Channel<client::Msg>,
    request: &str,
    event_sender: &SessionEventSender,
) -> Result<bool> {
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => return Ok(true),
            Some(ChannelMsg::Failure) => return Ok(false),
            Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                emit_session_event(event_sender, SessionEvent::Output(data.to_vec())).await?;
            }
            Some(_) => {}
            None => bail!("channel closed while waiting for the {request} reply"),
        }
    }
}

fn build_client_handler(
    profile: &SessionProfile,
    known_hosts: KnownHostsStore,
    event_sender: &SessionEventSender,
    hop: SessionHop,
) -> (
    ClientHandler,
    Arc<Mutex<Option<PendingHostKeyDecision>>>,
    RemoteForwardTargets,
) {
    let pending_host_key = Arc::new(Mutex::new(None));
    let remote_forward_targets = Arc::new(Mutex::new(HashMap::new()));

    (
        ClientHandler {
            known_hosts,
            host: profile.host.clone(),
            port: profile.port,
            event_sender: event_sender.clone(),
            pending_host_key: pending_host_key.clone(),
            hop,
            remote_forward_targets: remote_forward_targets.clone(),
            agent_forwarding_allowed: profile.agent_forwarding,
            strict_host_keys: false,
        },
        pending_host_key,
        remote_forward_targets,
    )
}

pub(super) fn build_strict_client_handler(
    profile: &SessionProfile,
    known_hosts: KnownHostsStore,
) -> (ClientHandler, RemoteForwardTargets) {
    let (event_sender, event_receiver) = channel(1);
    drop(event_receiver);
    let remote_forward_targets = Arc::new(Mutex::new(HashMap::new()));

    (
        ClientHandler {
            known_hosts,
            host: profile.host.clone(),
            port: profile.port,
            event_sender,
            pending_host_key: Arc::new(Mutex::new(None)),
            hop: SessionHop::new(profile.connection_label(), 1, 1),
            remote_forward_targets: remote_forward_targets.clone(),
            agent_forwarding_allowed: false,
            strict_host_keys: true,
        },
        remote_forward_targets,
    )
}

async fn await_session_connect<F>(
    connect_future: F,
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    pending_host_key: Arc<Mutex<Option<PendingHostKeyDecision>>>,
    event_sender: &SessionEventSender,
    configured_port_forward_rules: &mut Vec<PortForwardRule>,
) -> Result<client::Handle<ClientHandler>>
where
    F: Future<Output = Result<client::Handle<ClientHandler>, anyhow::Error>>,
{
    tokio::pin!(connect_future);

    loop {
        tokio::select! {
            result = &mut connect_future => {
                break result;
            }
            command = command_receiver.recv() => {
                match command {
                    Some(SessionCommand::HostKeyDecision { request_id, decision }) => {
                        let accepted = {
                            let mut guard = pending_host_key
                                .lock()
                                .map_err(|_| anyhow!("host key decision mutex poisoned"))?;
                            match guard.as_ref().map(|pending| pending.request_id) {
                                Some(pending_id) if pending_id == request_id => {
                                    if let Some(pending) = guard.take() {
                                        let _ = pending.decision.send(decision);
                                    }
                                    true
                                }
                                // A late answer for an old request: reject it.
                                _ => false,
                            }
                        };
                        if !accepted {
                            let _ = emit_session_event(
                                event_sender,
                                SessionEvent::RequestExpired { request_id },
                            )
                            .await;
                        }
                    }
                    Some(SessionCommand::KeyboardInteractiveResponse { request_id, .. }) => {
                        let _ = emit_session_event(
                            event_sender,
                            SessionEvent::RequestExpired { request_id },
                        )
                        .await;
                    }
                    Some(SessionCommand::SyncPortForwardRules(rules)) => {
                        *configured_port_forward_rules = rules;
                    }
                    Some(SessionCommand::Close) | None => {
                        let mut guard = pending_host_key
                            .lock()
                            .map_err(|_| anyhow!("host key decision mutex poisoned"))?;
                        if let Some(pending) = guard.take() {
                            let _ = pending.decision.send(HostKeyDecision::Reject);
                        }
                        return Err(ConnectionCancelled.into());
                    }
                    Some(_) => {}
                }
            }
        }
    }
}

pub(super) async fn connect_authenticated_session_internal(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    event_sender: &SessionEventSender,
) -> Result<ConnectedSession> {
    ProfileConnector::new(all_profiles, all_proxies, secrets, known_hosts)
        .connect_interactive(profile, command_receiver, event_sender)
        .await
}

/// Connect one authenticated interactive SSH route for a non-PTY subsystem.
///
/// Host-key prompts and keyboard-interactive challenges are delivered through
/// `event_sender` with request ids and hop labels; answers arrive on
/// `command_receiver`, exactly like an interactive terminal session. The
/// returned route stays usable until it is dropped or [`ConnectedSshRoute::disconnect`]
/// is called, so callers can open subsystem channels (SFTP) on it.
pub async fn connect_interactive_route(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    event_sender: &Sender<SessionEvent>,
) -> Result<ConnectedSshRoute> {
    connect_authenticated_session_internal(
        profile,
        all_profiles,
        all_proxies,
        secrets,
        known_hosts,
        command_receiver,
        event_sender,
    )
    .await
}

pub(super) async fn connect_profile_session(
    profile: &SessionProfile,
    config: Arc<client::Config>,
    known_hosts: KnownHostsStore,
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    event_sender: &SessionEventSender,
    configured_port_forward_rules: &mut Vec<PortForwardRule>,
    hop: SessionHop,
) -> Result<ConnectedClient> {
    let (handler, pending_host_key, remote_forward_targets) =
        build_client_handler(profile, known_hosts, event_sender, hop);
    let host = profile.host.clone();
    let port = profile.port;
    let connect_future = async move {
        let transport = crate::transport::connect_direct(&host, port).await?;
        client::connect_stream(config, transport, handler)
            .await
            .with_context(|| format!("failed to connect to {}:{}", host, port))
    };

    let handle = await_session_connect(
        connect_future,
        command_receiver,
        pending_host_key,
        event_sender,
        configured_port_forward_rules,
    )
    .await?;

    Ok(ConnectedClient {
        handle,
        remote_forward_targets,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn connect_profile_with_optional_proxy(
    profile: &SessionProfile,
    proxy: Option<&ProxyProfile>,
    secrets: &SecretStore,
    config: Arc<client::Config>,
    known_hosts: KnownHostsStore,
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    event_sender: &SessionEventSender,
    configured_port_forward_rules: &mut Vec<PortForwardRule>,
    hop: SessionHop,
) -> Result<ConnectedClient> {
    let Some(proxy) = proxy else {
        return connect_profile_session(
            profile,
            config,
            known_hosts,
            command_receiver,
            event_sender,
            configured_port_forward_rules,
            hop,
        )
        .await;
    };

    let (handler, pending_host_key, remote_forward_targets) =
        build_client_handler(profile, known_hosts, event_sender, hop);
    let host = profile.host.clone();
    let port = profile.port;
    let proxy = proxy.clone();
    let secrets = secrets.clone();
    let connect_future = async move {
        let transport = crate::transport::connect_via_proxy(&proxy, &host, port, &secrets).await?;
        client::connect_stream(config, transport, handler)
            .await
            .with_context(|| {
                format!(
                    "failed to connect to {}:{} through proxy {}",
                    host,
                    port,
                    proxy.connection_label()
                )
            })
    };

    let handle = await_session_connect(
        connect_future,
        command_receiver,
        pending_host_key,
        event_sender,
        configured_port_forward_rules,
    )
    .await?;

    Ok(ConnectedClient {
        handle,
        remote_forward_targets,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn connect_profile_stream<R>(
    profile: &SessionProfile,
    transport: R,
    config: Arc<client::Config>,
    known_hosts: KnownHostsStore,
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    event_sender: &SessionEventSender,
    configured_port_forward_rules: &mut Vec<PortForwardRule>,
    hop: SessionHop,
) -> Result<ConnectedClient>
where
    R: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (handler, pending_host_key, remote_forward_targets) =
        build_client_handler(profile, known_hosts, event_sender, hop);
    let host = profile.host.clone();
    let port = profile.port;
    let connect_future = async move {
        client::connect_stream(config, transport, handler)
            .await
            .with_context(|| format!("failed to connect to {}:{} through ProxyJump", host, port))
    };

    let handle = await_session_connect(
        connect_future,
        command_receiver,
        pending_host_key,
        event_sender,
        configured_port_forward_rules,
    )
    .await?;

    Ok(ConnectedClient {
        handle,
        remote_forward_targets,
    })
}

pub(super) async fn emit_status(event_sender: &SessionEventSender, message: String) -> Result<()> {
    if event_sender
        .send(SessionEvent::Status(message))
        .await
        .is_err()
    {
        bail!("session event receiver is closed");
    }

    Ok(())
}

fn shell_bootstrap_commands(
    environment_variables: &[SessionEnvironmentVariable],
    shell_type: ShellType,
    startup_command: &str,
) -> Option<String> {
    let mut commands = Vec::new();

    for variable in environment_variables {
        let cmd = match shell_type {
            ShellType::Posix => format!(
                "export {}={}",
                variable.name,
                shell_quote_posix(&variable.value)
            ),
            ShellType::Fish => format!(
                "set -x {} {}",
                variable.name,
                shell_quote_fish(&variable.value)
            ),
            ShellType::PowerShell => format!(
                "$env:{} = {}",
                variable.name,
                shell_quote_powershell(&variable.value)
            ),
            ShellType::Cmd => format!(
                "SET \"{}={}\"",
                variable.name,
                shell_quote_cmd(&variable.value)
            ),
        };
        commands.push(cmd);
    }

    if !startup_command.is_empty() {
        commands.push(startup_command.to_string());
    }

    if commands.is_empty() {
        None
    } else {
        // CMD and PowerShell PTYs treat \r as Enter (executes the line).
        // Sending \r\n would press Enter twice, producing a blank line between commands.
        // POSIX/Fish shells accept \n as Enter.
        let line_ending = match shell_type {
            ShellType::Cmd | ShellType::PowerShell => "\r",
            ShellType::Posix | ShellType::Fish => "\n",
        };
        let mut output = commands.join(line_ending);
        output.push_str(line_ending);
        Some(output)
    }
}

fn shell_quote_posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
fn shell_quote_fish(value: &str) -> String {
    // Fish uses double-quoted strings; escape backslashes and double-quotes.
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn shell_quote_powershell(value: &str) -> String {
    // PowerShell single-quoted strings: escape ' by doubling it.
    format!("'{}'", value.replace('\'', "''"))
}

fn shell_quote_cmd(value: &str) -> String {
    // Inside SET "NAME=value" the value must not contain double-quotes.
    // Percent signs are special in CMD; escape them by doubling.
    value.replace('"', "").replace('%', "%%")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forwarding::RemoteForwardTarget;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn interactive_test_commands(
        columns: usize,
        lines: usize,
    ) -> (
        SessionCommandSender,
        UnboundedReceiver<SessionCommand>,
        InitialPtySize,
    ) {
        let (sender, receiver) = unbounded_channel();
        let size = InitialPtySize::new(columns, lines);
        let mut commands = SessionCommandSender::new(sender, CancelToken::default());
        commands.initial_pty_size = Some(size.clone());
        (commands, receiver, size)
    }

    #[test]
    fn initial_pty_size_preserves_initial_geometry_and_minimums() {
        assert_eq!(InitialPtySize::new(120, 32).latest().unwrap(), (120, 32));
        assert_eq!(
            InitialPtySize::new(0, 0).latest().unwrap(),
            (MIN_TERMINAL_COLUMNS, 1)
        );
    }

    #[test]
    fn cloned_resizes_share_initial_geometry_and_still_enqueue_live_commands() {
        let (commands, mut receiver, size) = interactive_test_commands(120, 32);
        let cloned_commands = commands.clone();

        commands.resize(160, 40).unwrap();
        assert_eq!(size.latest().unwrap(), (160, 40));
        cloned_commands.resize(180, 48).unwrap();
        assert_eq!(size.latest().unwrap(), (180, 48));
        commands.resize(0, 0).unwrap();
        assert_eq!(size.latest().unwrap(), (MIN_TERMINAL_COLUMNS, 1));

        for expected in [(160, 40), (180, 48), (0, 0)] {
            assert!(matches!(
                receiver.try_recv(),
                Ok(SessionCommand::Resize { columns, lines }) if (columns, lines) == expected
            ));
        }
        assert!(matches!(
            receiver.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn non_pty_command_senders_keep_the_existing_resize_protocol() {
        let mut channels = session_channels();
        assert!(channels.connection.commands.initial_pty_size.is_none());
        channels.connection.commands.resize(0, 0).unwrap();
        assert!(matches!(
            channels.commands.try_recv(),
            Ok(SessionCommand::Resize {
                columns: 0,
                lines: 0,
            })
        ));
    }

    #[tokio::test]
    async fn connecting_router_cannot_discard_latest_initial_pty_geometry() {
        let (commands, mut receiver, size) = interactive_test_commands(120, 32);
        let (event_sender, _events) = session_event_channel();
        let (decision_sender, decision_receiver) = oneshot::channel();
        let pending_host_key = Arc::new(Mutex::new(Some(PendingHostKeyDecision {
            request_id: 7,
            decision: decision_sender,
        })));
        let mut configured_port_forward_rules = Vec::new();

        // The production router must consume both resizes before it can reach
        // the trust answer that releases the fake handshake. No transport or
        // timing-dependent sleep is needed to reproduce the pre-PTY loss.
        commands.resize(150, 38).unwrap();
        commands.clone().resize(180, 44).unwrap();
        commands
            .respond_host_key(7, HostKeyDecision::AcceptOnce)
            .unwrap();
        let connect_future = async move {
            assert!(matches!(
                decision_receiver.await.unwrap(),
                HostKeyDecision::AcceptOnce
            ));
            Err::<client::Handle<ClientHandler>, anyhow::Error>(anyhow!(
                "test handshake stops after trust"
            ))
        };

        let error = tokio::time::timeout(
            Duration::from_secs(1),
            await_session_connect(
                connect_future,
                &mut receiver,
                pending_host_key,
                &event_sender,
                &mut configured_port_forward_rules,
            ),
        )
        .await
        .expect("trust answer should release the router")
        .err()
        .expect("the fake handshake should return its sentinel error");
        assert_eq!(error.to_string(), "test handshake stops after trust");
        assert!(matches!(
            receiver.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        // run_session takes this same snapshot immediately before request_pty.
        assert_eq!(size.latest().unwrap(), (180, 44));
    }

    #[tokio::test]
    async fn abort_task_guard_cancels_detached_event_work_on_drop() {
        struct DropSignal(Option<oneshot::Sender<()>>);

        impl Drop for DropSignal {
            fn drop(&mut self) {
                if let Some(sender) = self.0.take() {
                    let _ = sender.send(());
                }
            }
        }

        let (started_sender, started_receiver) = oneshot::channel();
        let (dropped_sender, dropped_receiver) = oneshot::channel();
        let task = AbortTaskOnDrop::new(tokio::spawn(async move {
            let _drop_signal = DropSignal(Some(dropped_sender));
            let _ = started_sender.send(());
            std::future::pending::<()>().await;
        }));
        started_receiver
            .await
            .expect("guarded event task should start");

        drop(task);

        tokio::time::timeout(Duration::from_secs(1), dropped_receiver)
            .await
            .expect("dropping the guard should abort the event task")
            .expect("event task should report that its future was dropped");
    }

    #[tokio::test]
    async fn disabled_agent_forwarding_does_not_call_local_agent_connector() {
        let connector_called = AtomicBool::new(false);

        let result = connect_agent_if_authorized(false, || {
            connector_called.store(true, Ordering::SeqCst);
            async { Ok::<(), ()>(()) }
        })
        .await;

        assert_eq!(result, Ok(None));
        assert!(!connector_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn enabled_agent_forwarding_calls_local_agent_connector() {
        let connector_called = AtomicBool::new(false);

        let result = connect_agent_if_authorized(true, || {
            connector_called.store(true, Ordering::SeqCst);
            async { Ok::<_, ()>("connected") }
        })
        .await;

        assert_eq!(result, Ok(Some("connected")));
        assert!(connector_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn forwarding_relay_dispatch_does_not_wait_for_relay_completion() {
        let (started_sender, started_receiver) = oneshot::channel();
        let (release_sender, release_receiver) = oneshot::channel();
        let (finished_sender, finished_receiver) = oneshot::channel();

        let dispatch = spawn_forwarding_relay(async move {
            let _ = started_sender.send(());
            let _ = release_receiver.await;
            let _ = finished_sender.send(());
        });

        tokio::time::timeout(Duration::from_secs(1), dispatch)
            .await
            .expect("forwarding relay dispatch should return immediately")
            .expect("forwarding relay dispatch should succeed");
        tokio::time::timeout(Duration::from_secs(1), started_receiver)
            .await
            .expect("forwarding relay should start in the background")
            .expect("forwarding relay should report that it started");

        let _ = release_sender.send(());
        tokio::time::timeout(Duration::from_secs(1), finished_receiver)
            .await
            .expect("forwarding relay should finish after it is released")
            .expect("forwarding relay should report that it finished");
    }

    #[tokio::test]
    async fn session_event_channel_backpressures_without_dropping_or_reordering() {
        let (sender, mut receiver) = session_event_channel();
        let clone = sender.clone();
        for index in 0..SESSION_EVENT_QUEUE_CAPACITY {
            sender
                .try_send(SessionEvent::Output(vec![index as u8]))
                .expect("event queue should accept its configured capacity");
        }
        assert!(matches!(
            clone.try_send(SessionEvent::Status("full".into())),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_))
        ));

        let blocked_send = sender.send(SessionEvent::Status("ready".into()));
        tokio::pin!(blocked_send);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut blocked_send)
                .await
                .is_err(),
            "a saturated event queue must backpressure the producer"
        );

        assert!(matches!(
            receiver.recv().await,
            Some(SessionEvent::Output(bytes)) if bytes == vec![0]
        ));
        tokio::time::timeout(Duration::from_secs(1), &mut blocked_send)
            .await
            .expect("send should resume after capacity is released")
            .expect("receiver should remain open");

        for expected in 1..SESSION_EVENT_QUEUE_CAPACITY {
            assert!(matches!(
                receiver.recv().await,
                Some(SessionEvent::Output(bytes)) if bytes == vec![expected as u8]
            ));
        }
        assert!(matches!(
            receiver.recv().await,
            Some(SessionEvent::Status(status)) if status == "ready"
        ));
    }

    #[tokio::test]
    async fn dropping_event_receiver_unblocks_waiting_sender() {
        let (sender, mut receiver) = session_event_channel();
        for _ in 0..SESSION_EVENT_QUEUE_CAPACITY {
            sender
                .try_send(SessionEvent::Output(vec![1]))
                .expect("fill event queue");
        }
        let blocked_send = sender.send(SessionEvent::Closed);
        tokio::pin!(blocked_send);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut blocked_send)
                .await
                .is_err()
        );
        receiver.receiver.close();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), &mut blocked_send)
                .await
                .expect("closed receiver should wake sender")
                .is_err()
        );
    }

    #[test]
    fn client_config_uses_explicit_bounded_channel_settings() {
        let config = connection::default_client_config();
        assert_eq!(config.maximum_packet_size, SSH_MAXIMUM_PACKET_SIZE);
        assert_eq!(config.channel_buffer_size, SSH_CHANNEL_BUFFER_SIZE);
    }

    #[test]
    fn client_handler_copies_agent_forwarding_authorization_from_profile() {
        let (event_sender, _event_receiver) = session_event_channel();

        for agent_forwarding_allowed in [false, true] {
            let mut profile = SessionProfile::blank("test-profile", 1);
            profile.agent_forwarding = agent_forwarding_allowed;
            let known_hosts = KnownHostsStore::with_path(
                std::env::temp_dir().join("miaominal-agent-forwarding-known-hosts"),
            );

            let (handler, _pending_decision, _remote_forward_targets) = build_client_handler(
                &profile,
                known_hosts,
                &event_sender,
                SessionHop::new(profile.connection_label(), 1, 1),
            );

            assert_eq!(handler.agent_forwarding_allowed, agent_forwarding_allowed);
        }
    }

    #[test]
    fn client_handlers_isolate_remote_forward_targets_by_connection() {
        let (event_sender, _event_receiver) = session_event_channel();
        let known_hosts_path =
            std::env::temp_dir().join("miaominal-remote-forward-isolation-known-hosts");
        let jump_profile = SessionProfile::blank("jump-profile", 1);
        let target_profile = SessionProfile::blank("target-profile", 2);

        let (jump_handler, _jump_pending_decision, jump_targets) = build_client_handler(
            &jump_profile,
            KnownHostsStore::with_path(known_hosts_path.clone()),
            &event_sender,
            SessionHop::new(jump_profile.connection_label(), 1, 2),
        );
        let (target_handler, _target_pending_decision, target_targets) = build_client_handler(
            &target_profile,
            KnownHostsStore::with_path(known_hosts_path),
            &event_sender,
            SessionHop::new(target_profile.connection_label(), 2, 2),
        );

        assert!(Arc::ptr_eq(
            &jump_handler.remote_forward_targets,
            &jump_targets
        ));
        assert!(Arc::ptr_eq(
            &target_handler.remote_forward_targets,
            &target_targets
        ));
        assert!(!Arc::ptr_eq(&jump_targets, &target_targets));

        let key = ("127.0.0.1".to_string(), 5432);
        target_targets.lock().expect("target registry lock").insert(
            key.clone(),
            RemoteForwardTarget {
                rule_id: "rule-1".into(),
                label: "database".into(),
                target_host: "127.0.0.1".into(),
                target_port: 5432,
            },
        );

        assert!(
            target_handler
                .remote_forward_targets
                .lock()
                .expect("target handler registry lock")
                .contains_key(&key)
        );
        assert!(
            !jump_handler
                .remote_forward_targets
                .lock()
                .expect("jump handler registry lock")
                .contains_key(&key)
        );
    }
}
