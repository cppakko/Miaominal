//! Port-forwarding sessions for the mobile backend.
//!
//! One `ForwardSession` owns one dedicated SSH connection that serves exactly
//! one saved forwarding rule (local or remote). It reuses the shared
//! `miaominal_ssh::start_port_forward_session` connector, so entry proxies,
//! ProxyJump chains, host trust, keyboard-interactive challenges, request ids,
//! explicit cancellation, and reconnect backoff behave exactly like a terminal
//! session. The registry keeps sessions keyed by `(profile_id, rule_id)`:
//! starting the same rule twice returns the running session, and a rebuilt page
//! finds its handle again instead of opening a second connection.
//!
//! Saved rules and runtime state stay separate: a rule's `enabled` flag only
//! describes the stored configuration. After a process restart nothing is
//! started automatically; the user (or the page) must ask for a session.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::{Notify, watch};

use miaominal_ssh::{
    PortForwardFailureKind as NativeForwardFailureKind, SessionEvent, SessionEventReceiver,
    SessionHop, SessionPhase,
};

use crate::dto::ForwardKindDto;
use crate::error::BackendError;
use crate::events::{BackendEvent, EventBus};
use crate::session::{SshHostKeyDecision, SshPromptDto};

/// Maximum time `disconnect` waits for the forwarding worker to finish.
const DISCONNECT_GRACE: Duration = Duration::from_secs(8);

/// Key of a forwarding session: one saved rule of one saved host.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ForwardKey {
    pub(crate) profile_id: String,
    pub(crate) rule_id: String,
}

impl ForwardKey {
    pub(crate) fn new(profile_id: impl Into<String>, rule_id: impl Into<String>) -> Self {
        Self {
            profile_id: profile_id.into(),
            rule_id: rule_id.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ForwardPhase {
    /// Connecting to the target through its route (proxy, jump chain included).
    Starting,
    WaitingForTrust,
    WaitingForAuthentication,
    /// A local listening socket is bound.
    Listening,
    /// The server accepted the remote listening port.
    Connected,
    /// The transport was lost and the session is waiting for its next attempt.
    Reconnecting,
    /// The user asked to stop; the worker is winding down.
    Stopping,
    /// Orderly stop; the rule can be started again.
    Stopped,
    /// The session failed; inspect `failure` for the typed reason.
    Failed,
}

impl ForwardPhase {
    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Stopped | Self::Failed)
    }

    pub(crate) fn is_active(self) -> bool {
        !self.is_terminal()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ForwardRequestKind {
    HostKey,
    KeyboardInteractive,
}

/// Typed failure of a forwarding rule (start failure or a relay failure).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ForwardFailureKind {
    /// The local listening port could not be bound (usually already in use).
    PortOccupied,
    /// The server refused the remote listening port.
    RemoteRefused,
    /// A forwarded connection could not reach its target.
    TargetUnreachable,
    /// The SSH route rejected the credentials.
    Authentication,
    /// The SSH transport failed.
    Transport,
    Other,
}

fn failure_kind_from_native(kind: NativeForwardFailureKind) -> ForwardFailureKind {
    match kind {
        NativeForwardFailureKind::LocalBind => ForwardFailureKind::PortOccupied,
        NativeForwardFailureKind::RemoteBind => ForwardFailureKind::RemoteRefused,
        NativeForwardFailureKind::TargetUnreachable => ForwardFailureKind::TargetUnreachable,
        NativeForwardFailureKind::Transport => ForwardFailureKind::Transport,
    }
}

/// Classify a connection-stage error so the UI can distinguish an occupied
/// port, a refused remote bind, an unreachable target, and an auth failure
/// without parsing English sentences.
fn classify_failure(message: &str) -> ForwardFailureKind {
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("failed to bind")
        || lowered.contains("address already in use")
        || lowered.contains("os error 10048")
        || lowered.contains("os error 10049")
        || lowered.contains("only one usage of each socket address")
        || lowered.contains("cannot assign requested address")
    {
        return ForwardFailureKind::PortOccupied;
    }
    if lowered.contains("failed to start remote forward")
        || lowered.contains("tcpip forward")
        || lowered.contains("administratively prohibited")
    {
        return ForwardFailureKind::RemoteRefused;
    }
    if lowered.contains("direct-tcpip")
        || lowered.contains("could not open ssh channel")
        || lowered.contains("could not reach local target")
        || lowered.contains("relay failed")
        || lowered.contains("no local target is registered")
    {
        return ForwardFailureKind::TargetUnreachable;
    }
    if lowered.contains("authentication failed")
        || lowered.contains("auth failed")
        || lowered.contains("password")
        || lowered.contains("keyboard-interactive")
        || lowered.contains("no more authentication methods")
    {
        return ForwardFailureKind::Authentication;
    }
    if lowered.contains("connection")
        || lowered.contains("failed to connect")
        || lowered.contains("transport")
        || lowered.contains("broken pipe")
        || lowered.contains("reset by peer")
        || lowered.contains("eof")
        || lowered.contains("timed out")
    {
        return ForwardFailureKind::Transport;
    }
    ForwardFailureKind::Other
}

/// Typed failure record: what failed, and on which route node.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ForwardFailure {
    pub kind: ForwardFailureKind,
    /// Node the route was at when the failure happened (entry proxy name, jump
    /// `i/n`, or the target host). Empty before the route resolved.
    pub node_label: String,
    pub hop_index: u32,
    pub hop_total: u32,
    pub message: String,
}

/// Pending trust or authentication request of one forwarding session.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ForwardRequest {
    pub request_id: u64,
    pub connection_id: u64,
    pub profile_id: String,
    pub rule_id: String,
    pub kind: ForwardRequestKind,
    pub hop_label: String,
    pub hop_index: u32,
    pub hop_total: u32,
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
    pub previous_fingerprint: Option<String>,
    pub key_changed: bool,
    pub name: String,
    pub instructions: String,
    pub prompts: Vec<SshPromptDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ForwardSnapshot {
    pub connection_id: u64,
    pub profile_id: String,
    pub rule_id: String,
    pub label: String,
    pub kind: ForwardKindDto,
    pub listen_host: String,
    pub requested_port: u16,
    /// Actual bound port; for remote forwards the server may assign one.
    pub bound_port: Option<u16>,
    pub phase: ForwardPhase,
    /// Terminal failure (the phase is `Failed` when this is set).
    pub failure: Option<ForwardFailure>,
    /// Most recent relay failure while the session kept running.
    pub relay_failure: Option<ForwardFailure>,
    pub request: Option<ForwardRequest>,
    pub reconnect_error: Option<String>,
    pub reconnect_attempt: u32,
    pub reconnect_retry_after_secs: u64,
    pub revision: u64,
    pub started_at_ms: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct ForwardState {
    phase: ForwardPhase,
    failure: Option<ForwardFailure>,
    relay_failure: Option<ForwardFailure>,
    request: Option<ForwardRequest>,
    bound_port: Option<u16>,
    reconnect_error: Option<String>,
    reconnect_attempt: u32,
    reconnect_retry_after_secs: u64,
    hop: Option<SessionHop>,
    revision: u64,
    started_at_ms: u64,
}

/// Shared session state; the Kotlin handle and the event task both use it.
pub(crate) struct ForwardSessionInner {
    connection_id: u64,
    key: ForwardKey,
    label: String,
    kind: ForwardKindDto,
    listen_host: String,
    requested_port: u16,
    /// Weak back-reference: the registry owns the sessions, so a strong link
    /// here would keep every worker thread alive after a backend restart.
    registry: Weak<ForwardSessionRegistry>,
    state: Mutex<ForwardState>,
    wake: Notify,
    commands: Mutex<Option<miaominal_ssh::SessionCommandSender>>,
    consumer: Mutex<Option<tokio::task::JoinHandle<()>>>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    close_requested: AtomicBool,
}

impl ForwardSessionInner {
    #[allow(clippy::too_many_arguments)]
    fn new(
        connection_id: u64,
        registry: &Arc<ForwardSessionRegistry>,
        key: ForwardKey,
        label: String,
        kind: ForwardKindDto,
        listen_host: String,
        requested_port: u16,
    ) -> Arc<Self> {
        Arc::new(Self {
            connection_id,
            key,
            label,
            kind,
            listen_host,
            requested_port,
            registry: Arc::downgrade(registry),
            state: Mutex::new(ForwardState {
                phase: ForwardPhase::Starting,
                failure: None,
                relay_failure: None,
                request: None,
                bound_port: None,
                reconnect_error: None,
                reconnect_attempt: 0,
                reconnect_retry_after_secs: 0,
                hop: None,
                revision: 1,
                started_at_ms: now_ms(),
            }),
            wake: Notify::new(),
            commands: Mutex::new(None),
            consumer: Mutex::new(None),
            worker: Mutex::new(None),
            close_requested: AtomicBool::new(false),
        })
    }

    fn attach(
        &self,
        commands: miaominal_ssh::SessionCommandSender,
        consumer: tokio::task::JoinHandle<()>,
        worker: Option<std::thread::JoinHandle<()>>,
    ) {
        let mut slot = lock(&self.commands);
        if self.close_requested.load(Ordering::SeqCst) {
            let _ = commands.close();
        }
        *slot = Some(commands);
        drop(slot);
        *lock(&self.consumer) = Some(consumer);
        *lock(&self.worker) = worker;
    }

    fn publish(&self, event: BackendEvent) {
        if let Some(registry) = self.registry.upgrade() {
            registry.publish(event);
        }
    }

    fn publish_phase(&self) {
        self.publish(BackendEvent::ForwardPhaseChanged {
            connection_id: self.connection_id,
            profile_id: self.key.profile_id.clone(),
            rule_id: self.key.rule_id.clone(),
            phase: lock(&self.state).phase,
        });
    }

    fn set_phase(&self, phase: ForwardPhase, failure: Option<ForwardFailure>) {
        let changed = {
            let mut state = lock(&self.state);
            if state.phase.is_terminal() {
                return;
            }
            state.phase = phase;
            if failure.is_some() {
                state.failure = failure;
            }
            state.revision = state.revision.wrapping_add(1);
            true
        };
        if changed {
            self.wake.notify_waiters();
            self.publish_phase();
        }
    }

    fn mark_stopped(&self) {
        let phase = {
            let mut state = lock(&self.state);
            state.request = None;
            state.bound_port = None;
            if !state.phase.is_terminal() {
                state.phase = ForwardPhase::Stopped;
            }
            state.revision = state.revision.wrapping_add(1);
            state.phase
        };
        self.wake.notify_waiters();
        self.publish(BackendEvent::ForwardPhaseChanged {
            connection_id: self.connection_id,
            profile_id: self.key.profile_id.clone(),
            rule_id: self.key.rule_id.clone(),
            phase,
        });
    }

    fn publish_request(&self, request: ForwardRequest) {
        {
            let mut state = lock(&self.state);
            state.request = Some(request.clone());
            state.revision = state.revision.wrapping_add(1);
        }
        self.wake.notify_waiters();
        self.publish(BackendEvent::ForwardRequest { request });
    }

    fn expire_request(&self, request_id: u64) {
        {
            let mut state = lock(&self.state);
            if state
                .request
                .as_ref()
                .is_some_and(|request| request.request_id == request_id)
            {
                state.request = None;
            }
            state.revision = state.revision.wrapping_add(1);
        }
        self.wake.notify_waiters();
        self.publish(BackendEvent::ForwardRequestExpired {
            connection_id: self.connection_id,
            request_id,
        });
    }

    fn failure_with_hop(&self, kind: ForwardFailureKind, message: String) -> ForwardFailure {
        let state = lock(&self.state);
        let (node_label, hop_index, hop_total) = match &state.hop {
            Some(hop) => (hop.label.clone(), hop.index, hop.total),
            None => (String::new(), 0, 0),
        };
        ForwardFailure {
            kind,
            node_label,
            hop_index,
            hop_total,
            message,
        }
    }

    fn record_relay_failure(&self, failure: ForwardFailure) {
        {
            let mut state = lock(&self.state);
            state.relay_failure = Some(failure.clone());
            state.revision = state.revision.wrapping_add(1);
        }
        self.wake.notify_waiters();
        // Publish after releasing the state lock: the registry takes the
        // lifecycle lock, and a rebuilt page must learn about the failure
        // without polling.
        self.publish(BackendEvent::ForwardRelayFailed {
            connection_id: self.connection_id,
            profile_id: self.key.profile_id.clone(),
            rule_id: self.key.rule_id.clone(),
            failure,
        });
    }

    /// Apply one typed forwarding event. Runs on the session task.
    fn apply_event(&self, event: SessionEvent) {
        match event {
            SessionEvent::Phase(change) => {
                if change.hop.is_some() {
                    let mut state = lock(&self.state);
                    state.hop = change.hop;
                }
                match change.phase {
                    SessionPhase::Connecting => {
                        self.set_phase(ForwardPhase::Starting, None);
                    }
                    SessionPhase::WaitingForTrust => {
                        self.set_phase(ForwardPhase::WaitingForTrust, None);
                    }
                    SessionPhase::WaitingForAuthentication => {
                        self.set_phase(ForwardPhase::WaitingForAuthentication, None);
                    }
                    SessionPhase::Closing => {
                        self.set_phase(ForwardPhase::Stopping, None);
                    }
                    SessionPhase::Closed => {
                        self.mark_stopped();
                    }
                    // A dedicated forwarding route never reaches shell phases.
                    SessionPhase::OpeningShell | SessionPhase::Ready | SessionPhase::Failed => {}
                }
            }
            SessionEvent::HostKeyPrompt(request) => {
                let dto = self.host_key_request_dto(request);
                self.set_phase(ForwardPhase::WaitingForTrust, None);
                self.publish_request(dto);
            }
            SessionEvent::KeyboardInteractivePrompt(request) => {
                let dto = self.authentication_request_dto(request);
                self.set_phase(ForwardPhase::WaitingForAuthentication, None);
                self.publish_request(dto);
            }
            SessionEvent::RequestExpired { request_id } => {
                self.expire_request(request_id);
            }
            SessionEvent::PortForwardActive(binding) => {
                {
                    let mut state = lock(&self.state);
                    state.bound_port = Some(binding.bound_port);
                    state.reconnect_error = None;
                    state.reconnect_attempt = 0;
                    state.reconnect_retry_after_secs = 0;
                    if !state.phase.is_terminal() {
                        state.phase = match binding.kind {
                            miaominal_core::profile::PortForwardKind::Local => {
                                ForwardPhase::Listening
                            }
                            miaominal_core::profile::PortForwardKind::Remote => {
                                ForwardPhase::Connected
                            }
                        };
                    }
                    state.revision = state.revision.wrapping_add(1);
                }
                self.wake.notify_waiters();
                self.publish_phase();
            }
            SessionEvent::PortForwardFailed(failure) => {
                let mapped = self.failure_with_hop(
                    failure_kind_from_native(failure.kind),
                    failure.message,
                );
                let terminal = {
                    let state = lock(&self.state);
                    !matches!(
                        state.phase,
                        ForwardPhase::Listening
                            | ForwardPhase::Connected
                            | ForwardPhase::Reconnecting
                    )
                };
                if terminal {
                    self.set_phase(ForwardPhase::Failed, Some(mapped));
                } else {
                    self.record_relay_failure(mapped);
                }
            }
            SessionEvent::Connected(_) => {
                let kind = self.kind;
                self.set_phase(
                    match kind {
                        ForwardKindDto::Local => ForwardPhase::Listening,
                        ForwardKindDto::Remote => ForwardPhase::Connected,
                    },
                    None,
                );
            }
            SessionEvent::PortForwardReconnecting {
                error,
                attempt,
                max_attempts: _,
                retry_after_secs,
            } => {
                {
                    let mut state = lock(&self.state);
                    if state.phase.is_terminal() {
                        return;
                    }
                    state.phase = ForwardPhase::Reconnecting;
                    state.reconnect_error = Some(error);
                    state.reconnect_attempt = attempt;
                    state.reconnect_retry_after_secs = retry_after_secs;
                    state.bound_port = None;
                    state.revision = state.revision.wrapping_add(1);
                }
                self.wake.notify_waiters();
                self.publish_phase();
            }
            SessionEvent::Error(message) => {
                let existing = lock(&self.state).failure.clone();
                let failure = match existing {
                    Some(failure) => failure,
                    None => self.failure_with_hop(classify_failure(&message), message),
                };
                self.set_phase(ForwardPhase::Failed, Some(failure));
            }
            SessionEvent::Closed => {
                self.mark_stopped();
            }
            SessionEvent::Output(_)
            | SessionEvent::Status(_)
            | SessionEvent::MonitorUpdated(_)
            | SessionEvent::MonitorFailed(_)
            | SessionEvent::PortForwardNotice(_)
            | SessionEvent::Exited { .. } => {}
        }
    }

    fn host_key_request_dto(&self, request: miaominal_ssh::HostKeyRequest) -> ForwardRequest {
        ForwardRequest {
            request_id: request.request_id,
            connection_id: self.connection_id,
            profile_id: self.key.profile_id.clone(),
            rule_id: self.key.rule_id.clone(),
            kind: ForwardRequestKind::HostKey,
            hop_label: request.hop.label.clone(),
            hop_index: request.hop.index,
            hop_total: request.hop.total,
            host: request.prompt.host,
            port: request.prompt.port,
            algorithm: request.prompt.algorithm,
            fingerprint: request.prompt.fingerprint,
            previous_fingerprint: request.prompt.previous_fingerprint.clone(),
            key_changed: request.prompt.previous_fingerprint.is_some(),
            name: String::new(),
            instructions: String::new(),
            prompts: Vec::new(),
        }
    }

    fn authentication_request_dto(
        &self,
        request: miaominal_ssh::AuthenticationRequest,
    ) -> ForwardRequest {
        ForwardRequest {
            request_id: request.request_id,
            connection_id: self.connection_id,
            profile_id: self.key.profile_id.clone(),
            rule_id: self.key.rule_id.clone(),
            kind: ForwardRequestKind::KeyboardInteractive,
            hop_label: request.hop.label.clone(),
            hop_index: request.hop.index,
            hop_total: request.hop.total,
            host: String::new(),
            port: 0,
            algorithm: String::new(),
            fingerprint: String::new(),
            previous_fingerprint: None,
            key_changed: false,
            name: request.challenge.name,
            instructions: request.challenge.instructions,
            prompts: request
                .challenge
                .prompts
                .into_iter()
                .map(|prompt| SshPromptDto {
                    prompt: prompt.prompt,
                    echo: prompt.echo,
                })
                .collect(),
        }
    }

    fn phase(&self) -> ForwardPhase {
        lock(&self.state).phase
    }

    pub(crate) fn request_close(&self) {
        self.close_requested.store(true, Ordering::SeqCst);
        if !self.phase().is_terminal() {
            self.set_phase(ForwardPhase::Stopping, None);
        }
        if let Some(commands) = lock(&self.commands).clone() {
            let _ = commands.close();
        }
        self.wake.notify_waiters();
    }

    fn abort_consumer(&self) {
        if let Some(handle) = lock(&self.consumer).take() {
            handle.abort();
        }
    }

    /// Bounded wait for the consumer task and the forwarding worker thread.
    pub(crate) async fn wait_for_task(&self) {
        let handle = lock(&self.consumer).take();
        if let Some(mut handle) = handle {
            if tokio::time::timeout(DISCONNECT_GRACE, &mut handle)
                .await
                .is_err()
            {
                handle.abort();
            }
        }
        self.wait_for_worker(DISCONNECT_GRACE);
    }

    /// Join the forwarding worker with a bound, without blocking the async
    /// runtime: the join runs on a helper thread.
    fn wait_for_worker(&self, timeout: Duration) {
        let Some(worker) = lock(&self.worker).take() else {
            return;
        };
        let (done_sender, done_receiver) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("forward-join".to_string())
            .spawn(move || {
                let _ = worker.join();
                let _ = done_sender.send(());
            })
            .ok();
        if done_receiver.recv_timeout(timeout).is_err() {
            log::warn!("timed out waiting for a port-forward worker thread to finish");
        }
    }

    fn finish_task(&self) {
        if !self.phase().is_terminal() {
            self.mark_stopped();
        }
        self.wake.notify_waiters();
    }

    fn respond_host_key(
        &self,
        request_id: u64,
        decision: SshHostKeyDecision,
    ) -> Result<(), BackendError> {
        let is_current = lock(&self.state).request.as_ref().is_some_and(|request| {
            request.request_id == request_id && request.kind == ForwardRequestKind::HostKey
        });
        if !is_current {
            return Err(BackendError::RequestExpired { request_id });
        }
        let commands = lock(&self.commands)
            .clone()
            .ok_or(BackendError::Closed)?;
        commands
            .respond_host_key(
                request_id,
                match decision {
                    SshHostKeyDecision::AcceptOnce => {
                        miaominal_core::forwarding::HostKeyDecision::AcceptOnce
                    }
                    SshHostKeyDecision::AcceptAndSave => {
                        miaominal_core::forwarding::HostKeyDecision::AcceptAndSave
                    }
                    SshHostKeyDecision::Reject => {
                        miaominal_core::forwarding::HostKeyDecision::Reject
                    }
                },
            )
            .map_err(|error| BackendError::Storage {
                storage_failure: format!("failed to answer the host key prompt: {error}"),
            })?;
        {
            let mut state = lock(&self.state);
            state.request = None;
            state.revision = state.revision.wrapping_add(1);
        }
        self.wake.notify_waiters();
        Ok(())
    }

    fn respond_keyboard_interactive(
        &self,
        request_id: u64,
        answers: Vec<String>,
    ) -> Result<(), BackendError> {
        let is_current = lock(&self.state).request.as_ref().is_some_and(|request| {
            request.request_id == request_id
                && request.kind == ForwardRequestKind::KeyboardInteractive
        });
        if !is_current {
            return Err(BackendError::RequestExpired { request_id });
        }
        let commands = lock(&self.commands)
            .clone()
            .ok_or(BackendError::Closed)?;
        commands
            .respond_keyboard_interactive(request_id, answers)
            .map_err(|error| BackendError::Storage {
                storage_failure: format!("failed to answer the authentication challenge: {error}"),
            })?;
        {
            let mut state = lock(&self.state);
            state.request = None;
            state.revision = state.revision.wrapping_add(1);
        }
        self.wake.notify_waiters();
        Ok(())
    }

    fn snapshot(&self) -> ForwardSnapshot {
        let state = lock(&self.state);
        ForwardSnapshot {
            connection_id: self.connection_id,
            profile_id: self.key.profile_id.clone(),
            rule_id: self.key.rule_id.clone(),
            label: self.label.clone(),
            kind: self.kind,
            listen_host: self.listen_host.clone(),
            requested_port: self.requested_port,
            bound_port: state.bound_port,
            phase: state.phase,
            failure: state.failure.clone(),
            relay_failure: state.relay_failure.clone(),
            request: state.request.clone(),
            reconnect_error: state.reconnect_error.clone(),
            reconnect_attempt: state.reconnect_attempt,
            reconnect_retry_after_secs: state.reconnect_retry_after_secs,
            revision: state.revision,
            started_at_ms: state.started_at_ms,
        }
    }
}

/// Registry of forwarding sessions owned by one `Backend` handle.
pub(crate) struct ForwardSessionRegistry {
    lifecycle: Arc<Mutex<crate::backend::LifecycleState>>,
    events: Arc<EventBus>,
    sessions: Mutex<HashMap<ForwardKey, Arc<ForwardSessionInner>>>,
    /// Per-rule start locks: the active-check and the session creation must be
    /// one atomic step, otherwise two fast taps create two SSH connections and
    /// one of them is overwritten in `sessions` while still running.
    start_locks: Mutex<HashMap<ForwardKey, Weak<tokio::sync::Mutex<()>>>>,
    next_connection_id: AtomicU64,
}

impl ForwardSessionRegistry {
    pub(crate) fn new(
        lifecycle: Arc<Mutex<crate::backend::LifecycleState>>,
        events: Arc<EventBus>,
    ) -> Arc<Self> {
        Arc::new(Self {
            lifecycle,
            events,
            sessions: Mutex::new(HashMap::new()),
            start_locks: Mutex::new(HashMap::new()),
            next_connection_id: AtomicU64::new(1),
        })
    }

    /// Acquire the per-rule start lock. The returned mutex is shared by every
    /// caller of the same rule while one of them holds it; a lock with no
    /// owners is replaced on the next call.
    pub(crate) fn start_lock(&self, key: &ForwardKey) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = lock(&self.start_locks);
        locks.retain(|_, existing| existing.strong_count() > 0);
        if let Some(existing) = locks.get(key).and_then(Weak::upgrade) {
            return existing;
        }
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(key.clone(), Arc::downgrade(&lock));
        lock
    }

    pub(crate) fn publish(&self, event: BackendEvent) {
        let state = lock(&self.lifecycle);
        if state.closed {
            return;
        }
        self.events.publish(event);
    }

    fn next_connection_id(&self) -> u64 {
        self.next_connection_id.fetch_add(1, Ordering::SeqCst)
    }

    pub(crate) fn insert(&self, session: Arc<ForwardSessionInner>) {
        lock(&self.sessions).insert(session.key.clone(), session);
    }

    fn start(
        &self,
        connect: impl FnOnce() -> Arc<ForwardSessionInner>,
    ) -> Result<Arc<ForwardSessionInner>, BackendError> {
        // Worker creation and registration commit atomically with shutdown.
        let state = lock(&self.lifecycle);
        if state.closed {
            return Err(BackendError::Closed);
        }
        let inner = connect();
        self.insert(Arc::clone(&inner));
        Ok(inner)
    }

    pub(crate) fn get(&self, key: &ForwardKey) -> Option<Arc<ForwardSessionInner>> {
        lock(&self.sessions).get(key).cloned()
    }

    /// A running session for this key, if any (terminal entries are ignored so
    /// a failed or stopped rule can be started again).
    pub(crate) fn active(&self, key: &ForwardKey) -> Option<Arc<ForwardSessionInner>> {
        lock(&self.sessions)
            .get(key)
            .filter(|session| session.phase().is_active())
            .cloned()
    }

    pub(crate) fn remove(&self, key: &ForwardKey) -> Option<Arc<ForwardSessionInner>> {
        lock(&self.sessions).remove(key)
    }

    pub(crate) fn snapshots(&self) -> Vec<ForwardSnapshot> {
        let sessions = lock(&self.sessions)
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut snapshots: Vec<_> = sessions.iter().map(|session| session.snapshot()).collect();
        snapshots.sort_by(|left, right| {
            left.profile_id
                .cmp(&right.profile_id)
                .then_with(|| left.rule_id.cmp(&right.rule_id))
        });
        snapshots
    }

    /// Ask every session to stop; used by `Backend::shutdown`.
    pub(crate) fn request_close_all(&self) {
        let sessions = lock(&self.sessions)
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            session.request_close();
        }
    }

    /// Ask the sessions of one rule (or every rule of one profile) to stop.
    /// Used when a rule or its host is deleted while it is running.
    pub(crate) fn request_close_matching(&self, profile_id: &str, rule_id: Option<&str>) {
        let sessions = lock(&self.sessions)
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            if session.key.profile_id != profile_id {
                continue;
            }
            if rule_id.is_some_and(|rule_id| session.key.rule_id != rule_id) {
                continue;
            }
            session.request_close();
        }
    }

    /// Bounded wait for every worker thread to finish cancelling.
    pub(crate) fn wait_all(&self, timeout: Duration) {
        let sessions = lock(&self.sessions)
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            session.wait_for_worker(timeout);
        }
    }

    pub(crate) fn clear(&self) {
        let sessions = lock(&self.sessions)
            .drain()
            .map(|(_, session)| session)
            .collect::<Vec<_>>();
        for session in sessions {
            // `shutdown` returns only when every session is observably stopped:
            // the consumer task is aborted instead of being awaited, so mark the
            // terminal phase here before dropping it.
            session.finish_task();
            session.abort_consumer();
        }
    }
}

/// Event task: applies typed forwarding events until the worker closes.
async fn run_session_task(
    inner: Arc<ForwardSessionInner>,
    mut events: SessionEventReceiver,
    mut close_rx: watch::Receiver<bool>,
) {
    let mut closing = false;
    loop {
        tokio::select! {
            event = events.recv() => {
                match event {
                    Some(event) => {
                        let task_inner = Arc::clone(&inner);
                        let _ = crate::runtime::offload(move || task_inner.apply_event(event)).await;
                    }
                    None => break,
                }
            }
            _ = wait_for_close(&mut close_rx), if !closing => {
                closing = true;
                if let Some(commands) = lock(&inner.commands).clone() {
                    let _ = commands.close();
                }
            }
        }
    }
    inner.finish_task();
}

async fn wait_for_close(closed: &mut watch::Receiver<bool>) {
    loop {
        if *closed.borrow_and_update() {
            return;
        }
        if closed.changed().await.is_err() {
            return;
        }
    }
}

/// Start one dedicated forwarding session for a saved rule.
pub(crate) async fn start_forward_session(
    data: Arc<crate::data::MobileData>,
    registry: Arc<ForwardSessionRegistry>,
    close_rx: watch::Receiver<bool>,
    profile_id: String,
    rule_id: String,
) -> Result<Arc<ForwardSessionInner>, BackendError> {
    let context = {
        let profile_id = profile_id.clone();
        crate::runtime::offload(move || data.session_context(&profile_id)).await?
    };
    if context.vault_locked {
        return Err(BackendError::VaultLocked);
    }
    let mut rule = context
        .profile
        .port_forwarding_rules
        .iter()
        .find(|rule| rule.id == rule_id)
        .cloned()
        .ok_or_else(|| BackendError::InvalidArgument {
            detail: format!("forward rule {rule_id} does not exist on host {profile_id}"),
        })?;
    if rule.listen_host.trim().is_empty() {
        rule.listen_host = "127.0.0.1".to_string();
    }
    // The rule's stored `enabled` flag is configuration, not a runtime state:
    // an explicit start always enables this dedicated connection.
    rule.enabled = true;

    let kind = crate::dto::forward_kind_to_dto(rule.kind);
    let label = if rule.label.trim().is_empty() {
        format!("{}:{}", rule.listen_host, rule.listen_port)
    } else {
        rule.label.clone()
    };
    let listen_host = rule.listen_host.clone();
    let requested_port = rule.listen_port;

    registry.start(|| {
        let connection_id = registry.next_connection_id();
        let mut connection_profile = context.profile.clone();
        connection_profile.port_forwarding_rules = vec![rule.clone()];

        let connection = miaominal_ssh::start_port_forward_session(
            &crate::runtime::handle(),
            connection_profile,
            context.all_profiles,
            context.all_proxies,
            context.secrets,
            context.known_hosts,
        );
        let miaominal_ssh::SessionConnection {
            commands,
            events,
            thread,
        } = connection;

        let inner = ForwardSessionInner::new(
            connection_id,
            &registry,
            ForwardKey::new(profile_id, rule_id),
            label,
            kind,
            listen_host,
            requested_port,
        );
        let task = crate::runtime::runtime().spawn(run_session_task(
            Arc::clone(&inner),
            events,
            close_rx,
        ));
        inner.attach(commands, task, thread);
        inner
    })
}

/// Kotlin handle for one forwarding session.
#[derive(uniffi::Object)]
pub struct ForwardSession {
    inner: Arc<ForwardSessionInner>,
}

impl ForwardSession {
    pub(crate) fn new(inner: Arc<ForwardSessionInner>) -> Self {
        Self { inner }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl ForwardSession {
    pub fn connection_id(&self) -> u64 {
        self.inner.connection_id
    }

    pub fn profile_id(&self) -> String {
        self.inner.key.profile_id.clone()
    }

    pub fn rule_id(&self) -> String {
        self.inner.key.rule_id.clone()
    }

    /// Consistent view of the phase, bound port, typed failure, and the
    /// pending trust/authentication request.
    pub fn snapshot(&self) -> ForwardSnapshot {
        self.inner.snapshot()
    }

    pub fn respond_host_key(
        &self,
        request_id: u64,
        decision: SshHostKeyDecision,
    ) -> Result<(), BackendError> {
        self.inner.respond_host_key(request_id, decision)
    }

    pub fn respond_keyboard_interactive(
        &self,
        request_id: u64,
        answers: Vec<String>,
    ) -> Result<(), BackendError> {
        self.inner.respond_keyboard_interactive(request_id, answers)
    }

    /// Explicit cancellation: interrupts connect/authentication waits as well
    /// as the running forward. Idempotent.
    pub fn cancel(&self) {
        self.inner.request_close();
    }

    /// Orderly, idempotent close that waits for the worker to finish and drops
    /// the session from the backend registry.
    pub async fn disconnect(&self) -> Result<(), BackendError> {
        self.inner.request_close();
        let inner = Arc::clone(&self.inner);
        crate::runtime::run_task(async move {
            inner.wait_for_task().await;
            Ok::<(), BackendError>(())
        })
        .await??;
        if let Some(registry) = self.inner.registry.upgrade() {
            registry.remove(&self.inner.key);
        }
        Ok(())
    }

    pub fn is_closed(&self) -> bool {
        self.inner.phase().is_terminal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use miaominal_ssh::{SessionChannels, SessionCommand, SessionPhaseChange};

    struct Harness {
        inner: Arc<ForwardSessionInner>,
        commands: tokio::sync::mpsc::UnboundedReceiver<SessionCommand>,
    }

    fn harness(key: (&str, &str)) -> Harness {
        let lifecycle = Arc::new(Mutex::new(crate::backend::LifecycleState::default()));
        let registry = ForwardSessionRegistry::new(lifecycle, EventBus::new());
        let SessionChannels {
            connection,
            commands,
            events: _events,
        } = miaominal_ssh::session_channels();
        let inner = ForwardSessionInner::new(
            1,
            &registry,
            ForwardKey::new(key.0, key.1),
            "Forward".to_string(),
            ForwardKindDto::Local,
            "127.0.0.1".to_string(),
            8080,
        );
        let task = crate::runtime::runtime().spawn(async {});
        inner.attach(connection.commands.clone(), task, None);
        Harness { inner, commands }
    }

    fn hop(label: &str) -> SessionHop {
        SessionHop::new(label, 1, 1)
    }

    fn phase_event(phase: SessionPhase) -> SessionEvent {
        SessionEvent::Phase(SessionPhaseChange {
            phase,
            hop: Some(hop("Target")),
        })
    }

    fn host_key_request(request_id: u64) -> miaominal_ssh::HostKeyRequest {
        miaominal_ssh::HostKeyRequest {
            request_id,
            hop: hop("Jump"),
            prompt: miaominal_core::forwarding::HostKeyPrompt {
                host: "example.test".to_string(),
                port: 22,
                algorithm: "ssh-ed25519".to_string(),
                fingerprint: "SHA256:abc".to_string(),
                previous_fingerprint: None,
            },
        }
    }

    fn authentication_request(request_id: u64) -> miaominal_ssh::AuthenticationRequest {
        miaominal_ssh::AuthenticationRequest {
            request_id,
            hop: hop("Jump"),
            challenge: miaominal_core::forwarding::KbiChallenge {
                name: "Two-factor".to_string(),
                instructions: "Enter the code".to_string(),
                prompts: vec![miaominal_core::forwarding::KbiPrompt {
                    prompt: "OTP:".to_string(),
                    echo: true,
                }],
            },
        }
    }

    fn binding(rule_id: &str, kind: miaominal_core::profile::PortForwardKind) -> SessionEvent {
        SessionEvent::PortForwardActive(miaominal_ssh::PortForwardBinding {
            rule_id: rule_id.to_string(),
            label: "Forward".to_string(),
            kind,
            listen_host: "127.0.0.1".to_string(),
            bound_port: 9_000,
        })
    }

    #[test]
    fn registry_tracks_sessions_per_rule_and_releases_them() {
        let registry = ForwardSessionRegistry::new(
            Arc::new(Mutex::new(crate::backend::LifecycleState::default())),
            EventBus::new(),
        );
        let first = harness(("host-1", "rule-a"));
        let second = harness(("host-1", "rule-b"));
        registry.insert(Arc::clone(&first.inner));
        registry.insert(Arc::clone(&second.inner));

        assert_eq!(registry.snapshots().len(), 2);
        assert!(registry.get(&ForwardKey::new("host-1", "rule-a")).is_some());
        assert!(registry.get(&ForwardKey::new("host-1", "rule-b")).is_some());
        assert_eq!(
            registry.active(&ForwardKey::new("host-1", "rule-a")).map(|session| session.connection_id),
            Some(1)
        );

        assert!(registry.remove(&ForwardKey::new("host-1", "rule-a")).is_some());
        assert!(registry.remove(&ForwardKey::new("host-1", "rule-a")).is_none());
        assert_eq!(registry.snapshots().len(), 1);

        registry.clear();
        assert!(registry.snapshots().is_empty());
        assert_eq!(
            second.inner.snapshot().phase,
            ForwardPhase::Stopped,
            "a cleared registry must leave sessions observably stopped"
        );
    }

    #[test]
    fn same_host_rules_do_not_overwrite_each_other() {
        let registry = ForwardSessionRegistry::new(
            Arc::new(Mutex::new(crate::backend::LifecycleState::default())),
            EventBus::new(),
        );
        let first = harness(("host-1", "rule-a"));
        let second = harness(("host-1", "rule-b"));
        first.inner.apply_event(binding("rule-a", miaominal_core::profile::PortForwardKind::Local));
        second.inner.apply_event(binding("rule-b", miaominal_core::profile::PortForwardKind::Remote));
        registry.insert(Arc::clone(&first.inner));
        registry.insert(Arc::clone(&second.inner));

        let snapshots = registry.snapshots();
        assert_eq!(snapshots.len(), 2);
        assert!(snapshots.iter().any(|snapshot| {
            snapshot.rule_id == "rule-a"
                && snapshot.phase == ForwardPhase::Listening
                && snapshot.bound_port == Some(9_000)
        }));
        assert!(snapshots.iter().any(|snapshot| {
            snapshot.rule_id == "rule-b"
                && snapshot.phase == ForwardPhase::Connected
                && snapshot.bound_port == Some(9_000)
        }));
    }

    #[test]
    fn repeated_close_is_idempotent() {
        let mut harness = harness(("host-1", "rule-a"));
        let inner = &harness.inner;

        inner.request_close();
        let first = inner.snapshot();
        assert_eq!(first.phase, ForwardPhase::Stopping);
        assert!(matches!(
            harness.commands.try_recv(),
            Ok(SessionCommand::Close)
        ));
        inner.request_close();
        assert_eq!(inner.snapshot().phase, ForwardPhase::Stopping);

        inner.mark_stopped();
        let stopped = inner.snapshot();
        assert_eq!(stopped.phase, ForwardPhase::Stopped);
        inner.request_close();
        assert_eq!(inner.snapshot().phase, ForwardPhase::Stopped);
        assert!(inner.snapshot().bound_port.is_none());
    }

    #[test]
    fn shutdown_rejects_late_session_creation() {
        let lifecycle = Arc::new(Mutex::new(crate::backend::LifecycleState::default()));
        let registry = ForwardSessionRegistry::new(Arc::clone(&lifecycle), EventBus::new());
        lock(&lifecycle).closed = true;
        assert!(matches!(
            registry.start(|| panic!("must not create a worker after shutdown")),
            Err(BackendError::Closed)
        ));
        assert!(registry.snapshots().is_empty());
    }

    #[test]
    fn close_before_attach_is_delivered_to_worker() {
        let harness = harness(("host-1", "rule-a"));
        lock(&harness.inner.commands).take();
        harness.inner.request_close();
        let mut channels = miaominal_ssh::session_channels();
        harness.inner.attach(
            channels.connection.commands,
            crate::runtime::runtime().spawn(async {}),
            None,
        );
        assert!(matches!(channels.commands.try_recv(), Ok(SessionCommand::Close)));
    }

    #[test]
    fn stale_authentication_answers_are_rejected() {
        let harness = harness(("host-1", "rule-a"));
        let inner = &harness.inner;
        inner.apply_event(SessionEvent::KeyboardInteractivePrompt(
            authentication_request(7),
        ));

        let error = inner
            .respond_keyboard_interactive(6, vec!["123456".to_string()])
            .expect_err("an old request id must be rejected");
        assert!(matches!(error, BackendError::RequestExpired { request_id: 6 }));
        assert!(inner.snapshot().request.is_some());

        inner
            .respond_keyboard_interactive(7, vec!["123456".to_string()])
            .expect("the current request must be answerable");
        assert!(inner.snapshot().request.is_none());
    }

    #[test]
    fn host_key_prompt_carries_the_failing_node() {
        let harness = harness(("host-1", "rule-a"));
        let inner = &harness.inner;
        inner.apply_event(SessionEvent::HostKeyPrompt(host_key_request(3)));

        let snapshot = inner.snapshot();
        assert_eq!(snapshot.phase, ForwardPhase::WaitingForTrust);
        let request = snapshot.request.expect("pending request");
        assert_eq!(request.hop_label, "Jump");
        assert_eq!(request.request_id, 3);
    }

    #[test]
    fn bind_failure_is_typed_as_occupied_port_with_the_node() {
        let harness = harness(("host-1", "rule-a"));
        let inner = &harness.inner;
        inner.apply_event(phase_event(SessionPhase::Connecting));
        inner.apply_event(SessionEvent::PortForwardFailed(
            miaominal_ssh::PortForwardFailure {
                rule_id: "rule-a".to_string(),
                label: "Forward".to_string(),
                kind: NativeForwardFailureKind::LocalBind,
                message: "Forward Forward failed to bind on 127.0.0.1:8080".to_string(),
            },
        ));
        inner.apply_event(SessionEvent::Error(
            "failed to start port forward Forward: Forward Forward failed to bind on 127.0.0.1:8080"
                .to_string(),
        ));

        let failure = inner.snapshot().failure.expect("terminal failure");
        assert_eq!(failure.kind, ForwardFailureKind::PortOccupied);
        assert_eq!(failure.node_label, "Target");
        assert_eq!(failure.hop_index, 1);
        assert_eq!(inner.snapshot().phase, ForwardPhase::Failed);
    }

    #[tokio::test]
    async fn relay_failures_publish_a_backend_event() {
        let lifecycle = Arc::new(Mutex::new(crate::backend::LifecycleState::default()));
        let events = EventBus::new();
        let registry = ForwardSessionRegistry::new(Arc::clone(&lifecycle), Arc::clone(&events));
        let subscription = events.subscribe(0);
        let SessionChannels {
            connection,
            commands: _commands,
            events: _events,
        } = miaominal_ssh::session_channels();
        let inner = ForwardSessionInner::new(
            1,
            &registry,
            ForwardKey::new("host-1", "rule-a"),
            "Forward".to_string(),
            ForwardKindDto::Local,
            "127.0.0.1".to_string(),
            8080,
        );
        registry.insert(Arc::clone(&inner));
        inner.attach(connection.commands.clone(), crate::runtime::runtime().spawn(async {}), None);

        // Relay failures only happen while the session is running.
        inner.apply_event(binding("rule-a", miaominal_core::profile::PortForwardKind::Local));
        inner.apply_event(SessionEvent::PortForwardFailed(
            miaominal_ssh::PortForwardFailure {
                rule_id: "rule-a".to_string(),
                label: "Forward".to_string(),
                kind: NativeForwardFailureKind::TargetUnreachable,
                message: "Forward Forward could not open SSH channel".to_string(),
            },
        ));

        let batch = subscription.next_batch(16, 200).await.unwrap();
        assert!(
            batch.events.iter().any(|sequenced| matches!(
                &sequenced.event,
                BackendEvent::ForwardRelayFailed { rule_id, failure, .. }
                    if rule_id == "rule-a"
                        && failure.kind == ForwardFailureKind::TargetUnreachable
            )),
            "a relay failure must reach the event stream: {:?}",
            batch.events
        );
    }

    #[test]
    fn relay_failure_keeps_a_running_session_alive() {        let harness = harness(("host-1", "rule-a"));
        let inner = &harness.inner;
        inner.apply_event(binding("rule-a", miaominal_core::profile::PortForwardKind::Local));
        inner.apply_event(SessionEvent::PortForwardFailed(
            miaominal_ssh::PortForwardFailure {
                rule_id: "rule-a".to_string(),
                label: "Forward".to_string(),
                kind: NativeForwardFailureKind::TargetUnreachable,
                message: "Forward Forward could not open SSH channel".to_string(),
            },
        ));

        let snapshot = inner.snapshot();
        assert_eq!(snapshot.phase, ForwardPhase::Listening);
        assert!(snapshot.failure.is_none());
        let relay = snapshot.relay_failure.expect("relay failure");
        assert_eq!(relay.kind, ForwardFailureKind::TargetUnreachable);
    }

    #[test]
    fn reconnecting_state_is_reported_and_cleared_by_the_next_binding() {
        let harness = harness(("host-1", "rule-a"));
        let inner = &harness.inner;
        inner.apply_event(binding("rule-a", miaominal_core::profile::PortForwardKind::Local));
        inner.apply_event(SessionEvent::PortForwardReconnecting {
            error: "transport closed".to_string(),
            attempt: 2,
            max_attempts: 10,
            retry_after_secs: 4,
        });

        let snapshot = inner.snapshot();
        assert_eq!(snapshot.phase, ForwardPhase::Reconnecting);
        assert_eq!(snapshot.reconnect_attempt, 2);
        assert_eq!(snapshot.reconnect_retry_after_secs, 4);
        assert!(snapshot.bound_port.is_none());

        inner.apply_event(binding("rule-a", miaominal_core::profile::PortForwardKind::Local));
        let snapshot = inner.snapshot();
        assert_eq!(snapshot.phase, ForwardPhase::Listening);
        assert_eq!(snapshot.reconnect_attempt, 0);
        assert!(snapshot.reconnect_error.is_none());
    }

    #[test]
    fn connection_stage_errors_are_classified_for_the_ui() {
        assert_eq!(
            classify_failure("failed to connect to 127.0.0.1:1"),
            ForwardFailureKind::Transport
        );
        assert_eq!(
            classify_failure("Forward Forward failed to bind on 127.0.0.1:8080"),
            ForwardFailureKind::PortOccupied
        );
        assert_eq!(
            classify_failure("Failed to start remote forward Forward: administratively prohibited"),
            ForwardFailureKind::RemoteRefused
        );
        assert_eq!(
            classify_failure("Forward Forward could not open SSH channel"),
            ForwardFailureKind::TargetUnreachable
        );
        assert_eq!(
            classify_failure("authentication failed: no more authentication methods"),
            ForwardFailureKind::Authentication
        );
        assert_eq!(
            classify_failure("something unexpected"),
            ForwardFailureKind::Other
        );
    }

    #[test]
    fn default_local_listen_host_is_applied_for_blank_rules() {        // The shared rule model stores the host; the bridge only normalizes a
        // blank value to loopback, matching the mobile default.
        let mut rule = miaominal_core::profile::PortForwardRule {
            id: "rule-a".to_string(),
            label: String::new(),
            kind: miaominal_core::profile::PortForwardKind::Local,
            listen_host: String::new(),
            listen_port: 8080,
            target_host: "127.0.0.1".to_string(),
            target_port: 80,
            enabled: false,
        };
        if rule.listen_host.trim().is_empty() {
            rule.listen_host = "127.0.0.1".to_string();
        }
        assert_eq!(rule.listen_host, "127.0.0.1");
    }
}
