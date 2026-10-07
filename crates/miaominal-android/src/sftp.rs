//! Interactive SFTP sessions, directory browsing, and transfer tracking.
//!
//! One `SftpSession` owns one interactive SFTP connection (target plus any
//! jump nodes). Connection phases, host-key prompts, and keyboard-interactive
//! challenges are typed events with request ids, published through the
//! backend event bus while the lifecycle lock is held. Directory listings keep
//! the newest request id and drop late responses. Transfers are tracked per
//! session and exposed both as pull-based snapshots (progress) and reliable
//! control events (terminal state changes).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::StreamExt as _;
use tokio::sync::{Notify, watch};

use miaominal_core::forwarding::HostKeyPrompt;
use miaominal_core::sftp::{SftpEntry, SftpEntryKind, TransferDirection, TransferId};
use miaominal_sftp::{
    LocalTransferDestination, LocalTransferSource, SftpEvent, SftpEventReceiver,
    SftpProgressReceiver, SftpSessionPhase as NativeSftpPhase, SftpTransferChildState,
    SftpTransferProgress,
};

use crate::error::BackendError;
use crate::events::{BackendEvent, EventBus};
use crate::local_file::{LocalFileHandle, RegisteredFileSource, SafLocalFileSystem};
use crate::platform::PlatformHost;
use crate::session::{SshHostKeyDecision, SshPromptDto};

/// Android starts with two concurrent transfers, matching the mobile budget.
pub(crate) const ANDROID_TRANSFER_CONCURRENCY: usize = 2;
/// Number of child entries retained per transfer snapshot.
const MAX_TRACKED_CHILDREN: usize = 64;
/// Maximum time `close` waits for the session task to finish.
const CLOSE_GRACE: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SftpSessionPhase {
    Connecting,
    WaitingForTrust,
    WaitingForAuthentication,
    OpeningSubsystem,
    Ready,
    Closing,
    Closed,
    Failed,
}

impl SftpSessionPhase {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Closed | Self::Failed)
    }
}

fn phase_from_native(phase: NativeSftpPhase) -> SftpSessionPhase {
    match phase {
        NativeSftpPhase::Connecting => SftpSessionPhase::Connecting,
        NativeSftpPhase::WaitingForTrust => SftpSessionPhase::WaitingForTrust,
        NativeSftpPhase::WaitingForAuthentication => SftpSessionPhase::WaitingForAuthentication,
        NativeSftpPhase::OpeningSubsystem => SftpSessionPhase::OpeningSubsystem,
        NativeSftpPhase::Ready => SftpSessionPhase::Ready,
        NativeSftpPhase::Closing => SftpSessionPhase::Closing,
        NativeSftpPhase::Closed => SftpSessionPhase::Closed,
        NativeSftpPhase::Failed => SftpSessionPhase::Failed,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SftpRequestKind {
    HostKey,
    KeyboardInteractive,
}

/// Pending host-trust or authentication request, bound to its connection and
/// hop. The backend keeps it until it is answered or expires, so a rebuilt
/// page can still display and answer it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SftpRequest {
    pub request_id: u64,
    pub connection_id: u64,
    pub kind: SftpRequestKind,
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

/// Remote directory operation that failed outside a transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SftpOperation {
    CreateDirectory,
    RemoveFile,
    RemoveDirectory,
    Rename,
    ListDirectory,
    ListSubdirectory,
}

impl From<miaominal_sftp::SftpOperation> for SftpOperation {
    fn from(operation: miaominal_sftp::SftpOperation) -> Self {
        match operation {
            miaominal_sftp::SftpOperation::CreateDirectory => Self::CreateDirectory,
            miaominal_sftp::SftpOperation::RemoveFile => Self::RemoveFile,
            miaominal_sftp::SftpOperation::RemoveDirectory => Self::RemoveDirectory,
            miaominal_sftp::SftpOperation::Rename => Self::Rename,
            miaominal_sftp::SftpOperation::ListDirectory => Self::ListDirectory,
            miaominal_sftp::SftpOperation::ListSubdirectory => Self::ListSubdirectory,
        }
    }
}

/// Latest failed directory operation. `sequence` lets a rebuilt page avoid
/// showing the same failure twice.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SftpOperationError {
    pub sequence: u64,
    pub operation: SftpOperation,
    pub path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SftpEntryKindDto {
    File,
    Directory,
    Symlink,
    Other,
}

impl From<SftpEntryKind> for SftpEntryKindDto {
    fn from(kind: SftpEntryKind) -> Self {
        match kind {
            SftpEntryKind::File => Self::File,
            SftpEntryKind::Directory => Self::Directory,
            SftpEntryKind::Symlink => Self::Symlink,
            SftpEntryKind::Other => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SftpEntryDto {
    pub name: String,
    pub path: String,
    pub kind: SftpEntryKindDto,
    pub size: Option<u64>,
    pub modified_at_ms: Option<u64>,
    pub attributes: Option<String>,
    pub owner: Option<String>,
}

impl From<SftpEntry> for SftpEntryDto {
    fn from(entry: SftpEntry) -> Self {
        Self {
            name: entry.filename,
            path: entry.path,
            kind: entry.kind.into(),
            size: entry.size,
            modified_at_ms: entry.modified.and_then(system_time_to_ms),
            attributes: entry.attributes,
            owner: entry.owner,
        }
    }
}

fn validate_transfer_relative(what: &str, path: &str) -> Result<(), BackendError> {
    miaominal_sftp::validate_relative_path(path).map_err(|error| BackendError::InvalidArgument {
        detail: format!("{what} must stay inside the authorized tree: {error}"),
    })?;
    if path.is_empty() {
        return Err(BackendError::InvalidArgument {
            detail: format!("{what} must name a document inside the authorized tree"),
        });
    }
    Ok(())
}

fn system_time_to_ms(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

/// Newest directory listing for this session. `request_id` guards against late
/// responses from a superseded refresh.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SftpDirectorySnapshot {
    pub path: String,
    pub entries: Vec<SftpEntryDto>,
    pub error: Option<String>,
    pub loading: bool,
    pub request_id: u64,
    pub fetched_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SftpTransferDirection {
    Upload,
    Download,
}

impl From<TransferDirection> for SftpTransferDirection {
    fn from(direction: TransferDirection) -> Self {
        match direction {
            TransferDirection::Upload => Self::Upload,
            TransferDirection::Download => Self::Download,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SftpTransferState {
    Queued,
    Running,
    Paused,
    Done,
    Cancelled,
    Failed,
}

/// Stable classification of a transfer failure, so the UI can show the right
/// message without parsing English error strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SftpTransferFailureKind {
    None,
    Permission,
    NoSpace,
    ProviderRefused,
    LocalHandle,
    Network,
    Other,
}

fn classify_transfer_failure(message: &str) -> SftpTransferFailureKind {
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("no space") || lowered.contains("enospc") || lowered.contains("os error 28") {
        return SftpTransferFailureKind::NoSpace;
    }
    if lowered.contains("permission denied") || lowered.contains("os error 13") {
        return SftpTransferFailureKind::Permission;
    }
    if lowered.contains("no longer usable")
        || lowered.contains("file handle was already closed")
        || lowered.contains("bad file descriptor")
    {
        return SftpTransferFailureKind::LocalHandle;
    }
    if lowered.contains("unsupported")
        || lowered.contains("does not support")
        || lowered.contains("provider refused")
    {
        return SftpTransferFailureKind::ProviderRefused;
    }
    if lowered.contains("connection")
        || lowered.contains("channel")
        || lowered.contains("broken pipe")
        || lowered.contains("reset by peer")
        || lowered.contains("eof")
    {
        return SftpTransferFailureKind::Network;
    }
    SftpTransferFailureKind::Other
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SftpTransferChildStateDto {
    Running,
    Done,
    Cancelled,
    Failed,
}

impl From<&SftpTransferChildState> for SftpTransferChildStateDto {
    fn from(state: &SftpTransferChildState) -> Self {
        match state {
            SftpTransferChildState::Running => Self::Running,
            SftpTransferChildState::Done => Self::Done,
            SftpTransferChildState::Cancelled => Self::Cancelled,
            SftpTransferChildState::Failed(_) => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SftpTransferChildDto {
    pub child_id: u64,
    pub relative_path: String,
    pub bytes_complete: u64,
    pub bytes_total: Option<u64>,
    pub state: SftpTransferChildStateDto,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SftpTransferSnapshot {
    pub transfer_id: u64,
    pub connection_id: u64,
    pub direction: SftpTransferDirection,
    pub source_label: String,
    pub destination: String,
    pub state: SftpTransferState,
    pub bytes_complete: u64,
    pub bytes_total: Option<u64>,
    pub error: Option<String>,
    pub failure_kind: SftpTransferFailureKind,
    pub child_count: u32,
    pub children_done: u32,
    pub children_failed: u32,
    pub children_cancelled: u32,
    /// Most recent children, bounded; aggregate counters stay exact.
    pub children: Vec<SftpTransferChildDto>,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SftpSessionSnapshot {
    pub connection_id: u64,
    pub host_id: String,
    pub host_label: String,
    pub phase: SftpSessionPhase,
    pub failure: Option<String>,
    pub hop_label: Option<String>,
    pub request: Option<SftpRequest>,
    pub revision: u64,
    pub started_at_ms: u64,
    pub transfer_count: u32,
    pub active_transfer_count: u32,
    /// Last failed directory operation (create/remove/rename/listing), so a
    /// page that only queues commands sees the result.
    pub operation_error: Option<SftpOperationError>,
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

struct SessionState {
    host_id: String,
    host_label: String,
    phase: SftpSessionPhase,
    failure: Option<String>,
    hop_label: Option<String>,
    request: Option<SftpRequest>,
    operation_error: Option<SftpOperationError>,
    next_operation_error_sequence: u64,
    revision: u64,
    started_at_ms: u64,
}

struct DirectoryState {
    path: String,
    entries: Vec<SftpEntryDto>,
    error: Option<String>,
    loading: bool,
    request_id: u64,
    fetched_at_ms: u64,
}

struct TransferChildRecord {
    child_id: u64,
    relative_path: String,
    bytes_complete: u64,
    bytes_total: Option<u64>,
    state: SftpTransferChildStateDto,
    error: Option<String>,
}

#[derive(Default)]
struct TransferCounters {
    child_count: u32,
    children_done: u32,
    children_failed: u32,
    children_cancelled: u32,
}

struct TransferRecord {
    transfer_id: u64,
    connection_id: u64,
    direction: SftpTransferDirection,
    source_label: String,
    destination: String,
    state: SftpTransferState,
    bytes_complete: u64,
    bytes_total: Option<u64>,
    error: Option<String>,
    counters: TransferCounters,
    children: Vec<TransferChildRecord>,
    started_at_ms: u64,
    finished_at_ms: Option<u64>,
}

impl TransferRecord {
    fn snapshot(&self) -> SftpTransferSnapshot {
        SftpTransferSnapshot {
            transfer_id: self.transfer_id,
            connection_id: self.connection_id,
            direction: self.direction,
            source_label: self.source_label.clone(),
            destination: self.destination.clone(),
            state: self.state,
            bytes_complete: self.bytes_complete,
            bytes_total: self.bytes_total,
            error: self.error.clone(),
            failure_kind: self
                .error
                .as_deref()
                .map(classify_transfer_failure)
                .unwrap_or(SftpTransferFailureKind::None),
            child_count: self.counters.child_count,
            children_done: self.counters.children_done,
            children_failed: self.counters.children_failed,
            children_cancelled: self.counters.children_cancelled,
            children: self
                .children
                .iter()
                .map(|child| SftpTransferChildDto {
                    child_id: child.child_id,
                    relative_path: child.relative_path.clone(),
                    bytes_complete: child.bytes_complete,
                    bytes_total: child.bytes_total,
                    state: child.state,
                    error: child.error.clone(),
                })
                .collect(),
            started_at_ms: self.started_at_ms,
            finished_at_ms: self.finished_at_ms,
        }
    }

    fn track_child(&mut self, child_id: u64, relative_path: String, bytes_total: Option<u64>) {
        if !self
            .children
            .iter()
            .any(|child| child.child_id == child_id)
        {
            self.counters.child_count = self.counters.child_count.saturating_add(1);
        }
        let child = TransferChildRecord {
            child_id,
            relative_path,
            bytes_complete: 0,
            bytes_total,
            state: SftpTransferChildStateDto::Running,
            error: None,
        };
        if let Some(existing) = self
            .children
            .iter_mut()
            .find(|existing| existing.child_id == child_id)
        {
            *existing = child;
        } else {
            self.children.push(child);
            if self.children.len() > MAX_TRACKED_CHILDREN {
                self.children.remove(0);
            }
        }
    }

    fn update_child(
        &mut self,
        child_id: u64,
        bytes_complete: u64,
        state: SftpTransferChildStateDto,
        error: Option<String>,
    ) {
        let previous = self
            .children
            .iter()
            .find(|child| child.child_id == child_id)
            .map(|child| child.state);
        if let Some(child) = self
            .children
            .iter_mut()
            .find(|child| child.child_id == child_id)
        {
            child.bytes_complete = bytes_complete;
            child.state = state;
            child.error = error.clone();
        } else {
            self.counters.child_count = self.counters.child_count.saturating_add(1);
            self.children.push(TransferChildRecord {
                child_id,
                relative_path: String::new(),
                bytes_complete,
                bytes_total: None,
                state,
                error,
            });
            if self.children.len() > MAX_TRACKED_CHILDREN {
                self.children.remove(0);
            }
        }
        if previous != Some(state) {
            match state {
                SftpTransferChildStateDto::Done => {
                    self.counters.children_done = self.counters.children_done.saturating_add(1);
                }
                SftpTransferChildStateDto::Failed => {
                    self.counters.children_failed = self.counters.children_failed.saturating_add(1);
                }
                SftpTransferChildStateDto::Cancelled => {
                    self.counters.children_cancelled =
                        self.counters.children_cancelled.saturating_add(1);
                }
                SftpTransferChildStateDto::Running => {}
            }
        }
    }
}

/// Shared session state; the Kotlin handle and the event task both use it.
pub(crate) struct SftpSessionInner {
    connection_id: u64,
    host: Arc<dyn PlatformHost>,
    /// Weak back-reference: the registry owns the sessions, so a strong link
    /// here would keep every session (and its worker thread) alive after a
    /// backend restart.
    registry: Weak<SftpSessionRegistry>,
    state: Mutex<SessionState>,
    directory: Mutex<DirectoryState>,
    transfers: Mutex<HashMap<u64, TransferRecord>>,
    /// Registered upload handles retained until the transfer is terminal.
    files: Mutex<HashMap<u64, Arc<LocalFileHandle>>>,
    wake: Notify,
    commands: Mutex<Option<miaominal_sftp::SftpCommandSender>>,
    consumer: Mutex<Option<tokio::task::JoinHandle<()>>>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    close_requested: AtomicBool,
}

impl SftpSessionInner {
    fn new(
        connection_id: u64,
        host: Arc<dyn PlatformHost>,
        registry: &Arc<SftpSessionRegistry>,
        host_id: String,
        host_label: String,
    ) -> Arc<Self> {
        Arc::new(Self {
            connection_id,
            host,
            registry: Arc::downgrade(registry),
            state: Mutex::new(SessionState {
                host_id,
                host_label,
                phase: SftpSessionPhase::Connecting,
                failure: None,
                hop_label: None,
                request: None,
                operation_error: None,
                next_operation_error_sequence: 0,
                revision: 1,
                started_at_ms: now_ms(),
            }),
            directory: Mutex::new(DirectoryState {
                path: String::new(),
                entries: Vec::new(),
                error: None,
                loading: false,
                request_id: 0,
                fetched_at_ms: 0,
            }),
            transfers: Mutex::new(HashMap::new()),
            files: Mutex::new(HashMap::new()),
            wake: Notify::new(),
            commands: Mutex::new(None),
            consumer: Mutex::new(None),
            worker: Mutex::new(None),
            close_requested: AtomicBool::new(false),
        })
    }

    fn attach(
        &self,
        commands: miaominal_sftp::SftpCommandSender,
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

    fn bump_revision(&self) {
        let mut state = lock(&self.state);
        state.revision = state.revision.wrapping_add(1);
    }

    fn set_phase(&self, phase: SftpSessionPhase, hop_label: Option<String>, failure: Option<String>) {
        {
            let mut state = lock(&self.state);
            if state.phase.is_terminal() {
                return;
            }
            state.phase = phase;
            if let Some(hop) = hop_label {
                state.hop_label = Some(hop);
            }
            if phase == SftpSessionPhase::Failed {
                state.failure = failure;
            }
            state.revision = state.revision.wrapping_add(1);
        }
        self.wake.notify_waiters();
        self.publish(BackendEvent::SftpPhaseChanged {
            connection_id: self.connection_id,
            phase,
        });
    }

    fn publish_request(&self, request: SftpRequest) {
        {
            let mut state = lock(&self.state);
            state.request = Some(request.clone());
            state.revision = state.revision.wrapping_add(1);
        }
        self.wake.notify_waiters();
        self.publish(BackendEvent::SftpRequest {
            request,
        });
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
        self.publish(BackendEvent::SftpRequestExpired {
            connection_id: self.connection_id,
            request_id,
        });
    }

    fn apply_event(&self, event: SftpEvent) {
        match event {
            SftpEvent::Phase { phase, hop } => {
                let hop_label = hop.as_ref().map(|hop| hop.label.clone());
                if phase == NativeSftpPhase::Closed {
                    self.mark_closed();
                } else {
                    let phase = phase_from_native(phase);
                    if phase == SftpSessionPhase::Failed {
                        self.set_phase(phase, hop_label, Some("the SFTP connection failed".to_string()));
                    } else {
                        self.set_phase(phase, hop_label, None);
                    }
                }
            }
            SftpEvent::HostKeyRequest {
                request_id,
                hop,
                prompt,
            } => {
                let request = host_key_request_dto(self.connection_id, request_id, hop, prompt);
                self.set_phase(
                    SftpSessionPhase::WaitingForTrust,
                    Some(request.hop_label.clone()),
                    None,
                );
                self.publish_request(request);
            }
            SftpEvent::KeyboardInteractiveRequest {
                request_id,
                hop,
                challenge,
            } => {
                let request =
                    keyboard_interactive_request_dto(self.connection_id, request_id, hop, challenge);
                self.set_phase(
                    SftpSessionPhase::WaitingForAuthentication,
                    Some(request.hop_label.clone()),
                    None,
                );
                self.publish_request(request);
            }
            SftpEvent::RequestExpired { request_id } => {
                self.expire_request(request_id);
            }
            SftpEvent::HostKeyPrompt(_) => {}
            SftpEvent::DirectoryListing {
                request_id,
                path,
                entries,
            } => {
                self.apply_listing(request_id, path, entries);
            }
            SftpEvent::DirectoryListingFailed {
                request_id,
                path,
                message,
            } => {
                self.apply_listing_failure(request_id.0, path, message);
            }
            SftpEvent::SubdirectoryListing { .. } => {}
            SftpEvent::TransferQueued {
                transfer_id,
                direction,
                source,
                destination,
            } => {
                {
                    let mut transfers = lock(&self.transfers);
                    transfers.insert(
                        transfer_id.0,
                        TransferRecord {
                            transfer_id: transfer_id.0,
                            connection_id: self.connection_id,
                            direction: direction.into(),
                            source_label: source.display().to_string(),
                            destination,
                            state: SftpTransferState::Queued,
                            bytes_complete: 0,
                            bytes_total: None,
                            error: None,
                            counters: TransferCounters::default(),
                            children: Vec::new(),
                            started_at_ms: now_ms(),
                            finished_at_ms: None,
                        },
                    );
                }
                self.publish_transfer_changed(transfer_id.0);
            }
            SftpEvent::TransferChildStarted { transfer_id, child } => {
                let mut transfers = lock(&self.transfers);
                if let Some(record) = transfers.get_mut(&transfer_id.0) {
                    record.track_child(child.child_id.0, child.relative_path, child.bytes_total);
                }
            }
            SftpEvent::TransferChildFinished { transfer_id, child } => {
                let mut transfers = lock(&self.transfers);
                if let Some(record) = transfers.get_mut(&transfer_id.0) {
                    let error = match &child.state {
                        SftpTransferChildState::Failed(message) => Some(message.clone()),
                        _ => None,
                    };
                    record.update_child(
                        child.child_id.0,
                        child.bytes_complete,
                        (&child.state).into(),
                        error,
                    );
                }
            }
            SftpEvent::TransferProgressFinal(progress) => {
                apply_progress_to(&self.transfers, &progress);
            }
            SftpEvent::TransferPaused { transfer_id } => {
                self.set_transfer_state(transfer_id.0, SftpTransferState::Paused, None);
            }
            SftpEvent::TransferResumed { transfer_id } => {
                self.set_transfer_state(transfer_id.0, SftpTransferState::Running, None);
            }
            SftpEvent::TransferDone { transfer_id } => {
                self.set_transfer_state(transfer_id.0, SftpTransferState::Done, None);
            }
            SftpEvent::TransferCancelled { transfer_id } => {
                self.set_transfer_state(transfer_id.0, SftpTransferState::Cancelled, None);
            }
            SftpEvent::TransferFailed {
                transfer_id,
                message,
            } => {
                self.set_transfer_state(transfer_id.0, SftpTransferState::Failed, Some(message));
            }
            SftpEvent::Error {
                context,
                path,
                message,
            } => {
                if context == "sftp" {
                    self.fail(message);
                } else if let Some(path) = path {
                    self.apply_operation_error(&context, &path, message);
                }
            }
            SftpEvent::OperationFailed {
                operation,
                path,
                message,
            } => {
                {
                    let mut state = lock(&self.state);
                    state.next_operation_error_sequence =
                        state.next_operation_error_sequence.wrapping_add(1);
                    let sequence = state.next_operation_error_sequence;
                    state.operation_error = Some(SftpOperationError {
                        sequence,
                        operation: operation.into(),
                        path,
                        message,
                    });
                    state.revision = state.revision.wrapping_add(1);
                }
                self.wake.notify_waiters();
            }
            SftpEvent::Status(_) => {}
            SftpEvent::Closed => self.mark_closed(),
        }
    }

    fn apply_operation_error(&self, context: &str, path: &str, message: String) {
        if context.starts_with("list_directory") || context.starts_with("list_subdirectory") {
            let mut directory = lock(&self.directory);
            directory.error = Some(message);
            directory.loading = false;
            directory.path = path.to_string();
            directory.fetched_at_ms = now_ms();
        }
    }

    fn apply_listing(&self, request_id: Option<miaominal_sftp::SftpDirectoryRequestId>, path: String, entries: Vec<SftpEntry>) {
        let request_id = request_id.map(|id| id.0).unwrap_or(0);
        let accepted = {
            let mut directory = lock(&self.directory);
            // A late response for a superseded refresh is dropped, including
            // the initial listing when a user request is already outstanding.
            if request_id < directory.request_id {
                false
            } else {
                directory.request_id = request_id;
                directory.path = path;
                directory.entries = entries.into_iter().map(SftpEntryDto::from).collect();
                directory.error = None;
                directory.loading = false;
                directory.fetched_at_ms = now_ms();
                true
            }
        };
        if accepted {
            self.bump_revision();
            self.wake.notify_waiters();
            self.publish(BackendEvent::SftpDirectoryChanged {
                connection_id: self.connection_id,
                request_id,
            });
        }
    }

    fn apply_listing_failure(&self, request_id: u64, path: String, message: String) {
        let accepted = {
            let mut directory = lock(&self.directory);
            if request_id < directory.request_id {
                false
            } else {
                directory.request_id = request_id;
                directory.path = path;
                directory.error = Some(message);
                directory.loading = false;
                directory.fetched_at_ms = now_ms();
                true
            }
        };
        if accepted {
            self.bump_revision();
            self.wake.notify_waiters();
            self.publish(BackendEvent::SftpDirectoryChanged {
                connection_id: self.connection_id,
                request_id,
            });
        }
    }

    fn set_transfer_state(
        &self,
        transfer_id: u64,
        state: SftpTransferState,
        error: Option<String>,
    ) {
        let terminal = matches!(
            state,
            SftpTransferState::Done | SftpTransferState::Cancelled | SftpTransferState::Failed
        );
        let changed = {
            let mut transfers = lock(&self.transfers);
            match transfers.get_mut(&transfer_id) {
                Some(record) => {
                    if record.state == state {
                        false
                    } else {
                        record.state = state;
                        if error.is_some() {
                            record.error = error;
                        }
                        if terminal {
                            record.finished_at_ms = Some(now_ms());
                        }
                        true
                    }
                }
                None => false,
            }
        };
        if terminal {
            // The transfer engine no longer reads the registered document, so
            // its duplicate descriptor is closed here instead of waiting for
            // Kotlin to release the handle.
            if let Some(file) = lock(&self.files).remove(&transfer_id) {
                file.release();
            }
        }
        if changed {
            self.publish_transfer_changed(transfer_id);
        }
    }

    fn publish_transfer_changed(&self, transfer_id: u64) {
        self.publish(BackendEvent::SftpTransferChanged {
            connection_id: self.connection_id,
            transfer_id,
        });
    }

    fn fail(&self, message: String) {
        if lock(&self.state).phase.is_terminal() {
            return;
        }
        self.set_phase(SftpSessionPhase::Failed, None, Some(message));
    }

    fn mark_closed(&self) {
        {
            let mut state = lock(&self.state);
            state.request = None;
            if !state.phase.is_terminal() {
                state.phase = SftpSessionPhase::Closed;
            }
            state.revision = state.revision.wrapping_add(1);
        }
        self.wake.notify_waiters();
        self.publish(BackendEvent::SftpPhaseChanged {
            connection_id: self.connection_id,
            phase: lock(&self.state).phase,
        });
    }

    pub(crate) fn request_close(&self) {
        self.close_requested.store(true, Ordering::SeqCst);
        if !lock(&self.state).phase.is_terminal() {
            self.set_phase(SftpSessionPhase::Closing, None, None);
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

    pub(crate) async fn wait_for_task(&self) {
        let handle = lock(&self.consumer).take();
        if let Some(mut handle) = handle {
            if tokio::time::timeout(CLOSE_GRACE, &mut handle).await.is_err() {
                handle.abort();
            }
        }
        self.wait_for_worker(CLOSE_GRACE);
    }

    /// Join the SFTP worker thread with a bound.
    ///
    /// The worker cancels every transfer (and removes incomplete documents)
    /// before it exits, so a caller that observes the join knows the session
    /// left no task behind. The join runs on a helper thread so a stuck worker
    /// cannot block the caller forever.
    fn wait_for_worker(&self, timeout: Duration) {
        let Some(worker) = lock(&self.worker).take() else {
            return;
        };
        let (done_sender, done_receiver) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("sftp-join".to_string())
            .spawn(move || {
                let _ = worker.join();
                let _ = done_sender.send(());
            })
            .ok();
        if done_receiver.recv_timeout(timeout).is_err() {
            log::warn!("timed out waiting for an SFTP worker thread to finish");
        }
    }

    fn finish_task(&self) {
        if !lock(&self.state).phase.is_terminal() {
            self.mark_closed();
        }
        self.wake.notify_waiters();
    }

    fn snapshot(&self) -> SftpSessionSnapshot {
        let state = lock(&self.state);
        let (transfer_count, active_transfer_count) = {
            let transfers = lock(&self.transfers);
            let active = transfers
                .values()
                .filter(|record| {
                    matches!(
                        record.state,
                        SftpTransferState::Queued
                            | SftpTransferState::Running
                            | SftpTransferState::Paused
                    )
                })
                .count();
            (transfers.len() as u32, active as u32)
        };
        SftpSessionSnapshot {
            connection_id: self.connection_id,
            host_id: state.host_id.clone(),
            host_label: state.host_label.clone(),
            phase: state.phase,
            failure: state.failure.clone(),
            hop_label: state.hop_label.clone(),
            request: state.request.clone(),
            revision: state.revision,
            started_at_ms: state.started_at_ms,
            transfer_count,
            active_transfer_count,
            operation_error: state.operation_error.clone(),
        }
    }

    fn directory_snapshot(&self) -> SftpDirectorySnapshot {
        let directory = lock(&self.directory);
        SftpDirectorySnapshot {
            path: directory.path.clone(),
            entries: directory.entries.clone(),
            error: directory.error.clone(),
            loading: directory.loading,
            request_id: directory.request_id,
            fetched_at_ms: directory.fetched_at_ms,
        }
    }

    fn transfers_snapshot(&self) -> Vec<SftpTransferSnapshot> {
        let transfers = lock(&self.transfers);
        let mut snapshots: Vec<_> = transfers
            .values()
            .map(|record| record.snapshot())
            .collect();
        snapshots.sort_by(|left, right| right.transfer_id.cmp(&left.transfer_id));
        snapshots
    }

    fn transfer_snapshot(&self, transfer_id: u64) -> Option<SftpTransferSnapshot> {
        lock(&self.transfers)
            .get(&transfer_id)
            .map(TransferRecord::snapshot)
    }

    fn commands(&self) -> Result<miaominal_sftp::SftpCommandSender, BackendError> {
        lock(&self.commands)
            .clone()
            .ok_or(BackendError::SessionNotReady {
                session_detail: "the SFTP session has no command channel".to_string(),
            })
    }

    fn ensure_ready(&self) -> Result<(), BackendError> {
        let phase = lock(&self.state).phase;
        if phase != SftpSessionPhase::Ready {
            return Err(BackendError::SessionNotReady {
                session_detail: format!("the SFTP session is {phase:?}"),
            });
        }
        Ok(())
    }

    fn answer_host_key(
        &self,
        request_id: u64,
        decision: SshHostKeyDecision,
    ) -> Result<(), BackendError> {
        let is_current = lock(&self.state).request.as_ref().is_some_and(|request| {
            request.request_id == request_id && request.kind == SftpRequestKind::HostKey
        });
        if !is_current {
            return Err(BackendError::RequestExpired { request_id });
        }
        let commands = self.commands()?;
        commands
            .respond_host_key_request(
                request_id,
                match decision {
                    SshHostKeyDecision::AcceptOnce => miaominal_core::forwarding::HostKeyDecision::AcceptOnce,
                    SshHostKeyDecision::AcceptAndSave => {
                        miaominal_core::forwarding::HostKeyDecision::AcceptAndSave
                    }
                    SshHostKeyDecision::Reject => miaominal_core::forwarding::HostKeyDecision::Reject,
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

    fn answer_keyboard_interactive(
        &self,
        request_id: u64,
        answers: Vec<String>,
    ) -> Result<(), BackendError> {
        let is_current = lock(&self.state).request.as_ref().is_some_and(|request| {
            request.request_id == request_id && request.kind == SftpRequestKind::KeyboardInteractive
        });
        if !is_current {
            return Err(BackendError::RequestExpired { request_id });
        }
        let commands = self.commands()?;
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

    fn list_directory(&self, path: String) -> Result<u64, BackendError> {
        self.ensure_ready()?;
        let commands = self.commands()?;
        let request_id = commands
            .list_directory(path.clone())
            .map_err(|error| BackendError::Storage {
                storage_failure: format!("failed to request a directory listing: {error}"),
            })?;
        {
            let mut directory = lock(&self.directory);
            directory.request_id = request_id.0;
            directory.loading = true;
            directory.error = None;
        }
        self.bump_revision();
        self.wake.notify_waiters();
        Ok(request_id.0)
    }
}

fn apply_progress_to(transfers: &Mutex<HashMap<u64, TransferRecord>>, progress: &SftpTransferProgress) {
    let mut transfers = lock(transfers);
    let Some(record) = transfers.get_mut(&progress.transfer_id.0) else {
        return;
    };
    record.bytes_complete = progress.bytes_complete;
    if progress.bytes_total.is_some() {
        record.bytes_total = progress.bytes_total;
    }
    if record.state == SftpTransferState::Queued {
        record.state = SftpTransferState::Running;
    }
    if let Some(child) = &progress.child {
        let state = (&child.state).into();
        let error = match &child.state {
            SftpTransferChildState::Failed(message) => Some(message.clone()),
            _ => None,
        };
        record.update_child(child.child_id.0, child.bytes_complete, state, error);
    }
}

fn host_key_request_dto(
    connection_id: u64,
    request_id: u64,
    hop: miaominal_ssh::SessionHop,
    prompt: HostKeyPrompt,
) -> SftpRequest {
    SftpRequest {
        request_id,
        connection_id,
        kind: SftpRequestKind::HostKey,
        hop_label: hop.label,
        hop_index: hop.index,
        hop_total: hop.total,
        host: prompt.host,
        port: prompt.port,
        algorithm: prompt.algorithm,
        fingerprint: prompt.fingerprint,
        key_changed: prompt.previous_fingerprint.is_some(),
        previous_fingerprint: prompt.previous_fingerprint,
        name: String::new(),
        instructions: String::new(),
        prompts: Vec::new(),
    }
}

fn keyboard_interactive_request_dto(
    connection_id: u64,
    request_id: u64,
    hop: miaominal_ssh::SessionHop,
    challenge: miaominal_core::forwarding::KbiChallenge,
) -> SftpRequest {
    SftpRequest {
        request_id,
        connection_id,
        kind: SftpRequestKind::KeyboardInteractive,
        hop_label: hop.label,
        hop_index: hop.index,
        hop_total: hop.total,
        host: String::new(),
        port: 0,
        algorithm: String::new(),
        fingerprint: String::new(),
        previous_fingerprint: None,
        key_changed: false,
        name: challenge.name,
        instructions: challenge.instructions,
        prompts: challenge
            .prompts
            .into_iter()
            .map(|prompt| SshPromptDto {
                prompt: prompt.prompt,
                echo: prompt.echo,
            })
            .collect(),
    }
}

/// Registry of live SFTP sessions owned by one `Backend` handle.
pub(crate) struct SftpSessionRegistry {
    lifecycle: Arc<Mutex<crate::backend::LifecycleState>>,
    events: Arc<EventBus>,
    sessions: Mutex<HashMap<u64, Arc<SftpSessionInner>>>,
    next_connection_id: AtomicU64,
}

impl SftpSessionRegistry {
    pub(crate) fn new(
        lifecycle: Arc<Mutex<crate::backend::LifecycleState>>,
        events: Arc<EventBus>,
    ) -> Arc<Self> {
        Arc::new(Self {
            lifecycle,
            events,
            sessions: Mutex::new(HashMap::new()),
            next_connection_id: AtomicU64::new(1),
        })
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

    fn insert(&self, session: Arc<SftpSessionInner>) {
        lock(&self.sessions).insert(session.connection_id, session);
    }

    fn start(
        &self,
        connect: impl FnOnce() -> Arc<SftpSessionInner>,
    ) -> Result<Arc<SftpSessionInner>, BackendError> {
        // Worker creation and registration commit atomically with shutdown.
        let state = lock(&self.lifecycle);
        if state.closed {
            return Err(BackendError::Closed);
        }
        let inner = connect();
        self.insert(Arc::clone(&inner));
        Ok(inner)
    }

    pub(crate) fn get(&self, connection_id: u64) -> Option<Arc<SftpSessionInner>> {
        lock(&self.sessions).get(&connection_id).cloned()
    }

    pub(crate) fn remove(&self, connection_id: u64) -> Option<Arc<SftpSessionInner>> {
        lock(&self.sessions).remove(&connection_id)
    }

    pub(crate) fn snapshots(&self) -> Vec<SftpSessionSnapshot> {
        let sessions = lock(&self.sessions)
            .values()
            .cloned()
            .collect::<Vec<_>>();
        sessions.iter().map(|session| session.snapshot()).collect()
    }

    pub(crate) fn request_close_all(&self) {
        let sessions = lock(&self.sessions)
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for session in sessions {
            session.request_close();
        }
    }

    /// Wait (bounded) for every session worker to finish cancelling transfers.
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
            session.abort_consumer();
        }
    }
}

/// Event task: applies typed SFTP events and merged progress updates.
async fn run_session_task(
    inner: Arc<SftpSessionInner>,
    mut events: SftpEventReceiver,
    mut progress: SftpProgressReceiver,
    mut close_rx: watch::Receiver<bool>,
) {
    let mut closing = false;
    // The progress channel can close while the event channel still carries
    // terminal events (for example a failed connection), so it only disables
    // its select branch instead of ending the task.
    let mut progress_open = true;
    loop {
        tokio::select! {
            event = events.next() => {
                match event {
                    Some(event) => {
                        let task_inner = Arc::clone(&inner);
                        let _ = crate::runtime::offload(move || task_inner.apply_event(event)).await;
                    }
                    None => break,
                }
            }
            update = progress.recv(), if progress_open => {
                match update {
                    Some(progress) => {
                        apply_progress_to(&inner.transfers, &progress);
                    }
                    None => progress_open = false,
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

/// Start one interactive SFTP session through the shared connector.
pub(crate) async fn start_sftp_session(
    host: Arc<dyn PlatformHost>,
    data: Arc<crate::data::MobileData>,
    registry: Arc<SftpSessionRegistry>,
    close_rx: watch::Receiver<bool>,
    host_id: String,
) -> Result<Arc<SftpSessionInner>, BackendError> {
    let context = {
        let host_id = host_id.clone();
        crate::runtime::offload(move || data.session_context(&host_id)).await?
    };
    if context.vault_locked {
        return Err(BackendError::VaultLocked);
    }

    registry.start(|| {
        let connection_id = registry.next_connection_id();
        let connection = miaominal_sftp::start_interactive_session(
            &crate::runtime::handle(),
            context.profile,
            context.all_profiles,
            context.all_proxies,
            context.secrets,
            context.known_hosts,
            ANDROID_TRANSFER_CONCURRENCY,
        );
        let miaominal_sftp::SftpConnection {
            commands,
            events,
            progress,
            thread,
        } = connection;

        let inner = SftpSessionInner::new(
            connection_id,
            host,
            &registry,
            host_id,
            context.host_label,
        );
        let task = crate::runtime::runtime().spawn(run_session_task(
            Arc::clone(&inner),
            events,
            progress,
            close_rx,
        ));
        inner.attach(commands, task, thread);
        inner
    })
}

/// Kotlin handle for one interactive SFTP session.
#[derive(uniffi::Object)]
pub struct SftpSession {
    inner: Arc<SftpSessionInner>,
}

impl SftpSession {
    pub(crate) fn new(inner: Arc<SftpSessionInner>) -> Self {
        Self { inner }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl SftpSession {
    pub fn connection_id(&self) -> u64 {
        self.inner.connection_id
    }

    /// Consistent view of the phase and the pending authentication request.
    pub fn snapshot(&self) -> SftpSessionSnapshot {
        self.inner.snapshot()
    }

    pub fn directory_snapshot(&self) -> SftpDirectorySnapshot {
        self.inner.directory_snapshot()
    }

    /// Pull-based transfer snapshots (progress is merged, never queued).
    pub fn transfers_snapshot(&self) -> Vec<SftpTransferSnapshot> {
        self.inner.transfers_snapshot()
    }

    /// Request one directory listing. The returned request id is included in
    /// the `SftpDirectoryChanged` event and in `directory_snapshot`.
    pub fn list_directory(&self, path: String) -> Result<u64, BackendError> {
        self.inner.list_directory(path)
    }

    pub fn create_directory(&self, path: String) -> Result<(), BackendError> {
        self.inner.ensure_ready()?;
        self.inner
            .commands()?
            .create_directory(path)
            .map_err(|error| BackendError::Storage {
                storage_failure: format!("failed to create the remote directory: {error}"),
            })
    }

    pub fn remove_file(&self, path: String) -> Result<(), BackendError> {
        self.inner.ensure_ready()?;
        self.inner
            .commands()?
            .remove_file(path)
            .map_err(|error| BackendError::Storage {
                storage_failure: format!("failed to remove the remote file: {error}"),
            })
    }

    pub fn remove_directory(&self, path: String) -> Result<(), BackendError> {
        self.inner.ensure_ready()?;
        self.inner
            .commands()?
            .remove_directory(path)
            .map_err(|error| BackendError::Storage {
                storage_failure: format!("failed to remove the remote directory: {error}"),
            })
    }

    pub fn rename(&self, from: String, to: String) -> Result<(), BackendError> {
        self.inner.ensure_ready()?;
        self.inner
            .commands()?
            .rename(from, to)
            .map_err(|error| BackendError::Storage {
                storage_failure: format!("failed to rename the remote entry: {error}"),
            })
    }

    pub fn answer_host_key(
        &self,
        request_id: u64,
        decision: SshHostKeyDecision,
    ) -> Result<(), BackendError> {
        self.inner.answer_host_key(request_id, decision)
    }

    pub fn answer_keyboard_interactive(
        &self,
        request_id: u64,
        answers: Vec<String>,
    ) -> Result<(), BackendError> {
        self.inner.answer_keyboard_interactive(request_id, answers)
    }

    /// Upload one Kotlin-registered document to `remote_path`. The handle is
    /// closed when the transfer finishes, fails, or is cancelled.
    pub async fn upload_file(
        &self,
        file: Arc<LocalFileHandle>,
        remote_path: String,
    ) -> Result<Arc<SftpTransfer>, BackendError> {
        let source = LocalTransferSource {
            fs: Arc::new(RegisteredFileSource::new(Arc::clone(&file))),
            path: file.name(),
            label: file.name(),
        };
        let commands = self.inner.commands()?;
        self.inner.ensure_ready()?;
        // A terminal event must not clean up before the file is registered.
        let mut files = lock(&self.inner.files);
        let transfer_id = commands.queue_upload_source(source, remote_path).map_err(|error| {
            BackendError::Storage {
                storage_failure: format!("failed to queue the upload: {error}"),
            }
        })?;
        // The session closes the registered duplicate when the transfer
        // reaches a terminal state (done, failed, or cancelled).
        files.insert(transfer_id.0, file);
        Ok(Arc::new(SftpTransfer {
            session: Arc::downgrade(&self.inner),
            transfer_id: transfer_id.0,
        }))
    }

    /// Upload one authorized SAF directory tree under `remote_path`.
    pub async fn upload_directory(
        &self,
        tree_uri: String,
        relative_path: String,
        label: String,
        remote_path: String,
    ) -> Result<Arc<SftpTransfer>, BackendError> {
        validate_transfer_relative("upload source", &relative_path)?;
        let source = LocalTransferSource {
            fs: Arc::new(SafLocalFileSystem::new(
                Arc::clone(&self.inner.host),
                tree_uri,
                String::new(),
            )),
            path: relative_path,
            label,
        };
        self.inner.ensure_ready()?;
        let commands = self.inner.commands()?;
        let transfer_id = commands.queue_upload_source(source, remote_path).map_err(|error| {
            BackendError::Storage {
                storage_failure: format!("failed to queue the directory upload: {error}"),
            }
        })?;
        Ok(Arc::new(SftpTransfer {
            session: Arc::downgrade(&self.inner),
            transfer_id: transfer_id.0,
        }))
    }

    /// Download `remote_path` into one authorized SAF tree. `target_relative`
    /// is the document path inside the tree; existing documents are replaced
    /// through a staging document when the provider supports it.
    pub async fn download(
        &self,
        remote_path: String,
        tree_uri: String,
        target_relative: String,
    ) -> Result<Arc<SftpTransfer>, BackendError> {
        validate_transfer_relative("download destination", &target_relative)?;
        let destination = LocalTransferDestination {
            fs: Arc::new(SafLocalFileSystem::new(
                Arc::clone(&self.inner.host),
                tree_uri,
                String::new(),
            )),
            path: target_relative.clone(),
            label: target_relative,
        };
        self.inner.ensure_ready()?;
        let commands = self.inner.commands()?;
        let transfer_id = commands
            .queue_download_to(remote_path, destination)
            .map_err(|error| BackendError::Storage {
                storage_failure: format!("failed to queue the download: {error}"),
            })?;
        Ok(Arc::new(SftpTransfer {
            session: Arc::downgrade(&self.inner),
            transfer_id: transfer_id.0,
        }))
    }

    /// Idempotent close: cancels the connection and every transfer, then waits
    /// for the worker thread. Named disconnect because the generated Kotlin
    /// class already implements AutoCloseable.close.
    pub async fn disconnect(&self) -> Result<(), BackendError> {
        self.inner.request_close();
        let inner = Arc::clone(&self.inner);
        crate::runtime::run_task(async move {
            inner.wait_for_task().await;
            Ok::<(), BackendError>(())
        })
        .await??;
        Ok(())
    }
}

/// Kotlin handle for one queued or running transfer.
#[derive(uniffi::Object)]
pub struct SftpTransfer {
    session: Weak<SftpSessionInner>,
    transfer_id: u64,
}

impl std::fmt::Debug for SftpTransfer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SftpTransfer")
            .field("transfer_id", &self.transfer_id)
            .field("session_alive", &self.session.strong_count())
            .finish()
    }
}

#[uniffi::export]
impl SftpTransfer {
    pub fn transfer_id(&self) -> u64 {
        self.transfer_id
    }

    pub fn snapshot(&self) -> Option<SftpTransferSnapshot> {
        self.session
            .upgrade()
            .and_then(|session| session.transfer_snapshot(self.transfer_id))
    }

    pub fn pause(&self) -> Result<(), BackendError> {
        self.with_commands(|commands| commands.pause_transfer(TransferId(self.transfer_id)))
    }

    pub fn resume(&self) -> Result<(), BackendError> {
        self.with_commands(|commands| commands.resume_transfer(TransferId(self.transfer_id)))
    }

    pub fn cancel(&self) -> Result<(), BackendError> {
        self.with_commands(|commands| commands.cancel_transfer(TransferId(self.transfer_id)))
    }
}

impl SftpTransfer {
    fn with_commands(
        &self,
        action: impl FnOnce(&miaominal_sftp::SftpCommandSender) -> anyhow::Result<()>,
    ) -> Result<(), BackendError> {
        let session = self.session.upgrade().ok_or(BackendError::Closed)?;
        let commands = session.commands()?;
        action(&commands).map_err(|error| BackendError::Storage {
            storage_failure: format!("failed to control the transfer: {error}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_host;

    fn test_inner() -> (Arc<SftpSessionRegistry>, Arc<SftpSessionInner>) {
        let lifecycle = Arc::new(Mutex::new(crate::backend::LifecycleState::default()));
        let events = EventBus::new();
        let registry = SftpSessionRegistry::new(lifecycle, events);
        let inner = SftpSessionInner::new(
            1,
            test_host(),
            &registry,
            "host-1".to_string(),
            "Host".to_string(),
        );
        (registry, inner)
    }

    fn entry(name: &str) -> SftpEntry {
        SftpEntry {
            filename: name.to_string(),
            path: format!("/remote/{name}"),
            kind: SftpEntryKind::File,
            size: Some(3),
            modified: None,
            attributes: None,
            owner: None,
        }
    }

    #[test]
    fn shutdown_rejects_late_session_creation() {
        let (registry, _) = test_inner();
        lock(&registry.lifecycle).closed = true;
        assert!(matches!(
            registry.start(|| panic!("must not create a worker after shutdown")),
            Err(BackendError::Closed)
        ));
        assert!(registry.snapshots().is_empty());
    }

    #[test]
    fn late_directory_responses_are_dropped() {
        let (_registry, inner) = test_inner();

        inner.apply_listing(
            Some(miaominal_sftp::SftpDirectoryRequestId(2)),
            "/remote/two".to_string(),
            vec![entry("two.txt")],
        );
        inner.apply_listing(
            Some(miaominal_sftp::SftpDirectoryRequestId(1)),
            "/remote/one".to_string(),
            vec![entry("one.txt")],
        );

        let snapshot = inner.directory_snapshot();
        assert_eq!(snapshot.path, "/remote/two");
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].name, "two.txt");
    }

    #[test]
    fn expired_answers_are_rejected() {
        let (_registry, inner) = test_inner();
        inner.publish_request(SftpRequest {
            request_id: 7,
            connection_id: 1,
            kind: SftpRequestKind::KeyboardInteractive,
            hop_label: "Host".to_string(),
            hop_index: 1,
            hop_total: 1,
            host: String::new(),
            port: 0,
            algorithm: String::new(),
            fingerprint: String::new(),
            previous_fingerprint: None,
            key_changed: false,
            name: String::new(),
            instructions: String::new(),
            prompts: Vec::new(),
        });

        let error = inner
            .answer_keyboard_interactive(9, vec!["secret".to_string()])
            .expect_err("stale request ids must be rejected");
        assert!(matches!(error, BackendError::RequestExpired { request_id: 9 }));
        assert!(inner.snapshot().request.is_some(), "the pending request survives");
    }

    #[test]
    fn transfer_state_machine_tracks_terminal_states_and_children() {
        let (_registry, inner) = test_inner();
        inner.apply_event(SftpEvent::TransferQueued {
            transfer_id: TransferId(4),
            direction: TransferDirection::Upload,
            source: std::path::PathBuf::from("report.txt"),
            destination: "/remote/report.txt".to_string(),
        });
        assert_eq!(
            inner.transfer_snapshot(4).expect("queued transfer").state,
            SftpTransferState::Queued
        );

        inner.apply_event(SftpEvent::TransferChildStarted {
            transfer_id: TransferId(4),
            child: miaominal_sftp::SftpTransferChild {
                child_id: miaominal_sftp::TransferChildId(1),
                relative_path: "nested/report.txt".to_string(),
                bytes_total: Some(10),
            },
        });
        inner.apply_event(SftpEvent::TransferChildFinished {
            transfer_id: TransferId(4),
            child: miaominal_sftp::SftpTransferChildUpdate {
                child_id: miaominal_sftp::TransferChildId(1),
                bytes_complete: 10,
                state: SftpTransferChildState::Done,
            },
        });
        apply_progress_to(
            &inner.transfers,
            &SftpTransferProgress {
                transfer_id: TransferId(4),
                bytes_complete: 10,
                bytes_total: Some(10),
                child: None,
            },
        );
        inner.apply_event(SftpEvent::TransferPaused {
            transfer_id: TransferId(4),
        });
        assert_eq!(
            inner.transfer_snapshot(4).expect("paused transfer").state,
            SftpTransferState::Paused
        );
        inner.apply_event(SftpEvent::TransferResumed {
            transfer_id: TransferId(4),
        });
        inner.apply_event(SftpEvent::TransferDone {
            transfer_id: TransferId(4),
        });

        let snapshot = inner.transfer_snapshot(4).expect("done transfer");
        assert_eq!(snapshot.state, SftpTransferState::Done);
        assert_eq!(snapshot.bytes_complete, 10);
        assert_eq!(snapshot.children_done, 1);
        assert_eq!(snapshot.child_count, 1);
        assert!(snapshot.finished_at_ms.is_some());
    }

    #[test]
    fn closing_marks_the_session_terminal() {
        let (_registry, inner) = test_inner();
        inner.request_close();

        let snapshot = inner.snapshot();
        assert_eq!(snapshot.phase, SftpSessionPhase::Closing);
        inner.mark_closed();
        assert_eq!(inner.snapshot().phase, SftpSessionPhase::Closed);
    }
}
