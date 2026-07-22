use crate::backend::{BackendRoute, BackendRouter, ExecMode, SshExecRequest};
use crate::capabilities::RemoteCapabilities;
use crate::error::{AgentError, AgentResult};
use crate::jobs::{AgentJobId, AgentJobRegistry, AgentJobSummary, JobPollResult};
use crate::path_guard::{
    AuthorizedRemotePath, RemotePathKind, canonical_path_for_shell, resolve_workspace_path,
};
use crate::policy::{AgentPathAccess, AgentPolicy};
use crate::tools::{self, ListEntry};
use crate::web::{
    ConfiguredWebSearchProvider, DisabledWebSearchProvider, WebFetchConfig, WebSearchProvider,
};
use anyhow::anyhow;
use miaominal_core::profile::{AuthMethod, SessionProfile, ShellType};
use miaominal_secrets::SecretStore;
use miaominal_storage::known_hosts_store::KnownHostsStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::{Mutex, Notify, mpsc};

pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024;
pub const TERMINAL_OUTPUT_TAP_CAPACITY: usize = 8;
pub const TERMINAL_INTERRUPT_SETTLE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalOutputTapError {
    Closed,
    Overflow,
}

/// The state of a terminal command inside an Agent request's output lease.
///
/// A cancelled command is deliberately not releaseable until its unique sentinel is observed.
/// Ctrl-C only requests interruption; it does not prove that the shell has returned to a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalExecLeaseState {
    /// A request owns the tap but has not started a terminal command yet.
    Reserved,
    /// All command handles were dropped before a command was sent.
    Released,
    /// A terminal wrapper was sent and its sentinel has not been observed.
    Running,
    /// Ctrl-C was sent and the sentinel is still pending.
    InterruptRequested,
    /// The command's unique sentinel was observed. The owning Agent request may start another
    /// command until it explicitly retires the lease. A settled, retired lease may be handed off
    /// even while cleanup handles from the old request are still alive.
    Completed,
    /// Completion could not be confirmed before a timeout or transport failure.
    Unknown,
}

impl TerminalExecLeaseState {
    /// Whether the active command is known to have settled. This alone does not make the tap
    /// replaceable: the owning request must also retire it or drop all execution handles.
    pub fn is_settled(self) -> bool {
        matches!(self, Self::Released | Self::Completed)
    }
}

struct TerminalExecShared {
    state: StdMutex<TerminalExecState>,
    settled: Notify,
    // AgentExecChannel clones inside AgentToolSet/rig tools keep handles alive for the full
    // request. Per-command guards add owners so an executing command also remains protected if
    // its request container is concurrently dropped.
    owners: AtomicUsize,
}

struct TerminalExecState {
    lease_state: TerminalExecLeaseState,
    retired: bool,
    interrupt_attempted: bool,
    command_epoch: u64,
    sentinel: Option<TerminalExecSentinel>,
    used_sentinels: HashSet<Vec<u8>>,
    sentinel_tail: Vec<u8>,
}

#[derive(Clone)]
struct TerminalExecSentinel {
    command_epoch: u64,
    bytes: Vec<u8>,
}

impl Default for TerminalExecShared {
    fn default() -> Self {
        Self {
            state: StdMutex::new(TerminalExecState {
                lease_state: TerminalExecLeaseState::Reserved,
                retired: false,
                interrupt_attempted: false,
                command_epoch: 0,
                sentinel: None,
                used_sentinels: HashSet::new(),
                sentinel_tail: Vec::new(),
            }),
            settled: Notify::new(),
            owners: AtomicUsize::new(0),
        }
    }
}

impl TerminalExecShared {
    fn lease_state(&self) -> TerminalExecLeaseState {
        self.state
            .lock()
            .map(|state| state.lease_state)
            .unwrap_or(TerminalExecLeaseState::Unknown)
    }

    fn can_release(&self) -> bool {
        self.state.lock().is_ok_and(|state| {
            state.lease_state.is_settled()
                && (state.retired || self.owners.load(Ordering::Acquire) == 0)
        })
    }

    fn register_owner(&self) {
        self.owners.fetch_add(1, Ordering::Relaxed);
    }

    fn unregister_owner(&self) {
        let previous = self.owners.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "terminal exec owner count underflow");
        if previous != 1 {
            return;
        }

        let released = self.state.lock().is_ok_and(|mut state| {
            if state.lease_state != TerminalExecLeaseState::Reserved {
                return false;
            }
            state.lease_state = TerminalExecLeaseState::Released;
            true
        });
        if released {
            self.settled.notify_waiters();
        }
    }

    fn start_command(
        &self,
        sentinel: &str,
        send: impl FnOnce() -> AgentResult<()>,
    ) -> AgentResult<u64> {
        let mut state = self.state.lock().map_err(|_| {
            AgentError::Backend(anyhow!("terminal execution state lock is poisoned"))
        })?;
        if state.retired {
            return Err(AgentError::Backend(anyhow!(
                "terminal execution lease was retired before execution"
            )));
        }
        match state.lease_state {
            TerminalExecLeaseState::Reserved
            | TerminalExecLeaseState::Released
            | TerminalExecLeaseState::Completed => {}
            TerminalExecLeaseState::Running | TerminalExecLeaseState::InterruptRequested => {
                return Err(AgentError::Backend(anyhow!(
                    "another terminal command is already active"
                )));
            }
            TerminalExecLeaseState::Unknown => {
                return Err(AgentError::Backend(anyhow!(
                    "the previous terminal command may still be running"
                )));
            }
        }

        let command_epoch = state
            .command_epoch
            .checked_add(1)
            .ok_or_else(|| AgentError::Backend(anyhow!("terminal command epoch was exhausted")))?;
        let sentinel_bytes = sentinel.as_bytes().to_vec();
        if sentinel_bytes.is_empty() {
            return Err(AgentError::Backend(anyhow!(
                "terminal command sentinel cannot be empty"
            )));
        }
        if state.used_sentinels.contains(&sentinel_bytes) {
            return Err(AgentError::Backend(anyhow!(
                "terminal command sentinel was reused"
            )));
        }
        send()?;
        state.command_epoch = command_epoch;
        state.lease_state = TerminalExecLeaseState::Running;
        state.interrupt_attempted = false;
        state.used_sentinels.insert(sentinel_bytes.clone());
        state.sentinel = Some(TerminalExecSentinel {
            command_epoch,
            bytes: sentinel_bytes,
        });
        state.sentinel_tail.clear();
        Ok(command_epoch)
    }

    fn retire(&self) {
        let released = self.state.lock().is_ok_and(|mut state| {
            state.retired = true;
            if state.lease_state != TerminalExecLeaseState::Reserved {
                return false;
            }
            state.lease_state = TerminalExecLeaseState::Released;
            true
        });
        if released {
            self.settled.notify_waiters();
        }
    }

    fn cancel_matching(
        &self,
        expected_epoch: Option<u64>,
        send_interrupt: impl FnOnce() -> AgentResult<()>,
    ) -> AgentResult<bool> {
        let mut state = self.state.lock().map_err(|_| {
            AgentError::Backend(anyhow!("terminal execution state lock is poisoned"))
        })?;
        if expected_epoch.is_some_and(|epoch| epoch != state.command_epoch) {
            return Ok(false);
        }
        state.retired = true;
        match state.lease_state {
            TerminalExecLeaseState::Reserved => {
                state.lease_state = TerminalExecLeaseState::Released;
                drop(state);
                self.settled.notify_waiters();
                Ok(false)
            }
            TerminalExecLeaseState::Running | TerminalExecLeaseState::Unknown
                if state.sentinel.is_some() && !state.interrupt_attempted =>
            {
                state.interrupt_attempted = true;
                if state.lease_state == TerminalExecLeaseState::Running {
                    state.lease_state = TerminalExecLeaseState::InterruptRequested;
                }
                if let Err(error) = send_interrupt() {
                    state.lease_state = TerminalExecLeaseState::Unknown;
                    drop(state);
                    self.settled.notify_waiters();
                    return Err(error);
                }
                Ok(true)
            }
            TerminalExecLeaseState::Released
            | TerminalExecLeaseState::Running
            | TerminalExecLeaseState::InterruptRequested
            | TerminalExecLeaseState::Completed
            | TerminalExecLeaseState::Unknown => Ok(false),
        }
    }

    fn cancel(&self, send_interrupt: impl FnOnce() -> AgentResult<()>) -> AgentResult<bool> {
        self.cancel_matching(None, send_interrupt)
    }

    fn cancel_command(
        &self,
        command_epoch: u64,
        send_interrupt: impl FnOnce() -> AgentResult<()>,
    ) -> AgentResult<bool> {
        self.cancel_matching(Some(command_epoch), send_interrupt)
    }

    fn complete(&self, command_epoch: u64) {
        let completed = self.state.lock().is_ok_and(|mut state| {
            if state.command_epoch != command_epoch
                || state.lease_state == TerminalExecLeaseState::Completed
            {
                return false;
            }
            state.lease_state = TerminalExecLeaseState::Completed;
            state.sentinel = None;
            state.sentinel_tail.clear();
            true
        });
        if completed {
            self.settled.notify_waiters();
        }
    }

    fn mark_unknown_if_pending(&self, expected_epoch: Option<u64>) {
        let unknown = self.state.lock().is_ok_and(|mut state| {
            if expected_epoch.is_some_and(|epoch| epoch != state.command_epoch) {
                return false;
            }
            if !matches!(
                state.lease_state,
                TerminalExecLeaseState::Running | TerminalExecLeaseState::InterruptRequested
            ) {
                return false;
            }
            // Keep the sentinel detector armed. A late, unique sentinel is still definitive and
            // may safely move an Unknown lease to Completed.
            state.lease_state = TerminalExecLeaseState::Unknown;
            true
        });
        if unknown {
            self.settled.notify_waiters();
        }
    }

    fn observe_output(&self, bytes: &[u8]) -> Option<u64> {
        let (command_epoch, completed) = self.state.lock().ok().and_then(|mut state| {
            if !matches!(
                state.lease_state,
                TerminalExecLeaseState::Running
                    | TerminalExecLeaseState::InterruptRequested
                    | TerminalExecLeaseState::Unknown
            ) {
                return None;
            }
            let sentinel = state.sentinel.clone()?;
            if sentinel.command_epoch != state.command_epoch || sentinel.bytes.is_empty() {
                return None;
            }

            state.sentinel_tail.extend_from_slice(bytes);
            let pattern_len = sentinel.bytes.len() + 1;
            let found = state.sentinel_tail.windows(pattern_len).any(|window| {
                window.starts_with(&sentinel.bytes) && window[sentinel.bytes.len()].is_ascii_digit()
            });
            if found {
                state.lease_state = TerminalExecLeaseState::Completed;
                state.sentinel = None;
                state.sentinel_tail.clear();
                return Some((sentinel.command_epoch, true));
            }

            // Retain only enough suffix to recognize a sentinel split across output chunks.
            if state.sentinel_tail.len() > sentinel.bytes.len() {
                let drain_to = state.sentinel_tail.len() - sentinel.bytes.len();
                state.sentinel_tail.drain(..drain_to);
            }
            Some((sentinel.command_epoch, false))
        })?;
        if completed {
            self.settled.notify_waiters();
        }
        Some(command_epoch)
    }

    async fn wait_for_completion_matching(
        &self,
        expected_epoch: Option<u64>,
        timeout: Duration,
    ) -> TerminalExecLeaseState {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.settled.notified();
            let (command_epoch, state) = self
                .state
                .lock()
                .map(|state| (state.command_epoch, state.lease_state))
                .unwrap_or((0, TerminalExecLeaseState::Unknown));
            if expected_epoch.is_some_and(|epoch| epoch != command_epoch) {
                return state;
            }
            if !matches!(
                state,
                TerminalExecLeaseState::Running | TerminalExecLeaseState::InterruptRequested
            ) {
                return state;
            }

            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                self.mark_unknown_if_pending(expected_epoch);
                return self.lease_state();
            }
        }
    }

    async fn wait_for_completion(&self, timeout: Duration) -> TerminalExecLeaseState {
        self.wait_for_completion_matching(None, timeout).await
    }

    async fn wait_for_command_completion(
        &self,
        command_epoch: u64,
        timeout: Duration,
    ) -> TerminalExecLeaseState {
        self.wait_for_completion_matching(Some(command_epoch), timeout)
            .await
    }
}

#[derive(Clone)]
pub struct TerminalOutputTap {
    sender: Arc<StdMutex<Option<mpsc::Sender<Vec<u8>>>>>,
    overflowed: Arc<AtomicBool>,
    shared: Arc<TerminalExecShared>,
}

pub struct TerminalOutputReceiver {
    receiver: mpsc::Receiver<Vec<u8>>,
    overflowed: Arc<AtomicBool>,
    shared: Arc<TerminalExecShared>,
}

impl TerminalOutputTap {
    pub fn channel() -> (Self, TerminalOutputReceiver) {
        let (sender, receiver) = mpsc::channel(TERMINAL_OUTPUT_TAP_CAPACITY);
        let overflowed = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(TerminalExecShared::default());
        (
            Self {
                sender: Arc::new(StdMutex::new(Some(sender))),
                overflowed: overflowed.clone(),
                shared: shared.clone(),
            },
            TerminalOutputReceiver {
                receiver,
                overflowed,
                shared,
            },
        )
    }

    pub fn try_send(&self, bytes: Vec<u8>) -> Result<(), TerminalOutputTapError> {
        let mut sender = self
            .sender
            .lock()
            .map_err(|_| TerminalOutputTapError::Closed)?;
        let Some(active) = sender.as_ref() else {
            // Sentinel detection lives on the tap, not the receiver. It therefore keeps working
            // after a cancelled tool future drops its receiver.
            self.shared.observe_output(&bytes);
            return Err(TerminalOutputTapError::Closed);
        };
        // Keep the sender lock through both detection and delivery. A replacement may observe the
        // Completed state, but cannot close this sender until the sentinel chunk is queued.
        let observed_epoch = self.shared.observe_output(&bytes);
        match active.try_send(bytes) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.overflowed.store(true, Ordering::Release);
                sender.take();
                if let Some(command_epoch) = observed_epoch {
                    self.shared.mark_unknown_if_pending(Some(command_epoch));
                }
                Err(TerminalOutputTapError::Overflow)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                sender.take();
                if let Some(command_epoch) = observed_epoch {
                    self.shared.mark_unknown_if_pending(Some(command_epoch));
                }
                Err(TerminalOutputTapError::Closed)
            }
        }
    }

    pub fn close(&self) {
        if let Ok(mut sender) = self.sender.lock() {
            sender.take();
        }
        self.shared.mark_unknown_if_pending(None);
    }

    pub fn same_channel(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.sender, &other.sender)
    }

    pub fn lease_state(&self) -> TerminalExecLeaseState {
        self.shared.lease_state()
    }

    /// Permanently prevents the old request's handles from starting another command. Once the
    /// current command is settled, the terminal tap may be handed to a continuation even if
    /// cleanup handles from this request have not been dropped yet.
    pub fn retire_lease(&self) {
        self.shared.retire();
    }

    pub fn can_release_lease(&self) -> bool {
        self.shared.can_release()
    }
}

impl TerminalOutputReceiver {
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        self.receiver.recv().await
    }

    pub fn overflowed(&self) -> bool {
        self.overflowed.load(Ordering::Acquire)
    }
}

impl Drop for TerminalOutputReceiver {
    fn drop(&mut self) {
        self.shared.mark_unknown_if_pending(None);
    }
}

/// Handle for executing commands inside a user-visible terminal tab's PTY.
pub struct TerminalExecHandle {
    command_sender: miaominal_ssh::SessionCommandSender,
    output_tap: Arc<Mutex<Option<TerminalOutputReceiver>>>,
    shared: Arc<TerminalExecShared>,
}

struct TerminalExecCommandGuard {
    handle: TerminalExecHandle,
    command_epoch: u64,
    completed: bool,
}

impl TerminalExecCommandGuard {
    fn complete(&mut self) {
        // Mark the shared handle idle at the same synchronization boundary as command
        // completion. Otherwise a concurrent Stop can observe `active = true` after the
        // sentinel was received and send a stray Ctrl-C to the restored shell prompt.
        self.handle.shared.complete(self.command_epoch);
        self.completed = true;
    }

    async fn cancel_and_wait(&mut self, timeout: Duration) -> AgentResult<TerminalExecLeaseState> {
        let result = self
            .handle
            .cancel_command_and_wait(self.command_epoch, timeout)
            .await;
        self.completed = true;
        result
    }
}

impl Drop for TerminalExecCommandGuard {
    fn drop(&mut self) {
        if !self.completed
            && let Err(error) = self.handle.cancel_command(self.command_epoch)
        {
            log::debug!("failed to interrupt incomplete terminal command: {error:?}");
        }
    }
}

impl Clone for TerminalExecHandle {
    fn clone(&self) -> Self {
        self.shared.register_owner();
        Self {
            command_sender: self.command_sender.clone(),
            output_tap: self.output_tap.clone(),
            shared: self.shared.clone(),
        }
    }
}

impl Drop for TerminalExecHandle {
    fn drop(&mut self) {
        self.shared.unregister_owner();
    }
}

impl TerminalExecHandle {
    pub fn new(
        command_sender: miaominal_ssh::SessionCommandSender,
        output_tap: TerminalOutputReceiver,
    ) -> Self {
        let shared = output_tap.shared.clone();
        shared.register_owner();
        Self {
            command_sender,
            output_tap: Arc::new(Mutex::new(Some(output_tap))),
            shared,
        }
    }

    /// Cancels this request's terminal execution. If its wrapper has already been sent, this
    /// attempts exactly one Ctrl-C, including after an output transport failure changed the
    /// lease state to Unknown; otherwise it prevents the wrapper from being sent later.
    pub fn cancel(&self) -> AgentResult<bool> {
        self.shared.cancel(|| {
            self.command_sender
                .send_bytes(vec![0x03])
                .map_err(AgentError::from)
        })
    }

    fn cancel_command(&self, command_epoch: u64) -> AgentResult<bool> {
        self.shared.cancel_command(command_epoch, || {
            self.command_sender
                .send_bytes(vec![0x03])
                .map_err(AgentError::from)
        })
    }

    async fn cancel_command_and_wait(
        &self,
        command_epoch: u64,
        timeout: Duration,
    ) -> AgentResult<TerminalExecLeaseState> {
        self.cancel_command(command_epoch)?;
        Ok(self
            .shared
            .wait_for_command_completion(command_epoch, timeout)
            .await)
    }

    pub async fn cancel_and_wait(&self, timeout: Duration) -> AgentResult<TerminalExecLeaseState> {
        self.cancel()?;
        Ok(self.shared.wait_for_completion(timeout).await)
    }

    pub fn lease_state(&self) -> TerminalExecLeaseState {
        self.shared.lease_state()
    }

    pub fn can_release_lease(&self) -> bool {
        self.shared.can_release()
    }

    fn begin_command(
        &self,
        bytes: Vec<u8>,
        sentinel: &str,
    ) -> AgentResult<TerminalExecCommandGuard> {
        let command_epoch = self.shared.start_command(sentinel, || {
            self.command_sender
                .send_bytes(bytes)
                .map_err(AgentError::from)
        })?;
        Ok(TerminalExecCommandGuard {
            handle: self.clone(),
            command_epoch,
            completed: false,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolCallRequest {
    pub tool_name: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub route: Option<BackendRoute>,
    #[serde(default)]
    pub skip_policy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolCallResponse {
    pub tool_name: String,
    pub route: BackendRoute,
    pub output: ToolOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserQuestionChoice {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolOutput {
    Cancelled,
    WorkspaceInfo {
        host: String,
        user: String,
        platform: String,
        arch: String,
        shell: String,
        cwd: String,
        workspace_roots: Vec<String>,
        trusted_read_roots: Vec<String>,
        sensitive_paths: Vec<String>,
        capabilities: RemoteCapabilities,
        route: BackendRoute,
        supported_tools: Vec<String>,
    },
    Text {
        content: String,
        truncated: bool,
    },
    List {
        entries: Vec<String>,
        truncated: bool,
    },
    DirectoryList {
        path: String,
        entries: Vec<ListEntry>,
        truncated: bool,
    },
    Shell {
        result: ShellCommandResult,
    },
    JobStarted {
        job_id: AgentJobId,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        exec_shell: String,
        poll_after_ms: u64,
        next_action: String,
    },
    JobList {
        jobs: Vec<AgentJobSummary>,
    },
    JobPoll {
        result: JobPollResult,
    },
    WebSearch {
        results: Value,
    },
    WebFetch {
        url: String,
        content: String,
        truncated: bool,
    },
    Patch {
        summary: String,
        validation: Option<Box<ToolOutput>>,
    },
    Approval {
        message: String,
        operation_hash: Option<String>,
    },
    UserQuestion {
        message: String,
        choices: Vec<UserQuestionChoice>,
        allow_custom: bool,
        operation_hash: Option<String>,
    },
    UserResponse {
        answer: String,
        selected_index: Option<usize>,
        custom: bool,
        operation_hash: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShellCommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_status: i32,
    pub timed_out: bool,
    pub truncated: bool,
}

/// Shared cache for the actual shell used by SSH exec channels.
///
/// `AgentExecChannel` instances are intentionally cheap to rebuild, but the
/// detected shell must survive those rebuilds. Entries are scoped to a
/// profile connection fingerprint so editing the destination, user, proxy
/// route, or configured shell starts with a fresh detection state.
#[derive(Clone, Default)]
pub struct AgentShellRegistry {
    entries: Arc<StdMutex<HashMap<String, AgentShellRegistryEntry>>>,
}

struct AgentShellRegistryEntry {
    connection_fingerprint: String,
    detected_shell: Arc<AtomicU8>,
}

impl AgentShellRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn detected_shell_for_profile(&self, profile: &SessionProfile) -> Arc<AtomicU8> {
        let fingerprint = shell_connection_fingerprint(profile);
        let Ok(mut entries) = self.entries.lock() else {
            return Arc::new(AtomicU8::new(0));
        };
        let entry = entries
            .entry(profile.id.clone())
            .or_insert_with(|| AgentShellRegistryEntry {
                connection_fingerprint: fingerprint.clone(),
                detected_shell: Arc::new(AtomicU8::new(0)),
            });
        if entry.connection_fingerprint != fingerprint {
            *entry = AgentShellRegistryEntry {
                connection_fingerprint: fingerprint,
                detected_shell: Arc::new(AtomicU8::new(0)),
            };
        }
        entry.detected_shell.clone()
    }
}

fn shell_connection_fingerprint(profile: &SessionProfile) -> String {
    let configured_shell = match profile.shell_type {
        ShellType::Posix => 1,
        ShellType::Fish => 2,
        ShellType::PowerShell => 3,
        ShellType::Cmd => 4,
    };
    format!(
        "{}\0{}\0{}\0{}\0{}",
        profile.host.trim().to_ascii_lowercase(),
        profile.port,
        profile.username,
        configured_shell,
        profile.proxy_jump_profile_ids.join("\0"),
    )
}

#[derive(Clone)]
pub struct AgentExecChannel {
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    policy: AgentPolicy,
    backend_router: BackendRouter,
    jobs: AgentJobRegistry,
    web_search: Arc<dyn WebSearchProvider>,
    web_search_configured: bool,
    web_fetch: WebFetchConfig,
    use_pty: bool,
    terminal_exec: Option<TerminalExecHandle>,
    aux_channels: HashMap<String, AgentExecChannel>,
    policy_bypass: bool,
    /// Override set by workspace_info probe when the actual remote shell differs
    /// from the profile's configured shell_type.  `0` = unset (use profile).
    /// Uses AtomicU8 for lock-free interior mutability across clones.
    detected_shell: Arc<AtomicU8>,
}

impl AgentExecChannel {
    pub fn for_profile(
        profile: SessionProfile,
        all_profiles: Vec<SessionProfile>,
        secrets: SecretStore,
        known_hosts: KnownHostsStore,
    ) -> Self {
        Self::for_profile_with_jobs(
            profile,
            all_profiles,
            secrets,
            known_hosts,
            AgentJobRegistry::new(),
        )
    }

    pub fn for_profile_with_jobs(
        profile: SessionProfile,
        all_profiles: Vec<SessionProfile>,
        secrets: SecretStore,
        known_hosts: KnownHostsStore,
        jobs: AgentJobRegistry,
    ) -> Self {
        Self::for_profile_with_state(
            profile,
            all_profiles,
            secrets,
            known_hosts,
            jobs,
            Arc::new(AtomicU8::new(0)),
        )
    }

    pub fn for_profile_with_registries(
        profile: SessionProfile,
        all_profiles: Vec<SessionProfile>,
        secrets: SecretStore,
        known_hosts: KnownHostsStore,
        jobs: AgentJobRegistry,
        shells: AgentShellRegistry,
    ) -> Self {
        let detected_shell = shells.detected_shell_for_profile(&profile);
        Self::for_profile_with_state(
            profile,
            all_profiles,
            secrets,
            known_hosts,
            jobs,
            detected_shell,
        )
    }

    fn for_profile_with_state(
        profile: SessionProfile,
        all_profiles: Vec<SessionProfile>,
        secrets: SecretStore,
        known_hosts: KnownHostsStore,
        jobs: AgentJobRegistry,
        detected_shell: Arc<AtomicU8>,
    ) -> Self {
        Self {
            profile,
            all_profiles,
            secrets,
            known_hosts,
            policy: AgentPolicy,
            backend_router: BackendRouter::new(),
            jobs,
            web_search: Arc::new(DisabledWebSearchProvider),
            web_search_configured: false,
            web_fetch: WebFetchConfig::default(),
            use_pty: false,
            terminal_exec: None,
            aux_channels: HashMap::new(),
            policy_bypass: false,
            detected_shell,
        }
    }

    pub fn profile_id(&self) -> &str {
        &self.profile.id
    }

    pub fn policy(&self) -> &AgentPolicy {
        &self.policy
    }

    pub fn policy_bypass_enabled(&self) -> bool {
        self.policy_bypass
    }

    pub fn jobs(&self) -> AgentJobRegistry {
        self.jobs.clone()
    }

    pub fn profile_name(&self) -> &str {
        &self.profile.name
    }

    pub fn shell_label(&self) -> &'static str {
        match self.effective_shell_type() {
            ShellType::Posix => "posix-sh",
            ShellType::Fish => "fish",
            ShellType::PowerShell => "powershell",
            ShellType::Cmd => "cmd",
        }
    }

    pub fn shell_type(&self) -> ShellType {
        self.effective_shell_type()
    }

    pub fn detected_shell_type(&self) -> Option<ShellType> {
        match self.detected_shell.load(Ordering::Relaxed) {
            1 => Some(ShellType::Posix),
            2 => Some(ShellType::Fish),
            3 => Some(ShellType::PowerShell),
            4 => Some(ShellType::Cmd),
            _ => None,
        }
    }

    fn effective_shell_type(&self) -> ShellType {
        self.detected_shell_type()
            .unwrap_or(self.profile.shell_type)
    }

    /// Record the actual shell type detected by workspace_info probing.
    /// Once set, all tools dispatch to this shell type instead of the profile's
    /// configured value.
    pub fn set_detected_shell(&self, shell_type: ShellType) {
        let code: u8 = match shell_type {
            ShellType::Posix => 1,
            ShellType::Fish => 2,
            ShellType::PowerShell => 3,
            ShellType::Cmd => 4,
        };
        self.detected_shell.store(code, Ordering::Relaxed);
    }

    pub fn is_fish_shell(&self) -> bool {
        matches!(self.effective_shell_type(), ShellType::Fish)
    }

    pub fn web_search(&self) -> &dyn WebSearchProvider {
        self.web_search.as_ref()
    }

    pub fn web_search_enabled(&self) -> bool {
        self.web_search_configured
    }

    pub fn web_fetch_config(&self) -> &WebFetchConfig {
        &self.web_fetch
    }

    pub fn with_web_search_config(
        mut self,
        config: miaominal_settings::WebSearchConfig,
        api_key: Option<String>,
    ) -> Self {
        self.web_search = Arc::new(ConfiguredWebSearchProvider::new(config, api_key));
        self.web_search_configured = true;
        self
    }

    pub fn with_pty(mut self) -> Self {
        self.use_pty = true;
        self
    }

    pub fn with_policy_bypass(mut self, bypass: bool) -> Self {
        self.policy_bypass = bypass;
        self
    }

    pub fn with_terminal_exec(mut self, handle: TerminalExecHandle) -> Self {
        self.terminal_exec = Some(handle);
        self.use_pty = true;
        self
    }

    pub fn with_aux_channels(mut self, channels: HashMap<String, AgentExecChannel>) -> Self {
        self.aux_channels = channels;
        self
    }

    pub fn uses_pty(&self) -> bool {
        self.use_pty
    }

    pub fn terminal_exec(&self) -> Option<&TerminalExecHandle> {
        self.terminal_exec.as_ref()
    }

    pub(crate) async fn authorize_existing_path(
        &self,
        requested_path: &str,
        access: AgentPathAccess,
        expected_kind: RemotePathKind,
    ) -> AgentResult<AuthorizedRemotePath> {
        let normalized = resolve_workspace_path(requested_path)?;
        if self.policy_bypass_enabled() {
            return Ok(AuthorizedRemotePath::new(normalized));
        }

        self.policy.enforce_path(access, &normalized, true)?;
        let mut resolved = miaominal_sftp::resolve_profile_paths(
            self.profile.clone(),
            self.all_profiles.clone(),
            self.secrets.clone(),
            self.known_hosts.clone(),
            vec![normalized.clone()],
        )
        .await
        .map_err(|error| AgentError::Denied {
            tool_name: format!("{access:?}:{normalized}"),
            reason: format!("remote path could not be resolved safely: {error:#}"),
        })?;
        let resolved = resolved.pop().ok_or_else(|| AgentError::Denied {
            tool_name: format!("{access:?}:{normalized}"),
            reason: "remote path resolver returned no result".into(),
        })?;

        authorize_resolved_path(
            &self.policy,
            self.shell_type(),
            access,
            expected_kind,
            &normalized,
            resolved,
        )
    }

    pub async fn call_tool(
        &self,
        request: AgentToolCallRequest,
    ) -> AgentResult<AgentToolCallResponse> {
        if let Some(target) = request.arguments.get("target") {
            let target = target.as_str().ok_or_else(|| {
                AgentError::InvalidArguments("execution target must be a string".into())
            })?;
            let target = target.trim();
            let normalized_target = normalize_execution_target(target)?;
            let channel = self
                .aux_channels
                .get(target)
                .or_else(|| self.aux_channels.get(&normalized_target))
                .ok_or_else(|| {
                    AgentError::InvalidArguments(format!("unknown execution target `{target}`"))
                })?;
            let mut routed_request = request;
            remove_arg(&mut routed_request.arguments, "target");
            return Box::pin(channel.call_tool(routed_request)).await;
        }

        #[cfg(debug_assertions)]
        log::info!(
            "agent tool call requested: tool={} approved={} arguments={}",
            request.tool_name,
            request.approved,
            request.arguments
        );
        if !request.skip_policy {
            self.policy.enforce(&request.tool_name, request.approved)?;
            self.enforce_context_policy(&request)?;
        }
        let route = request.route.unwrap_or(BackendRoute::SshExec);
        self.backend_router.ensure_supported(route)?;
        self.ensure_posix_supported()?;
        let bypass_channel = request
            .skip_policy
            .then(|| self.clone().with_policy_bypass(true));
        let channel = bypass_channel.as_ref().unwrap_or(self);

        let output = match request.tool_name.as_str() {
            "workspace_info" => tools::workspace_info(channel).await?,
            "read" => tools::read(channel, parse_args(request.arguments)?).await?,
            "list" => tools::list(channel, parse_args(request.arguments)?).await?,
            "glob" => tools::glob(channel, parse_args(request.arguments)?).await?,
            "grep" => tools::grep(channel, parse_args(request.arguments)?).await?,
            "apply_patch" => tools::apply_patch(channel, parse_args(request.arguments)?).await?,
            "run_shell" => tools::run_shell(channel, parse_args(request.arguments)?).await?,
            "web_search" => tools::web_search(channel, parse_args(request.arguments)?).await?,
            "web_fetch" => {
                tools::web_fetch(channel, parse_args(request.arguments)?, request.approved).await?
            }
            "ask_user" => tools::ask_user(parse_args(request.arguments)?)?,
            "approval" => tools::approval(parse_args(request.arguments)?)?,
            other => return Err(AgentError::UnknownTool(other.to_string())),
        };

        Ok(AgentToolCallResponse {
            tool_name: request.tool_name,
            route,
            output,
        })
    }

    fn enforce_context_policy(&self, request: &AgentToolCallRequest) -> AgentResult<()> {
        match request.tool_name.as_str() {
            "read" | "list" => {
                if let Some(path) = string_arg(&request.arguments, "path") {
                    let path = resolve_workspace_path(path)?;
                    self.policy
                        .enforce_path(AgentPathAccess::Read, &path, request.approved)?;
                }
            }
            "glob" => {
                if let Some(root) = string_arg(&request.arguments, "root") {
                    let root = resolve_workspace_path(root)?;
                    self.policy
                        .enforce_path(AgentPathAccess::Read, &root, request.approved)?;
                }
            }
            "grep" => {
                if let Some(root) = string_arg(&request.arguments, "root") {
                    let root = resolve_workspace_path(root)?;
                    self.policy
                        .enforce_path(AgentPathAccess::Read, &root, request.approved)?;
                }
            }
            "apply_patch" => {
                let base_dir = string_arg(&request.arguments, "base_dir").unwrap_or(".");
                let base_dir = resolve_workspace_path(base_dir)?;
                self.policy
                    .enforce_path(AgentPathAccess::Edit, &base_dir, request.approved)?;
            }
            "run_shell" => {
                if let Some(cwd) = string_arg(&request.arguments, "cwd") {
                    let cwd = resolve_workspace_path(cwd)?;
                    self.policy
                        .enforce_path(AgentPathAccess::Read, &cwd, request.approved)?;
                }
                if let Some(command) = string_arg(&request.arguments, "command") {
                    self.policy.enforce_command(command, request.approved)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn ensure_posix_supported(&self) -> AgentResult<()> {
        if matches!(
            self.profile.effective_auth_method(),
            AuthMethod::KeyboardInteractive
        ) {
            return Err(AgentError::Backend(anyhow!(
                "keyboard-interactive authentication is not supported for agent exec channel"
            )));
        }
        // All ShellType variants are now supported (Posix, Fish, PowerShell, Cmd).
        Ok(())
    }

    pub async fn exec(&self, command: impl Into<String>) -> AgentResult<String> {
        self.exec_with_mode(BackendRoute::SshExec, command, ExecMode::Raw)
            .await
    }

    pub async fn exec_via(
        &self,
        route: BackendRoute,
        command: impl Into<String>,
    ) -> AgentResult<String> {
        self.exec_with_mode(route, command, ExecMode::Raw).await
    }

    pub async fn exec_with_mode(
        &self,
        route: BackendRoute,
        command: impl Into<String>,
        mode: ExecMode,
    ) -> AgentResult<String> {
        self.backend_router
            .exec(
                route,
                SshExecRequest {
                    profile: self.profile.clone(),
                    all_profiles: self.all_profiles.clone(),
                    secrets: self.secrets.clone(),
                    known_hosts: self.known_hosts.clone(),
                    command: command.into(),
                    mode,
                },
            )
            .await
    }

    /// Execute a command inside the user-visible terminal tab's PTY.
    ///
    /// WinkTerm-inspired approach: no stty-echo, no bracketed paste.
    /// The wrapper is sent as raw bytes + \r. The caller provides a unique
    /// sentinel string that the wrapper prints after the command finishes,
    /// along with the exit code and $PWD. Output is collected from the
    /// output tap until the sentinel is confirmed (followed by a digit).
    ///
    /// The sentinel appears twice in the tap: once as a literal inside the
    /// echoed wrapper text, and once as the actual evaluated output.
    /// `rfind` + digit-check discriminates the real occurrence.
    pub async fn exec_via_terminal(
        &self,
        command: impl Into<String>,
        sentinel: &str,
        timeout_secs: u64,
    ) -> AgentResult<String> {
        let handle = self
            .terminal_exec
            .as_ref()
            .ok_or_else(|| AgentError::Backend(anyhow!("terminal exec is not available")))?;

        // Send wrapper + Enter as raw bytes (no stty, no bracketed paste).
        let wrapper = format!("{}\r", command.into());
        let mut command_guard = handle.begin_command(wrapper.into_bytes(), sentinel)?;

        let mut collected = String::new();
        let sentinel_owned = sentinel.to_string();
        let wait_result = {
            let mut guard = handle.output_tap.lock().await;
            let receiver = guard
                .as_mut()
                .ok_or_else(|| AgentError::Backend(anyhow!("terminal output tap is closed")))?;

            tokio::time::timeout(Duration::from_secs(timeout_secs.max(1)), async {
                loop {
                    let Some(chunk) = receiver.recv().await else {
                        let message = if receiver.overflowed() {
                            "terminal output tap overflowed before command completion"
                        } else {
                            "terminal output tap closed before command completion"
                        };
                        return Err(AgentError::Backend(anyhow!(message)));
                    };
                    collected.push_str(&String::from_utf8_lossy(&chunk));
                    // The sentinel string appears twice:
                    //   (a) Literal in echoed wrapper → followed by '%' (not a digit)
                    //   (b) Real output                     → followed by '0'..'9'
                    // rfind + digit-check picks the real one.
                    if let Some(pos) = collected.rfind(&sentinel_owned) {
                        let after = &collected[pos + sentinel_owned.len()..];
                        if after.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                            return Ok(());
                        }
                    }
                    if collected.len() > DEFAULT_MAX_OUTPUT_BYTES * 4 {
                        let drain_to = collected
                            .ceil_char_boundary(collected.len() - DEFAULT_MAX_OUTPUT_BYTES * 2);
                        collected.drain(..drain_to);
                    }
                }
            })
            .await
        };

        match wait_result {
            Ok(Ok(())) => command_guard.complete(),
            Ok(Err(error)) => {
                let _ = command_guard
                    .cancel_and_wait(TERMINAL_INTERRUPT_SETTLE_TIMEOUT)
                    .await;
                return Err(error);
            }
            Err(_) => {
                let _ = command_guard
                    .cancel_and_wait(TERMINAL_INTERRUPT_SETTLE_TIMEOUT)
                    .await;
                return Err(AgentError::Backend(anyhow!(
                    "timed out waiting for terminal command completion"
                )));
            }
        }

        Ok(collected)
    }
}

fn authorize_resolved_path(
    policy: &AgentPolicy,
    shell_type: ShellType,
    access: AgentPathAccess,
    expected_kind: RemotePathKind,
    normalized_request: &str,
    resolved: miaominal_sftp::ResolvedRemotePath,
) -> AgentResult<AuthorizedRemotePath> {
    policy.enforce_path(access, &resolved.canonical_path, true)?;
    let kind_matches = match expected_kind {
        RemotePathKind::File => matches!(resolved.kind, miaominal_sftp::SftpEntryKind::File),
        RemotePathKind::Directory => {
            matches!(resolved.kind, miaominal_sftp::SftpEntryKind::Directory)
        }
    };
    if !kind_matches {
        return Err(AgentError::InvalidPath(format!(
            "`{normalized_request}` does not resolve to the required {expected_kind:?} path"
        )));
    }

    let execution_path = canonical_path_for_shell(&resolved.canonical_path, shell_type)?;
    Ok(AuthorizedRemotePath::new(execution_path))
}

fn string_arg<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(Value::as_str)
}

fn normalize_execution_target(target: &str) -> AgentResult<String> {
    let target = target.trim();
    let name = target.trim_start_matches('@').trim();
    if name.is_empty() {
        return Err(AgentError::InvalidArguments(
            "execution target cannot be empty".into(),
        ));
    }
    Ok(format!("@{name}"))
}

fn remove_arg(arguments: &mut Value, key: &str) {
    if let Some(object) = arguments.as_object_mut() {
        object.remove(key);
    }
}

fn parse_args<T>(arguments: Value) -> AgentResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(arguments)
        .map_err(|error| AgentError::InvalidArguments(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(shell_type: ShellType) -> SessionProfile {
        let mut profile = SessionProfile::blank("session-1", 1);
        profile.host = "example.com".into();
        profile.username = "akko".into();
        profile.shell_type = shell_type;
        profile
    }

    #[tokio::test]
    async fn terminal_output_tap_is_bounded_and_closes_on_overflow() {
        let (tap, mut receiver) = TerminalOutputTap::channel();
        for index in 0..TERMINAL_OUTPUT_TAP_CAPACITY {
            tap.try_send(vec![index as u8])
                .expect("tap should accept configured capacity");
        }
        assert_eq!(
            tap.try_send(vec![255]),
            Err(TerminalOutputTapError::Overflow)
        );
        assert!(receiver.overflowed());

        for expected in 0..TERMINAL_OUTPUT_TAP_CAPACITY {
            assert_eq!(receiver.recv().await, Some(vec![expected as u8]));
        }
        assert_eq!(receiver.recv().await, None);
    }

    #[tokio::test]
    async fn terminal_output_tap_clones_share_close_state() {
        let (tap, mut receiver) = TerminalOutputTap::channel();
        let clone = tap.clone();
        assert!(tap.same_channel(&clone));
        clone.close();
        assert_eq!(tap.try_send(vec![1]), Err(TerminalOutputTapError::Closed));
        assert_eq!(receiver.recv().await, None);
        assert!(!receiver.overflowed());
    }

    #[test]
    fn terminal_lease_requires_the_evaluated_sentinel() {
        let (tap, receiver) = TerminalOutputTap::channel();
        receiver
            .shared
            .start_command("MIAOMINAL_TEST_", || Ok(()))
            .unwrap();

        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Running);
        assert!(!tap.can_release_lease());

        // The echoed wrapper contains the sentinel followed by formatting syntax, not an exit
        // status, and therefore must not release the lease.
        tap.try_send(b"printf 'MIAOMINAL_TEST_%s'".to_vec())
            .unwrap();
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Running);

        // Verify that the definitive sentinel can be split across terminal output chunks.
        tap.try_send(b"\r\nMIAOMINAL_".to_vec()).unwrap();
        tap.try_send(b"TEST_130 /tmp\r\n".to_vec()).unwrap();
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Completed);
        assert!(tap.can_release_lease());
    }

    #[test]
    fn completed_terminal_lease_stays_owned_until_request_handles_are_dropped() {
        let (tap, receiver) = TerminalOutputTap::channel();
        receiver.shared.register_owner();
        receiver
            .shared
            .start_command("MIAOMINAL_FIRST_", || Ok(()))
            .unwrap();

        tap.try_send(b"MIAOMINAL_FIRST_0 /tmp\r\n".to_vec())
            .unwrap();
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Completed);
        assert!(TerminalExecLeaseState::Completed.is_settled());
        assert!(
            !tap.can_release_lease(),
            "another request must not replace a completed tap while its owner can reuse it"
        );

        receiver
            .shared
            .start_command("MIAOMINAL_SECOND_", || Ok(()))
            .expect("the owning request may run another command on its lease");
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Running);
        tap.try_send(b"MIAOMINAL_SECOND_0 /tmp\r\n".to_vec())
            .unwrap();

        receiver.shared.unregister_owner();
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Completed);
        assert!(tap.can_release_lease());
    }

    #[test]
    fn stale_command_completion_cannot_complete_the_next_epoch() {
        let (tap, receiver) = TerminalOutputTap::channel();
        let first_epoch = receiver
            .shared
            .start_command("MIAOMINAL_EPOCH_FIRST_", || Ok(()))
            .unwrap();
        tap.try_send(b"MIAOMINAL_EPOCH_FIRST_0 /tmp\r\n".to_vec())
            .unwrap();

        let second_epoch = receiver
            .shared
            .start_command("MIAOMINAL_EPOCH_SECOND_", || Ok(()))
            .unwrap();
        assert!(second_epoch > first_epoch);
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Running);

        // Models command 1's receiver observing its already-delivered sentinel only after
        // command 2 has started. Its guard must not clear command 2's active sentinel/state.
        receiver.shared.complete(first_epoch);
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Running);
        // Likewise, a late delivery failure for command 1 must not mark command 2 Unknown.
        receiver.shared.mark_unknown_if_pending(Some(first_epoch));
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Running);

        tap.try_send(b"MIAOMINAL_EPOCH_SECOND_0 /tmp\r\n".to_vec())
            .unwrap();
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Completed);
    }

    #[tokio::test]
    async fn stale_command_wait_cannot_timeout_the_next_epoch() {
        let (tap, receiver) = TerminalOutputTap::channel();
        let first_epoch = receiver
            .shared
            .start_command("MIAOMINAL_WAIT_FIRST_", || Ok(()))
            .unwrap();
        tap.try_send(b"MIAOMINAL_WAIT_FIRST_0 /tmp\r\n".to_vec())
            .unwrap();
        receiver
            .shared
            .start_command("MIAOMINAL_WAIT_SECOND_", || Ok(()))
            .unwrap();

        let observed = receiver
            .shared
            .wait_for_command_completion(first_epoch, Duration::from_millis(1))
            .await;
        assert_eq!(observed, TerminalExecLeaseState::Running);
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Running);

        tap.try_send(b"MIAOMINAL_WAIT_SECOND_0 /tmp\r\n".to_vec())
            .unwrap();
    }

    #[test]
    fn terminal_command_rejects_a_reused_sentinel_across_epochs() {
        let (tap, receiver) = TerminalOutputTap::channel();
        receiver
            .shared
            .start_command("MIAOMINAL_REUSED_", || Ok(()))
            .unwrap();
        tap.try_send(b"MIAOMINAL_REUSED_0 /tmp\r\n".to_vec())
            .unwrap();

        let mut command_sent = false;
        assert!(
            receiver
                .shared
                .start_command("MIAOMINAL_REUSED_", || {
                    command_sent = true;
                    Ok(())
                })
                .is_err()
        );
        assert!(!command_sent);
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Completed);
    }

    #[test]
    fn stale_command_guard_drop_cannot_cancel_or_retire_the_next_epoch() {
        let (tap, receiver) = TerminalOutputTap::channel();
        let first_epoch = receiver
            .shared
            .start_command("MIAOMINAL_DROP_FIRST_", || Ok(()))
            .unwrap();
        tap.try_send(b"MIAOMINAL_DROP_FIRST_0 /tmp\r\n".to_vec())
            .unwrap();

        let second_epoch = receiver
            .shared
            .start_command("MIAOMINAL_DROP_SECOND_", || Ok(()))
            .unwrap();
        let mut interrupt_attempts = 0;
        assert!(
            !receiver
                .shared
                .cancel_command(first_epoch, || {
                    interrupt_attempts += 1;
                    Ok(())
                })
                .unwrap()
        );
        assert_eq!(interrupt_attempts, 0);
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Running);

        tap.try_send(b"MIAOMINAL_DROP_SECOND_0 /tmp\r\n".to_vec())
            .unwrap();
        let third_epoch = receiver
            .shared
            .start_command("MIAOMINAL_DROP_THIRD_", || Ok(()))
            .expect("a stale guard must not retire the request lease");
        assert!(third_epoch > second_epoch);
        tap.try_send(b"MIAOMINAL_DROP_THIRD_0 /tmp\r\n".to_vec())
            .unwrap();
    }

    #[test]
    fn retired_completed_terminal_lease_is_replaceable_while_cleanup_handle_survives() {
        let (tap, receiver) = TerminalOutputTap::channel();
        receiver.shared.register_owner();
        receiver
            .shared
            .start_command("MIAOMINAL_HANDOFF_", || Ok(()))
            .unwrap();
        tap.try_send(b"MIAOMINAL_HANDOFF_0 /tmp\r\n".to_vec())
            .unwrap();

        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Completed);
        assert!(!tap.can_release_lease());
        let mut interrupt_attempts = 0;
        assert!(
            !receiver
                .shared
                .cancel(|| {
                    interrupt_attempts += 1;
                    Ok(())
                })
                .unwrap()
        );
        assert_eq!(interrupt_attempts, 0);
        assert!(
            tap.can_release_lease(),
            "a settled retired lease must not wait for cleanup handles to drop"
        );

        let mut command_sent = false;
        assert!(
            receiver
                .shared
                .start_command("MIAOMINAL_STALE_", || {
                    command_sent = true;
                    Ok(())
                })
                .is_err()
        );
        assert!(!command_sent, "a retired handle must never send a command");

        assert!(
            !receiver
                .shared
                .cancel(|| {
                    interrupt_attempts += 1;
                    Ok(())
                })
                .unwrap()
        );
        assert_eq!(
            interrupt_attempts, 0,
            "cleanup of a retired settled handle must not interrupt its successor"
        );

        receiver.shared.unregister_owner();
    }

    #[test]
    fn overflowed_terminal_tap_still_attempts_one_interrupt_and_accepts_late_sentinel() {
        let (tap, receiver) = TerminalOutputTap::channel();
        receiver.shared.register_owner();
        receiver
            .shared
            .start_command("MIAOMINAL_OVERFLOW_", || Ok(()))
            .unwrap();

        for index in 0..TERMINAL_OUTPUT_TAP_CAPACITY {
            tap.try_send(vec![index as u8]).unwrap();
        }
        assert_eq!(
            tap.try_send(vec![255]),
            Err(TerminalOutputTapError::Overflow)
        );
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Unknown);

        let mut interrupt_attempts = 0;
        assert!(
            receiver
                .shared
                .cancel(|| {
                    interrupt_attempts += 1;
                    Ok(())
                })
                .unwrap()
        );
        assert!(
            !receiver
                .shared
                .cancel(|| {
                    interrupt_attempts += 1;
                    Ok(())
                })
                .unwrap()
        );
        assert_eq!(interrupt_attempts, 1);
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Unknown);
        assert!(!tap.can_release_lease());

        assert_eq!(
            tap.try_send(b"MIAOMINAL_OVERFLOW_130 /tmp\r\n".to_vec()),
            Err(TerminalOutputTapError::Closed)
        );
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Completed);
        assert!(tap.can_release_lease());

        receiver.shared.unregister_owner();
        assert!(tap.can_release_lease());
    }

    #[tokio::test]
    async fn interrupted_terminal_lease_stays_busy_after_timeout_until_late_sentinel() {
        let (tap, receiver) = TerminalOutputTap::channel();
        receiver
            .shared
            .start_command("MIAOMINAL_STOP_", || Ok(()))
            .unwrap();
        receiver.shared.state.lock().unwrap().lease_state =
            TerminalExecLeaseState::InterruptRequested;

        let state = receiver
            .shared
            .wait_for_completion(Duration::from_millis(1))
            .await;
        assert_eq!(state, TerminalExecLeaseState::Unknown);
        assert!(!tap.can_release_lease());

        // Dropping the tool receiver must not disable the tap-side sentinel detector. A unique
        // late sentinel is still definitive and is the only event that makes reuse safe.
        drop(receiver);
        assert_eq!(
            tap.try_send(b"MIAOMINAL_STOP_130 /tmp\r\n".to_vec()),
            Err(TerminalOutputTapError::Closed)
        );
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Completed);
        assert!(tap.can_release_lease());
    }

    #[test]
    fn abandoned_reserved_terminal_lease_becomes_replaceable() {
        let (tap, receiver) = TerminalOutputTap::channel();
        receiver.shared.register_owner();
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Reserved);
        assert!(!tap.can_release_lease());

        receiver.shared.unregister_owner();
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Released);
        assert!(tap.can_release_lease());
    }

    #[test]
    fn retired_reserved_terminal_lease_is_replaceable_while_cleanup_handle_survives() {
        let (tap, receiver) = TerminalOutputTap::channel();
        receiver.shared.register_owner();
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Reserved);

        let mut interrupt_attempts = 0;
        assert!(
            !receiver
                .shared
                .cancel(|| {
                    interrupt_attempts += 1;
                    Ok(())
                })
                .unwrap()
        );
        assert_eq!(interrupt_attempts, 0);
        assert_eq!(tap.lease_state(), TerminalExecLeaseState::Released);
        assert!(tap.can_release_lease());

        let mut command_sent = false;
        assert!(
            receiver
                .shared
                .start_command("MIAOMINAL_STALE_", || {
                    command_sent = true;
                    Ok(())
                })
                .is_err()
        );
        assert!(!command_sent);

        assert!(
            !receiver
                .shared
                .cancel(|| {
                    interrupt_attempts += 1;
                    Ok(())
                })
                .unwrap()
        );
        assert_eq!(interrupt_attempts, 0);

        receiver.shared.unregister_owner();
    }

    #[test]
    fn default_route_is_ssh_exec() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Posix),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-route")),
        );

        assert!(
            channel
                .backend_router
                .ensure_supported(BackendRoute::SshExec)
                .is_ok()
        );
    }

    #[test]
    fn execution_targets_are_normalized_to_exact_at_markers() {
        assert_eq!(normalize_execution_target("host-a").unwrap(), "@host-a");
        assert_eq!(normalize_execution_target(" @host-a ").unwrap(), "@host-a");
        assert!(normalize_execution_target("@@  ").is_err());
    }

    #[tokio::test]
    async fn unknown_execution_targets_fail_instead_of_falling_back_to_active_channel() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Posix),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-target")),
        );
        let error = channel
            .call_tool(AgentToolCallRequest {
                tool_name: "run_shell".to_string(),
                arguments: serde_json::json!({
                    "command": "pwd",
                    "target": "missing-host",
                }),
                approved: true,
                route: None,
                skip_policy: true,
            })
            .await
            .expect_err("unknown target must not execute on the active channel");

        assert!(matches!(error, AgentError::InvalidArguments(_)));
    }

    #[tokio::test]
    async fn malformed_execution_targets_fail_instead_of_falling_back_to_active_channel() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Posix),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-bad-target")),
        );

        for target in [
            serde_json::Value::Null,
            serde_json::json!(1),
            serde_json::json!(true),
            serde_json::json!({ "host": "example" }),
        ] {
            let error = channel
                .call_tool(AgentToolCallRequest {
                    tool_name: "run_shell".to_string(),
                    arguments: serde_json::json!({
                        "command": "pwd",
                        "target": target,
                    }),
                    approved: true,
                    route: None,
                    skip_policy: true,
                })
                .await
                .expect_err("malformed target must not execute on the active channel");

            assert!(matches!(error, AgentError::InvalidArguments(_)));
        }
    }

    #[test]
    fn unsupported_routes_return_typed_errors() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Posix),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-unsupported")),
        );

        for route in [BackendRoute::Sftp, BackendRoute::Local] {
            assert!(matches!(
                channel.backend_router.ensure_supported(route),
                Err(AgentError::UnsupportedRoute(_))
            ));
        }
    }

    #[test]
    fn non_posix_profiles_are_now_supported() {
        for shell_type in [ShellType::PowerShell, ShellType::Cmd] {
            let channel = AgentExecChannel::for_profile(
                profile(shell_type),
                Vec::new(),
                SecretStore::new_locked_vault(),
                KnownHostsStore::with_path(
                    std::env::temp_dir().join("agent-known-hosts-non-posix"),
                ),
            );

            assert!(
                channel.ensure_posix_supported().is_ok(),
                "{shell_type:?} should be supported by ensure_posix_supported",
            );
        }
    }

    #[test]
    fn powershell_profile_passes_ensure_supported() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::PowerShell),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-powershell")),
        );

        assert!(
            channel.ensure_posix_supported().is_ok(),
            "PowerShell profile should be accepted by ensure_posix_supported",
        );
    }

    #[test]
    fn cmd_profile_passes_ensure_supported() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Cmd),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-cmd")),
        );

        assert!(
            channel.ensure_posix_supported().is_ok(),
            "Cmd profile should be accepted by ensure_posix_supported",
        );
    }

    #[test]
    fn detected_shell_overrides_profile_shell() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::PowerShell),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-detected-shell")),
        );

        assert_eq!(channel.detected_shell_type(), None);
        assert_eq!(channel.shell_label(), "powershell");

        channel.set_detected_shell(ShellType::Cmd);

        assert_eq!(channel.detected_shell_type(), Some(ShellType::Cmd));
        assert_eq!(channel.shell_label(), "cmd");
    }

    #[test]
    fn legacy_job_started_output_defaults_exec_shell() {
        let value = serde_json::json!({
            "kind": "job_started",
            "job_id": AgentJobId::new(),
            "poll_after_ms": 1000,
            "next_action": "poll"
        });
        let output: ToolOutput = serde_json::from_value(value).unwrap();

        match output {
            ToolOutput::JobStarted { exec_shell, .. } => assert!(exec_shell.is_empty()),
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[test]
    fn policy_bypass_is_explicit() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Posix),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-policy-bypass")),
        );

        assert!(!channel.policy_bypass_enabled());
        assert!(channel.with_policy_bypass(true).policy_bypass_enabled());
    }

    #[test]
    fn context_policy_denies_sensitive_read_paths() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Posix),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-policy-read")),
        );
        for path in [".env", ".ssh/id_rsa"] {
            let request = AgentToolCallRequest {
                tool_name: "read".into(),
                arguments: serde_json::json!({ "path": path }),
                approved: true,
                route: None,
                skip_policy: false,
            };

            assert!(matches!(
                channel.enforce_context_policy(&request),
                Err(AgentError::Denied { .. })
            ));
        }
    }

    #[test]
    fn canonical_sensitive_link_targets_are_denied() {
        let resolved = miaominal_sftp::ResolvedRemotePath {
            requested_path: "safe-link".into(),
            canonical_path: "/home/user/.ssh/id_rsa".into(),
            kind: miaominal_sftp::SftpEntryKind::File,
            is_symlink: true,
        };

        assert!(matches!(
            authorize_resolved_path(
                &AgentPolicy,
                ShellType::Posix,
                AgentPathAccess::Read,
                RemotePathKind::File,
                "safe-link",
                resolved,
            ),
            Err(AgentError::Denied { .. })
        ));
    }

    #[test]
    fn safe_canonical_link_targets_are_allowed_and_used() {
        let resolved = miaominal_sftp::ResolvedRemotePath {
            requested_path: "safe-link".into(),
            canonical_path: "/home/user/project/real.txt".into(),
            kind: miaominal_sftp::SftpEntryKind::File,
            is_symlink: true,
        };
        let authorized = authorize_resolved_path(
            &AgentPolicy,
            ShellType::Posix,
            AgentPathAccess::Read,
            RemotePathKind::File,
            "safe-link",
            resolved,
        )
        .unwrap();

        assert_eq!(authorized.as_str(), "/home/user/project/real.txt");
    }

    #[test]
    fn context_policy_requires_approval_for_service_restart() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Posix),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-policy-shell")),
        );
        let mut request = AgentToolCallRequest {
            tool_name: "run_shell".into(),
            arguments: serde_json::json!({ "command": "systemctl restart nginx", "cwd": "." }),
            approved: false,
            route: None,
            skip_policy: false,
        };

        assert!(matches!(
            channel.enforce_context_policy(&request),
            Err(AgentError::ApprovalRequired { .. })
        ));
        request.approved = true;
        assert!(channel.enforce_context_policy(&request).is_ok());
    }

    #[test]
    fn terminal_output_compaction_handles_multibyte_boundaries() {
        let mut collected = "🚀".repeat(DEFAULT_MAX_OUTPUT_BYTES);
        collected.push('你');

        assert!(collected.len() > DEFAULT_MAX_OUTPUT_BYTES * 4);

        let target_boundary = collected.len() - DEFAULT_MAX_OUTPUT_BYTES * 2;
        assert!(!collected.is_char_boundary(target_boundary));

        let compacted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let drain_to = collected.ceil_char_boundary(target_boundary);
            collected.drain(..drain_to);
        }));

        assert!(compacted.is_ok());
        assert!(collected.len() <= DEFAULT_MAX_OUTPUT_BYTES * 2);
        assert!(collected.is_char_boundary(collected.len()));
        assert!(
            collected
                .chars()
                .all(|character| character == '🚀' || character == '你')
        );
    }
}
