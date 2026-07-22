use super::paths::join_remote_path;
use super::session::{
    SftpEvent, SftpEventSender, SftpProgressSender, SftpTransferChild, SftpTransferChildState,
    SftpTransferChildUpdate, SftpTransferProgress, TransferChildId, send_event,
};
use anyhow::{Context, Result, anyhow};
use miaominal_core::sftp::{TransferDirection, TransferId};
use russh_sftp::{
    client::{SftpSession, error::Error as SftpError},
    protocol::{FileType, OpenFlags, StatusCode},
};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;
use tempfile::{Builder as TempFileBuilder, TempPath};
use tokio::fs::File as TokioFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;

const TRANSFER_CHUNK_SIZE: usize = 256 * 1024;
const TRANSFER_CANCEL_TIMEOUT: Duration = Duration::from_secs(15);
const TRANSFER_TEMP_PREFIX: &str = ".miaominal-transfer-";
const TRANSFER_TEMP_SUFFIX: &str = ".part";
const TRANSFER_BACKUP_PREFIX: &str = ".miaominal-backup-";
const TRANSFER_BACKUP_SUFFIX: &str = ".bak";
static TRANSFER_SIDECAR_SEQUENCE: AtomicU64 = AtomicU64::new(0);

type RemoteOperation<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RemoteDestinationKind {
    Missing,
    File,
    Directory,
    Symlink,
    Other,
}

trait RemoteReplaceOps: Sync {
    fn rename_remote<'a>(&'a self, from: &'a str, to: &'a str) -> RemoteOperation<'a, ()>;

    fn destination_kind<'a>(&'a self, path: &'a str) -> RemoteOperation<'a, RemoteDestinationKind>;

    fn remove_remote_file<'a>(&'a self, path: &'a str) -> RemoteOperation<'a, ()>;
}

impl RemoteReplaceOps for SftpSession {
    fn rename_remote<'a>(&'a self, from: &'a str, to: &'a str) -> RemoteOperation<'a, ()> {
        Box::pin(async move {
            self.rename(from.to_string(), to.to_string())
                .await
                .map_err(anyhow::Error::from)
        })
    }

    fn destination_kind<'a>(&'a self, path: &'a str) -> RemoteOperation<'a, RemoteDestinationKind> {
        Box::pin(async move {
            match self.symlink_metadata(path.to_string()).await {
                Ok(metadata) => Ok(match metadata.file_type() {
                    FileType::File => RemoteDestinationKind::File,
                    FileType::Dir => RemoteDestinationKind::Directory,
                    FileType::Symlink => RemoteDestinationKind::Symlink,
                    FileType::Other => RemoteDestinationKind::Other,
                }),
                Err(SftpError::Status(status)) if status.status_code == StatusCode::NoSuchFile => {
                    Ok(RemoteDestinationKind::Missing)
                }
                Err(error) => Err(error).with_context(|| {
                    format!("failed to inspect existing remote destination {path}")
                }),
            }
        })
    }

    fn remove_remote_file<'a>(&'a self, path: &'a str) -> RemoteOperation<'a, ()> {
        Box::pin(async move {
            self.remove_file(path.to_string())
                .await
                .map_err(anyhow::Error::from)
        })
    }
}

struct LocalTemporaryFile {
    // Struct fields are dropped in declaration order, so the open handle is
    // always closed before TempPath attempts to remove the temporary file.
    file: TokioFile,
    path: TempPath,
}

pub(super) struct TransferControl {
    cancelled: AtomicBool,
    paused: AtomicBool,
    notify: Notify,
}

impl TransferControl {
    pub(super) fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            notify: Notify::new(),
        }
    }

    pub(super) fn pause(&self) -> bool {
        if self.cancelled.load(Ordering::Relaxed) {
            return false;
        }

        let was_paused = self.paused.swap(true, Ordering::Relaxed);
        if !was_paused {
            self.notify_state_change();
        }
        !was_paused
    }

    pub(super) fn resume(&self) -> bool {
        if self.cancelled.load(Ordering::Relaxed) {
            return false;
        }

        let was_paused = self.paused.swap(false, Ordering::Relaxed);
        if was_paused {
            self.notify_state_change();
        }
        was_paused
    }

    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.notify_state_change();
    }

    fn notify_state_change(&self) {
        // Each control has a single transfer task waiting on it. `notify_one`
        // retains a permit when that task has not registered its wait yet.
        self.notify.notify_one();
    }

    async fn wait_until_active(&self) -> TransferControlState {
        loop {
            if self.cancelled.load(Ordering::Relaxed) {
                return TransferControlState::Cancelled;
            }

            if !self.paused.load(Ordering::Relaxed) {
                return TransferControlState::Active;
            }

            self.notify.notified().await;
        }
    }
}

enum TransferControlState {
    Active,
    Cancelled,
}

struct TransferCompletionNotifier {
    transfer_id: TransferId,
    sender: UnboundedSender<TransferId>,
}

struct ActiveTransferPermit {
    semaphore: Arc<Semaphore>,
    permit: Option<OwnedSemaphorePermit>,
}

impl ActiveTransferPermit {
    fn new(semaphore: Arc<Semaphore>) -> Self {
        Self {
            semaphore,
            permit: None,
        }
    }

    async fn wait_until_active(
        &mut self,
        control: &TransferControl,
    ) -> Result<TransferControlState> {
        loop {
            if control.cancelled.load(Ordering::Relaxed) {
                self.permit.take();
                return Ok(TransferControlState::Cancelled);
            }

            if control.paused.load(Ordering::Relaxed) {
                self.permit.take();
                if matches!(
                    control.wait_until_active().await,
                    TransferControlState::Cancelled
                ) {
                    return Ok(TransferControlState::Cancelled);
                }
                continue;
            }

            if self.permit.is_some() {
                return Ok(TransferControlState::Active);
            }

            tokio::select! {
                permit = self.semaphore.clone().acquire_owned() => {
                    self.permit = Some(
                        permit.map_err(|_| anyhow!("SFTP transfer scheduler is closed"))?
                    );
                }
                _ = control.notify.notified() => {}
            }
        }
    }
}

impl Drop for TransferCompletionNotifier {
    fn drop(&mut self) {
        let _ = self.sender.send(self.transfer_id);
    }
}

pub(super) fn remove_completed_transfer(
    transfer_id: TransferId,
    transfer_tasks: &mut HashMap<TransferId, JoinHandle<()>>,
    transfer_controls: &mut HashMap<TransferId, Arc<TransferControl>>,
) {
    transfer_tasks.remove(&transfer_id);
    transfer_controls.remove(&transfer_id);
}

pub(super) async fn cancel_all_transfers(
    transfer_tasks: &mut HashMap<TransferId, JoinHandle<()>>,
    transfer_controls: &mut HashMap<TransferId, Arc<TransferControl>>,
) {
    cancel_all_transfers_with_timeout(transfer_tasks, transfer_controls, TRANSFER_CANCEL_TIMEOUT)
        .await;
}

async fn cancel_all_transfers_with_timeout(
    transfer_tasks: &mut HashMap<TransferId, JoinHandle<()>>,
    transfer_controls: &mut HashMap<TransferId, Arc<TransferControl>>,
    timeout_duration: Duration,
) {
    for control in transfer_controls.values() {
        control.cancel();
    }

    transfer_controls.clear();
    let mut handles: Vec<_> = transfer_tasks.drain().map(|(_, handle)| handle).collect();
    let task_count = handles.len();

    let timed_out = tokio::time::timeout(timeout_duration, async {
        for handle in &mut handles {
            if let Err(error) = handle.await
                && !error.is_cancelled()
            {
                log::debug!("SFTP transfer task failed while cancelling: {error}");
            }
        }
    })
    .await
    .is_err();

    if timed_out {
        log::warn!(
            "timed out after {timeout_duration:?} while cancelling {task_count} SFTP transfer task(s); aborting remaining tasks"
        );
        let pending_handles: Vec<_> = handles
            .into_iter()
            .filter(|handle| !handle.is_finished())
            .collect();
        for handle in &pending_handles {
            handle.abort();
        }
        for handle in pending_handles {
            let _ = handle.await;
        }
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "the task boundary explicitly owns its session, paths, control, concurrency, and event channels"
)]
pub(super) fn spawn_upload_task(
    sftp: Arc<SftpSession>,
    event_sender: SftpEventSender,
    transfer_id: TransferId,
    local_path: PathBuf,
    remote_path: String,
    control: Arc<TransferControl>,
    semaphore: Arc<Semaphore>,
    completion_sender: UnboundedSender<TransferId>,
    progress_sender: SftpProgressSender,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let _completion = TransferCompletionNotifier {
            transfer_id,
            sender: completion_sender,
        };
        let mut active_permit = ActiveTransferPermit::new(semaphore);
        let result = match active_permit.wait_until_active(&control).await {
            Ok(TransferControlState::Active) => {
                upload_path(
                    &sftp,
                    transfer_id,
                    &local_path,
                    &remote_path,
                    &control,
                    &mut active_permit,
                    &event_sender,
                    &progress_sender,
                )
                .await
            }
            Ok(TransferControlState::Cancelled) => Ok(TransferOutcome::Cancelled),
            Err(error) => Err(error),
        };
        drop(active_permit);
        finish_transfer_task(&event_sender, &progress_sender, transfer_id, result).await;
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "the task boundary explicitly owns its session, paths, control, concurrency, and event channels"
)]
pub(super) fn spawn_download_task(
    sftp: Arc<SftpSession>,
    event_sender: SftpEventSender,
    transfer_id: TransferId,
    remote_path: String,
    local_path: PathBuf,
    control: Arc<TransferControl>,
    semaphore: Arc<Semaphore>,
    completion_sender: UnboundedSender<TransferId>,
    progress_sender: SftpProgressSender,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let _completion = TransferCompletionNotifier {
            transfer_id,
            sender: completion_sender,
        };
        let mut active_permit = ActiveTransferPermit::new(semaphore);
        let result = match active_permit.wait_until_active(&control).await {
            Ok(TransferControlState::Active) => {
                download_path(
                    &sftp,
                    transfer_id,
                    &remote_path,
                    &local_path,
                    &control,
                    &mut active_permit,
                    &event_sender,
                    &progress_sender,
                )
                .await
            }
            Ok(TransferControlState::Cancelled) => Ok(TransferOutcome::Cancelled),
            Err(error) => Err(error),
        };
        drop(active_permit);
        finish_transfer_task(&event_sender, &progress_sender, transfer_id, result).await;
    })
}

enum TransferOutcome {
    Done,
    Cancelled,
}

struct ActiveTransferChild {
    child_id: TransferChildId,
    bytes_complete: u64,
}

struct TransferProgress<'a> {
    event_sender: &'a SftpEventSender,
    progress_sender: &'a SftpProgressSender,
    transfer_id: TransferId,
    bytes_total: Option<u64>,
    bytes_complete: u64,
    active_child: Option<ActiveTransferChild>,
    next_child_id: u64,
}

impl TransferProgress<'_> {
    async fn emit(&self, child: Option<SftpTransferChildUpdate>) -> Result<()> {
        emit_transfer_progress(
            self.progress_sender,
            self.transfer_id,
            self.bytes_complete,
            self.bytes_total,
            child,
        )
        .await
    }

    async fn begin_child(
        &mut self,
        relative_path: String,
        bytes_total: Option<u64>,
    ) -> Result<TransferChildId> {
        let child_id = TransferChildId(self.next_child_id);
        self.next_child_id = self.next_child_id.saturating_add(1);
        self.active_child = Some(ActiveTransferChild {
            child_id,
            bytes_complete: 0,
        });
        emit_transfer_child_started(
            self.event_sender,
            self.transfer_id,
            SftpTransferChild {
                child_id,
                relative_path,
                bytes_total,
            },
        )
        .await?;
        Ok(child_id)
    }

    async fn advance(&mut self, bytes: u64) -> Result<()> {
        self.bytes_complete = self.bytes_complete.saturating_add(bytes);
        let child = self.active_child.as_mut().map(|child| {
            child.bytes_complete = child.bytes_complete.saturating_add(bytes);
            SftpTransferChildUpdate {
                child_id: child.child_id,
                bytes_complete: child.bytes_complete,
                state: SftpTransferChildState::Running,
            }
        });
        self.emit(child).await
    }

    async fn finish_child(&mut self, state: SftpTransferChildState) -> Result<()> {
        let Some(child) = self.active_child.take() else {
            return Ok(());
        };
        emit_transfer_child_finished(
            self.event_sender,
            self.transfer_id,
            SftpTransferChildUpdate {
                child_id: child.child_id,
                bytes_complete: child.bytes_complete,
                state,
            },
        )
        .await
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "transfer recursion needs the path pair plus shared control, permit, and progress channels"
)]
async fn upload_path(
    sftp: &SftpSession,
    transfer_id: TransferId,
    local_path: &Path,
    remote_path: &str,
    control: &TransferControl,
    active_permit: &mut ActiveTransferPermit,
    event_sender: &SftpEventSender,
    progress_sender: &SftpProgressSender,
) -> Result<TransferOutcome> {
    let source = inspect_local_upload_source(local_path).await?;
    match source {
        LocalUploadSource::Directory => {
            let mut progress = TransferProgress {
                event_sender,
                progress_sender,
                transfer_id,
                bytes_total: None,
                bytes_complete: 0,
                active_child: None,
                next_child_id: 0,
            };
            progress.emit(None).await?;
            upload_directory(
                sftp,
                local_path,
                remote_path,
                String::new(),
                control,
                active_permit,
                &mut progress,
            )
            .await
        }
        LocalUploadSource::File { len } => {
            let mut progress = TransferProgress {
                event_sender,
                progress_sender,
                transfer_id,
                bytes_total: Some(len),
                bytes_complete: 0,
                active_child: None,
                next_child_id: 0,
            };
            progress.emit(None).await?;
            upload_regular_file(
                sftp,
                local_path,
                remote_path,
                control,
                active_permit,
                &mut progress,
            )
            .await
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum LocalUploadSource {
    Directory,
    File { len: u64 },
}

async fn inspect_local_upload_source(local_path: &Path) -> Result<LocalUploadSource> {
    let metadata = tokio::fs::symlink_metadata(local_path)
        .await
        .with_context(|| format!("failed to read metadata for {}", local_path.display()))?;
    let file_type = metadata.file_type();

    if file_type.is_symlink() {
        return Err(anyhow!(
            "refusing to upload symbolic link {}",
            local_path.display()
        ));
    }
    if file_type.is_dir() {
        return Ok(LocalUploadSource::Directory);
    }
    if file_type.is_file() {
        return Ok(LocalUploadSource::File {
            len: metadata.len(),
        });
    }

    Err(anyhow!(
        "refusing to upload unsupported file type {}",
        local_path.display()
    ))
}

#[cfg(test)]
async fn compute_local_directory_size(local_root: &Path) -> Result<u64> {
    let mut total = 0_u64;
    let mut stack = vec![local_root.to_path_buf()];
    while let Some(local_dir) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&local_dir)
            .await
            .with_context(|| format!("failed to read {}", local_dir.display()))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .with_context(|| format!("failed to iterate {}", local_dir.display()))?
        {
            let local_child = entry.path();
            match inspect_local_upload_source(&local_child).await? {
                LocalUploadSource::Directory => stack.push(local_child),
                LocalUploadSource::File { len } => total = total.saturating_add(len),
            }
        }
    }
    Ok(total)
}

fn upload_directory<'a, 'event>(
    sftp: &'a SftpSession,
    local_dir: &'a Path,
    remote_dir: &'a str,
    relative_dir: String,
    control: &'a TransferControl,
    active_permit: &'a mut ActiveTransferPermit,
    progress: &'a mut TransferProgress<'event>,
) -> Pin<Box<dyn Future<Output = Result<TransferOutcome>> + Send + 'a>>
where
    'event: 'a,
{
    Box::pin(async move {
        if matches!(
            active_permit.wait_until_active(control).await?,
            TransferControlState::Cancelled
        ) {
            return Ok(TransferOutcome::Cancelled);
        }
        if !matches!(
            inspect_local_upload_source(local_dir).await?,
            LocalUploadSource::Directory
        ) {
            return Err(anyhow!(
                "expected local upload directory {}",
                local_dir.display()
            ));
        }
        let _ = sftp.create_dir(remote_dir.to_string()).await;

        let mut entries = tokio::fs::read_dir(local_dir)
            .await
            .with_context(|| format!("failed to read {}", local_dir.display()))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .with_context(|| format!("failed to iterate {}", local_dir.display()))?
        {
            if matches!(
                active_permit.wait_until_active(control).await?,
                TransferControlState::Cancelled
            ) {
                return Ok(TransferOutcome::Cancelled);
            }

            let local_child = entry.path();
            let filename = entry.file_name().to_string_lossy().into_owned();
            let remote_child = join_remote_path(remote_dir, &filename);
            let relative_child = join_remote_path(&relative_dir, &filename);
            match inspect_local_upload_source(&local_child).await? {
                LocalUploadSource::Directory => {
                    if matches!(
                        upload_directory(
                            sftp,
                            &local_child,
                            &remote_child,
                            relative_child,
                            control,
                            active_permit,
                            progress,
                        )
                        .await?,
                        TransferOutcome::Cancelled
                    ) {
                        return Ok(TransferOutcome::Cancelled);
                    }
                }
                LocalUploadSource::File { len } => {
                    progress.begin_child(relative_child, Some(len)).await?;
                    match upload_regular_file(
                        sftp,
                        &local_child,
                        &remote_child,
                        control,
                        active_permit,
                        progress,
                    )
                    .await
                    {
                        Ok(TransferOutcome::Done) => {
                            progress.finish_child(SftpTransferChildState::Done).await?;
                        }
                        Ok(TransferOutcome::Cancelled) => {
                            progress
                                .finish_child(SftpTransferChildState::Cancelled)
                                .await?;
                            return Ok(TransferOutcome::Cancelled);
                        }
                        Err(error) => {
                            let _ = progress
                                .finish_child(SftpTransferChildState::Failed(error.to_string()))
                                .await;
                            return Err(error);
                        }
                    }
                }
            }
        }

        Ok(TransferOutcome::Done)
    })
}

async fn upload_regular_file(
    sftp: &SftpSession,
    local_path: &Path,
    remote_path: &str,
    control: &TransferControl,
    active_permit: &mut ActiveTransferPermit,
    progress: &mut TransferProgress<'_>,
) -> Result<TransferOutcome> {
    if !matches!(
        inspect_local_upload_source(local_path).await?,
        LocalUploadSource::File { .. }
    ) {
        return Err(anyhow!(
            "expected regular file for upload {}",
            local_path.display()
        ));
    }

    let mut local_file = TokioFile::open(local_path)
        .await
        .with_context(|| format!("failed to open {} for upload", local_path.display()))?;
    let temporary_path = remote_temporary_path(remote_path, progress.transfer_id);
    let mut remote_file = sftp
        .open_with_flags(
            temporary_path.clone(),
            OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
        )
        .await
        .with_context(|| {
            format!("failed to create temporary remote file {temporary_path} for upload")
        })?;

    let mut buffer = vec![0; TRANSFER_CHUNK_SIZE];
    let transfer_result = async {
        loop {
            if matches!(
                active_permit.wait_until_active(control).await?,
                TransferControlState::Cancelled
            ) {
                return Ok(TransferOutcome::Cancelled);
            }

            let read = local_file.read(&mut buffer).await.with_context(|| {
                format!("failed to read {} while uploading", local_path.display())
            })?;
            if read == 0 {
                break;
            }

            remote_file
                .write_all(&buffer[..read])
                .await
                .with_context(|| {
                    format!("failed to write temporary remote file {temporary_path}")
                })?;

            progress.advance(read as u64).await?;
        }

        remote_file
            .sync_all()
            .await
            .with_context(|| format!("failed to sync temporary remote file {temporary_path}"))?;

        if matches!(
            active_permit.wait_until_active(control).await?,
            TransferControlState::Cancelled
        ) {
            return Ok(TransferOutcome::Cancelled);
        }
        Ok(TransferOutcome::Done)
    }
    .await;

    let shutdown_result = remote_file
        .shutdown()
        .await
        .with_context(|| format!("failed to close temporary remote file {temporary_path}"));

    match transfer_result {
        Ok(TransferOutcome::Done) => {
            if let Err(error) = shutdown_result {
                remove_remote_temporary_file(sftp, &temporary_path).await;
                return Err(error);
            }

            if let Err(error) =
                replace_remote_upload(sftp, &temporary_path, remote_path, progress.transfer_id)
                    .await
            {
                remove_remote_temporary_file(sftp, &temporary_path).await;
                return Err(error);
            }

            Ok(TransferOutcome::Done)
        }
        Ok(TransferOutcome::Cancelled) => {
            remove_remote_temporary_file(sftp, &temporary_path).await;
            Ok(TransferOutcome::Cancelled)
        }
        Err(error) => {
            remove_remote_temporary_file(sftp, &temporary_path).await;
            Err(error)
        }
    }
}

async fn replace_remote_upload<R: RemoteReplaceOps + ?Sized>(
    remote: &R,
    temporary_path: &str,
    destination_path: &str,
    transfer_id: TransferId,
) -> Result<()> {
    let direct_error = match remote.rename_remote(temporary_path, destination_path).await {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };

    match remote.destination_kind(destination_path).await? {
        RemoteDestinationKind::Missing => {
            return Err(direct_error).with_context(|| {
                format!(
                    "failed to install uploaded remote file {temporary_path} at {destination_path}"
                )
            });
        }
        RemoteDestinationKind::Directory => {
            return Err(anyhow!(
                "cannot replace remote directory {destination_path} with an uploaded file"
            ));
        }
        RemoteDestinationKind::Other => {
            return Err(anyhow!(
                "cannot replace special remote entry {destination_path} with an uploaded file"
            ));
        }
        RemoteDestinationKind::File | RemoteDestinationKind::Symlink => {}
    }

    let backup_path = remote_backup_path(destination_path, transfer_id);
    remote
        .rename_remote(destination_path, &backup_path)
        .await
        .with_context(|| {
            format!(
                "failed to move existing remote file {destination_path} to backup {backup_path}"
            )
        })?;

    if let Err(install_error) = remote.rename_remote(temporary_path, destination_path).await {
        return match remote
            .rename_remote(&backup_path, destination_path)
            .await
        {
            Ok(()) => Err(install_error).with_context(|| {
                format!(
                    "failed to replace remote file {destination_path}; restored the original from {backup_path}"
                )
            }),
            Err(rollback_error) => Err(anyhow!(
                "failed to replace remote file {destination_path}: {install_error:#}; failed to restore the original from {backup_path}: {rollback_error:#}; the original remains at {backup_path}"
            )),
        };
    }

    if let Err(error) = remote.remove_remote_file(&backup_path).await {
        log::warn!(
            "uploaded remote file {destination_path}, but failed to remove backup {backup_path}: {error:#}"
        );
    }

    Ok(())
}

fn remote_temporary_path(remote_path: &str, transfer_id: TransferId) -> String {
    remote_transfer_sidecar_path(
        remote_path,
        transfer_id,
        TRANSFER_TEMP_PREFIX,
        TRANSFER_TEMP_SUFFIX,
    )
}

fn remote_backup_path(remote_path: &str, transfer_id: TransferId) -> String {
    remote_transfer_sidecar_path(
        remote_path,
        transfer_id,
        TRANSFER_BACKUP_PREFIX,
        TRANSFER_BACKUP_SUFFIX,
    )
}

fn remote_transfer_sidecar_path(
    remote_path: &str,
    transfer_id: TransferId,
    prefix: &str,
    suffix: &str,
) -> String {
    let parent = match remote_path.rsplit_once('/') {
        Some(("", _)) => "/",
        Some((parent, _)) => parent,
        None => ".",
    };
    let sequence = TRANSFER_SIDECAR_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let filename = format!(
        "{prefix}{}-{}-{sequence}{suffix}",
        std::process::id(),
        transfer_id.0
    );
    join_remote_path(parent, &filename)
}

async fn remove_remote_temporary_file(sftp: &SftpSession, temporary_path: &str) {
    if let Err(error) = sftp.remove_file(temporary_path.to_string()).await {
        log::warn!("failed to remove temporary remote file {temporary_path}: {error}");
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "transfer recursion needs the path pair plus shared control, permit, and progress channels"
)]
async fn download_path(
    sftp: &SftpSession,
    transfer_id: TransferId,
    remote_path: &str,
    local_path: &Path,
    control: &TransferControl,
    active_permit: &mut ActiveTransferPermit,
    event_sender: &SftpEventSender,
    progress_sender: &SftpProgressSender,
) -> Result<TransferOutcome> {
    let metadata = sftp
        .metadata(remote_path.to_string())
        .await
        .with_context(|| format!("failed to read remote metadata for {remote_path}"))?;
    if metadata.is_dir() {
        let mut progress = TransferProgress {
            event_sender,
            progress_sender,
            transfer_id,
            bytes_total: None,
            bytes_complete: 0,
            active_child: None,
            next_child_id: 0,
        };
        progress.emit(None).await?;
        download_directory(
            sftp,
            remote_path,
            local_path,
            String::new(),
            control,
            active_permit,
            &mut progress,
        )
        .await
    } else {
        let mut progress = TransferProgress {
            event_sender,
            progress_sender,
            transfer_id,
            bytes_total: metadata.size,
            bytes_complete: 0,
            active_child: None,
            next_child_id: 0,
        };
        progress.emit(None).await?;
        download_regular_file(
            sftp,
            remote_path,
            local_path,
            control,
            active_permit,
            &mut progress,
        )
        .await
    }
}

fn download_directory<'a, 'event>(
    sftp: &'a SftpSession,
    remote_dir: &'a str,
    local_dir: &'a Path,
    relative_dir: String,
    control: &'a TransferControl,
    active_permit: &'a mut ActiveTransferPermit,
    progress: &'a mut TransferProgress<'event>,
) -> Pin<Box<dyn Future<Output = Result<TransferOutcome>> + Send + 'a>>
where
    'event: 'a,
{
    Box::pin(async move {
        if matches!(
            active_permit.wait_until_active(control).await?,
            TransferControlState::Cancelled
        ) {
            return Ok(TransferOutcome::Cancelled);
        }
        tokio::fs::create_dir_all(local_dir)
            .await
            .with_context(|| format!("failed to create {}", local_dir.display()))?;

        for entry in sftp
            .read_dir(remote_dir)
            .await
            .with_context(|| format!("failed to read remote directory {remote_dir}"))?
        {
            if matches!(
                active_permit.wait_until_active(control).await?,
                TransferControlState::Cancelled
            ) {
                return Ok(TransferOutcome::Cancelled);
            }

            let metadata = entry.metadata();
            let filename = entry.file_name();
            let remote_child = join_remote_path(remote_dir, &filename);
            let local_child = local_dir.join(&filename);
            let relative_child = join_remote_path(&relative_dir, &filename);
            if metadata.is_dir() {
                if matches!(
                    download_directory(
                        sftp,
                        &remote_child,
                        &local_child,
                        relative_child,
                        control,
                        active_permit,
                        progress,
                    )
                    .await?,
                    TransferOutcome::Cancelled
                ) {
                    return Ok(TransferOutcome::Cancelled);
                }
                continue;
            }

            progress.begin_child(relative_child, metadata.size).await?;
            match download_regular_file(
                sftp,
                &remote_child,
                &local_child,
                control,
                active_permit,
                progress,
            )
            .await
            {
                Ok(TransferOutcome::Done) => {
                    progress.finish_child(SftpTransferChildState::Done).await?;
                }
                Ok(TransferOutcome::Cancelled) => {
                    progress
                        .finish_child(SftpTransferChildState::Cancelled)
                        .await?;
                    return Ok(TransferOutcome::Cancelled);
                }
                Err(error) => {
                    let _ = progress
                        .finish_child(SftpTransferChildState::Failed(error.to_string()))
                        .await;
                    return Err(error);
                }
            }
        }

        Ok(TransferOutcome::Done)
    })
}

async fn download_regular_file(
    sftp: &SftpSession,
    remote_path: &str,
    local_path: &Path,
    control: &TransferControl,
    active_permit: &mut ActiveTransferPermit,
    progress: &mut TransferProgress<'_>,
) -> Result<TransferOutcome> {
    if let Some(parent) = local_path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let mut remote_file = sftp
        .open(remote_path.to_string())
        .await
        .with_context(|| format!("failed to open remote file {remote_path} for download"))?;
    let transfer_result = async {
        let mut temporary_file = create_local_temporary_file(local_path)?;
        let mut buffer = vec![0; TRANSFER_CHUNK_SIZE];
        loop {
            if matches!(
                active_permit.wait_until_active(control).await?,
                TransferControlState::Cancelled
            ) {
                let _ = temporary_file.file.shutdown().await;
                return Ok(TransferOutcome::Cancelled);
            }

            let read = remote_file
                .read(&mut buffer)
                .await
                .with_context(|| format!("failed to read remote file {remote_path}"))?;
            if read == 0 {
                break;
            }

            temporary_file
                .file
                .write_all(&buffer[..read])
                .await
                .with_context(|| {
                    format!(
                        "failed to write temporary file for {} while downloading",
                        local_path.display()
                    )
                })?;

            progress.advance(read as u64).await?;
        }

        temporary_file.file.flush().await.with_context(|| {
            format!(
                "failed to flush temporary file for {} after download",
                local_path.display()
            )
        })?;
        temporary_file.file.sync_all().await.with_context(|| {
            format!(
                "failed to sync temporary file for {} after download",
                local_path.display()
            )
        })?;

        if matches!(
            active_permit.wait_until_active(control).await?,
            TransferControlState::Cancelled
        ) {
            let _ = temporary_file.file.shutdown().await;
            return Ok(TransferOutcome::Cancelled);
        }

        let LocalTemporaryFile { file, path } = temporary_file;
        drop(file);
        persist_local_temporary_file(path, local_path)?;
        Ok(TransferOutcome::Done)
    }
    .await;

    let shutdown_result = remote_file
        .shutdown()
        .await
        .with_context(|| format!("failed to close remote file {remote_path} after download"));

    match transfer_result {
        Ok(outcome) => {
            shutdown_result?;
            Ok(outcome)
        }
        Err(error) => {
            if let Err(shutdown_error) = shutdown_result {
                log::warn!(
                    "failed to close remote file {remote_path} after download error: {shutdown_error:#}"
                );
            }
            Err(error)
        }
    }
}

fn create_local_temporary_file(local_path: &Path) -> Result<LocalTemporaryFile> {
    let parent = local_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let temporary = TempFileBuilder::new()
        .prefix(TRANSFER_TEMP_PREFIX)
        .suffix(TRANSFER_TEMP_SUFFIX)
        .tempfile_in(parent)
        .with_context(|| {
            format!(
                "failed to create temporary download file in {}",
                parent.display()
            )
        })?;
    let (file, path) = temporary.into_parts();
    Ok(LocalTemporaryFile {
        file: TokioFile::from_std(file),
        path,
    })
}

fn persist_local_temporary_file(temporary_path: TempPath, local_path: &Path) -> Result<()> {
    temporary_path
        .persist(local_path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to atomically replace {}", local_path.display()))
}

async fn finish_transfer_task(
    event_sender: &SftpEventSender,
    progress_sender: &SftpProgressSender,
    transfer_id: TransferId,
    result: Result<TransferOutcome>,
) {
    if let Some(progress) = progress_sender.take_latest(transfer_id) {
        let _ = send_event(event_sender, SftpEvent::TransferProgressFinal(progress)).await;
    }
    match result {
        Ok(TransferOutcome::Done) => {
            let _ = emit_transfer_done(event_sender, transfer_id).await;
        }
        Ok(TransferOutcome::Cancelled) => {
            let _ = emit_transfer_cancelled(event_sender, transfer_id).await;
        }
        Err(error) => {
            let _ = emit_transfer_failed(event_sender, transfer_id, error.to_string()).await;
        }
    }
}

pub(super) async fn emit_transfer_queued(
    event_sender: &SftpEventSender,
    transfer_id: TransferId,
    direction: TransferDirection,
    source: PathBuf,
    destination: String,
) -> Result<()> {
    send_event(
        event_sender,
        SftpEvent::TransferQueued {
            transfer_id,
            direction,
            source,
            destination,
        },
    )
    .await
}

async fn emit_transfer_child_started(
    event_sender: &SftpEventSender,
    transfer_id: TransferId,
    child: SftpTransferChild,
) -> Result<()> {
    send_event(
        event_sender,
        SftpEvent::TransferChildStarted { transfer_id, child },
    )
    .await
}

async fn emit_transfer_child_finished(
    event_sender: &SftpEventSender,
    transfer_id: TransferId,
    child: SftpTransferChildUpdate,
) -> Result<()> {
    send_event(
        event_sender,
        SftpEvent::TransferChildFinished { transfer_id, child },
    )
    .await
}

async fn emit_transfer_progress(
    progress_sender: &SftpProgressSender,
    transfer_id: TransferId,
    bytes_complete: u64,
    bytes_total: Option<u64>,
    child: Option<SftpTransferChildUpdate>,
) -> Result<()> {
    progress_sender.send(SftpTransferProgress {
        transfer_id,
        bytes_complete,
        bytes_total,
        child,
    });
    Ok(())
}

async fn emit_transfer_done(event_sender: &SftpEventSender, transfer_id: TransferId) -> Result<()> {
    send_event(event_sender, SftpEvent::TransferDone { transfer_id }).await
}

pub(super) async fn emit_transfer_paused(
    event_sender: &SftpEventSender,
    transfer_id: TransferId,
) -> Result<()> {
    send_event(event_sender, SftpEvent::TransferPaused { transfer_id }).await
}

pub(super) async fn emit_transfer_resumed(
    event_sender: &SftpEventSender,
    transfer_id: TransferId,
) -> Result<()> {
    send_event(event_sender, SftpEvent::TransferResumed { transfer_id }).await
}

async fn emit_transfer_cancelled(
    event_sender: &SftpEventSender,
    transfer_id: TransferId,
) -> Result<()> {
    send_event(event_sender, SftpEvent::TransferCancelled { transfer_id }).await
}

async fn emit_transfer_failed(
    event_sender: &SftpEventSender,
    transfer_id: TransferId,
    message: String,
) -> Result<()> {
    send_event(
        event_sender,
        SftpEvent::TransferFailed {
            transfer_id,
            message,
        },
    )
    .await
}

pub(super) async fn emit_error(
    event_sender: &SftpEventSender,
    context: &str,
    message: String,
) -> Result<()> {
    emit_error_with_path(event_sender, context, None, message).await
}

pub(super) async fn emit_error_with_path(
    event_sender: &SftpEventSender,
    context: &str,
    path: Option<String>,
    message: String,
) -> Result<()> {
    send_event(
        event_sender,
        SftpEvent::Error {
            context: context.into(),
            path,
            message,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt as _;
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio::time::timeout;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("build current-thread runtime")
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct FakeRemoteEntry {
        kind: RemoteDestinationKind,
        contents: String,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum FakeRemoteOperation {
        Rename(String, String),
        Inspect(String),
        Remove(String),
    }

    #[derive(Default)]
    struct FakeRemoteReplaceState {
        entries: HashMap<String, FakeRemoteEntry>,
        operations: Vec<FakeRemoteOperation>,
        fail_install_after_backup: bool,
        fail_rollback: bool,
        fail_backup_cleanup: bool,
    }

    #[derive(Default)]
    struct FakeRemoteReplaceOps {
        state: Mutex<FakeRemoteReplaceState>,
    }

    impl FakeRemoteReplaceOps {
        fn with_entries(
            entries: impl IntoIterator<Item = (&'static str, RemoteDestinationKind, &'static str)>,
        ) -> Self {
            let entries = entries
                .into_iter()
                .map(|(path, kind, contents)| {
                    (
                        path.to_string(),
                        FakeRemoteEntry {
                            kind,
                            contents: contents.to_string(),
                        },
                    )
                })
                .collect();
            Self {
                state: Mutex::new(FakeRemoteReplaceState {
                    entries,
                    ..FakeRemoteReplaceState::default()
                }),
            }
        }

        fn update(&self, update: impl FnOnce(&mut FakeRemoteReplaceState)) {
            update(&mut self.state.lock().expect("fake remote lock"));
        }

        fn entry(&self, path: &str) -> Option<FakeRemoteEntry> {
            self.state
                .lock()
                .expect("fake remote lock")
                .entries
                .get(path)
                .cloned()
        }

        fn backup_entries(&self) -> Vec<(String, FakeRemoteEntry)> {
            self.state
                .lock()
                .expect("fake remote lock")
                .entries
                .iter()
                .filter(|(path, _)| path.contains(TRANSFER_BACKUP_PREFIX))
                .map(|(path, entry)| (path.clone(), entry.clone()))
                .collect()
        }

        fn operations(&self) -> Vec<FakeRemoteOperation> {
            self.state
                .lock()
                .expect("fake remote lock")
                .operations
                .clone()
        }
    }

    impl RemoteReplaceOps for FakeRemoteReplaceOps {
        fn rename_remote<'a>(&'a self, from: &'a str, to: &'a str) -> RemoteOperation<'a, ()> {
            Box::pin(async move {
                let mut state = self.state.lock().expect("fake remote lock");
                state
                    .operations
                    .push(FakeRemoteOperation::Rename(from.into(), to.into()));

                let backup_exists = state
                    .entries
                    .keys()
                    .any(|path| path.contains(TRANSFER_BACKUP_PREFIX));
                if state.fail_install_after_backup
                    && backup_exists
                    && from.contains(TRANSFER_TEMP_PREFIX)
                {
                    return Err(anyhow!("injected replacement install failure"));
                }
                if state.fail_rollback && from.contains(TRANSFER_BACKUP_PREFIX) {
                    return Err(anyhow!("injected replacement rollback failure"));
                }
                if state.entries.contains_key(to) {
                    return Err(anyhow!("destination already exists"));
                }

                let entry = state
                    .entries
                    .remove(from)
                    .ok_or_else(|| anyhow!("source does not exist"))?;
                state.entries.insert(to.into(), entry);
                Ok(())
            })
        }

        fn destination_kind<'a>(
            &'a self,
            path: &'a str,
        ) -> RemoteOperation<'a, RemoteDestinationKind> {
            Box::pin(async move {
                let mut state = self.state.lock().expect("fake remote lock");
                state
                    .operations
                    .push(FakeRemoteOperation::Inspect(path.into()));
                Ok(state
                    .entries
                    .get(path)
                    .map(|entry| entry.kind)
                    .unwrap_or(RemoteDestinationKind::Missing))
            })
        }

        fn remove_remote_file<'a>(&'a self, path: &'a str) -> RemoteOperation<'a, ()> {
            Box::pin(async move {
                let mut state = self.state.lock().expect("fake remote lock");
                state
                    .operations
                    .push(FakeRemoteOperation::Remove(path.into()));
                if state.fail_backup_cleanup && path.contains(TRANSFER_BACKUP_PREFIX) {
                    return Err(anyhow!("injected backup cleanup failure"));
                }
                state
                    .entries
                    .remove(path)
                    .map(|_| ())
                    .ok_or_else(|| anyhow!("file does not exist"))
            })
        }
    }

    #[cfg(unix)]
    fn create_file_symlink(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).expect("create file symlink");
        true
    }

    #[cfg(unix)]
    fn create_directory_symlink(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).expect("create directory symlink");
        true
    }

    #[cfg(windows)]
    fn windows_symlink_created(result: std::io::Result<()>, kind: &str) -> bool {
        match result {
            Ok(()) => true,
            Err(error) if error.raw_os_error() == Some(1314) => {
                eprintln!("skipping {kind} symlink test without symlink privilege");
                false
            }
            Err(error) => panic!("create {kind} symlink: {error}"),
        }
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) -> bool {
        windows_symlink_created(std::os::windows::fs::symlink_file(target, link), "file")
    }

    #[cfg(windows)]
    fn create_directory_symlink(target: &Path, link: &Path) -> bool {
        windows_symlink_created(std::os::windows::fs::symlink_dir(target, link), "directory")
    }

    #[test]
    fn new_control_starts_active() {
        rt().block_on(async {
            let control = TransferControl::new();
            let state = timeout(Duration::from_millis(50), control.wait_until_active())
                .await
                .expect("active control should not block");
            assert!(matches!(state, TransferControlState::Active));
        });
    }

    #[test]
    fn pause_then_resume_round_trip() {
        let control = TransferControl::new();
        assert!(control.pause(), "first pause transitions to paused");
        assert!(!control.pause(), "second pause is a no-op");
        assert!(control.resume(), "resume from paused returns true");
        assert!(!control.resume(), "second resume is a no-op");
    }

    #[test]
    fn cancel_blocks_pause_and_resume() {
        let control = TransferControl::new();
        control.cancel();
        assert!(!control.pause(), "pause must fail after cancel");
        assert!(!control.resume(), "resume must fail after cancel");
    }

    #[test]
    fn resume_retains_wakeup_until_waiter_registers() {
        rt().block_on(async {
            let control = TransferControl::new();
            assert!(control.pause());
            assert!(
                control.paused.load(Ordering::Relaxed),
                "simulate the worker observing the paused state"
            );

            assert!(control.resume());
            timeout(Duration::from_millis(50), control.notify.notified())
                .await
                .expect("resume wakeup must be retained for a late waiter");
        });
    }

    #[test]
    fn cancel_retains_wakeup_until_waiter_registers() {
        rt().block_on(async {
            let control = TransferControl::new();
            assert!(control.pause());
            assert!(
                control.paused.load(Ordering::Relaxed),
                "simulate the worker observing the paused state"
            );

            control.cancel();
            timeout(Duration::from_millis(50), control.notify.notified())
                .await
                .expect("cancel wakeup must be retained for a late waiter");
        });
    }

    #[test]
    fn wait_until_active_blocks_while_paused_and_unblocks_on_resume() {
        rt().block_on(async {
            let control = Arc::new(TransferControl::new());
            assert!(control.pause());

            let waiter_control = control.clone();
            let waiter = tokio::spawn(async move { waiter_control.wait_until_active().await });

            tokio::time::sleep(Duration::from_millis(20)).await;
            assert!(
                !waiter.is_finished(),
                "waiter must remain blocked while paused"
            );

            assert!(control.resume());
            let state = timeout(Duration::from_millis(100), waiter)
                .await
                .expect("waiter must complete after resume")
                .expect("waiter task panicked");
            assert!(matches!(state, TransferControlState::Active));
        });
    }

    #[test]
    fn wait_until_active_returns_cancelled_when_cancelled_during_wait() {
        rt().block_on(async {
            let control = Arc::new(TransferControl::new());
            assert!(control.pause());

            let waiter_control = control.clone();
            let waiter = tokio::spawn(async move { waiter_control.wait_until_active().await });

            tokio::time::sleep(Duration::from_millis(20)).await;
            control.cancel();

            let state = timeout(Duration::from_millis(100), waiter)
                .await
                .expect("waiter must complete after cancel")
                .expect("waiter task panicked");
            assert!(matches!(state, TransferControlState::Cancelled));
        });
    }

    #[test]
    fn completed_transfer_cleanup_removes_only_requested_transfer() {
        rt().block_on(async {
            let mut tasks: HashMap<TransferId, JoinHandle<()>> = HashMap::new();
            let mut controls: HashMap<TransferId, Arc<TransferControl>> = HashMap::new();

            let finished_id = TransferId(1);
            let finished_handle = tokio::spawn(async {});
            finished_handle
                .await
                .expect("finished task should not panic");
            tasks.insert(finished_id, tokio::spawn(async {}));
            controls.insert(finished_id, Arc::new(TransferControl::new()));

            tokio::time::sleep(Duration::from_millis(10)).await;

            let pending_id = TransferId(2);
            let pending_control = Arc::new(TransferControl::new());
            let pending_handle = tokio::spawn(async {
                tokio::time::sleep(Duration::from_secs(60)).await;
            });
            tasks.insert(pending_id, pending_handle);
            controls.insert(pending_id, pending_control.clone());

            remove_completed_transfer(finished_id, &mut tasks, &mut controls);

            assert!(!tasks.contains_key(&finished_id));
            assert!(!controls.contains_key(&finished_id));
            assert!(tasks.contains_key(&pending_id));
            assert!(controls.contains_key(&pending_id));

            pending_control.cancel();
            if let Some(handle) = tasks.remove(&pending_id) {
                handle.abort();
            }
        });
    }

    #[test]
    fn queued_transfer_can_be_cancelled_before_acquiring_a_slot() {
        rt().block_on(async {
            let semaphore = Arc::new(Semaphore::new(1));
            let _occupied = semaphore
                .clone()
                .acquire_owned()
                .await
                .expect("acquire occupied slot");
            let control = Arc::new(TransferControl::new());
            let waiter_control = control.clone();
            let waiter_semaphore = semaphore.clone();
            let waiter = tokio::spawn(async move {
                let mut active_permit = ActiveTransferPermit::new(waiter_semaphore);
                active_permit.wait_until_active(&waiter_control).await
            });

            tokio::task::yield_now().await;
            control.cancel();
            let result = timeout(Duration::from_millis(100), waiter)
                .await
                .expect("cancelled queued transfer must wake")
                .expect("slot waiter must not panic")
                .expect("slot acquisition must not fail");
            assert!(matches!(result, TransferControlState::Cancelled));
        });
    }

    #[test]
    fn paused_transfer_releases_and_reacquires_its_slot() {
        rt().block_on(async {
            let semaphore = Arc::new(Semaphore::new(1));
            let control = Arc::new(TransferControl::new());
            let mut active_permit = ActiveTransferPermit::new(semaphore.clone());
            assert!(matches!(
                active_permit
                    .wait_until_active(&control)
                    .await
                    .expect("acquire initial slot"),
                TransferControlState::Active
            ));
            assert_eq!(semaphore.available_permits(), 0);

            assert!(control.pause());
            let waiter_control = control.clone();
            let waiter = tokio::spawn(async move {
                let state = active_permit.wait_until_active(&waiter_control).await;
                (state, active_permit)
            });

            let competing_permit = timeout(
                Duration::from_millis(100),
                semaphore.clone().acquire_owned(),
            )
            .await
            .expect("paused transfer must release its slot")
            .expect("semaphore must remain open");

            assert!(control.resume());
            tokio::task::yield_now().await;
            assert!(
                !waiter.is_finished(),
                "resumed transfer must wait behind the current slot owner"
            );

            drop(competing_permit);
            let (state, _active_permit) = timeout(Duration::from_millis(100), waiter)
                .await
                .expect("resumed transfer must reacquire a released slot")
                .expect("slot waiter must not panic");
            assert!(matches!(
                state.expect("slot reacquisition must not fail"),
                TransferControlState::Active
            ));
        });
    }

    #[test]
    fn cancel_all_transfers_cancels_controls_and_clears_maps() {
        rt().block_on(async {
            let mut tasks: HashMap<TransferId, JoinHandle<()>> = HashMap::new();
            let mut controls: HashMap<TransferId, Arc<TransferControl>> = HashMap::new();

            let id = TransferId(7);
            let control = Arc::new(TransferControl::new());
            assert!(control.pause());
            let task_control = control.clone();
            let handle = tokio::spawn(async move {
                assert!(matches!(
                    task_control.wait_until_active().await,
                    TransferControlState::Cancelled
                ));
            });
            tasks.insert(id, handle);
            controls.insert(id, control.clone());

            cancel_all_transfers(&mut tasks, &mut controls).await;

            assert!(tasks.is_empty());
            assert!(controls.is_empty());
            assert!(
                control.cancelled.load(Ordering::Relaxed),
                "underlying control was cancelled",
            );
        });
    }

    #[test]
    fn cancel_all_transfers_aborts_tasks_after_timeout() {
        struct DropFlag(Arc<AtomicBool>);

        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Relaxed);
            }
        }

        rt().block_on(async {
            let mut tasks: HashMap<TransferId, JoinHandle<()>> = HashMap::new();
            let mut controls: HashMap<TransferId, Arc<TransferControl>> = HashMap::new();

            let id = TransferId(8);
            let control = Arc::new(TransferControl::new());
            let dropped = Arc::new(AtomicBool::new(false));
            let task_dropped = dropped.clone();
            let handle = tokio::spawn(async move {
                let _drop_flag = DropFlag(task_dropped);
                tokio::time::sleep(Duration::from_secs(60)).await;
            });
            tasks.insert(id, handle);
            controls.insert(id, control.clone());

            tokio::task::yield_now().await;
            cancel_all_transfers_with_timeout(&mut tasks, &mut controls, Duration::from_millis(20))
                .await;

            assert!(tasks.is_empty());
            assert!(controls.is_empty());
            assert!(control.cancelled.load(Ordering::Relaxed));
            assert!(
                dropped.load(Ordering::Relaxed),
                "aborted task should be dropped before cancellation returns"
            );
        });
    }

    #[test]
    fn compute_local_directory_size_counts_regular_files() {
        rt().block_on(async {
            let directory = tempfile::tempdir().expect("create test directory");
            let nested = directory.path().join("nested");
            std::fs::create_dir(&nested).expect("create nested directory");
            std::fs::write(directory.path().join("first.txt"), b"abc").expect("write first file");
            std::fs::write(nested.join("second.txt"), b"defgh").expect("write second file");

            let size = compute_local_directory_size(directory.path())
                .await
                .expect("compute directory size");
            assert_eq!(size, 8);
        });
    }

    #[test]
    fn transfer_progress_is_latest_value_while_child_lifecycle_is_reliable() {
        rt().block_on(async {
            let (sender, receiver) = crate::session::sftp_event_channel();
            let (progress_sender, mut progress_receiver) = crate::session::sftp_progress_channel();
            {
                let mut progress = TransferProgress {
                    event_sender: &sender,
                    progress_sender: &progress_sender,
                    transfer_id: TransferId(42),
                    bytes_total: Some(3),
                    bytes_complete: 0,
                    active_child: None,
                    next_child_id: 0,
                };
                progress
                    .emit(None)
                    .await
                    .expect("emit initial parent progress");
                progress
                    .begin_child("data.txt".to_string(), Some(3))
                    .await
                    .expect("start first child");
                progress.advance(3).await.expect("advance first child");
                progress
                    .finish_child(SftpTransferChildState::Done)
                    .await
                    .expect("finish first child");
                progress
                    .begin_child("empty.txt".to_string(), Some(0))
                    .await
                    .expect("start zero-byte child");
                progress
                    .finish_child(SftpTransferChildState::Done)
                    .await
                    .expect("finish zero-byte child");
            }
            drop(sender);
            let events = receiver.collect::<Vec<_>>().await;
            let latest = progress_receiver.recv().await.expect("latest progress");

            assert_eq!(events.len(), 4);
            assert!(matches!(
                &events[0],
                SftpEvent::TransferChildStarted {
                    transfer_id,
                    child: SftpTransferChild {
                        child_id: TransferChildId(0),
                        relative_path,
                        bytes_total: Some(3),
                    },
                } if *transfer_id == TransferId(42) && relative_path == "data.txt"
            ));
            assert!(matches!(
                latest,
                SftpTransferProgress {
                    transfer_id,
                    bytes_complete: 3,
                    bytes_total: Some(3),
                    child: Some(SftpTransferChildUpdate {
                        child_id: TransferChildId(0),
                        bytes_complete: 3,
                        state: SftpTransferChildState::Running,
                    }),
                } if transfer_id == TransferId(42)
            ));
            assert!(matches!(
                &events[3],
                SftpEvent::TransferChildFinished {
                    transfer_id: TransferId(42),
                    child: SftpTransferChildUpdate {
                        child_id: TransferChildId(1),
                        bytes_complete: 0,
                        state: SftpTransferChildState::Done,
                    },
                }
            ));
        });
    }

    #[test]
    fn transfer_terminal_event_is_preceded_by_final_progress_snapshot() {
        rt().block_on(async {
            let (event_sender, event_receiver) = crate::session::sftp_event_channel();
            let (progress_sender, mut progress_receiver) = crate::session::sftp_progress_channel();
            progress_sender.send(SftpTransferProgress {
                transfer_id: TransferId(9),
                bytes_complete: 7,
                bytes_total: Some(7),
                child: None,
            });
            let sampled = progress_receiver
                .recv()
                .await
                .expect("UI may sample progress before transfer completion");
            assert_eq!(sampled.bytes_complete, 7);

            finish_transfer_task(
                &event_sender,
                &progress_sender,
                TransferId(9),
                Ok(TransferOutcome::Done),
            )
            .await;
            drop(event_sender);
            let events = event_receiver.collect::<Vec<_>>().await;

            assert!(matches!(
                &events[..],
                [
                    SftpEvent::TransferProgressFinal(SftpTransferProgress {
                        transfer_id: TransferId(9),
                        bytes_complete: 7,
                        bytes_total: Some(7),
                        ..
                    }),
                    SftpEvent::TransferDone {
                        transfer_id: TransferId(9)
                    }
                ]
            ));
        });
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn inspect_local_upload_source_rejects_directory_symlink() {
        rt().block_on(async {
            let directory = tempfile::tempdir().expect("create test directory");
            let target = directory.path().join("target");
            let selected = directory.path().join("selected");
            std::fs::create_dir(&target).expect("create symlink target");
            if !create_directory_symlink(&target, &selected) {
                return;
            }

            let error = inspect_local_upload_source(&selected)
                .await
                .expect_err("top-level directory symlink must be rejected");
            let message = format!("{error:#}");
            assert!(message.contains("symbolic link"));
            assert!(message.contains(selected.to_string_lossy().as_ref()));
        });
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn compute_local_directory_size_rejects_file_symlink() {
        rt().block_on(async {
            let directory = tempfile::tempdir().expect("create test directory");
            let selected = directory.path().join("selected");
            let outside = directory.path().join("secret.txt");
            let link = selected.join("secret-link.txt");
            std::fs::create_dir(&selected).expect("create selected directory");
            std::fs::write(&outside, b"secret").expect("write outside file");
            if !create_file_symlink(&outside, &link) {
                return;
            }

            let error = timeout(
                Duration::from_secs(1),
                compute_local_directory_size(&selected),
            )
            .await
            .expect("symlink inspection must not hang")
            .expect_err("file symlink must be rejected");
            let message = format!("{error:#}");
            assert!(message.contains("symbolic link"));
            assert!(message.contains(link.to_string_lossy().as_ref()));
        });
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn compute_local_directory_size_rejects_parent_symlink_cycle() {
        rt().block_on(async {
            let directory = tempfile::tempdir().expect("create test directory");
            let selected = directory.path().join("selected");
            let child = selected.join("child");
            let back = child.join("back");
            std::fs::create_dir_all(&child).expect("create nested directory");
            if !create_directory_symlink(&selected, &back) {
                return;
            }

            let error = timeout(
                Duration::from_secs(1),
                compute_local_directory_size(&selected),
            )
            .await
            .expect("symlink cycle inspection must terminate")
            .expect_err("parent symlink cycle must be rejected");
            let message = format!("{error:#}");
            assert!(message.contains("symbolic link"));
            assert!(message.contains(back.to_string_lossy().as_ref()));
        });
    }

    #[test]
    fn remote_temporary_paths_stay_beside_the_destination_and_are_unique() {
        let first = remote_temporary_path("/srv/data/file.txt", TransferId(11));
        let second = remote_temporary_path("/srv/data/file.txt", TransferId(11));

        assert!(first.starts_with("/srv/data/.miaominal-transfer-"));
        assert!(first.ends_with(TRANSFER_TEMP_SUFFIX));
        assert_ne!(first, second);

        let relative = remote_temporary_path("file.txt", TransferId(12));
        assert!(relative.starts_with(TRANSFER_TEMP_PREFIX));
        assert!(!relative.contains('/'));
    }

    #[test]
    fn remote_backup_paths_stay_beside_the_destination_and_are_unique() {
        let first = remote_backup_path("/srv/data/file.txt", TransferId(13));
        let second = remote_backup_path("/srv/data/file.txt", TransferId(13));

        assert!(first.starts_with("/srv/data/.miaominal-backup-"));
        assert!(first.ends_with(TRANSFER_BACKUP_SUFFIX));
        assert_ne!(first, second);

        let relative = remote_backup_path("file.txt", TransferId(14));
        assert!(relative.starts_with(TRANSFER_BACKUP_PREFIX));
        assert!(!relative.contains('/'));
    }

    #[test]
    fn remote_upload_install_uses_direct_rename_when_destination_is_absent() {
        rt().block_on(async {
            let remote = FakeRemoteReplaceOps::with_entries([(
                "/remote/.miaominal-transfer-new.part",
                RemoteDestinationKind::File,
                "new",
            )]);

            replace_remote_upload(
                &remote,
                "/remote/.miaominal-transfer-new.part",
                "/remote/file.txt",
                TransferId(20),
            )
            .await
            .expect("install new remote file");

            assert_eq!(
                remote.entry("/remote/file.txt").map(|entry| entry.contents),
                Some("new".to_string())
            );
            assert_eq!(
                remote.operations(),
                vec![FakeRemoteOperation::Rename(
                    "/remote/.miaominal-transfer-new.part".into(),
                    "/remote/file.txt".into(),
                )]
            );
        });
    }

    #[test]
    fn remote_upload_replaces_existing_file_through_backup_swap() {
        rt().block_on(async {
            let remote = FakeRemoteReplaceOps::with_entries([
                (
                    "/remote/.miaominal-transfer-new.part",
                    RemoteDestinationKind::File,
                    "new",
                ),
                (
                    "/remote/file.txt",
                    RemoteDestinationKind::File,
                    "old",
                ),
            ]);

            replace_remote_upload(
                &remote,
                "/remote/.miaominal-transfer-new.part",
                "/remote/file.txt",
                TransferId(21),
            )
            .await
            .expect("replace existing remote file");

            assert_eq!(
                remote.entry("/remote/file.txt").map(|entry| entry.contents),
                Some("new".to_string())
            );
            assert!(remote.backup_entries().is_empty());
            assert!(
                remote
                    .operations()
                    .iter()
                    .any(|operation| matches!(operation, FakeRemoteOperation::Remove(path) if path.contains(TRANSFER_BACKUP_PREFIX)))
            );
        });
    }

    #[test]
    fn remote_upload_restores_original_when_replacement_install_fails() {
        rt().block_on(async {
            let remote = FakeRemoteReplaceOps::with_entries([
                (
                    "/remote/.miaominal-transfer-new.part",
                    RemoteDestinationKind::File,
                    "new",
                ),
                ("/remote/file.txt", RemoteDestinationKind::File, "old"),
            ]);
            remote.update(|state| state.fail_install_after_backup = true);

            let error = replace_remote_upload(
                &remote,
                "/remote/.miaominal-transfer-new.part",
                "/remote/file.txt",
                TransferId(22),
            )
            .await
            .expect_err("replacement install should fail");

            assert!(format!("{error:#}").contains("restored the original"));
            assert_eq!(
                remote.entry("/remote/file.txt").map(|entry| entry.contents),
                Some("old".to_string())
            );
            assert_eq!(
                remote
                    .entry("/remote/.miaominal-transfer-new.part")
                    .map(|entry| entry.contents),
                Some("new".to_string())
            );
            assert!(remote.backup_entries().is_empty());
        });
    }

    #[test]
    fn remote_upload_reports_backup_path_when_rollback_fails() {
        rt().block_on(async {
            let remote = FakeRemoteReplaceOps::with_entries([
                (
                    "/remote/.miaominal-transfer-new.part",
                    RemoteDestinationKind::File,
                    "new",
                ),
                ("/remote/file.txt", RemoteDestinationKind::File, "old"),
            ]);
            remote.update(|state| {
                state.fail_install_after_backup = true;
                state.fail_rollback = true;
            });

            let error = replace_remote_upload(
                &remote,
                "/remote/.miaominal-transfer-new.part",
                "/remote/file.txt",
                TransferId(23),
            )
            .await
            .expect_err("replacement rollback should fail");
            let message = format!("{error:#}");

            assert!(message.contains("the original remains at"));
            assert!(message.contains(TRANSFER_BACKUP_PREFIX));
            assert!(remote.entry("/remote/file.txt").is_none());
            let backups = remote.backup_entries();
            assert_eq!(backups.len(), 1);
            assert_eq!(backups[0].1.contents, "old");
        });
    }

    #[test]
    fn remote_upload_rejects_replacing_directory_or_special_entry() {
        rt().block_on(async {
            for (kind, expected_message) in [
                (RemoteDestinationKind::Directory, "remote directory"),
                (RemoteDestinationKind::Other, "special remote entry"),
            ] {
                let remote = FakeRemoteReplaceOps::with_entries([
                    (
                        "/remote/.miaominal-transfer-new.part",
                        RemoteDestinationKind::File,
                        "new",
                    ),
                    ("/remote/existing", kind, "old"),
                ]);

                let error = replace_remote_upload(
                    &remote,
                    "/remote/.miaominal-transfer-new.part",
                    "/remote/existing",
                    TransferId(24),
                )
                .await
                .expect_err("unsupported destination replacement must be rejected");

                assert!(format!("{error:#}").contains(expected_message));
                assert_eq!(
                    remote.entry("/remote/existing").map(|entry| entry.kind),
                    Some(kind)
                );
                assert!(remote.backup_entries().is_empty());
            }
        });
    }

    #[test]
    fn remote_upload_succeeds_when_backup_cleanup_fails() {
        rt().block_on(async {
            let remote = FakeRemoteReplaceOps::with_entries([
                (
                    "/remote/.miaominal-transfer-new.part",
                    RemoteDestinationKind::File,
                    "new",
                ),
                (
                    "/remote/file.txt",
                    RemoteDestinationKind::Symlink,
                    "old-link",
                ),
            ]);
            remote.update(|state| state.fail_backup_cleanup = true);

            replace_remote_upload(
                &remote,
                "/remote/.miaominal-transfer-new.part",
                "/remote/file.txt",
                TransferId(25),
            )
            .await
            .expect("backup cleanup failure must not fail completed upload");

            assert_eq!(
                remote.entry("/remote/file.txt").map(|entry| entry.contents),
                Some("new".to_string())
            );
            let backups = remote.backup_entries();
            assert_eq!(backups.len(), 1);
            assert_eq!(backups[0].1.contents, "old-link");
        });
    }

    #[test]
    fn local_temporary_download_replaces_destination_only_when_persisted() {
        rt().block_on(async {
            let directory = tempfile::tempdir().expect("create test directory");
            let destination = directory.path().join("download.txt");
            std::fs::write(&destination, b"original").expect("write original file");

            let mut temporary_file =
                create_local_temporary_file(&destination).expect("create temporary file");
            temporary_file
                .file
                .write_all(b"replacement")
                .await
                .expect("write replacement");
            temporary_file
                .file
                .sync_all()
                .await
                .expect("sync replacement");

            assert_eq!(
                std::fs::read(&destination).expect("read original before persist"),
                b"original"
            );

            let LocalTemporaryFile { file, path } = temporary_file;
            drop(file);
            persist_local_temporary_file(path, &destination).expect("persist replacement");
            assert_eq!(
                std::fs::read(&destination).expect("read replacement"),
                b"replacement"
            );
        });
    }

    #[test]
    fn dropping_local_temporary_download_preserves_destination_and_cleans_up() {
        rt().block_on(async {
            let directory = tempfile::tempdir().expect("create test directory");
            let destination = directory.path().join("download.txt");
            std::fs::write(&destination, b"original").expect("write original file");

            let mut temporary_file =
                create_local_temporary_file(&destination).expect("create temporary file");
            let temporary_path_buf = temporary_file.path.to_path_buf();
            temporary_file
                .file
                .write_all(b"partial")
                .await
                .expect("write partial download");

            drop(temporary_file);

            assert_eq!(
                std::fs::read(&destination).expect("read preserved original"),
                b"original"
            );
            assert!(!temporary_path_buf.exists());
        });
    }
}
