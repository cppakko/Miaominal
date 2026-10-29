//! AI CLI channel sessions exposed to Kotlin.
//!
//! Each agent session is one dedicated SSH connection (same profiles, proxies,
//! jump hosts, credentials, and known hosts as terminal sessions) that carries
//! structured `exec` channels and temporary loopback tunnels. Terminal PTY
//! sessions and agent channels are managed separately: closing one never
//! affects the other.
//!
//! Events are delivered through a bounded, reliable queue. The host pulls them
//! with [`AgentSshSession::read_events`]; a slow consumer backpressures the SSH
//! window instead of dropping protocol bytes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use miaominal_core::forwarding::HostKeyDecision;
use tokio::sync::Mutex as AsyncMutex;

use crate::error::BackendError;
use crate::runtime;

/// Host-key decision for one pending prompt.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentHostKeyDecision {
    AcceptOnce,
    AcceptAndSave,
    Reject,
}

impl From<AgentHostKeyDecision> for HostKeyDecision {
    fn from(decision: AgentHostKeyDecision) -> Self {
        match decision {
            AgentHostKeyDecision::AcceptOnce => HostKeyDecision::AcceptOnce,
            AgentHostKeyDecision::AcceptAndSave => HostKeyDecision::AcceptAndSave,
            AgentHostKeyDecision::Reject => HostKeyDecision::Reject,
        }
    }
}

/// One pending host-key trust decision.
#[derive(uniffi::Record, Debug, Clone)]
pub struct AgentHostKeyPromptDto {
    pub request_id: u64,
    pub hop_label: String,
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
    pub previous_fingerprint: Option<String>,
    pub key_changed: bool,
}

/// One keyboard-interactive question.
#[derive(uniffi::Record, Debug, Clone)]
pub struct AgentPromptDto {
    pub prompt: String,
    pub echo: bool,
}

/// One pending keyboard-interactive challenge.
#[derive(uniffi::Record, Debug, Clone)]
pub struct AgentAuthenticationPromptDto {
    pub request_id: u64,
    pub hop_label: String,
    pub name: String,
    pub instructions: String,
    pub prompts: Vec<AgentPromptDto>,
}

/// One environment variable for an exec channel.
#[derive(uniffi::Record, Debug, Clone)]
pub struct AgentEnvVar {
    pub name: String,
    pub value: String,
}

/// Typed event produced by an agent channel session.
#[derive(uniffi::Enum, Debug, Clone)]
pub enum AgentChannelEvent {
    /// Authentication finished; exec and tunnel commands can be issued.
    Ready,
    ExecOpened {
        channel_id: u64,
    },
    ExecFailed {
        channel_id: u64,
        message: String,
    },
    Stdout {
        channel_id: u64,
        data: Vec<u8>,
    },
    Stderr {
        channel_id: u64,
        data: Vec<u8>,
    },
    ExecExited {
        channel_id: u64,
        exit_code: Option<u32>,
        signal: Option<String>,
    },
    ChannelClosed {
        channel_id: u64,
    },
    TunnelReady {
        tunnel_id: u64,
        local_port: u16,
    },
    TunnelFailed {
        tunnel_id: u64,
        message: String,
    },
    TunnelClosed {
        tunnel_id: u64,
    },
    HostKeyPrompt {
        prompt: AgentHostKeyPromptDto,
    },
    AuthenticationPrompt {
        prompt: AgentAuthenticationPromptDto,
    },
    RequestExpired {
        request_id: u64,
    },
    Error {
        message: String,
    },
    /// Terminal event: the session is gone and no further events follow.
    Closed,
}

/// One batch of channel events pulled by the host.
#[derive(uniffi::Record, Debug, Clone)]
pub struct AgentEventBatch {
    pub events: Vec<AgentChannelEvent>,
    /// True when the batch ends with the terminal `Closed` event.
    pub closed: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Shared state of one agent session; the Kotlin handle owns the strong count
/// and the registry owns it through a weak back-reference pattern instead.
pub(crate) struct AgentSessionInner {
    connection_id: u64,
    commands: miaominal_ssh::ChannelCommandSender,
    events: AsyncMutex<miaominal_ssh::ChannelEventReceiver>,
    ssh_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    closed: AtomicBool,
}

impl AgentSessionInner {
    fn new(connection_id: u64, mut connection: miaominal_ssh::ChannelConnection) -> Arc<Self> {
        Arc::new(Self {
            connection_id,
            commands: connection.commands.clone(),
            events: AsyncMutex::new(connection.take_events()),
            ssh_thread: Mutex::new(connection.take_thread()),
            closed: AtomicBool::new(false),
        })
    }

    pub(crate) fn request_close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let _ = self.commands.close();
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst) || self.commands.is_closed()
    }

    /// Wait for the worker thread to finish; safe to call repeatedly.
    pub(crate) async fn wait_for_thread(&self) {
        let thread = lock(&self.ssh_thread).take();
        if let Some(thread) = thread {
            let _ = runtime::offload(move || thread.join()).await;
        }
    }
}

/// Registry of every agent session owned by one backend handle.
pub(crate) struct AgentSessionRegistry {
    lifecycle: Arc<Mutex<crate::backend::LifecycleState>>,
    sessions: Mutex<HashMap<u64, Arc<AgentSessionInner>>>,
    next_connection_id: AtomicU64,
}

impl AgentSessionRegistry {
    pub(crate) fn new(lifecycle: Arc<Mutex<crate::backend::LifecycleState>>) -> Arc<Self> {
        Arc::new(Self {
            lifecycle,
            sessions: Mutex::new(HashMap::new()),
            next_connection_id: AtomicU64::new(1),
        })
    }

    fn next_connection_id(&self) -> u64 {
        self.next_connection_id.fetch_add(1, Ordering::SeqCst)
    }

    fn start(
        &self,
        connect: impl FnOnce() -> miaominal_ssh::ChannelConnection,
    ) -> Result<Arc<AgentSessionInner>, BackendError> {
        // Starting and registering a worker commit atomically with shutdown.
        // A context loaded before shutdown must never launch a later worker.
        let state = lock(&self.lifecycle);
        if state.closed {
            return Err(BackendError::Closed);
        }
        let inner = AgentSessionInner::new(self.next_connection_id(), connect());
        lock(&self.sessions).insert(inner.connection_id, Arc::clone(&inner));
        Ok(inner)
    }

    pub(crate) fn get(&self, connection_id: u64) -> Option<Arc<AgentSessionInner>> {
        lock(&self.sessions).get(&connection_id).cloned()
    }

    pub(crate) fn remove(&self, connection_id: u64) -> Option<Arc<AgentSessionInner>> {
        lock(&self.sessions).remove(&connection_id)
    }

    pub(crate) fn request_close_all(&self) {
        let sessions = lock(&self.sessions).values().cloned().collect::<Vec<_>>();
        for session in sessions {
            session.request_close();
        }
    }

    pub(crate) async fn wait_all(&self, timeout: Duration) {
        let sessions = lock(&self.sessions).values().cloned().collect::<Vec<_>>();
        let deadline = tokio::time::Instant::now() + timeout;
        for session in sessions {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let _ = tokio::time::timeout(remaining, session.wait_for_thread()).await;
        }
    }

    pub(crate) fn clear(&self) {
        lock(&self.sessions).clear();
    }
}

/// Start one agent channel session through the shared authenticated connector.
pub(crate) async fn start_agent_session(
    data: Arc<crate::data::MobileData>,
    registry: Arc<AgentSessionRegistry>,
    host_id: String,
) -> Result<Arc<AgentSessionInner>, BackendError> {
    let context = {
        let host_id = host_id.clone();
        runtime::offload(move || data.session_context(&host_id)).await?
    };

    if context.vault_locked {
        return Err(BackendError::VaultLocked);
    }

    registry.start(|| {
        miaominal_ssh::start_channel_session(
            &runtime::handle(),
            context.profile,
            context.all_profiles,
            context.all_proxies,
            context.secrets,
            context.known_hosts,
        )
    })
}

/// Kotlin handle for one AI CLI channel session.
#[derive(uniffi::Object)]
pub struct AgentSshSession {
    inner: Arc<AgentSessionInner>,
}

impl AgentSshSession {
    pub(crate) fn new(inner: Arc<AgentSessionInner>) -> Self {
        Self { inner }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AgentSshSession {
    pub fn connection_id(&self) -> u64 {
        self.inner.connection_id
    }

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Pull up to `max_events` events, waiting at most `timeout_ms` for the
    /// first one. Returns an empty batch on timeout, never blocks forever.
    pub async fn read_events(
        &self,
        max_events: u32,
        timeout_ms: u64,
    ) -> Result<AgentEventBatch, BackendError> {
        let inner = Arc::clone(&self.inner);
        let max = max_events.max(1) as usize;
        let timeout = Duration::from_millis(timeout_ms.min(30_000));
        runtime::run_task(async move {
            let mut events = Vec::new();
            // Closing the command side does not mean the queued events have
            // been drained. Report stream closure only after its terminal event.
            let mut closed = false;
            {
                let mut receiver = inner.events.lock().await;
                let deadline = tokio::time::Instant::now() + timeout;
                loop {
                    if events.len() >= max {
                        break;
                    }
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if remaining.is_zero() && !events.is_empty() {
                        break;
                    }
                    let wait = if events.is_empty() {
                        remaining
                    } else {
                        Duration::from_millis(0)
                    };
                    match tokio::time::timeout(wait, receiver.recv()).await {
                        Ok(Some(event)) => {
                            let terminal = event.is_terminal();
                            events.push(event);
                            if terminal {
                                inner.closed.store(true, Ordering::SeqCst);
                                closed = true;
                                break;
                            }
                        }
                        Ok(None) => {
                            inner.closed.store(true, Ordering::SeqCst);
                            closed = true;
                            break;
                        }
                        Err(_) => break,
                    }
                }
            }
            Ok(AgentEventBatch {
                events: events.into_iter().map(channel_event_to_dto).collect(),
                closed,
            })
        })
        .await?
    }

    /// Open an exec channel running `command` (already escaped by the caller).
    pub fn open_exec(
        &self,
        channel_id: u64,
        command: String,
        environment: Vec<AgentEnvVar>,
    ) -> Result<(), BackendError> {
        self.ensure_open()?;
        self.inner
            .commands
            .open_exec(
                channel_id,
                command,
                environment
                    .into_iter()
                    .map(|variable| (variable.name, variable.value))
                    .collect(),
            )
            .map_err(|_| BackendError::Closed)
    }

    pub fn write_stdin(&self, channel_id: u64, data: Vec<u8>) -> Result<(), BackendError> {
        self.ensure_open()?;
        self.inner
            .commands
            .write_stdin(channel_id, data)
            .map_err(|_| BackendError::Closed)
    }

    pub fn signal(&self, channel_id: u64, signal: String) -> Result<(), BackendError> {
        self.ensure_open()?;
        self.inner
            .commands
            .signal(channel_id, signal)
            .map_err(|_| BackendError::Closed)
    }

    pub fn close_channel(&self, channel_id: u64) -> Result<(), BackendError> {
        self.ensure_open()?;
        self.inner
            .commands
            .close_channel(channel_id)
            .map_err(|_| BackendError::Closed)
    }

    /// Bind a local loopback listener forwarding to `remote_host:remote_port`
    /// through this SSH connection; `TunnelReady` carries the local port.
    pub fn open_loopback_tunnel(
        &self,
        tunnel_id: u64,
        remote_host: String,
        remote_port: u16,
    ) -> Result<(), BackendError> {
        self.ensure_open()?;
        self.inner
            .commands
            .open_tunnel(tunnel_id, remote_host, remote_port)
            .map_err(|_| BackendError::Closed)
    }

    pub fn close_tunnel(&self, tunnel_id: u64) -> Result<(), BackendError> {
        self.ensure_open()?;
        self.inner
            .commands
            .close_tunnel(tunnel_id)
            .map_err(|_| BackendError::Closed)
    }

    pub fn respond_host_key(
        &self,
        request_id: u64,
        decision: AgentHostKeyDecision,
    ) -> Result<(), BackendError> {
        self.ensure_open()?;
        self.inner
            .commands
            .respond_host_key(request_id, decision.into())
            .map_err(|_| BackendError::Closed)
    }

    pub fn respond_keyboard_interactive(
        &self,
        request_id: u64,
        answers: Vec<String>,
    ) -> Result<(), BackendError> {
        self.ensure_open()?;
        self.inner
            .commands
            .respond_keyboard_interactive(request_id, answers)
            .map_err(|_| BackendError::Closed)
    }

    /// Explicit close; idempotent.
    pub fn cancel(&self) {
        self.inner.request_close();
    }

    pub async fn disconnect(&self) -> Result<(), BackendError> {
        self.inner.request_close();
        self.inner.wait_for_thread().await;
        Ok(())
    }
}

impl AgentSshSession {
    fn ensure_open(&self) -> Result<(), BackendError> {
        if self.inner.is_closed() {
            return Err(BackendError::Closed);
        }
        Ok(())
    }
}

fn channel_event_to_dto(event: miaominal_ssh::ChannelEvent) -> AgentChannelEvent {
    match event {
        miaominal_ssh::ChannelEvent::Ready => AgentChannelEvent::Ready,
        miaominal_ssh::ChannelEvent::ExecOpened { channel_id } => {
            AgentChannelEvent::ExecOpened { channel_id }
        }
        miaominal_ssh::ChannelEvent::ExecFailed {
            channel_id,
            message,
        } => AgentChannelEvent::ExecFailed {
            channel_id,
            message,
        },
        miaominal_ssh::ChannelEvent::Stdout { channel_id, data } => {
            AgentChannelEvent::Stdout { channel_id, data }
        }
        miaominal_ssh::ChannelEvent::Stderr { channel_id, data } => {
            AgentChannelEvent::Stderr { channel_id, data }
        }
        miaominal_ssh::ChannelEvent::ExecExited {
            channel_id,
            exit_code,
            signal,
        } => AgentChannelEvent::ExecExited {
            channel_id,
            exit_code,
            signal,
        },
        miaominal_ssh::ChannelEvent::ChannelClosed { channel_id } => {
            AgentChannelEvent::ChannelClosed { channel_id }
        }
        miaominal_ssh::ChannelEvent::TunnelReady {
            tunnel_id,
            local_port,
        } => AgentChannelEvent::TunnelReady {
            tunnel_id,
            local_port,
        },
        miaominal_ssh::ChannelEvent::TunnelFailed { tunnel_id, message } => {
            AgentChannelEvent::TunnelFailed { tunnel_id, message }
        }
        miaominal_ssh::ChannelEvent::TunnelClosed { tunnel_id } => {
            AgentChannelEvent::TunnelClosed { tunnel_id }
        }
        miaominal_ssh::ChannelEvent::HostKeyPrompt(request) => AgentChannelEvent::HostKeyPrompt {
            prompt: AgentHostKeyPromptDto {
                request_id: request.request_id,
                hop_label: request.hop.label,
                host: request.prompt.host,
                port: request.prompt.port,
                algorithm: request.prompt.algorithm,
                fingerprint: request.prompt.fingerprint,
                previous_fingerprint: request.prompt.previous_fingerprint.clone(),
                key_changed: request.prompt.previous_fingerprint.is_some(),
            },
        },
        miaominal_ssh::ChannelEvent::AuthenticationPrompt(request) => {
            AgentChannelEvent::AuthenticationPrompt {
                prompt: AgentAuthenticationPromptDto {
                    request_id: request.request_id,
                    hop_label: request.hop.label,
                    name: request.challenge.name,
                    instructions: request.challenge.instructions,
                    prompts: request
                        .challenge
                        .prompts
                        .into_iter()
                        .map(|prompt| AgentPromptDto {
                            prompt: prompt.prompt,
                            echo: prompt.echo,
                        })
                        .collect(),
                },
            }
        }
        miaominal_ssh::ChannelEvent::RequestExpired { request_id } => {
            AgentChannelEvent::RequestExpired { request_id }
        }
        miaominal_ssh::ChannelEvent::Error(message) => AgentChannelEvent::Error { message },
        miaominal_ssh::ChannelEvent::Closed => AgentChannelEvent::Closed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::LifecycleState;

    #[test]
    fn shutdown_rejects_a_late_launch_before_creating_its_worker() {
        let lifecycle = Arc::new(Mutex::new(LifecycleState::default()));
        let registry = AgentSessionRegistry::new(Arc::clone(&lifecycle));
        // This is the commit point reached by a context load that finishes
        // after shutdown. No connection may be created or registered then.
        lock(&lifecycle).closed = true;
        let result = registry.start(|| panic!("a closed backend must not start an SSH worker"));
        assert!(matches!(result, Err(BackendError::Closed)));
        assert!(lock(&registry.sessions).is_empty());
    }
}
