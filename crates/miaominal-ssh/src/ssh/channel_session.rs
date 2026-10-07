//! Reusable SSH channel sessions for structured programs and loopback tunnels.
//!
//! The interactive terminal session owns a PTY and a single shell channel. AI
//! CLI sessions need different primitives on top of the same authentication
//! route (profiles, proxies, jump hosts, known hosts, credentials):
//!
//! - an `exec` channel whose stdin/stdout/stderr/exit status stay separate, for
//!   newline-delimited JSON-RPC CLIs such as `codex app-server`;
//! - a temporary local listener that forwards to a remote loopback port, for
//!   HTTP/SSE servers such as `opencode serve --hostname 127.0.0.1`.
//!
//! This module exposes exactly those primitives. The SSH worker thread owns the
//! connection; the host drives it through [`ChannelCommand`] and consumes
//! [`ChannelEvent`]. Protocol byte queues are bounded and must never be dropped,
//! so a slow consumer naturally backpressures the SSH window instead of losing
//! data.

use super::session::{
    AuthenticationRequest, CancelToken, ClientHandler, ConnectedSession, ConnectionCancelled,
    HostKeyRequest, SessionCommand, SessionEvent, connect_authenticated_session_internal,
    session_event_channel,
};
use anyhow::{Result, bail};
use miaominal_core::forwarding::HostKeyDecision;
use miaominal_core::profile::SessionProfile;
use miaominal_core::proxy::ProxyProfile;
use miaominal_secrets::SecretStore;
use miaominal_storage::KnownHostsStore;
use russh::client;
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Disconnect, Sig};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::copy_bidirectional;
use tokio::net::TcpListener;
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, channel, unbounded_channel};
use tokio::sync::oneshot;
use tokio::task::{JoinHandle, JoinSet};

/// Capacity of the reliable channel-event queue between the SSH worker and the
/// host. The queue is intentionally bounded: stdout/stderr protocol bytes are
/// lossless, so a slow consumer stalls the SSH window instead of dropping data.
pub const CHANNEL_EVENT_QUEUE_CAPACITY: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Exec,
    Tunnel,
}

/// Typed events produced by one channel session.
#[derive(Debug)]
pub enum ChannelEvent {
    /// Authentication finished and the connection is usable.
    Ready,
    /// The server accepted an `exec` request.
    ExecOpened { channel_id: u64 },
    /// The server refused an `exec` request.
    ExecFailed { channel_id: u64, message: String },
    Stdout { channel_id: u64, data: Vec<u8> },
    Stderr { channel_id: u64, data: Vec<u8> },
    /// The executed program finished; `exit_code` and `signal` are exclusive.
    ExecExited {
        channel_id: u64,
        exit_code: Option<u32>,
        signal: Option<String>,
    },
    /// The channel ended without an exit status.
    ChannelClosed { channel_id: u64 },
    /// A local loopback listener is bound.
    TunnelReady { tunnel_id: u64, local_port: u16 },
    /// A tunnel could not be opened or a forwarded connection failed.
    TunnelFailed { tunnel_id: u64, message: String },
    /// A tunnel was closed explicitly.
    TunnelClosed { tunnel_id: u64 },
    HostKeyPrompt(HostKeyRequest),
    AuthenticationPrompt(AuthenticationRequest),
    /// A queued answer arrived after its request was superseded.
    RequestExpired { request_id: u64 },
    Error(String),
    /// The whole channel session is gone; no further events follow.
    Closed,
}

impl ChannelEvent {
    /// Whether the event terminates the session stream.
    pub fn is_terminal(&self) -> bool {
        matches!(self, ChannelEvent::Closed)
    }
}

/// Commands accepted by a running channel session.
#[derive(Debug)]
pub enum ChannelCommand {
    OpenExec {
        channel_id: u64,
        command: String,
        environment: Vec<(String, String)>,
    },
    WriteStdin {
        channel_id: u64,
        data: Vec<u8>,
    },
    Signal {
        channel_id: u64,
        signal: String,
    },
    CloseChannel {
        channel_id: u64,
    },
    OpenTunnel {
        tunnel_id: u64,
        remote_host: String,
        remote_port: u16,
    },
    CloseTunnel {
        tunnel_id: u64,
    },
    HostKeyDecision {
        request_id: u64,
        decision: HostKeyDecision,
    },
    KeyboardInteractiveResponse {
        request_id: u64,
        answers: Vec<String>,
    },
    /// Close the connection and every channel on it.
    Close,
}

/// Cloneable command handle for one channel session.
#[derive(Clone, Debug)]
pub struct ChannelCommandSender {
    sender: UnboundedSender<ChannelCommand>,
    cancel: CancelToken,
}

impl ChannelCommandSender {
    fn new(sender: UnboundedSender<ChannelCommand>, cancel: CancelToken) -> Self {
        Self { sender, cancel }
    }

    fn send(&self, command: ChannelCommand) -> Result<()> {
        self.sender
            .send(command)
            .map_err(|_| anyhow::anyhow!("channel session is no longer available"))
    }

    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    pub fn open_exec(
        &self,
        channel_id: u64,
        command: String,
        environment: Vec<(String, String)>,
    ) -> Result<()> {
        self.send(ChannelCommand::OpenExec {
            channel_id,
            command,
            environment,
        })
    }

    pub fn write_stdin(&self, channel_id: u64, data: Vec<u8>) -> Result<()> {
        self.send(ChannelCommand::WriteStdin { channel_id, data })
    }

    pub fn signal(&self, channel_id: u64, signal: String) -> Result<()> {
        self.send(ChannelCommand::Signal { channel_id, signal })
    }

    pub fn close_channel(&self, channel_id: u64) -> Result<()> {
        self.send(ChannelCommand::CloseChannel { channel_id })
    }

    pub fn open_tunnel(&self, tunnel_id: u64, remote_host: String, remote_port: u16) -> Result<()> {
        self.send(ChannelCommand::OpenTunnel {
            tunnel_id,
            remote_host,
            remote_port,
        })
    }

    pub fn close_tunnel(&self, tunnel_id: u64) -> Result<()> {
        self.send(ChannelCommand::CloseTunnel { tunnel_id })
    }

    pub fn respond_host_key(&self, request_id: u64, decision: HostKeyDecision) -> Result<()> {
        self.send(ChannelCommand::HostKeyDecision {
            request_id,
            decision,
        })
    }

    pub fn respond_keyboard_interactive(
        &self,
        request_id: u64,
        answers: Vec<String>,
    ) -> Result<()> {
        self.send(ChannelCommand::KeyboardInteractiveResponse { request_id, answers })
    }

    /// Explicit close, including connections still inside a non-cancellable
    /// await (TCP connect, SSH handshake, authentication).
    pub fn close(&self) -> Result<()> {
        self.cancel.cancel();
        self.send(ChannelCommand::Close)
    }
}

/// Reliable event receiver for one channel session.
pub struct ChannelEventReceiver {
    receiver: tokio::sync::mpsc::Receiver<ChannelEvent>,
    // Completion must not wait for room in the reliable data queue. The
    // receiver delivers it after all already-queued protocol bytes.
    terminal: Option<oneshot::Receiver<Option<String>>>,
    terminal_events: VecDeque<ChannelEvent>,
}

impl ChannelEventReceiver {
    pub async fn recv(&mut self) -> Option<ChannelEvent> {
        loop {
            match self.try_recv() {
                Ok(event) => return Some(event),
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)
                    if self.terminal.is_none() =>
                {
                    return None;
                }
                Err(_) => {}
            }
            let Some(terminal) = self.terminal.as_mut() else {
                return self.receiver.recv().await;
            };
            tokio::select! {
                event = self.receiver.recv() => {
                    if event.is_some() {
                        return event;
                    }
                    let result = self.terminal.as_mut().expect("completion receiver").await;
                    self.finish(result.unwrap_or(None));
                }
                result = terminal => self.finish(result.unwrap_or(None)),
            }
        }
    }

    pub fn try_recv(
        &mut self,
    ) -> std::result::Result<ChannelEvent, tokio::sync::mpsc::error::TryRecvError> {
        if let Some(terminal) = self.terminal.as_mut() {
            match terminal.try_recv() {
                Ok(error) => self.finish(error),
                Err(oneshot::error::TryRecvError::Closed) => self.finish(None),
                Err(oneshot::error::TryRecvError::Empty) => {}
            }
        }
        self.receiver
            .try_recv()
            .or_else(|error| self.terminal_events.pop_front().ok_or(error))
    }

    fn finish(&mut self, error: Option<String>) {
        self.terminal = None;
        self.receiver.close();
        if let Some(error) = error {
            self.terminal_events.push_back(ChannelEvent::Error(error));
        }
        self.terminal_events.push_back(ChannelEvent::Closed);
    }
}

/// One running channel session.
pub struct ChannelConnection {
    pub commands: ChannelCommandSender,
    pub events: ChannelEventReceiver,
    /// Worker thread; `None` for host-provided backends.
    pub thread: Option<std::thread::JoinHandle<()>>,
}

impl ChannelConnection {
    /// Take the worker thread handle so one waiter can join it.
    pub fn take_thread(&mut self) -> Option<std::thread::JoinHandle<()>> {
        self.thread.take()
    }

    /// Take the event receiver so a non-terminal consumer can forward typed
    /// events itself; the field keeps a placeholder that never yields events.
    pub fn take_events(&mut self) -> ChannelEventReceiver {
        let placeholder = ChannelEventReceiver {
            receiver: tokio::sync::mpsc::channel(1).1,
            terminal: None,
            terminal_events: VecDeque::new(),
        };
        std::mem::replace(&mut self.events, placeholder)
    }
}

/// Start one channel session through the shared authenticated connector.
///
/// The returned connection is ready for commands immediately; `Ready` arrives
/// once authentication finishes. Host-key and keyboard-interactive prompts are
/// relayed to the host so a first-time trust decision can still be made.
pub fn start_channel_session(
    runtime: &TokioHandle,
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
) -> ChannelConnection {
    let (event_sender, event_receiver) = channel(CHANNEL_EVENT_QUEUE_CAPACITY);
    let (terminal_sender, terminal_receiver) = oneshot::channel();
    let (command_sender, command_receiver) = unbounded_channel();
    let runtime = runtime.clone();
    let cancel = CancelToken::default();
    let worker_cancel = cancel.clone();

    let thread = std::thread::Builder::new()
        .name(format!("ssh-channel-{}", profile.id))
        .spawn(move || {
            runtime.block_on(async move {
                let result = tokio::select! {
                    result = run_channel_session(
                        profile,
                        all_profiles,
                        all_proxies,
                        secrets,
                        known_hosts,
                        command_receiver,
                        event_sender.clone(),
                    ) => result,
                    () = worker_cancel.cancelled() => Err(ConnectionCancelled.into()),
                };

                let error = result
                    .err()
                    .filter(|error| error.downcast_ref::<ConnectionCancelled>().is_none())
                    .map(|error| format!("{error:#}"));
                let _ = terminal_sender.send(error);
            });
        })
        .expect("failed to spawn SSH channel session thread");

    ChannelConnection {
        commands: ChannelCommandSender::new(command_sender, cancel),
        events: ChannelEventReceiver {
            receiver: event_receiver,
            terminal: Some(terminal_receiver),
            terminal_events: VecDeque::new(),
        },
        thread: Some(thread),
    }
}

struct AbortTaskOnDrop<T>(JoinHandle<T>);

impl<T> Drop for AbortTaskOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_channel_session(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    mut command_receiver: UnboundedReceiver<ChannelCommand>,
    event_sender: tokio::sync::mpsc::Sender<ChannelEvent>,
) -> Result<()> {
    // The shared connector answers host-key and keyboard-interactive prompts
    // through a `SessionCommand` queue; relay those from our own command queue
    // while the connect future is pending.
    let (session_commands, mut session_command_receiver) = unbounded_channel::<SessionCommand>();
    let (connect_events, mut connect_event_receiver) = session_event_channel();

    let connect = connect_authenticated_session_internal(
        profile,
        all_profiles,
        all_proxies,
        secrets,
        known_hosts,
        &mut session_command_receiver,
        &connect_events,
    );
    tokio::pin!(connect);

    let mut pending_commands: Vec<ChannelCommand> = Vec::new();
    let connected = loop {
        tokio::select! {
            result = &mut connect => break result?,
            Some(command) = command_receiver.recv() => {
                match command {
                    ChannelCommand::HostKeyDecision { request_id, decision } => {
                        let _ = session_commands.send(SessionCommand::HostKeyDecision {
                            request_id,
                            decision,
                        });
                    }
                    ChannelCommand::KeyboardInteractiveResponse { request_id, answers } => {
                        let _ = session_commands.send(SessionCommand::KeyboardInteractiveResponse {
                            request_id,
                            answers,
                        });
                    }
                    ChannelCommand::Close => {
                        let _ = session_commands.send(SessionCommand::Close);
                        bail!("channel session closed during connect");
                    }
                    other => pending_commands.push(other),
                }
            }
            Some(event) = connect_event_receiver.recv() => {
                if !forward_connection_event(&event_sender, event).await? {
                    bail!("channel session closed during connect");
                }
            }
        }
    };

    let ConnectedSession {
        session: handle,
        jump_sessions: _jump_sessions,
        ..
    } = connected;

    if event_sender.send(ChannelEvent::Ready).await.is_err() {
        return Ok(());
    }

    // Drain prompts that were already queued when connect resolved.
    while let Ok(event) = connect_event_receiver.try_recv() {
        let _ = forward_connection_event(&event_sender, event).await?;
    }

    let mut writers: HashMap<u64, ChannelWriteHalf<client::Msg>> = HashMap::new();
    let mut readers: HashMap<u64, AbortTaskOnDrop<()>> = HashMap::new();
    let mut tunnels: HashMap<u64, AbortTaskOnDrop<()>> = HashMap::new();
    for command in pending_commands.drain(..) {
        apply_channel_command(
            command,
            &handle,
            &event_sender,
            &mut writers,
            &mut readers,
            &mut tunnels,
        )
        .await;
    }

    let mut health_check = tokio::time::interval(Duration::from_millis(100));
    health_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let transport_closed = loop {
        tokio::select! {
            command = command_receiver.recv() => {
                match command {
                    Some(ChannelCommand::Close) | None => break false,
                    Some(command) => {
                        apply_channel_command(
                            command,
                            &handle,
                            &event_sender,
                            &mut writers,
                            &mut readers,
                            &mut tunnels,
                        )
                        .await;
                    }
                }
            }
            Some(event) = connect_event_receiver.recv() => {
                if !forward_connection_event(&event_sender, event).await? {
                    break false;
                }
            }
            _ = health_check.tick() => {
                if handle.is_closed() {
                    break true;
                }
            }
        }
    };

    // Orderly teardown: abort readers/listeners, close writers, disconnect the
    // SSH handle, then report the terminal event.
    for (_, writer) in writers.drain() {
        let _ = writer.eof().await;
        let _ = writer.close().await;
    }
    if transport_closed {
        // russh closes channel receivers with the transport. Let readers
        // forward buffered output and exit status before publishing Closed.
        // Explicit cancellation still interrupts these backpressured waits.
        for (_, mut reader) in readers.drain() {
            let _ = (&mut reader.0).await;
        }
    }
    readers.clear();
    tunnels.clear();
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        handle.disconnect(Disconnect::ByApplication, "AI session ended", ""),
    )
    .await;
    Ok(())
}

async fn apply_channel_command(
    command: ChannelCommand,
    handle: &Arc<client::Handle<ClientHandler>>,
    event_sender: &tokio::sync::mpsc::Sender<ChannelEvent>,
    writers: &mut HashMap<u64, ChannelWriteHalf<client::Msg>>,
    readers: &mut HashMap<u64, AbortTaskOnDrop<()>>,
    tunnels: &mut HashMap<u64, AbortTaskOnDrop<()>>,
) {
    match command {
        ChannelCommand::OpenExec {
            channel_id,
            command,
            environment,
        } => {
            if writers.contains_key(&channel_id) {
                let _ = send_event(
                    event_sender,
                    ChannelEvent::ExecFailed {
                        channel_id,
                        message: "channel id is already open".to_string(),
                    },
                )
                .await;
                return;
            }
            let channel = match handle.channel_open_session().await {
                Ok(channel) => channel,
                Err(error) => {
                    let _ = send_event(
                        event_sender,
                        ChannelEvent::ExecFailed {
                            channel_id,
                            message: format!("failed to open SSH session channel: {error}"),
                        },
                    )
                    .await;
                    return;
                }
            };
            for (name, value) in environment {
                // RFC 4254 environment requests may be ignored by the server;
                // the CLI then falls back to its own configuration.
                let _ = channel.set_env(false, name, value).await;
            }
            let (mut read_half, write_half) = channel.split();
            if let Err(error) = write_half.exec(true, command).await {
                let _ = send_event(
                    event_sender,
                    ChannelEvent::ExecFailed {
                        channel_id,
                        message: format!("failed to start remote command: {error}"),
                    },
                )
                .await;
                return;
            }
            writers.insert(channel_id, write_half);
            let sender = event_sender.clone();
            let reader = tokio::spawn(async move {
                read_exec_channel(channel_id, &mut read_half, &sender).await;
            });
            readers.insert(channel_id, AbortTaskOnDrop(reader));
        }
        ChannelCommand::WriteStdin { channel_id, data } => match writers.get(&channel_id) {
            Some(writer) => {
                if let Err(error) = writer.data_bytes(data).await {
                    let _ = send_event(
                        event_sender,
                        ChannelEvent::Error(format!(
                            "failed to write to channel {channel_id}: {error}"
                        )),
                    )
                    .await;
                }
            }
            None => {
                let _ = send_event(
                    event_sender,
                    ChannelEvent::Error(format!("channel {channel_id} is not open")),
                )
                .await;
            }
        },
        ChannelCommand::Signal { channel_id, signal } => {
            if let Some(writer) = writers.get(&channel_id) {
                if let Err(error) = writer.signal(parse_signal(&signal)).await {
                    let _ = send_event(
                        event_sender,
                        ChannelEvent::Error(format!(
                            "failed to signal channel {channel_id}: {error}"
                        )),
                    )
                    .await;
                }
            }
        }
        ChannelCommand::CloseChannel { channel_id } => {
            if let Some(writer) = writers.remove(&channel_id) {
                let _ = writer.eof().await;
                let _ = writer.close().await;
            }
            // Keep the reader alive: the server still sends its exit status
            // before the channel closes. The reader ends on Eof/Close and is
            // aborted with the rest of the session during teardown.
        }
        ChannelCommand::OpenTunnel {
            tunnel_id,
            remote_host,
            remote_port,
        } => {
            let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
                Ok(listener) => listener,
                Err(error) => {
                    let _ = send_event(
                        event_sender,
                        ChannelEvent::TunnelFailed {
                            tunnel_id,
                            message: format!("failed to bind loopback listener: {error}"),
                        },
                    )
                    .await;
                    return;
                }
            };
            let local_port = match listener.local_addr() {
                Ok(address) => address.port(),
                Err(error) => {
                    let _ = send_event(
                        event_sender,
                        ChannelEvent::TunnelFailed {
                            tunnel_id,
                            message: format!("failed to read loopback listener address: {error}"),
                        },
                    )
                    .await;
                    return;
                }
            };
            let sender = event_sender.clone();
            let handle = Arc::clone(handle);
            let tunnel = tokio::spawn(async move {
                if send_event(
                    &sender,
                    ChannelEvent::TunnelReady {
                        tunnel_id,
                        local_port,
                    },
                )
                .await
                .is_err()
                {
                    return;
                }
                // Dropping the listener task must also cancel its accepted
                // connections. Reap completed relays while the tunnel is open.
                let mut connections = JoinSet::new();
                loop {
                    let (stream, _) = tokio::select! {
                        accepted = listener.accept() => match accepted {
                            Ok(accepted) => accepted,
                            Err(_) => break,
                        },
                        _ = connections.join_next(), if !connections.is_empty() => continue,
                    };
                    let sender = sender.clone();
                    let handle = Arc::clone(&handle);
                    let target_host = remote_host.clone();
                    connections.spawn(async move {
                        match handle
                            .channel_open_direct_tcpip(
                                target_host.clone(),
                                u32::from(remote_port),
                                "127.0.0.1",
                                0,
                            )
                            .await
                        {
                            Ok(channel) => {
                                let mut forwarded = channel.into_stream();
                                let mut stream = stream;
                                let _ = copy_bidirectional(&mut forwarded, &mut stream).await;
                            }
                            Err(error) => {
                                let _ = send_event(
                                    &sender,
                                    ChannelEvent::TunnelFailed {
                                        tunnel_id,
                                        message: format!(
                                            "failed to reach {target_host}:{remote_port}: {error}"
                                        ),
                                    },
                                )
                                .await;
                            }
                        }
                    });
                }
            });
            tunnels.insert(tunnel_id, AbortTaskOnDrop(tunnel));
        }
        ChannelCommand::CloseTunnel { tunnel_id } => {
            if let Some(mut tunnel) = tunnels.remove(&tunnel_id) {
                tunnel.0.abort();
                let _ = (&mut tunnel.0).await;
            }
            let _ = send_event(event_sender, ChannelEvent::TunnelClosed { tunnel_id }).await;
        }
        // Late answers after the connection phase are dropped; the request
        // either was already resolved or expired.
        ChannelCommand::HostKeyDecision { .. }
        | ChannelCommand::KeyboardInteractiveResponse { .. } => {}
        ChannelCommand::Close => {}
    }
}

async fn read_exec_channel(
    channel_id: u64,
    read_half: &mut ChannelReadHalf,
    event_sender: &tokio::sync::mpsc::Sender<ChannelEvent>,
) {
    let mut exit: Option<(Option<u32>, Option<String>)> = None;
    let mut eof = false;
    while let Some(message) = read_half.wait().await {
        match message {
            ChannelMsg::Success => {
                if send_event(event_sender, ChannelEvent::ExecOpened { channel_id })
                    .await
                    .is_err()
                {
                    return;
                }
            }
            ChannelMsg::Failure => {
                let _ = send_event(
                    event_sender,
                    ChannelEvent::ExecFailed {
                        channel_id,
                        message: "server refused the exec request".to_string(),
                    },
                )
                .await;
                return;
            }
            ChannelMsg::Data { data } => {
                if send_event(
                    event_sender,
                    ChannelEvent::Stdout {
                        channel_id,
                        data: data.to_vec(),
                    },
                )
                .await
                .is_err()
                {
                    return;
                }
            }
            ChannelMsg::ExtendedData { data, ext } => {
                let event = if ext == 1 {
                    ChannelEvent::Stderr {
                        channel_id,
                        data: data.to_vec(),
                    }
                } else {
                    ChannelEvent::Stdout {
                        channel_id,
                        data: data.to_vec(),
                    }
                };
                if send_event(event_sender, event).await.is_err() {
                    return;
                }
            }
            ChannelMsg::ExitStatus { exit_status } => {
                exit = Some((Some(exit_status), None));
            }
            ChannelMsg::ExitSignal { signal_name, .. } => {
                exit = Some((None, Some(format!("{signal_name:?}"))));
            }
            // EOF ends output, not the channel: exit status may follow it.
            ChannelMsg::Eof => eof = true,
            ChannelMsg::Close => break,
            _ => {}
        }
        // Once both output and process status are complete, a peer need not
        // send another message for us to report the process exit.
        if eof && exit.is_some() {
            break;
        }
    }
    let event = match exit {
        Some((exit_code, signal)) => ChannelEvent::ExecExited {
            channel_id,
            exit_code,
            signal,
        },
        None => ChannelEvent::ChannelClosed { channel_id },
    };
    let _ = send_event(event_sender, event).await;
}

/// Relay connection-phase events (prompts, errors, closure) to the host.
/// Returns `false` when the receiver is gone.
async fn forward_connection_event(
    event_sender: &tokio::sync::mpsc::Sender<ChannelEvent>,
    event: SessionEvent,
) -> Result<bool> {
    let forwarded = match event {
        SessionEvent::HostKeyPrompt(request) => Some(ChannelEvent::HostKeyPrompt(request)),
        SessionEvent::KeyboardInteractivePrompt(request) => {
            Some(ChannelEvent::AuthenticationPrompt(request))
        }
        SessionEvent::RequestExpired { request_id } => {
            Some(ChannelEvent::RequestExpired { request_id })
        }
        SessionEvent::Error(message) => Some(ChannelEvent::Error(message)),
        SessionEvent::Closed => return Ok(false),
        _ => None,
    };
    match forwarded {
        Some(event) => Ok(send_event(event_sender, event).await.is_ok()),
        None => Ok(true),
    }
}

async fn send_event(
    event_sender: &tokio::sync::mpsc::Sender<ChannelEvent>,
    event: ChannelEvent,
) -> std::result::Result<(), tokio::sync::mpsc::error::SendError<ChannelEvent>> {
    event_sender.send(event).await
}

/// Map a host-supplied signal name to the SSH signal enum.
pub fn parse_signal(name: &str) -> Sig {
    match name.trim().to_ascii_uppercase().as_str() {
        "ABRT" => Sig::ABRT,
        "ALRM" => Sig::ALRM,
        "FPE" => Sig::FPE,
        "HUP" => Sig::HUP,
        "ILL" => Sig::ILL,
        "INT" => Sig::INT,
        "KILL" => Sig::KILL,
        "PIPE" => Sig::PIPE,
        "QUIT" => Sig::QUIT,
        "SEGV" => Sig::SEGV,
        "TERM" => Sig::TERM,
        "USR1" => Sig::USR1,
        other => Sig::Custom(other.to_string()),
    }
}

/// Escape one path component for POSIX shells.
pub fn shell_escape_path(path: &str) -> String {
    if path.is_empty() {
        return "''".to_string();
    }
    let safe = path
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"-_./@%+,:=".contains(&byte));
    if safe {
        return path.to_string();
    }
    let mut escaped = String::with_capacity(path.len() + 2);
    escaped.push('\'');
    for character in path.chars() {
        if character == '\'' {
            escaped.push_str("'\\''");
        } else {
            escaped.push(character);
        }
    }
    escaped.push('\'');
    escaped
}

/// Build the remote command line that starts a CLI in `cwd` without relying on
/// login-shell startup files. The whole command is a single shell invocation so
/// the CLI keeps its own argument quoting.
pub fn build_cwd_command(cwd: &str, command: &str) -> String {
    format!("cd {} && {}", shell_escape_path(cwd), command)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[tokio::test]
    async fn completion_does_not_wait_for_a_full_queue_and_follows_buffered_data() {
        let (sender, receiver) = channel(2);
        let (terminal_sender, terminal) = oneshot::channel();
        let mut events = ChannelEventReceiver {
            receiver,
            terminal: Some(terminal),
            terminal_events: VecDeque::new(),
        };
        for byte in [1, 2] {
            sender
                .send(ChannelEvent::Stdout {
                    channel_id: 1,
                    data: vec![byte],
                })
                .await
                .unwrap();
        }
        // Keep the data sender alive, as a detached relay might do. Worker
        // completion still closes the stream without waiting for that sender.
        terminal_sender
            .send(Some("transport failed".into()))
            .unwrap();
        for byte in [1, 2] {
            assert!(matches!(events.recv().await,
                Some(ChannelEvent::Stdout { data, .. }) if data == vec![byte]));
        }
        assert!(
            matches!(events.recv().await, Some(ChannelEvent::Error(message)) if message == "transport failed")
        );
        assert!(matches!(events.recv().await, Some(ChannelEvent::Closed)));
        assert!(events.recv().await.is_none());
        assert!(sender.send(ChannelEvent::Ready).await.is_err());
    }

    #[tokio::test]
    async fn completion_wakes_an_idle_receiver() {
        let (_sender, receiver) = channel(1);
        let (terminal_sender, terminal) = oneshot::channel();
        let mut events = ChannelEventReceiver {
            receiver,
            terminal: Some(terminal),
            terminal_events: VecDeque::new(),
        };
        let waiter = tokio::spawn(async move { events.recv().await });
        terminal_sender.send(None).unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), waiter)
                .await
                .unwrap()
                .unwrap(),
            Some(ChannelEvent::Closed)
        ));
    }

    #[tokio::test]
    async fn cancelled_read_keeps_the_completion_receiver() {
        let (sender, receiver) = channel(1);
        let (terminal_sender, terminal) = oneshot::channel();
        let mut events = ChannelEventReceiver {
            receiver,
            terminal: Some(terminal),
            terminal_events: VecDeque::new(),
        };
        drop(sender);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), events.recv())
                .await
                .is_err()
        );
        terminal_sender
            .send(Some("connection failed".into()))
            .unwrap();
        assert!(
            matches!(events.recv().await, Some(ChannelEvent::Error(message)) if message == "connection failed")
        );
        assert!(matches!(events.recv().await, Some(ChannelEvent::Closed)));
    }

    #[test]
    fn paths_are_escaped_with_single_quotes() {
        assert_eq!(shell_escape_path("/home/me/project"), "/home/me/project");
        assert_eq!(shell_escape_path("/home/me/my project"), "'/home/me/my project'");
        assert_eq!(shell_escape_path("/home/me/it's"), "'/home/me/it'\\''s'");
        assert_eq!(shell_escape_path(""), "''");
        assert_eq!(shell_escape_path("/tmp/$USER"), "'/tmp/$USER'");
        assert_eq!(shell_escape_path("/tmp/a;b"), "'/tmp/a;b'");
    }

    #[test]
    fn cwd_command_escapes_once() {
        assert_eq!(
            build_cwd_command("/home/me/a b", "codex app-server"),
            "cd '/home/me/a b' && codex app-server"
        );
    }

    #[test]
    fn signal_names_map_to_the_protocol_enum() {
        assert!(matches!(parse_signal("term"), Sig::TERM));
        assert!(matches!(parse_signal("INT"), Sig::INT));
        assert!(matches!(parse_signal("SIGUSR1"), Sig::Custom(name) if name == "SIGUSR1"));
    }

    #[test]
    fn channel_events_are_sendable() {
        assert_send::<ChannelEvent>();
        assert_send::<ChannelCommand>();
        assert_send::<ChannelEventReceiver>();
    }
}
