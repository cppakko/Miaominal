use super::auth::{authenticate_full, connect_local_agent_stream, hydrate_profile_from_secrets};
use super::forwarding::{
    ActiveLocalForward, ActiveRemoteForward, RemoteForwardTargets, emit_port_forward_notice,
    sync_port_forward_rules,
};
use super::monitor::{run_exec_command, run_exec_pty_command, run_monitor_loop};
use anyhow::{Context, Result, anyhow, bail};
use miaominal_core::forwarding::{
    HostKeyDecision, HostKeyPrompt, KbiChallenge, SessionMonitorSnapshot,
};
use miaominal_core::known_host::HostKeyCheck;
use miaominal_core::profile::{
    AuthMethod, PortForwardRule, SessionEnvironmentVariable, SessionProfile, ShellType,
};
use miaominal_secrets::SecretStore;
use miaominal_storage::KnownHostsStore;
use miaominal_terminal::MIN_TERMINAL_COLUMNS;
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

pub const SESSION_EVENT_QUEUE_CAPACITY: usize = 64;
const SSH_CHANNEL_BUFFER_SIZE: usize = 64;
const SSH_MAXIMUM_PACKET_SIZE: u32 = 32 * 1024;

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

        client::connect(config, (host.clone(), port), handler)
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
            .with_context(|| format!("failed to connect to {}:{} through ProxyJump", host, port))
    }
}

#[derive(Debug)]
pub enum SessionEvent {
    Connected(String),
    Output(Vec<u8>),
    Status(String),
    Error(String),
    MonitorUpdated(SessionMonitorSnapshot),
    MonitorFailed(String),
    PortForwardNotice(String),
    HostKeyPrompt(HostKeyPrompt),
    KeyboardInteractivePrompt(KbiChallenge),
    Closed,
}

#[derive(Debug)]
pub enum SessionCommand {
    Send(Vec<u8>),
    Resize { columns: usize, lines: usize },
    SetMonitoringEnabled(bool),
    HostKeyDecision(HostKeyDecision),
    KeyboardInteractiveResponse(Vec<String>),
    SyncPortForwardRules(Vec<PortForwardRule>),
    Close,
}

#[derive(Clone, Debug)]
pub struct SessionCommandSender {
    sender: UnboundedSender<SessionCommand>,
}

impl SessionCommandSender {
    pub(super) fn new(sender: UnboundedSender<SessionCommand>) -> Self {
        Self { sender }
    }

    pub fn send_bytes(&self, bytes: Vec<u8>) -> Result<()> {
        self.sender
            .send(SessionCommand::Send(bytes))
            .map_err(|_| anyhow!("session is no longer available"))
    }

    pub fn resize(&self, columns: usize, lines: usize) -> Result<()> {
        self.sender
            .send(SessionCommand::Resize { columns, lines })
            .map_err(|_| anyhow!("session is no longer available"))
    }

    pub fn set_monitoring_enabled(&self, enabled: bool) -> Result<()> {
        self.sender
            .send(SessionCommand::SetMonitoringEnabled(enabled))
            .map_err(|_| anyhow!("session is no longer available"))
    }

    pub fn respond_host_key(&self, decision: HostKeyDecision) -> Result<()> {
        self.sender
            .send(SessionCommand::HostKeyDecision(decision))
            .map_err(|_| anyhow!("session is no longer available"))
    }

    pub fn respond_keyboard_interactive(&self, responses: Vec<String>) -> Result<()> {
        self.sender
            .send(SessionCommand::KeyboardInteractiveResponse(responses))
            .map_err(|_| anyhow!("session is no longer available"))
    }

    pub fn sync_port_forward_rules(&self, rules: Vec<PortForwardRule>) -> Result<()> {
        self.sender
            .send(SessionCommand::SyncPortForwardRules(rules))
            .map_err(|_| anyhow!("session is no longer available"))
    }

    pub fn close(&self) -> Result<()> {
        self.sender
            .send(SessionCommand::Close)
            .map_err(|_| anyhow!("session is no longer available"))
    }
}

pub struct SessionConnection {
    pub commands: SessionCommandSender,
    pub events: SessionEventReceiver,
}

impl SessionConnection {
    pub(super) fn new(commands: SessionCommandSender, events: SessionEventReceiver) -> Self {
        Self { commands, events }
    }
}

pub(super) struct ConnectedSession {
    pub session: Arc<client::Handle<ClientHandler>>,
    pub configured_port_forward_rules: Vec<PortForwardRule>,
    pub remote_forward_targets: RemoteForwardTargets,
    pub jump_sessions: Vec<Arc<client::Handle<ClientHandler>>>,
}

struct ConnectedClient {
    handle: client::Handle<ClientHandler>,
    remote_forward_targets: RemoteForwardTargets,
}

#[allow(clippy::too_many_arguments)]
pub fn start_session(
    runtime: &TokioHandle,
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    columns: usize,
    lines: usize,
    monitoring_enabled: bool,
) -> SessionConnection {
    let (event_sender, event_receiver) = session_event_channel();
    let (command_sender, command_receiver) = unbounded_channel();
    let runtime = runtime.clone();

    let columns = columns.max(MIN_TERMINAL_COLUMNS);
    let lines = lines.max(1);

    std::thread::Builder::new()
        .name(format!("ssh-session-{}", profile.id))
        .spawn(move || {
            runtime.block_on(async move {
                if let Err(error) = run_session(
                    profile,
                    all_profiles,
                    secrets,
                    known_hosts,
                    command_receiver,
                    event_sender.clone(),
                    columns,
                    lines,
                    monitoring_enabled,
                )
                .await
                {
                    if event_sender
                        .send(SessionEvent::Error(error.to_string()))
                        .await
                        .is_err()
                    {
                        return;
                    }

                    let _ = event_sender.send(SessionEvent::Closed).await;
                }
            });
        })
        .expect("failed to spawn SSH session thread");

    SessionConnection::new(SessionCommandSender::new(command_sender), event_receiver)
}

pub async fn execute_profile_command(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
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
    let event_task = tokio::spawn(async move {
        while let Some(event) = event_receiver.recv().await {
            match event {
                SessionEvent::HostKeyPrompt(prompt) => {
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
                    let _ = event_command_sender
                        .send(SessionCommand::HostKeyDecision(HostKeyDecision::Reject));
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
    });

    let result = async {
        let ConnectedSession {
            session,
            jump_sessions,
            ..
        } = connect_authenticated_session_internal(
            profile,
            all_profiles,
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

pub async fn execute_profile_pty_command(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
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
    let event_task = tokio::spawn(async move {
        while let Some(event) = event_receiver.recv().await {
            match event {
                SessionEvent::HostKeyPrompt(prompt) => {
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
                    let _ = event_command_sender
                        .send(SessionCommand::HostKeyDecision(HostKeyDecision::Reject));
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
    });

    let result = async {
        let ConnectedSession {
            session,
            jump_sessions,
            ..
        } = connect_authenticated_session_internal(
            profile,
            all_profiles,
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

pub(super) struct ClientHandler {
    pub(super) known_hosts: KnownHostsStore,
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) event_sender: SessionEventSender,
    pub(super) decision_inbox: Arc<Mutex<Option<oneshot::Receiver<HostKeyDecision>>>>,
    pub(super) remote_forward_targets: RemoteForwardTargets,
    pub(super) agent_forwarding_allowed: bool,
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

        if self
            .event_sender
            .send(SessionEvent::HostKeyPrompt(prompt))
            .await
            .is_err()
        {
            return Ok(false);
        }

        let receiver = {
            let mut guard = self
                .decision_inbox
                .lock()
                .map_err(|_| anyhow!("host key decision mutex poisoned"))?;
            guard
                .take()
                .ok_or_else(|| anyhow!("host key decision channel already consumed"))?
        };

        let decision = receiver
            .await
            .map_err(|_| anyhow!("host key decision channel closed"))?;

        match decision {
            HostKeyDecision::AcceptOnce => Ok(true),
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
                        emit_port_forward_notice(
                            &event_sender,
                            format!(
                                "Remote forward {} relay failed from {}:{}: {}",
                                target.label, originator_address, originator_port, error
                            ),
                        )
                        .await;
                    }
                }
                Err(error) => {
                    emit_port_forward_notice(
                        &event_sender,
                        format!(
                            "Remote forward {} could not reach local target {}: {}",
                            target.label, local_target, error
                        ),
                    )
                    .await;
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
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    mut command_receiver: UnboundedReceiver<SessionCommand>,
    event_sender: SessionEventSender,
    columns: usize,
    lines: usize,
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
    } = connect_authenticated_session_internal(
        profile.clone(),
        all_profiles,
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

    let mut channel = session
        .channel_open_session()
        .await
        .context("failed to open SSH session channel")?;

    if agent_forwarding {
        channel
            .agent_forward(true)
            .await
            .context("failed to enable SSH agent forwarding")?;
        let _ = event_sender
            .send(SessionEvent::Status("SSH agent forwarding enabled".into()))
            .await;
    }

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

    if let Some(bootstrap_commands) =
        shell_bootstrap_commands(&environment_variables, shell_type, startup_command.trim())
    {
        channel
            .data(bootstrap_commands.as_bytes())
            .await
            .context("failed to send startup commands to the remote shell")?;
    }

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
                    Some(SessionCommand::HostKeyDecision(_)) => {}
                    Some(SessionCommand::KeyboardInteractiveResponse(_)) => {}
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

fn build_client_handler(
    profile: &SessionProfile,
    known_hosts: KnownHostsStore,
    event_sender: &SessionEventSender,
) -> (
    ClientHandler,
    Arc<Mutex<Option<oneshot::Sender<HostKeyDecision>>>>,
    RemoteForwardTargets,
) {
    let (decision_sender, decision_receiver) = oneshot::channel();
    let decision_inbox = Arc::new(Mutex::new(Some(decision_receiver)));
    let pending_decision = Arc::new(Mutex::new(Some(decision_sender)));
    let remote_forward_targets = Arc::new(Mutex::new(HashMap::new()));

    (
        ClientHandler {
            known_hosts,
            host: profile.host.clone(),
            port: profile.port,
            event_sender: event_sender.clone(),
            decision_inbox,
            remote_forward_targets: remote_forward_targets.clone(),
            agent_forwarding_allowed: profile.agent_forwarding,
        },
        pending_decision,
        remote_forward_targets,
    )
}

async fn await_session_connect<F>(
    connect_future: F,
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    pending_decision: Arc<Mutex<Option<oneshot::Sender<HostKeyDecision>>>>,
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
                    Some(SessionCommand::HostKeyDecision(decision)) => {
                        let mut guard = pending_decision
                            .lock()
                            .map_err(|_| anyhow!("host key decision mutex poisoned"))?;
                        if let Some(sender) = guard.take() {
                            let _ = sender.send(decision);
                        }
                    }
                    Some(SessionCommand::SyncPortForwardRules(rules)) => {
                        *configured_port_forward_rules = rules;
                    }
                    Some(SessionCommand::Close) | None => {
                        let mut guard = pending_decision
                            .lock()
                            .map_err(|_| anyhow!("host key decision mutex poisoned"))?;
                        if let Some(sender) = guard.take() {
                            let _ = sender.send(HostKeyDecision::Reject);
                        }
                        bail!("connection cancelled");
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
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    event_sender: &SessionEventSender,
) -> Result<ConnectedSession> {
    let profile = hydrate_profile_from_secrets(profile, &secrets);
    let remote_label = profile.connection_label();
    let proxy_jump_profiles = connection::resolve_proxy_jump_profiles(&profile, &all_profiles)?
        .into_iter()
        .map(|profile| hydrate_profile_from_secrets(profile, &secrets))
        .collect::<Vec<_>>();
    let mut configured_port_forward_rules = profile.port_forwarding_rules.clone();
    let config = connection::default_client_config();
    let mut jump_sessions = Vec::new();

    let (session, remote_forward_targets) = if let Some(first_hop) = proxy_jump_profiles.first() {
        emit_status(
            event_sender,
            format!(
                "Connecting to jump host 1/{}: {}",
                proxy_jump_profiles.len(),
                first_hop.connection_label()
            ),
        )
        .await?;

        let ConnectedClient {
            handle: mut current_session,
            remote_forward_targets: mut current_remote_forward_targets,
        } = connect_profile_session(
            first_hop,
            config.clone(),
            known_hosts.clone(),
            command_receiver,
            event_sender,
            &mut configured_port_forward_rules,
        )
        .await?;
        emit_status(
            event_sender,
            format!("Authenticating jump host 1/{}", proxy_jump_profiles.len()),
        )
        .await?;
        authenticate_full(
            &mut current_session,
            first_hop.clone(),
            &secrets,
            command_receiver,
            event_sender,
        )
        .await?;
        let mut current_session = Arc::new(current_session);

        let mut remaining_chain: Vec<_> = proxy_jump_profiles.iter().skip(1).cloned().collect();
        remaining_chain.push(profile);
        let total_hops = proxy_jump_profiles.len();

        for (index, next_profile) in remaining_chain.into_iter().enumerate() {
            let is_target = index + 1 == total_hops;
            let status = if is_target {
                format!("Connecting to {remote_label} through ProxyJump")
            } else {
                format!(
                    "Connecting to jump host {}/{}: {}",
                    index + 2,
                    total_hops,
                    next_profile.connection_label()
                )
            };
            emit_status(event_sender, status).await?;

            let transport = current_session
                .channel_open_direct_tcpip(
                    next_profile.host.clone(),
                    u32::from(next_profile.port),
                    "127.0.0.1".to_string(),
                    0,
                )
                .await
                .with_context(|| {
                    format!(
                        "failed to open ProxyJump channel to {}:{}",
                        next_profile.host, next_profile.port
                    )
                })?
                .into_stream();

            let ConnectedClient {
                handle: mut next_session,
                remote_forward_targets: next_remote_forward_targets,
            } = connect_profile_stream(
                &next_profile,
                transport,
                config.clone(),
                known_hosts.clone(),
                command_receiver,
                event_sender,
                &mut configured_port_forward_rules,
            )
            .await?;
            emit_status(
                event_sender,
                if is_target {
                    format!("Authenticating {remote_label}")
                } else {
                    format!("Authenticating jump host {}/{}", index + 2, total_hops)
                },
            )
            .await?;
            authenticate_full(
                &mut next_session,
                next_profile,
                &secrets,
                command_receiver,
                event_sender,
            )
            .await?;

            jump_sessions.push(current_session);
            current_session = Arc::new(next_session);
            current_remote_forward_targets = next_remote_forward_targets;
        }

        (current_session, current_remote_forward_targets)
    } else {
        emit_status(event_sender, format!("Connecting to {remote_label}")).await?;
        let ConnectedClient {
            handle: mut session,
            remote_forward_targets,
        } = connect_profile_session(
            &profile,
            config,
            known_hosts,
            command_receiver,
            event_sender,
            &mut configured_port_forward_rules,
        )
        .await?;
        emit_status(event_sender, format!("Authenticating {remote_label}")).await?;
        authenticate_full(
            &mut session,
            profile,
            &secrets,
            command_receiver,
            event_sender,
        )
        .await?;
        (Arc::new(session), remote_forward_targets)
    };

    Ok(ConnectedSession {
        session,
        configured_port_forward_rules,
        remote_forward_targets,
        jump_sessions,
    })
}

async fn connect_profile_session(
    profile: &SessionProfile,
    config: Arc<client::Config>,
    known_hosts: KnownHostsStore,
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    event_sender: &SessionEventSender,
    configured_port_forward_rules: &mut Vec<PortForwardRule>,
) -> Result<ConnectedClient> {
    let (handler, pending_decision, remote_forward_targets) =
        build_client_handler(profile, known_hosts, event_sender);
    let host = profile.host.clone();
    let port = profile.port;
    let connect_future = async move {
        client::connect(config, (host.clone(), port), handler)
            .await
            .with_context(|| format!("failed to connect to {}:{}", host, port))
    };

    let handle = await_session_connect(
        connect_future,
        command_receiver,
        pending_decision,
        configured_port_forward_rules,
    )
    .await?;

    Ok(ConnectedClient {
        handle,
        remote_forward_targets,
    })
}

#[allow(clippy::too_many_arguments)]
async fn connect_profile_stream<R>(
    profile: &SessionProfile,
    transport: R,
    config: Arc<client::Config>,
    known_hosts: KnownHostsStore,
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    event_sender: &SessionEventSender,
    configured_port_forward_rules: &mut Vec<PortForwardRule>,
) -> Result<ConnectedClient>
where
    R: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (handler, pending_decision, remote_forward_targets) =
        build_client_handler(profile, known_hosts, event_sender);
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
        pending_decision,
        configured_port_forward_rules,
    )
    .await?;

    Ok(ConnectedClient {
        handle,
        remote_forward_targets,
    })
}

async fn emit_status(event_sender: &SessionEventSender, message: String) -> Result<()> {
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

            let (handler, _pending_decision, _remote_forward_targets) =
                build_client_handler(&profile, known_hosts, &event_sender);

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
        );
        let (target_handler, _target_pending_decision, target_targets) = build_client_handler(
            &target_profile,
            KnownHostsStore::with_path(known_hosts_path),
            &event_sender,
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
