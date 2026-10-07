use super::paths::{
    canonical_remote_path, emit_directory_listing, emit_subdirectory_listing,
    list_directory_entries, remove_directory_recursive,
};
use super::transfer::{
    TransferControl, cancel_all_transfers, emit_error, emit_error_with_path, emit_transfer_paused,
    emit_transfer_queued, emit_transfer_resumed, remove_completed_transfer, spawn_download_task,
    spawn_upload_task,
};
use anyhow::{Context, Result, anyhow, bail};
use futures::SinkExt as _;
use futures::channel::mpsc::{
    Receiver as FuturesReceiver, Sender as FuturesSender, channel as futures_channel,
};
use miaominal_core::forwarding::{HostKeyDecision, HostKeyPrompt};
use miaominal_core::known_host::HostKeyCheck;
use miaominal_core::profile::SessionProfile;
use miaominal_core::proxy::ProxyProfile;
use miaominal_core::sftp::{SftpEntry, TransferDirection, TransferId};
use miaominal_secrets::SecretStore;
use miaominal_ssh as ssh;
use miaominal_storage::KnownHostsStore;
use russh::keys::{HashAlg, PublicKey};
use russh::{Disconnect, client};
use russh_sftp::{
    client::{SftpSession, error::Error as SftpClientError},
    protocol::{FileType, StatusCode},
};
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::{Mutex, Semaphore};

const SFTP_EVENT_CHANNEL_CAPACITY: usize = 256;
/// Desktop default; Android starts with a lower bound (see `transfer_concurrency`).
pub const DEFAULT_TRANSFER_CONCURRENCY: usize = 6;

#[derive(Debug)]
enum SftpCommand {
    RespondHostKey {
        request_id: u64,
        decision: HostKeyDecision,
    },
    RespondKeyboardInteractive {
        request_id: u64,
        answers: Vec<String>,
    },
    ListDirectory {
        request_id: SftpDirectoryRequestId,
        path: String,
    },
    ListSubdirectory {
        path: String,
    },
    CreateDirectory {
        path: String,
    },
    RemoveFile {
        path: String,
    },
    RemoveDirectoryRecursive {
        path: String,
    },
    Rename {
        from: String,
        to: String,
    },
    UploadSource {
        transfer_id: TransferId,
        source: super::transfer::LocalTransferSource,
        remote_path: String,
    },
    DownloadTo {
        transfer_id: TransferId,
        remote_path: String,
        destination: super::transfer::LocalTransferDestination,
    },
    PauseTransfer {
        transfer_id: TransferId,
    },
    ResumeTransfer {
        transfer_id: TransferId,
    },
    CancelTransfer {
        transfer_id: TransferId,
    },
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransferChildId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SftpDirectoryRequestId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpTransferChild {
    pub child_id: TransferChildId,
    pub relative_path: String,
    pub bytes_total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SftpTransferChildState {
    Running,
    Done,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpTransferChildUpdate {
    pub child_id: TransferChildId,
    pub bytes_complete: u64,
    pub state: SftpTransferChildState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpTransferProgress {
    pub transfer_id: TransferId,
    pub bytes_complete: u64,
    pub bytes_total: Option<u64>,
    pub child: Option<SftpTransferChildUpdate>,
}

/// Remote directory operation that can fail outside a transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SftpOperation {
    CreateDirectory,
    RemoveFile,
    RemoveDirectory,
    Rename,
    ListDirectory,
    ListSubdirectory,
}

impl SftpOperation {
    fn context(self) -> &'static str {
        match self {
            Self::CreateDirectory => "create_directory",
            Self::RemoveFile => "remove_file",
            Self::RemoveDirectory => "remove_directory_recursive",
            Self::Rename => "rename",
            Self::ListDirectory => "list_directory",
            Self::ListSubdirectory => "list_subdirectory",
        }
    }
}

/// Typed connection phase for a non-PTY SFTP session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

fn phase_from_session_phase(phase: ssh::SessionPhase) -> SftpSessionPhase {
    match phase {
        ssh::SessionPhase::Connecting => SftpSessionPhase::Connecting,
        ssh::SessionPhase::WaitingForTrust => SftpSessionPhase::WaitingForTrust,
        ssh::SessionPhase::WaitingForAuthentication => SftpSessionPhase::WaitingForAuthentication,
        ssh::SessionPhase::OpeningShell => SftpSessionPhase::OpeningSubsystem,
        ssh::SessionPhase::Ready => SftpSessionPhase::Ready,
        ssh::SessionPhase::Closing => SftpSessionPhase::Closing,
        ssh::SessionPhase::Closed => SftpSessionPhase::Closed,
        ssh::SessionPhase::Failed => SftpSessionPhase::Failed,
    }
}

#[derive(Debug, Clone)]
pub enum SftpEvent {
    Status(String),
    /// Typed phase transition; interactive sessions emit these so hosts never
    /// parse the English status strings.
    Phase {
        phase: SftpSessionPhase,
        hop: Option<ssh::SessionHop>,
    },
    HostKeyPrompt(HostKeyPrompt),
    /// Interactive host-key prompt with the request id and hop it belongs to.
    HostKeyRequest {
        request_id: u64,
        hop: ssh::SessionHop,
        prompt: HostKeyPrompt,
    },
    /// Interactive keyboard-interactive challenge (SFTP may prompt on the
    /// target host and on every jump node).
    KeyboardInteractiveRequest {
        request_id: u64,
        hop: ssh::SessionHop,
        challenge: miaominal_core::forwarding::KbiChallenge,
    },
    /// A queued authentication answer arrived after its request expired.
    RequestExpired {
        request_id: u64,
    },
    /// A directory operation the host initiated failed. Hosts that only queue
    /// commands (like the mobile bridge) need this typed event to report the
    /// failure to the user instead of losing it.
    OperationFailed {
        operation: SftpOperation,
        path: Option<String>,
        message: String,
    },
    DirectoryListing {
        request_id: Option<SftpDirectoryRequestId>,
        path: String,
        entries: Vec<SftpEntry>,
    },
    DirectoryListingFailed {
        request_id: SftpDirectoryRequestId,
        path: String,
        message: String,
    },
    SubdirectoryListing {
        parent_path: String,
        entries: Vec<SftpEntry>,
    },
    TransferQueued {
        transfer_id: TransferId,
        direction: TransferDirection,
        source: PathBuf,
        destination: String,
    },
    TransferChildStarted {
        transfer_id: TransferId,
        child: SftpTransferChild,
    },
    TransferChildFinished {
        transfer_id: TransferId,
        child: SftpTransferChildUpdate,
    },
    TransferProgressFinal(SftpTransferProgress),
    TransferPaused {
        transfer_id: TransferId,
    },
    TransferResumed {
        transfer_id: TransferId,
    },
    TransferDone {
        transfer_id: TransferId,
    },
    TransferCancelled {
        transfer_id: TransferId,
    },
    TransferFailed {
        transfer_id: TransferId,
        message: String,
    },
    Error {
        context: String,
        path: Option<String>,
        message: String,
    },
    Closed,
}

#[derive(Clone)]
pub(crate) struct SftpEventSender(Arc<Mutex<FuturesSender<SftpEvent>>>);
pub type SftpEventReceiver = FuturesReceiver<SftpEvent>;

pub(crate) fn sftp_event_channel() -> (SftpEventSender, SftpEventReceiver) {
    let (sender, receiver) = futures_channel(SFTP_EVENT_CHANNEL_CAPACITY);
    (SftpEventSender(Arc::new(Mutex::new(sender))), receiver)
}

pub(crate) async fn send_event(event_sender: &SftpEventSender, event: SftpEvent) -> Result<()> {
    let mut sender = event_sender.0.lock().await;
    sender
        .send(event)
        .await
        .map_err(|_| anyhow!("SFTP event receiver is closed"))
}

#[derive(Default)]
struct SftpProgressState {
    latest: HashMap<TransferId, SftpTransferProgress>,
    queued: HashSet<TransferId>,
}

#[derive(Clone)]
pub(crate) struct SftpProgressSender {
    state: Arc<std::sync::Mutex<SftpProgressState>>,
    notifications: UnboundedSender<TransferId>,
}

pub struct SftpProgressReceiver {
    state: Arc<std::sync::Mutex<SftpProgressState>>,
    notifications: UnboundedReceiver<TransferId>,
}

pub(crate) fn sftp_progress_channel() -> (SftpProgressSender, SftpProgressReceiver) {
    let state = Arc::new(std::sync::Mutex::new(SftpProgressState::default()));
    let (notifications, receiver) = unbounded_channel();
    (
        SftpProgressSender {
            state: state.clone(),
            notifications,
        },
        SftpProgressReceiver {
            state,
            notifications: receiver,
        },
    )
}

impl SftpProgressSender {
    pub(crate) fn send(&self, progress: SftpTransferProgress) {
        let transfer_id = progress.transfer_id;
        let should_notify = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.latest.insert(transfer_id, progress);
            state.queued.insert(transfer_id)
        };
        if should_notify && self.notifications.send(transfer_id).is_err() {
            self.take_latest(transfer_id);
        }
    }

    pub(crate) fn take_latest(&self, transfer_id: TransferId) -> Option<SftpTransferProgress> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.queued.remove(&transfer_id);
        state.latest.remove(&transfer_id)
    }
}

impl SftpProgressReceiver {
    pub async fn recv(&mut self) -> Option<SftpTransferProgress> {
        loop {
            let transfer_id = self.notifications.recv().await?;
            if let Some(progress) = self.take(transfer_id) {
                return Some(progress);
            }
        }
    }

    pub fn try_recv(&mut self) -> Option<SftpTransferProgress> {
        while let Ok(transfer_id) = self.notifications.try_recv() {
            if let Some(progress) = self.take(transfer_id) {
                return Some(progress);
            }
        }
        None
    }

    fn take(&self, transfer_id: TransferId) -> Option<SftpTransferProgress> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.queued.remove(&transfer_id);
        state.latest.get(&transfer_id).cloned()
    }
}

#[derive(Clone)]
pub(crate) struct SftpHostKeyPrompts {
    event_sender: SftpEventSender,
    decisions: Arc<Mutex<UnboundedReceiver<HostKeyDecision>>>,
}

#[derive(Clone, Debug)]
pub struct SftpCommandSender {
    sender: UnboundedSender<SftpCommand>,
    host_key_decisions: UnboundedSender<HostKeyDecision>,
    next_transfer_id: Arc<AtomicU64>,
    next_directory_request_id: Arc<AtomicU64>,
}

impl SftpCommandSender {
    /// Answer a host-key prompt; the request id must match the pending prompt
    /// (interactive sessions). The legacy prompt-free path uses
    /// [`SftpCommandSender::respond_host_key`].
    pub fn respond_host_key_request(
        &self,
        request_id: u64,
        decision: HostKeyDecision,
    ) -> Result<()> {
        self.send_command(SftpCommand::RespondHostKey {
            request_id,
            decision,
        })
    }

    /// Answer one keyboard-interactive challenge round.
    pub fn respond_keyboard_interactive(
        &self,
        request_id: u64,
        answers: Vec<String>,
    ) -> Result<()> {
        self.send_command(SftpCommand::RespondKeyboardInteractive { request_id, answers })
    }

    pub fn list_directory(&self, path: impl Into<String>) -> Result<SftpDirectoryRequestId> {
        let request_id = SftpDirectoryRequestId(
            self.next_directory_request_id
                .fetch_add(1, Ordering::Relaxed),
        );
        self.send_command(SftpCommand::ListDirectory {
            request_id,
            path: path.into(),
        })?;
        Ok(request_id)
    }

    pub fn list_subdirectory(&self, path: impl Into<String>) -> Result<()> {
        self.send_command(SftpCommand::ListSubdirectory { path: path.into() })
    }

    pub fn create_directory(&self, path: impl Into<String>) -> Result<()> {
        self.send_command(SftpCommand::CreateDirectory { path: path.into() })
    }

    pub fn remove_file(&self, path: impl Into<String>) -> Result<()> {
        self.send_command(SftpCommand::RemoveFile { path: path.into() })
    }

    pub fn remove_directory(&self, path: impl Into<String>) -> Result<()> {
        self.send_command(SftpCommand::RemoveDirectoryRecursive { path: path.into() })
    }

    pub fn rename(&self, from: impl Into<String>, to: impl Into<String>) -> Result<()> {
        self.send_command(SftpCommand::Rename {
            from: from.into(),
            to: to.into(),
        })
    }

    pub fn queue_upload(
        &self,
        local_path: PathBuf,
        remote_path: impl Into<String>,
    ) -> Result<TransferId> {
        let source = super::transfer::source_for_path(&local_path)?;
        self.queue_upload_source(source, remote_path)
    }

    /// Queue an upload from an arbitrary local backend (Android SAF/FD).
    pub fn queue_upload_source(
        &self,
        source: super::transfer::LocalTransferSource,
        remote_path: impl Into<String>,
    ) -> Result<TransferId> {
        let transfer_id = self.next_transfer_id();
        self.send_command(SftpCommand::UploadSource {
            transfer_id,
            source,
            remote_path: remote_path.into(),
        })?;
        Ok(transfer_id)
    }

    pub fn queue_download(
        &self,
        remote_path: impl Into<String>,
        local_path: PathBuf,
    ) -> Result<TransferId> {
        let destination = super::transfer::destination_for_path(&local_path)?;
        self.queue_download_to(remote_path, destination)
    }

    /// Queue a download into an arbitrary local backend (Android SAF/FD).
    pub fn queue_download_to(
        &self,
        remote_path: impl Into<String>,
        destination: super::transfer::LocalTransferDestination,
    ) -> Result<TransferId> {
        let transfer_id = self.next_transfer_id();
        self.send_command(SftpCommand::DownloadTo {
            transfer_id,
            remote_path: remote_path.into(),
            destination,
        })?;
        Ok(transfer_id)
    }

    pub fn cancel_transfer(&self, transfer_id: TransferId) -> Result<()> {
        self.send_command(SftpCommand::CancelTransfer { transfer_id })
    }

    pub fn pause_transfer(&self, transfer_id: TransferId) -> Result<()> {
        self.send_command(SftpCommand::PauseTransfer { transfer_id })
    }

    pub fn resume_transfer(&self, transfer_id: TransferId) -> Result<()> {
        self.send_command(SftpCommand::ResumeTransfer { transfer_id })
    }

    pub fn respond_host_key(&self, decision: HostKeyDecision) -> Result<()> {
        self.host_key_decisions
            .send(decision)
            .map_err(|_| anyhow!("SFTP session is no longer available"))
    }

    pub fn close(&self) -> Result<()> {
        self.send_command(SftpCommand::Close)
    }

    fn next_transfer_id(&self) -> TransferId {
        TransferId(self.next_transfer_id.fetch_add(1, Ordering::Relaxed))
    }

    fn send_command(&self, command: SftpCommand) -> Result<()> {
        self.sender
            .send(command)
            .map_err(|_| anyhow!("SFTP session is no longer available"))
    }
}

pub struct SftpConnection {
    pub commands: SftpCommandSender,
    pub events: SftpEventReceiver,
    pub progress: SftpProgressReceiver,
    /// Worker thread for native sessions; join it after `close()` to prove no
    /// connection task is left behind. `None` for host-provided backends.
    pub thread: Option<std::thread::JoinHandle<()>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRemotePath {
    pub requested_path: String,
    pub canonical_path: String,
    pub kind: miaominal_core::sftp::SftpEntryKind,
    pub is_symlink: bool,
}

pub async fn resolve_profile_paths(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    paths: Vec<String>,
) -> Result<Vec<ResolvedRemotePath>> {
    let (event_sender, _event_receiver) = sftp_event_channel();
    let connected_session = connect_authenticated_session(
        profile,
        all_profiles,
        all_proxies,
        secrets,
        known_hosts,
        None,
        &event_sender,
    )
    .await?;

    let result = async {
        let sftp = open_sftp_session(&connected_session).await?;
        let mut resolved = Vec::with_capacity(paths.len());

        for requested_path in paths {
            let link_metadata = sftp
                .symlink_metadata(requested_path.clone())
                .await
                .with_context(|| format!("failed to inspect remote path {requested_path}"))?;
            let canonical_path = sftp
                .canonicalize(requested_path.clone())
                .await
                .with_context(|| format!("failed to canonicalize remote path {requested_path}"))?;
            let target_metadata = sftp
                .metadata(canonical_path.clone())
                .await
                .with_context(|| format!("failed to inspect canonical path {canonical_path}"))?;

            resolved.push(ResolvedRemotePath {
                requested_path,
                canonical_path,
                kind: sftp_entry_kind(target_metadata.file_type()),
                is_symlink: link_metadata.file_type().is_symlink(),
            });
        }

        Ok(resolved)
    }
    .await;

    connected_session.disconnect().await;
    result
}

pub fn start_session(
    runtime: &TokioHandle,
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
) -> SftpConnection {
    start_session_with_concurrency(
        runtime,
        profile,
        all_profiles,
        all_proxies,
        secrets,
        known_hosts,
        DEFAULT_TRANSFER_CONCURRENCY,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn start_session_with_concurrency(
    runtime: &TokioHandle,
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    transfer_concurrency: usize,
) -> SftpConnection {
    let (event_sender, event_receiver) = sftp_event_channel();
    let (progress_sender, progress_receiver) = sftp_progress_channel();
    let (command_sender, command_receiver) = unbounded_channel();
    let (host_key_decision_sender, host_key_decision_receiver) = unbounded_channel();
    let host_key_prompts = SftpHostKeyPrompts {
        event_sender: event_sender.clone(),
        decisions: Arc::new(Mutex::new(host_key_decision_receiver)),
    };
    let runtime = runtime.clone();
    let next_transfer_id = Arc::new(AtomicU64::new(1));
    let next_directory_request_id = Arc::new(AtomicU64::new(1));
    let transfer_concurrency = transfer_concurrency.max(1);

    let thread = std::thread::Builder::new()
        .name(format!("sftp-session-{}", profile.id))
        .spawn(move || {
            let result = runtime.block_on(run_session(
                profile,
                all_profiles,
                all_proxies,
                secrets,
                known_hosts,
                host_key_prompts,
                command_receiver,
                event_sender.clone(),
                progress_sender,
                transfer_concurrency,
            ));
            runtime.block_on(async {
                if let Err(error) = result {
                    let _ = send_event(
                        &event_sender,
                        SftpEvent::Error {
                            context: "sftp".into(),
                            path: None,
                            message: format!("{error:#}"),
                        },
                    )
                    .await;
                }
                let _ = send_event(&event_sender, SftpEvent::Closed).await;
            });
        })
        .expect("failed to spawn SFTP session thread");

    SftpConnection {
        commands: SftpCommandSender {
            sender: command_sender,
            host_key_decisions: host_key_decision_sender,
            next_transfer_id,
            next_directory_request_id,
        },
        events: event_receiver,
        progress: progress_receiver,
        thread: Some(thread),
    }
}

/// Interactive SFTP session: host-key prompts and keyboard-interactive
/// challenges are surfaced through [`SftpEvent`] with request ids and hop
/// labels, and the whole connection (including jump nodes) is cancellable even
/// while the handshake or authentication is still running.
#[allow(clippy::too_many_arguments)]
pub fn start_interactive_session(
    runtime: &TokioHandle,
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    transfer_concurrency: usize,
) -> SftpConnection {
    let (event_sender, event_receiver) = sftp_event_channel();
    let (progress_sender, progress_receiver) = sftp_progress_channel();
    let (command_sender, command_receiver) = unbounded_channel();
    let runtime = runtime.clone();
    let next_transfer_id = Arc::new(AtomicU64::new(1));
    let next_directory_request_id = Arc::new(AtomicU64::new(1));
    let transfer_concurrency = transfer_concurrency.max(1);

    let thread = std::thread::Builder::new()
        .name(format!("sftp-interactive-{}", profile.id))
        .spawn(move || {
            let result = runtime.block_on(run_interactive_session(
                profile,
                all_profiles,
                all_proxies,
                secrets,
                known_hosts,
                command_receiver,
                event_sender.clone(),
                progress_sender,
                transfer_concurrency,
            ));
            runtime.block_on(async {
                match result {
                    Ok(()) => {}
                    Err(error) => {
                        let _ = send_event(
                            &event_sender,
                            SftpEvent::Phase {
                                phase: SftpSessionPhase::Closing,
                                hop: None,
                            },
                        )
                        .await;
                        let _ = send_event(
                            &event_sender,
                            SftpEvent::Error {
                                context: "sftp".into(),
                                path: None,
                                message: format!("{error:#}"),
                            },
                        )
                        .await;
                        let _ = send_event(
                            &event_sender,
                            SftpEvent::Phase {
                                phase: SftpSessionPhase::Failed,
                                hop: None,
                            },
                        )
                        .await;
                    }
                }
                let _ = send_event(
                    &event_sender,
                    SftpEvent::Phase {
                        phase: SftpSessionPhase::Closed,
                        hop: None,
                    },
                )
                .await;
                let _ = send_event(&event_sender, SftpEvent::Closed).await;
            });
        })
        .expect("failed to spawn interactive SFTP session thread");

    SftpConnection {
        commands: SftpCommandSender {
            // The legacy prompt-free decision channel is unused for
            // interactive sessions; answers arrive as commands.
            host_key_decisions: unbounded_channel().0,
            sender: command_sender,
            next_transfer_id,
            next_directory_request_id,
        },
        events: event_receiver,
        progress: progress_receiver,
        thread: Some(thread),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_session(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    host_key_prompts: SftpHostKeyPrompts,
    command_receiver: UnboundedReceiver<SftpCommand>,
    event_sender: SftpEventSender,
    progress_sender: SftpProgressSender,
    transfer_concurrency: usize,
) -> Result<()> {
    let remote = format!("{}@{}:{}", profile.username, profile.host, profile.port);
    let connected_session = connect_authenticated_session(
        profile.clone(),
        all_profiles,
        all_proxies,
        secrets,
        known_hosts,
        Some(host_key_prompts),
        &event_sender,
    )
    .await?;

    let session_result = run_connected_session(
        Arc::new(open_sftp_session(&connected_session).await?),
        remote,
        command_receiver,
        event_sender,
        progress_sender,
        transfer_concurrency,
    )
    .await;

    connected_session.disconnect().await;

    session_result
}

/// Shared command loop for one connected SFTP session.
#[allow(clippy::too_many_arguments)]
async fn run_connected_session(
    sftp: Arc<SftpSession>,
    remote: String,
    mut command_receiver: UnboundedReceiver<SftpCommand>,
    event_sender: SftpEventSender,
    progress_sender: SftpProgressSender,
    transfer_concurrency: usize,
) -> Result<()> {
    let mut transfer_controls = HashMap::new();
    let mut transfer_tasks = HashMap::new();
    let transfer_semaphore = Arc::new(Semaphore::new(transfer_concurrency));
    let (transfer_completion_sender, mut transfer_completion_receiver) = unbounded_channel();

    let command_result: Result<()> = async {
            emit_status(&event_sender, format!("Connected SFTP session to {remote}")).await?;

            let initial_result: Result<_> = async {
                let initial_path = canonical_remote_path(&sftp, ".")
                    .await
                    .context("failed to resolve initial remote directory")?;
                let entries = list_directory_entries(&sftp, &initial_path).await?;
                Ok((initial_path, entries))
            }
            .await;
            if let Some((initial_path, entries)) =
                recover_operation_result(&event_sender, SftpOperation::ListDirectory, Some("."), initial_result)
                    .await?
            {
                emit_directory_listing(&event_sender, None, initial_path, entries).await?;
            }

            loop {
                let command = tokio::select! {
                    completed = transfer_completion_receiver.recv() => {
                        if let Some(transfer_id) = completed {
                            remove_completed_transfer(
                                transfer_id,
                                &mut transfer_tasks,
                                &mut transfer_controls,
                            );
                        }
                        continue;
                    }
                    command = command_receiver.recv() => command,
                };
                let Some(command) = command else {
                    break;
                };

                match command {
                    SftpCommand::ListDirectory { request_id, path } => {
                        let result: Result<_> = async {
                            let canonical_path =
                                canonical_remote_path(&sftp, &path).await.with_context(|| {
                                    format!("failed to resolve remote directory {path}")
                                })?;
                            let entries = list_directory_entries(&sftp, &canonical_path).await?;
                            Ok((canonical_path, entries))
                        }
                        .await;
                        match result {
                            Ok((canonical_path, entries)) => {
                                emit_directory_listing(
                                    &event_sender,
                                    Some(request_id),
                                    canonical_path,
                                    entries,
                                )
                                .await?;
                            }
                            Err(error) if is_recoverable_operation_error(&error) => {
                                send_event(
                                    &event_sender,
                                    SftpEvent::DirectoryListingFailed {
                                        request_id,
                                        path,
                                        message: format!("{error:#}"),
                                    },
                                )
                                .await?;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    SftpCommand::ListSubdirectory { path } => {
                        let result: Result<_> = async {
                            let canonical_path =
                                canonical_remote_path(&sftp, &path).await.with_context(|| {
                                    format!("failed to resolve remote directory {path}")
                                })?;
                            let entries = list_directory_entries(&sftp, &canonical_path).await?;
                            Ok((canonical_path, entries))
                        }
                        .await;
                        if let Some((canonical_path, entries)) = recover_operation_result(
                            &event_sender,
                            SftpOperation::ListSubdirectory,
                            Some(&path),
                            result,
                        )
                        .await?
                        {
                            emit_subdirectory_listing(&event_sender, canonical_path, entries)
                                .await?;
                        }
                    }
                    SftpCommand::CreateDirectory { path } => {
                        let result = sftp
                            .create_dir(path.clone())
                            .await
                            .with_context(|| format!("failed to create remote directory {path}"));
                        if recover_operation_result(&event_sender, SftpOperation::CreateDirectory, Some(&path), result)
                            .await?
                            .is_some()
                        {
                            emit_status(&event_sender, format!("Created remote directory {path}"))
                                .await?;
                        }
                    }
                    SftpCommand::RemoveFile { path } => {
                        let result = sftp
                            .remove_file(path.clone())
                            .await
                            .with_context(|| format!("failed to remove remote file {path}"));
                        if recover_operation_result(&event_sender, SftpOperation::RemoveFile, Some(&path), result)
                            .await?
                            .is_some()
                        {
                            emit_status(&event_sender, format!("Removed remote file {path}"))
                                .await?;
                        }
                    }
                    SftpCommand::RemoveDirectoryRecursive { path } => {
                        let result = remove_directory_recursive(&sftp, &path).await;
                        if recover_operation_result(
                            &event_sender,
                            SftpOperation::RemoveDirectory,
                            Some(&path),
                            result,
                        )
                        .await?
                        .is_some()
                        {
                            emit_status(&event_sender, format!("Removed remote directory {path}"))
                                .await?;
                        }
                    }
                    SftpCommand::Rename { from, to } => {
                        let result = sftp
                            .rename(from.clone(), to.clone())
                            .await
                            .with_context(|| format!("failed to rename {from} to {to}"));
                        if recover_operation_result(&event_sender, SftpOperation::Rename, Some(&to), result)
                            .await?
                            .is_some()
                        {
                            emit_status(&event_sender, format!("Renamed {from} to {to}")).await?;
                        }
                    }
                    SftpCommand::RespondHostKey { .. }
                    | SftpCommand::RespondKeyboardInteractive { .. } => {
                        // Authentication answers only apply while connecting;
                        // the interactive start path consumes them there.
                        emit_error(
                            &event_sender,
                            "sftp_authentication",
                            "no authentication request is pending".to_string(),
                        )
                        .await?;
                    }
                    SftpCommand::UploadSource {
                        transfer_id,
                        source,
                        remote_path,
                    } => {
                        let label = PathBuf::from(&source.label);
                        emit_transfer_queued(
                            &event_sender,
                            transfer_id,
                            TransferDirection::Upload,
                            label,
                            remote_path.clone(),
                        )
                        .await?;

                        let control = Arc::new(TransferControl::new());
                        let task = spawn_upload_task(
                            sftp.clone(),
                            event_sender.clone(),
                            transfer_id,
                            source,
                            remote_path,
                            control.clone(),
                            transfer_semaphore.clone(),
                            transfer_completion_sender.clone(),
                            progress_sender.clone(),
                        );
                        transfer_controls.insert(transfer_id, control);
                        transfer_tasks.insert(transfer_id, task);
                    }
                    SftpCommand::DownloadTo {
                        transfer_id,
                        remote_path,
                        destination,
                    } => {
                        let label = PathBuf::from(&destination.label);
                        emit_transfer_queued(
                            &event_sender,
                            transfer_id,
                            TransferDirection::Download,
                            label,
                            remote_path.clone(),
                        )
                        .await?;

                        let control = Arc::new(TransferControl::new());
                        let task = spawn_download_task(
                            sftp.clone(),
                            event_sender.clone(),
                            transfer_id,
                            remote_path,
                            destination,
                            control.clone(),
                            transfer_semaphore.clone(),
                            transfer_completion_sender.clone(),
                            progress_sender.clone(),
                        );
                        transfer_controls.insert(transfer_id, control);
                        transfer_tasks.insert(transfer_id, task);
                    }
                    SftpCommand::PauseTransfer { transfer_id } => {
                        if let Some(control) = transfer_controls.get(&transfer_id) {
                            if control.pause() {
                                emit_transfer_paused(&event_sender, transfer_id).await?;
                            }
                        } else {
                            emit_error(
                                &event_sender,
                                "pause_transfer",
                                format!("transfer {} is no longer active", transfer_id.0),
                            )
                            .await?;
                        }
                    }
                    SftpCommand::ResumeTransfer { transfer_id } => {
                        if let Some(control) = transfer_controls.get(&transfer_id) {
                            if control.resume() {
                                emit_transfer_resumed(&event_sender, transfer_id).await?;
                            }
                        } else {
                            emit_error(
                                &event_sender,
                                "resume_transfer",
                                format!("transfer {} is no longer active", transfer_id.0),
                            )
                            .await?;
                        }
                    }
                    SftpCommand::CancelTransfer { transfer_id } => {
                        if let Some(control) = transfer_controls.get(&transfer_id) {
                            control.cancel();
                        } else {
                            emit_error(
                                &event_sender,
                                "cancel_transfer",
                                format!("transfer {} is no longer active", transfer_id.0),
                            )
                            .await?;
                        }
                    }
                    SftpCommand::Close => break,
                }
            }

            Ok(())
    }
    .await;

    cancel_all_transfers(&mut transfer_tasks, &mut transfer_controls).await;
    if let Err(error) = sftp.close().await {
        log::debug!("failed to close SFTP session cleanly: {error:?}");
    }

    command_result
}

/// Connect one SFTP session through the shared interactive SSH connector, so
/// host trust and keyboard-interactive requests carry request ids and hop
/// labels for the target and every jump node, and a cancel interrupts the
/// connection even while it is still handshaking.
#[allow(clippy::too_many_arguments)]
async fn run_interactive_session(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    command_receiver: UnboundedReceiver<SftpCommand>,
    event_sender: SftpEventSender,
    progress_sender: SftpProgressSender,
    transfer_concurrency: usize,
) -> Result<()> {
    let remote = format!("{}@{}:{}", profile.username, profile.host, profile.port);
    let ssh::SessionChannels {
        connection: mut ssh_connection,
        commands: mut ssh_commands,
        events: ssh_event_sender,
    } = ssh::session_channels();
    let ssh_command_sender = ssh_connection.commands.clone();
    let mut ssh_events = ssh_connection.take_events();
    let forward_sender = event_sender.clone();
    let forward = tokio::spawn(async move {
        while let Some(event) = ssh_events.recv().await {
            let mapped = match event {
                ssh::SessionEvent::Phase(change) => Some(SftpEvent::Phase {
                    phase: phase_from_session_phase(change.phase),
                    hop: change.hop,
                }),
                ssh::SessionEvent::HostKeyPrompt(request) => Some(SftpEvent::HostKeyRequest {
                    request_id: request.request_id,
                    hop: request.hop,
                    prompt: request.prompt,
                }),
                ssh::SessionEvent::KeyboardInteractivePrompt(request) => {
                    Some(SftpEvent::KeyboardInteractiveRequest {
                        request_id: request.request_id,
                        hop: request.hop,
                        challenge: request.challenge,
                    })
                }
                ssh::SessionEvent::RequestExpired { request_id } => {
                    Some(SftpEvent::RequestExpired { request_id })
                }
                ssh::SessionEvent::Status(message) => Some(SftpEvent::Status(message)),
                _ => None,
            };
            if let Some(event) = mapped
                && send_event(&forward_sender, event).await.is_err()
            {
                break;
            }
        }
    });

    let mut command_receiver = command_receiver;
    // `close()` cancels this signal and queues a Close command. Racing the
    // signal against the connect and subsystem setup means a cancel also
    // interrupts stages that never read the command channel (password
    // authentication against a silent server, a hanging subsystem request).
    let cancellation = ssh_command_sender.cancellation();
    let connect = ssh::connect_interactive_route(
        profile,
        all_profiles,
        all_proxies,
        secrets,
        known_hosts,
        &mut ssh_commands,
        &ssh_event_sender,
    );
    // The connect future borrows the command receiver; `tokio::pin!` lets the
    // select loop poll it while still reading answers from the SFTP commands.
    tokio::pin!(connect);

    let route = 'connect: loop {
        tokio::select! {
            result = &mut connect => {
                match result {
                    Ok(connected) => break 'connect connected,
                    Err(error) => {
                        forward.abort();
                        return Err(error);
                    }
                }
            }
            command = command_receiver.recv() => match command {
                Some(SftpCommand::RespondHostKey { request_id, decision }) => {
                    let _ = ssh_command_sender.respond_host_key(request_id, decision);
                }
                Some(SftpCommand::RespondKeyboardInteractive { request_id, answers }) => {
                    let _ = ssh_command_sender.respond_keyboard_interactive(request_id, answers);
                }
                Some(SftpCommand::Close) | None => {
                    // Cancel and drop the connect future immediately: waiting
                    // for it could block forever on a server that never
                    // answers authentication.
                    let _ = ssh_command_sender.close();
                    forward.abort();
                    return Ok(());
                }
                Some(_) => {
                    emit_error(
                        &event_sender,
                        "sftp",
                        "the SFTP session is not ready yet".to_string(),
                    )
                    .await?;
                }
            },
            () = cancellation.cancelled() => {
                forward.abort();
                return Ok(());
            }
        }
    };

    let result = async {
        emit_phase(
            &event_sender,
            SftpSessionPhase::OpeningSubsystem,
            Some(route.target_hop()),
        )
        .await?;
        let setup = async {
            let channel = route.open_session().await?;
            channel
                .request_subsystem(true, "sftp")
                .await
                .context("failed to start SFTP subsystem")?;
            let sftp = SftpSession::new(channel.into_stream())
                .await
                .context("failed to initialize SFTP session")?;
            Ok::<_, anyhow::Error>(sftp)
        };
        tokio::pin!(setup);
        let sftp = loop {
            tokio::select! {
                result = &mut setup => break result?,
                command = command_receiver.recv() => match command {
                    Some(SftpCommand::Close) | None => {
                        let _ = ssh_command_sender.close();
                        return Ok(());
                    }
                    Some(_) => {
                        emit_error(
                            &event_sender,
                            "sftp",
                            "the SFTP session is not ready yet".to_string(),
                        )
                        .await?;
                    }
                },
                () = cancellation.cancelled() => return Ok(()),
            }
        };
        emit_phase(
            &event_sender,
            SftpSessionPhase::Ready,
            Some(route.target_hop()),
        )
        .await?;

        run_connected_session(
            Arc::new(sftp),
            remote,
            command_receiver,
            event_sender.clone(),
            progress_sender,
            transfer_concurrency,
        )
        .await
    }
    .await;

    drop(ssh_connection);
    drop(ssh_command_sender);
    route.disconnect().await;
    forward.abort();

    result
}

async fn emit_phase(
    event_sender: &SftpEventSender,
    phase: SftpSessionPhase,
    hop: Option<ssh::SessionHop>,
) -> Result<()> {
    send_event(event_sender, SftpEvent::Phase { phase, hop }).await
}

async fn recover_operation_result<T>(
    event_sender: &SftpEventSender,
    operation: SftpOperation,
    path: Option<&str>,
    result: Result<T>,
) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if is_recoverable_operation_error(&error) => {
            let message = format!("{error:#}");
            emit_error_with_path(
                event_sender,
                operation.context(),
                path.map(str::to_owned),
                message.clone(),
            )
            .await?;
            send_event(
                event_sender,
                SftpEvent::OperationFailed {
                    operation,
                    path: path.map(str::to_owned),
                    message,
                },
            )
            .await?;
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn is_recoverable_operation_error(error: &anyhow::Error) -> bool {
    match error.downcast_ref::<SftpClientError>() {
        Some(SftpClientError::Limited(_)) => true,
        Some(SftpClientError::Status(status)) => matches!(
            status.status_code,
            StatusCode::Eof
                | StatusCode::NoSuchFile
                | StatusCode::PermissionDenied
                | StatusCode::Failure
                | StatusCode::OpUnsupported
        ),
        _ => false,
    }
}

struct SftpConnectedSession {
    session: Arc<client::Handle<SftpClientHandler>>,
    jump_sessions: Vec<Arc<client::Handle<SftpClientHandler>>>,
}

async fn open_sftp_session(connected_session: &SftpConnectedSession) -> Result<SftpSession> {
    let channel = connected_session
        .session
        .channel_open_session()
        .await
        .context("failed to open SSH session channel for SFTP")?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .context("failed to start SFTP subsystem")?;
    SftpSession::new(channel.into_stream())
        .await
        .context("failed to initialize SFTP session")
}

fn sftp_entry_kind(file_type: FileType) -> miaominal_core::sftp::SftpEntryKind {
    match file_type {
        FileType::Dir => miaominal_core::sftp::SftpEntryKind::Directory,
        FileType::File => miaominal_core::sftp::SftpEntryKind::File,
        FileType::Symlink => miaominal_core::sftp::SftpEntryKind::Symlink,
        FileType::Other => miaominal_core::sftp::SftpEntryKind::Other,
    }
}

impl SftpConnectedSession {
    async fn disconnect(self) {
        if let Err(error) = self
            .session
            .disconnect(Disconnect::ByApplication, "", "English")
            .await
        {
            log::debug!("failed to disconnect SFTP session cleanly: {error:?}");
        }

        for jump_session in self.jump_sessions.into_iter().rev() {
            if let Err(error) = jump_session
                .disconnect(Disconnect::ByApplication, "", "English")
                .await
            {
                log::debug!("failed to disconnect ProxyJump SFTP session cleanly: {error:?}");
            }
        }
    }
}

struct SftpClientHandler {
    known_hosts: KnownHostsStore,
    host: String,
    port: u16,
    host_key_prompts: Option<SftpHostKeyPrompts>,
}

fn fingerprint_of(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

fn algorithm_of(key: &PublicKey) -> String {
    key.algorithm().to_string()
}

impl client::Handler for SftpClientHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        let check = self
            .known_hosts
            .check(&self.host, self.port, server_public_key)?;

        let Some(host_key_prompts) = self.host_key_prompts.clone() else {
            return match check {
                HostKeyCheck::Match => Ok(true),
                HostKeyCheck::Unknown => bail!(
                    "SFTP requires a saved host key for {}:{}. Connect once via SSH and choose accept-and-save before opening SFTP.",
                    self.host,
                    self.port
                ),
                HostKeyCheck::Mismatch { .. } => bail!(
                    "SFTP refused to connect because the saved host key for {}:{} does not match.",
                    self.host,
                    self.port
                ),
            };
        };

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

        if send_event(
            &host_key_prompts.event_sender,
            SftpEvent::HostKeyPrompt(prompt),
        )
        .await
        .is_err()
        {
            return Ok(false);
        }

        let decision = {
            let mut decisions = host_key_prompts.decisions.lock().await;
            decisions
                .recv()
                .await
                .ok_or_else(|| anyhow!("host key decision channel closed"))?
        };

        match decision {
            HostKeyDecision::AcceptOnce => Ok(true),
            HostKeyDecision::AcceptAndSave => {
                if let Err(error) = self
                    .known_hosts
                    .learn(&self.host, self.port, server_public_key)
                {
                    log::warn!("failed to record host key: {error:?}");
                    let _ = send_event(
                        &host_key_prompts.event_sender,
                        SftpEvent::Status(format!("Could not save host key: {error}")),
                    )
                    .await;
                }
                Ok(true)
            }
            HostKeyDecision::Reject => Ok(false),
        }
    }
}

async fn connect_authenticated_session(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    host_key_prompts: Option<SftpHostKeyPrompts>,
    event_sender: &SftpEventSender,
) -> Result<SftpConnectedSession> {
    let profile = ssh::hydrate_profile_from_secrets(profile, &secrets);
    let remote = format!("{}@{}:{}", profile.username, profile.host, profile.port);
    let entry_proxy =
        ssh::transport::resolve_entry_proxy(profile.entry_proxy_id.as_deref(), &all_proxies)?
            .cloned();
    let proxy_jump_profiles =
        ssh::connection::resolve_proxy_jump_profiles(&profile, &all_profiles)?
            .into_iter()
            .map(|profile| ssh::hydrate_profile_from_secrets(profile, &secrets))
            .collect::<Vec<_>>();
    let config = ssh::connection::default_client_config();
    let mut jump_sessions = Vec::new();

    let session = if let Some(first_hop) = proxy_jump_profiles.first() {
        emit_status(
            event_sender,
            format!(
                "Connecting SFTP via jump host 1/{}: {}",
                proxy_jump_profiles.len(),
                first_hop.summary()
            ),
        )
        .await?;

        let mut current_session = connect_profile_with_optional_proxy(
            first_hop,
            entry_proxy.as_ref(),
            &secrets,
            config.clone(),
            known_hosts.clone(),
            host_key_prompts.clone(),
        )
        .await?;
        emit_status(
            event_sender,
            format!(
                "Authenticating SFTP jump host 1/{}",
                proxy_jump_profiles.len()
            ),
        )
        .await?;
        ssh::authenticate(&mut current_session, first_hop.clone(), &secrets).await?;
        let mut current_session = Arc::new(current_session);

        let mut remaining_chain: Vec<_> = proxy_jump_profiles.iter().skip(1).cloned().collect();
        remaining_chain.push(profile);
        let total_hops = proxy_jump_profiles.len();

        for (index, next_profile) in remaining_chain.into_iter().enumerate() {
            let is_target = index + 1 == total_hops;
            emit_status(
                event_sender,
                if is_target {
                    format!("Connecting SFTP to {remote} through ProxyJump")
                } else {
                    format!(
                        "Connecting SFTP jump host {}/{}: {}",
                        index + 2,
                        total_hops,
                        next_profile.summary()
                    )
                },
            )
            .await?;

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
                        "failed to open ProxyJump SFTP channel to {}:{}",
                        next_profile.host, next_profile.port
                    )
                })?
                .into_stream();

            let mut next_session = connect_profile_stream(
                &next_profile,
                transport,
                config.clone(),
                known_hosts.clone(),
                host_key_prompts.clone(),
            )
            .await?;
            emit_status(
                event_sender,
                if is_target {
                    format!("Authenticating SFTP to {remote}")
                } else {
                    format!("Authenticating SFTP jump host {}/{}", index + 2, total_hops)
                },
            )
            .await?;
            ssh::authenticate(&mut next_session, next_profile, &secrets).await?;

            jump_sessions.push(current_session);
            current_session = Arc::new(next_session);
        }

        current_session
    } else {
        emit_status(event_sender, format!("Connecting SFTP to {remote}")).await?;
        let mut session = connect_profile_with_optional_proxy(
            &profile,
            entry_proxy.as_ref(),
            &secrets,
            config,
            known_hosts,
            host_key_prompts,
        )
        .await?;
        emit_status(event_sender, format!("Authenticating SFTP to {remote}")).await?;
        ssh::authenticate(&mut session, profile, &secrets).await?;
        Arc::new(session)
    };

    Ok(SftpConnectedSession {
        session,
        jump_sessions,
    })
}

async fn connect_profile_session(
    profile: &SessionProfile,
    config: Arc<client::Config>,
    known_hosts: KnownHostsStore,
    host_key_prompts: Option<SftpHostKeyPrompts>,
) -> Result<client::Handle<SftpClientHandler>> {
    let handler = SftpClientHandler {
        known_hosts,
        host: profile.host.clone(),
        port: profile.port,
        host_key_prompts,
    };
    ssh::connection::connect_profile_session(profile, config, handler).await
}

async fn connect_profile_with_optional_proxy(
    profile: &SessionProfile,
    proxy: Option<&ProxyProfile>,
    secrets: &SecretStore,
    config: Arc<client::Config>,
    known_hosts: KnownHostsStore,
    host_key_prompts: Option<SftpHostKeyPrompts>,
) -> Result<client::Handle<SftpClientHandler>> {
    let Some(proxy) = proxy else {
        return connect_profile_session(profile, config, known_hosts, host_key_prompts).await;
    };
    let transport =
        ssh::transport::connect_via_proxy(proxy, &profile.host, profile.port, secrets).await?;
    connect_profile_stream(profile, transport, config, known_hosts, host_key_prompts).await
}

async fn connect_profile_stream<R>(
    profile: &SessionProfile,
    transport: R,
    config: Arc<client::Config>,
    known_hosts: KnownHostsStore,
    host_key_prompts: Option<SftpHostKeyPrompts>,
) -> Result<client::Handle<SftpClientHandler>>
where
    R: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let handler = SftpClientHandler {
        known_hosts,
        host: profile.host.clone(),
        port: profile.port,
        host_key_prompts,
    };
    ssh::connection::connect_profile_stream(profile, transport, config, handler).await
}

async fn emit_status(event_sender: &SftpEventSender, message: String) -> Result<()> {
    send_event(event_sender, SftpEvent::Status(message)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use russh::client::Handler as _;
    use russh_sftp::protocol::Status;
    use std::time::Duration;

    const TEST_PUBLIC_KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn test_command_sender() -> (
        SftpCommandSender,
        UnboundedReceiver<SftpCommand>,
        UnboundedReceiver<HostKeyDecision>,
    ) {
        let (sender, command_receiver) = unbounded_channel();
        let (decision_sender, decision_receiver) = unbounded_channel();
        (
            SftpCommandSender {
                sender,
                host_key_decisions: decision_sender,
                next_transfer_id: Arc::new(AtomicU64::new(1)),
                next_directory_request_id: Arc::new(AtomicU64::new(1)),
            },
            command_receiver,
            decision_receiver,
        )
    }

    fn status_error(status_code: StatusCode, message: &str) -> anyhow::Error {
        SftpClientError::Status(Status {
            id: 1,
            status_code,
            error_message: message.into(),
            language_tag: "en".into(),
        })
        .into()
    }

    #[test]
    fn event_channel_backpressures_when_the_receiver_is_slow() {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("build runtime")
            .block_on(async {
                let (event_sender, _event_receiver) = sftp_event_channel();
                let producer_sender = event_sender.clone();
                let mut producer = tokio::spawn(async move {
                    for index in 0..(SFTP_EVENT_CHANNEL_CAPACITY * 4) {
                        send_event(&producer_sender, SftpEvent::Status(index.to_string()))
                            .await
                            .expect("receiver remains open");
                    }
                });

                assert!(
                    tokio::time::timeout(Duration::from_millis(25), &mut producer)
                        .await
                        .is_err(),
                    "producer should block instead of growing the event queue without bound"
                );
                producer.abort();
            });
    }

    #[test]
    fn directory_commands_receive_monotonic_request_ids() {
        let (commands, mut receiver, _decisions) = test_command_sender();

        assert_eq!(
            commands.list_directory("/first").expect("queue first"),
            SftpDirectoryRequestId(1)
        );
        assert_eq!(
            commands.list_directory("/second").expect("queue second"),
            SftpDirectoryRequestId(2)
        );
        assert!(matches!(
            receiver.try_recv(),
            Ok(SftpCommand::ListDirectory {
                request_id: SftpDirectoryRequestId(1),
                path,
            }) if path == "/first"
        ));
        assert!(matches!(
            receiver.try_recv(),
            Ok(SftpCommand::ListDirectory {
                request_id: SftpDirectoryRequestId(2),
                path,
            }) if path == "/second"
        ));
    }

    #[test]
    fn remove_directory_queues_recursive_directory_deletion() {
        let (commands, mut receiver, _decisions) = test_command_sender();

        commands
            .remove_directory("/remote/tree")
            .expect("queue recursive directory deletion");

        assert!(matches!(
            receiver.try_recv(),
            Ok(SftpCommand::RemoveDirectoryRecursive { path }) if path == "/remote/tree"
        ));
    }

    #[test]
    fn host_key_decisions_are_delivered_to_the_session() {
        let (commands, _receiver, mut decisions) = test_command_sender();

        commands
            .respond_host_key(HostKeyDecision::AcceptAndSave)
            .expect("deliver host key decision");

        assert!(matches!(
            decisions.try_recv(),
            Ok(HostKeyDecision::AcceptAndSave)
        ));
    }

    #[test]
    fn interactive_handler_prompts_when_the_host_key_is_unknown() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("build runtime");
        let temp = tempfile::tempdir().expect("create temporary directory");
        let known_hosts_path = temp.path().join("known_hosts");
        let known_hosts = KnownHostsStore::with_path(known_hosts_path);
        let (event_sender, mut event_receiver) = sftp_event_channel();
        let (decision_sender, decision_receiver) = unbounded_channel();
        let mut handler = SftpClientHandler {
            known_hosts: known_hosts.clone(),
            host: "example.test".into(),
            port: 22,
            host_key_prompts: Some(SftpHostKeyPrompts {
                event_sender,
                decisions: Arc::new(Mutex::new(decision_receiver)),
            }),
        };
        let key = PublicKey::from_openssh(TEST_PUBLIC_KEY).expect("parse test public key");

        runtime.block_on(async {
            let check = handler.check_server_key(&key);
            let respond = async {
                match event_receiver.next().await {
                    Some(SftpEvent::HostKeyPrompt(prompt)) => {
                        assert_eq!(prompt.host, "example.test");
                        assert_eq!(prompt.port, 22);
                        assert!(prompt.previous_fingerprint.is_none());
                        assert!(prompt.fingerprint.starts_with("SHA256:"));
                    }
                    other => panic!("expected host key prompt, got {other:?}"),
                }
                decision_sender
                    .send(HostKeyDecision::AcceptAndSave)
                    .expect("deliver host key decision");
            };
            let (trusted, ()) = tokio::join!(check, respond);
            assert!(trusted.expect("accept the prompted host key"));
        });

        assert!(matches!(
            known_hosts
                .check("example.test", 22, &key)
                .expect("check learned host key"),
            HostKeyCheck::Match
        ));
    }

    #[test]
    fn interactive_handler_rejects_on_user_decision() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("build runtime");
        let temp = tempfile::tempdir().expect("create temporary directory");
        let known_hosts = KnownHostsStore::with_path(temp.path().join("known_hosts"));
        let (event_sender, mut event_receiver) = sftp_event_channel();
        let (decision_sender, decision_receiver) = unbounded_channel();
        let mut handler = SftpClientHandler {
            known_hosts,
            host: "example.test".into(),
            port: 22,
            host_key_prompts: Some(SftpHostKeyPrompts {
                event_sender,
                decisions: Arc::new(Mutex::new(decision_receiver)),
            }),
        };
        let key = PublicKey::from_openssh(TEST_PUBLIC_KEY).expect("parse test public key");

        runtime.block_on(async {
            let check = handler.check_server_key(&key);
            let respond = async {
                assert!(matches!(
                    event_receiver.next().await,
                    Some(SftpEvent::HostKeyPrompt(_))
                ));
                decision_sender
                    .send(HostKeyDecision::Reject)
                    .expect("deliver host key decision");
            };
            let (trusted, ()) = tokio::join!(check, respond);
            assert!(
                !trusted.expect("rejection is reported as an untrusted key"),
                "rejected host key must not be trusted"
            );
        });
    }

    #[test]
    fn non_interactive_handler_fails_closed_without_prompting() {
        let temp = tempfile::tempdir().expect("create temporary directory");
        let known_hosts = KnownHostsStore::with_path(temp.path().join("known_hosts"));
        let (event_sender, mut event_receiver) = sftp_event_channel();
        let mut handler = SftpClientHandler {
            known_hosts,
            host: "example.test".into(),
            port: 22,
            host_key_prompts: None,
        };
        let key = PublicKey::from_openssh(TEST_PUBLIC_KEY).expect("parse test public key");

        let error = futures::executor::block_on(handler.check_server_key(&key))
            .expect_err("strict handler must reject unknown host keys");
        assert!(
            error.to_string().contains("requires a saved host key"),
            "unexpected error: {error}"
        );

        drop(event_sender);
        assert!(
            futures::executor::block_on(event_receiver.next()).is_none(),
            "strict handler must not emit a host key prompt"
        );
    }

    #[test]
    fn closed_progress_receiver_does_not_retain_transfer_ids() {
        let (sender, receiver) = sftp_progress_channel();
        drop(receiver);
        sender.send(SftpTransferProgress {
            transfer_id: TransferId(77),
            bytes_complete: 1,
            bytes_total: Some(1),
            child: None,
        });

        let state = sender
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert!(state.latest.is_empty());
        assert!(state.queued.is_empty());
    }

    #[test]
    fn subdirectory_failure_emits_requested_path_and_is_recoverable() {
        let (event_sender, mut event_receiver) = sftp_event_channel();
        let operation_result: Result<()> = Err(status_error(
            StatusCode::PermissionDenied,
            "permission denied",
        ))
        .context("failed to read remote directory /protected");

        let recovered = futures::executor::block_on(recover_operation_result(
            &event_sender,
            SftpOperation::ListSubdirectory,
            Some("/protected"),
            operation_result,
        ))
        .expect("operation failure should be reported without becoming fatal");

        assert!(recovered.is_none());
        let event = futures::executor::block_on(event_receiver.next())
            .expect("operation failure should emit an event");
        let expected_message =
            "failed to read remote directory /protected: Permission denied: permission denied";
        match event {
            SftpEvent::Error {
                context,
                path,
                message,
            } => {
                assert_eq!(context, "list_subdirectory");
                assert_eq!(path.as_deref(), Some("/protected"));
                assert_eq!(message, expected_message);
            }
            other => panic!("expected operation error event, got {other:?}"),
        }
        // Hosts that only queue commands need the typed event to report the
        // failure (the legacy error event has no operation identity).
        let typed = futures::executor::block_on(event_receiver.next())
            .expect("operation failure should emit a typed event");
        match typed {
            SftpEvent::OperationFailed {
                operation,
                path,
                message,
            } => {
                assert_eq!(operation, SftpOperation::ListSubdirectory);
                assert_eq!(path.as_deref(), Some("/protected"));
                assert_eq!(message, expected_message);
            }
            other => panic!("expected typed operation failure, got {other:?}"),
        }
    }

    #[test]
    fn operation_failure_is_fatal_when_error_event_cannot_be_delivered() {
        let (event_sender, event_receiver) = sftp_event_channel();
        drop(event_receiver);

        let error = futures::executor::block_on(recover_operation_result::<()>(
            &event_sender,
            SftpOperation::ListDirectory,
            Some("/missing"),
            Err(status_error(StatusCode::NoSuchFile, "path does not exist")),
        ))
        .expect_err("closed event receiver should stop the session loop");

        assert_eq!(error.to_string(), "SFTP event receiver is closed");
    }

    #[test]
    fn operation_limit_is_reported_without_closing_the_session() {
        let (event_sender, mut event_receiver) = sftp_event_channel();

        let recovered = futures::executor::block_on(recover_operation_result::<()>(
            &event_sender,
            SftpOperation::ListDirectory,
            Some("/remote"),
            Err(SftpClientError::Limited("request limit exceeded".into()).into()),
        ))
        .expect("operation limit should remain recoverable");

        assert!(recovered.is_none());
        let event = futures::executor::block_on(event_receiver.next())
            .expect("operation limit should emit an error event");
        assert!(matches!(
            event,
            SftpEvent::Error {
                context,
                path: Some(path),
                ..
            } if context == "list_directory" && path == "/remote"
        ));
    }

    #[test]
    fn operation_limits_and_file_status_errors_are_recoverable() {
        assert!(is_recoverable_operation_error(
            &SftpClientError::Limited("request limit exceeded".into()).into()
        ));

        for status_code in [
            StatusCode::Eof,
            StatusCode::NoSuchFile,
            StatusCode::PermissionDenied,
            StatusCode::Failure,
            StatusCode::OpUnsupported,
        ] {
            assert!(is_recoverable_operation_error(&status_error(
                status_code,
                "file operation failed"
            )));
        }

        for status_code in [
            StatusCode::Ok,
            StatusCode::BadMessage,
            StatusCode::NoConnection,
            StatusCode::ConnectionLost,
        ] {
            assert!(!is_recoverable_operation_error(&status_error(
                status_code,
                "session failure"
            )));
        }
    }

    #[test]
    fn transport_and_connection_errors_remain_fatal() {
        let fatal_errors = [
            SftpClientError::IO("connection reset".into()),
            SftpClientError::Timeout,
            SftpClientError::UnexpectedPacket,
            SftpClientError::UnexpectedBehavior("session closed".into()),
            SftpClientError::Status(Status {
                id: 2,
                status_code: StatusCode::NoConnection,
                error_message: "no connection".into(),
                language_tag: "en".into(),
            }),
            SftpClientError::Status(Status {
                id: 3,
                status_code: StatusCode::ConnectionLost,
                error_message: "connection lost".into(),
                language_tag: "en".into(),
            }),
        ];

        for fatal_error in fatal_errors {
            let (event_sender, mut event_receiver) = sftp_event_channel();
            let error = futures::executor::block_on(recover_operation_result::<()>(
                &event_sender,
                SftpOperation::ListDirectory,
                Some("/remote"),
                Err(fatal_error.into()),
            ))
            .expect_err("connection-level failure should terminate the session loop");

            drop(event_sender);
            assert!(
                futures::executor::block_on(event_receiver.next()).is_none(),
                "fatal operation errors must not be emitted as recoverable errors"
            );
            assert!(
                error.downcast_ref::<SftpClientError>().is_some(),
                "original SFTP error should propagate"
            );
        }
    }
}
