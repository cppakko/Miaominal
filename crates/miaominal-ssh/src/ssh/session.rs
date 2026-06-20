use super::auth::{authenticate_full, connect_local_agent_stream, hydrate_profile_from_secrets};
use super::forwarding::{
    ActiveLocalForward, ActiveRemoteForward, RemoteForwardTargets, emit_port_forward_notice,
    sync_port_forward_rules,
};
use super::monitor::{run_exec_command, run_exec_pty_command, run_monitor_loop};
use anyhow::{Context, Result, anyhow, bail};
use futures::StreamExt;
use futures::channel::mpsc::{
    UnboundedReceiver as FuturesUnboundedReceiver, UnboundedSender as FuturesUnboundedSender,
    unbounded as futures_unbounded,
};
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
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::copy_bidirectional;
use tokio::net::TcpStream;
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tokio::sync::watch;

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
    pub events: FuturesUnboundedReceiver<SessionEvent>,
}

impl SessionConnection {
    pub(super) fn new(
        commands: SessionCommandSender,
        events: FuturesUnboundedReceiver<SessionEvent>,
    ) -> Self {
        Self { commands, events }
    }
}

pub(super) struct ConnectedSession {
    pub session: Arc<client::Handle<ClientHandler>>,
    pub configured_port_forward_rules: Vec<PortForwardRule>,
    pub remote_forward_targets: RemoteForwardTargets,
    pub jump_sessions: Vec<Arc<client::Handle<ClientHandler>>>,
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
    let (event_sender, event_receiver) = futures_unbounded();
    let (command_sender, command_receiver) = unbounded_channel();
    let runtime = runtime.clone();

    let columns = columns.max(MIN_TERMINAL_COLUMNS);
    let lines = lines.max(1);

    std::thread::Builder::new()
        .name(format!("ssh-session-{}", profile.id))
        .spawn(move || {
            if let Err(error) = runtime.block_on(run_session(
                profile,
                all_profiles,
                secrets,
                known_hosts,
                command_receiver,
                event_sender.clone(),
                columns,
                lines,
                monitoring_enabled,
            )) {
                if event_sender
                    .unbounded_send(SessionEvent::Error(error.to_string()))
                    .is_err()
                {
                    return;
                }

                let _ = event_sender.unbounded_send(SessionEvent::Closed);
            }
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

    let (event_sender, event_receiver) = futures_unbounded();
    let (command_sender, mut command_receiver) = unbounded_channel();
    let non_interactive_error = Arc::new(Mutex::new(None::<String>));
    let non_interactive_error_for_events = non_interactive_error.clone();
    let event_command_sender = command_sender.clone();
    let event_task = tokio::spawn(async move {
        let mut event_receiver = event_receiver;
        while let Some(event) = event_receiver.next().await {
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

    let (event_sender, event_receiver) = futures_unbounded();
    let (command_sender, mut command_receiver) = unbounded_channel();
    let non_interactive_error = Arc::new(Mutex::new(None::<String>));
    let non_interactive_error_for_events = non_interactive_error.clone();
    let event_command_sender = command_sender.clone();
    let event_task = tokio::spawn(async move {
        let mut event_receiver = event_receiver;
        while let Some(event) = event_receiver.next().await {
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
    pub(super) event_sender: FuturesUnboundedSender<SessionEvent>,
    pub(super) decision_inbox: Arc<Mutex<Option<oneshot::Receiver<HostKeyDecision>>>>,
    pub(super) remote_forward_targets: RemoteForwardTargets,
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
            .unbounded_send(SessionEvent::HostKeyPrompt(prompt))
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
                        .unbounded_send(SessionEvent::Status(format!(
                            "Could not save host key: {error}"
                        )));
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
        async move {
            match connect_local_agent_stream().await {
                Ok(mut agent_stream) => {
                    let mut forwarded = channel.into_stream();
                    if let Err(error) = copy_bidirectional(&mut forwarded, &mut agent_stream).await
                    {
                        log::warn!("agent forwarding relay ended with error: {error:?}");
                    }
                }
                Err(error) => {
                    let _ = event_sender.unbounded_send(SessionEvent::Status(format!(
                        "Agent forwarding unavailable: {error}"
                    )));
                    if let Err(close_error) = channel.close().await {
                        log::debug!("failed to close forwarded agent channel: {close_error:?}");
                    }
                }
            }

            Ok(())
        }
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

        async move {
            let connected_port = match u16::try_from(connected_port) {
                Ok(port) => port,
                Err(_) => {
                    emit_port_forward_notice(
                        &event_sender,
                        format!(
                            "Remote forwarding used an unsupported port value for {}",
                            connected_address
                        ),
                    );
                    if let Err(error) = channel.close().await {
                        log::debug!("failed to close forwarded tcpip channel: {error:?}");
                    }
                    return Ok(());
                }
            };

            let target = match remote_forward_targets.lock() {
                Ok(targets) => targets
                    .get(&(connected_address.clone(), connected_port))
                    .cloned(),
                Err(_) => {
                    emit_port_forward_notice(
                        &event_sender,
                        "Remote forwarding target registry is unavailable",
                    );
                    None
                }
            };

            let Some(target) = target else {
                emit_port_forward_notice(
                    &event_sender,
                    format!(
                        "No local target is registered for remote forward {}:{}",
                        connected_address, connected_port
                    ),
                );
                if let Err(error) = channel.close().await {
                    log::debug!("failed to close forwarded tcpip channel: {error:?}");
                }
                return Ok(());
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
                        );
                    }
                }
                Err(error) => {
                    emit_port_forward_notice(
                        &event_sender,
                        format!(
                            "Remote forward {} could not reach local target {}: {}",
                            target.label, local_target, error
                        ),
                    );
                    if let Err(close_error) = channel.close().await {
                        log::debug!(
                            "failed to close forwarded tcpip channel after connect error: {close_error:?}"
                        );
                    }
                }
            }

            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_session(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    mut command_receiver: UnboundedReceiver<SessionCommand>,
    event_sender: FuturesUnboundedSender<SessionEvent>,
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
            .unbounded_send(SessionEvent::Status("SSH agent forwarding enabled".into()));
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
        .unbounded_send(SessionEvent::Connected(remote_label))
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
                        if event_sender.unbounded_send(SessionEvent::Output(data.to_vec())).is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        if event_sender.unbounded_send(SessionEvent::Output(data.to_vec())).is_err() {
                            break;
                        }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        if event_sender.unbounded_send(SessionEvent::Status(format!("Remote exit status: {exit_status}"))).is_err() {
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

    if event_sender.unbounded_send(SessionEvent::Closed).is_err() {
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
    event_sender: &FuturesUnboundedSender<SessionEvent>,
    remote_forward_targets: &RemoteForwardTargets,
) -> (
    ClientHandler,
    Arc<Mutex<Option<oneshot::Sender<HostKeyDecision>>>>,
) {
    let (decision_sender, decision_receiver) = oneshot::channel();
    let decision_inbox = Arc::new(Mutex::new(Some(decision_receiver)));
    let pending_decision = Arc::new(Mutex::new(Some(decision_sender)));

    (
        ClientHandler {
            known_hosts,
            host: profile.host.clone(),
            port: profile.port,
            event_sender: event_sender.clone(),
            decision_inbox,
            remote_forward_targets: remote_forward_targets.clone(),
        },
        pending_decision,
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
    event_sender: &FuturesUnboundedSender<SessionEvent>,
) -> Result<ConnectedSession> {
    let profile = hydrate_profile_from_secrets(profile, &secrets);
    let remote_label = profile.connection_label();
    let proxy_jump_profiles = connection::resolve_proxy_jump_profiles(&profile, &all_profiles)?
        .into_iter()
        .map(|profile| hydrate_profile_from_secrets(profile, &secrets))
        .collect::<Vec<_>>();
    let mut configured_port_forward_rules = profile.port_forwarding_rules.clone();
    let remote_forward_targets = Arc::new(Mutex::new(HashMap::new()));
    let config = connection::default_client_config();
    let mut jump_sessions = Vec::new();

    let session = if let Some(first_hop) = proxy_jump_profiles.first() {
        emit_status(
            event_sender,
            format!(
                "Connecting to jump host 1/{}: {}",
                proxy_jump_profiles.len(),
                first_hop.connection_label()
            ),
        )?;

        let mut current_session = connect_profile_session(
            first_hop,
            config.clone(),
            known_hosts.clone(),
            command_receiver,
            event_sender,
            &remote_forward_targets,
            &mut configured_port_forward_rules,
        )
        .await?;
        emit_status(
            event_sender,
            format!("Authenticating jump host 1/{}", proxy_jump_profiles.len()),
        )?;
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
            emit_status(event_sender, status)?;

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

            let mut next_session = connect_profile_stream(
                &next_profile,
                transport,
                config.clone(),
                known_hosts.clone(),
                command_receiver,
                event_sender,
                &remote_forward_targets,
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
            )?;
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
        }

        current_session
    } else {
        emit_status(event_sender, format!("Connecting to {remote_label}"))?;
        let mut session = connect_profile_session(
            &profile,
            config,
            known_hosts,
            command_receiver,
            event_sender,
            &remote_forward_targets,
            &mut configured_port_forward_rules,
        )
        .await?;
        emit_status(event_sender, format!("Authenticating {remote_label}"))?;
        authenticate_full(
            &mut session,
            profile,
            &secrets,
            command_receiver,
            event_sender,
        )
        .await?;
        Arc::new(session)
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
    event_sender: &FuturesUnboundedSender<SessionEvent>,
    remote_forward_targets: &RemoteForwardTargets,
    configured_port_forward_rules: &mut Vec<PortForwardRule>,
) -> Result<client::Handle<ClientHandler>> {
    let (handler, pending_decision) =
        build_client_handler(profile, known_hosts, event_sender, remote_forward_targets);
    let host = profile.host.clone();
    let port = profile.port;
    let connect_future = async move {
        client::connect(config, (host.clone(), port), handler)
            .await
            .with_context(|| format!("failed to connect to {}:{}", host, port))
    };

    await_session_connect(
        connect_future,
        command_receiver,
        pending_decision,
        configured_port_forward_rules,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn connect_profile_stream<R>(
    profile: &SessionProfile,
    transport: R,
    config: Arc<client::Config>,
    known_hosts: KnownHostsStore,
    command_receiver: &mut UnboundedReceiver<SessionCommand>,
    event_sender: &FuturesUnboundedSender<SessionEvent>,
    remote_forward_targets: &RemoteForwardTargets,
    configured_port_forward_rules: &mut Vec<PortForwardRule>,
) -> Result<client::Handle<ClientHandler>>
where
    R: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (handler, pending_decision) =
        build_client_handler(profile, known_hosts, event_sender, remote_forward_targets);
    let host = profile.host.clone();
    let port = profile.port;
    let connect_future = async move {
        client::connect_stream(config, transport, handler)
            .await
            .with_context(|| format!("failed to connect to {}:{} through ProxyJump", host, port))
    };

    await_session_connect(
        connect_future,
        command_receiver,
        pending_decision,
        configured_port_forward_rules,
    )
    .await
}

fn emit_status(event_sender: &FuturesUnboundedSender<SessionEvent>, message: String) -> Result<()> {
    if event_sender
        .unbounded_send(SessionEvent::Status(message))
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
