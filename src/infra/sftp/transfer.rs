use super::paths::join_remote_path;
use super::session::SftpEvent;
use crate::domain::sftp::{TransferDirection, TransferId};
use anyhow::{Context, Result, anyhow};
use futures::channel::mpsc::UnboundedSender as FuturesUnboundedSender;
use russh_sftp::{client::SftpSession, protocol::OpenFlags};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::fs::File as TokioFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

const TRANSFER_CHUNK_SIZE: usize = 256 * 1024;

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

        !self.paused.swap(true, Ordering::Relaxed)
    }

    pub(super) fn resume(&self) -> bool {
        if self.cancelled.load(Ordering::Relaxed) {
            return false;
        }

        let was_paused = self.paused.swap(false, Ordering::Relaxed);
        if was_paused {
            self.notify.notify_waiters();
        }
        was_paused
    }

    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.notify.notify_waiters();
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

pub(super) fn cleanup_finished_transfers(
    transfer_tasks: &mut HashMap<TransferId, JoinHandle<()>>,
    transfer_controls: &mut HashMap<TransferId, Arc<TransferControl>>,
) {
    let finished_ids: Vec<_> = transfer_tasks
        .iter()
        .filter_map(|(transfer_id, handle)| handle.is_finished().then_some(*transfer_id))
        .collect();

    for transfer_id in finished_ids {
        transfer_tasks.remove(&transfer_id);
        transfer_controls.remove(&transfer_id);
    }
}

pub(super) fn cancel_all_transfers(
    transfer_tasks: &mut HashMap<TransferId, JoinHandle<()>>,
    transfer_controls: &mut HashMap<TransferId, Arc<TransferControl>>,
) {
    for control in transfer_controls.values() {
        control.cancel();
    }

    for handle in transfer_tasks.values() {
        handle.abort();
    }

    transfer_tasks.clear();
    transfer_controls.clear();
}

pub(super) fn spawn_upload_task(
    sftp: Arc<SftpSession>,
    event_sender: FuturesUnboundedSender<SftpEvent>,
    transfer_id: TransferId,
    local_path: PathBuf,
    remote_path: String,
    control: Arc<TransferControl>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let result = upload_path(
            &sftp,
            transfer_id,
            &local_path,
            &remote_path,
            &control,
            &event_sender,
        )
        .await;
        finish_transfer_task(&event_sender, transfer_id, result);
    })
}

pub(super) fn spawn_download_task(
    sftp: Arc<SftpSession>,
    event_sender: FuturesUnboundedSender<SftpEvent>,
    transfer_id: TransferId,
    remote_path: String,
    local_path: PathBuf,
    control: Arc<TransferControl>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let result = download_path(
            &sftp,
            transfer_id,
            &remote_path,
            &local_path,
            &control,
            &event_sender,
        )
        .await;
        finish_transfer_task(&event_sender, transfer_id, result);
    })
}

enum TransferOutcome {
    Done,
    Cancelled,
}

async fn upload_path(
    sftp: &SftpSession,
    transfer_id: TransferId,
    local_path: &Path,
    remote_path: &str,
    control: &TransferControl,
    event_sender: &FuturesUnboundedSender<SftpEvent>,
) -> Result<TransferOutcome> {
    let metadata = tokio::fs::metadata(local_path)
        .await
        .with_context(|| format!("failed to read metadata for {}", local_path.display()))?;

    let bytes_total = if metadata.is_dir() {
        Some(compute_local_directory_size(local_path).await?)
    } else {
        Some(metadata.len())
    };
    let mut bytes_complete = 0_u64;

    if metadata.is_dir() {
        upload_directory(
            sftp,
            transfer_id,
            local_path,
            remote_path,
            control,
            event_sender,
            bytes_total,
            &mut bytes_complete,
        )
        .await
    } else {
        upload_regular_file(
            sftp,
            transfer_id,
            local_path,
            remote_path,
            control,
            event_sender,
            bytes_total,
            &mut bytes_complete,
        )
        .await
    }
}

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
            let metadata = entry.metadata().await.with_context(|| {
                format!("failed to read metadata for {}", entry.path().display())
            })?;
            if metadata.is_dir() {
                stack.push(entry.path());
            } else {
                total += metadata.len();
            }
        }
    }
    Ok(total)
}

async fn upload_directory(
    sftp: &SftpSession,
    transfer_id: TransferId,
    local_root: &Path,
    remote_root: &str,
    control: &TransferControl,
    event_sender: &FuturesUnboundedSender<SftpEvent>,
    bytes_total: Option<u64>,
    bytes_complete: &mut u64,
) -> Result<TransferOutcome> {
    if matches!(
        control.wait_until_active().await,
        TransferControlState::Cancelled
    ) {
        return Ok(TransferOutcome::Cancelled);
    }

    let _ = sftp.create_dir(remote_root.to_string()).await;
    emit_transfer_progress(event_sender, transfer_id, *bytes_complete, bytes_total)?;

    let mut stack = vec![(local_root.to_path_buf(), remote_root.to_string())];

    while let Some((local_dir, remote_dir)) = stack.pop() {
        if matches!(
            control.wait_until_active().await,
            TransferControlState::Cancelled
        ) {
            return Ok(TransferOutcome::Cancelled);
        }

        let _ = sftp.create_dir(remote_dir.clone()).await;

        let mut entries = tokio::fs::read_dir(&local_dir)
            .await
            .with_context(|| format!("failed to read {}", local_dir.display()))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .with_context(|| format!("failed to iterate {}", local_dir.display()))?
        {
            let local_child = entry.path();
            let filename = entry.file_name().to_string_lossy().into_owned();
            let remote_child = join_remote_path(&remote_dir, &filename);
            let metadata = entry.metadata().await.with_context(|| {
                format!("failed to read metadata for {}", local_child.display())
            })?;
            if metadata.is_dir() {
                stack.push((local_child, remote_child));
                continue;
            }

            if matches!(
                upload_regular_file(
                    sftp,
                    transfer_id,
                    &local_child,
                    &remote_child,
                    control,
                    event_sender,
                    bytes_total,
                    bytes_complete,
                )
                .await?,
                TransferOutcome::Cancelled
            ) {
                return Ok(TransferOutcome::Cancelled);
            }
        }
    }

    Ok(TransferOutcome::Done)
}

async fn upload_regular_file(
    sftp: &SftpSession,
    transfer_id: TransferId,
    local_path: &Path,
    remote_path: &str,
    control: &TransferControl,
    event_sender: &FuturesUnboundedSender<SftpEvent>,
    bytes_total: Option<u64>,
    bytes_complete: &mut u64,
) -> Result<TransferOutcome> {
    let mut local_file = TokioFile::open(local_path)
        .await
        .with_context(|| format!("failed to open {} for upload", local_path.display()))?;
    let mut remote_file = sftp
        .open_with_flags(
            remote_path.to_string(),
            OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
        )
        .await
        .with_context(|| format!("failed to open remote file {remote_path} for upload"))?;

    let mut buffer = vec![0; TRANSFER_CHUNK_SIZE];

    loop {
        if matches!(
            control.wait_until_active().await,
            TransferControlState::Cancelled
        ) {
            let _ = remote_file.shutdown().await;
            return Ok(TransferOutcome::Cancelled);
        }

        let read = local_file
            .read(&mut buffer)
            .await
            .with_context(|| format!("failed to read {} while uploading", local_path.display()))?;
        if read == 0 {
            break;
        }

        remote_file
            .write_all(&buffer[..read])
            .await
            .with_context(|| format!("failed to write remote file {remote_path}"))?;

        *bytes_complete += read as u64;
        emit_transfer_progress(event_sender, transfer_id, *bytes_complete, bytes_total)?;
    }

    remote_file
        .shutdown()
        .await
        .with_context(|| format!("failed to finalize remote file {remote_path}"))?;
    Ok(TransferOutcome::Done)
}

async fn download_path(
    sftp: &SftpSession,
    transfer_id: TransferId,
    remote_path: &str,
    local_path: &Path,
    control: &TransferControl,
    event_sender: &FuturesUnboundedSender<SftpEvent>,
) -> Result<TransferOutcome> {
    let metadata = sftp
        .metadata(remote_path.to_string())
        .await
        .with_context(|| format!("failed to read remote metadata for {remote_path}"))?;
    let bytes_total = if metadata.is_dir() {
        Some(compute_remote_directory_size(sftp, remote_path).await?)
    } else {
        metadata.size
    };
    let mut bytes_complete = 0_u64;

    if metadata.is_dir() {
        download_directory(
            sftp,
            transfer_id,
            remote_path,
            local_path,
            control,
            event_sender,
            bytes_total,
            &mut bytes_complete,
        )
        .await
    } else {
        download_regular_file(
            sftp,
            transfer_id,
            remote_path,
            local_path,
            control,
            event_sender,
            bytes_total,
            &mut bytes_complete,
        )
        .await
    }
}

async fn compute_remote_directory_size(sftp: &SftpSession, remote_root: &str) -> Result<u64> {
    let mut total = 0_u64;
    let mut stack = vec![remote_root.to_string()];

    while let Some(remote_dir) = stack.pop() {
        for entry in sftp
            .read_dir(&remote_dir)
            .await
            .with_context(|| format!("failed to read remote directory {remote_dir}"))?
        {
            let metadata = entry.metadata();
            if metadata.is_dir() {
                stack.push(join_remote_path(&remote_dir, &entry.file_name()));
            } else {
                total += metadata.size.unwrap_or(0);
            }
        }
    }

    Ok(total)
}

async fn download_directory(
    sftp: &SftpSession,
    transfer_id: TransferId,
    remote_root: &str,
    local_root: &Path,
    control: &TransferControl,
    event_sender: &FuturesUnboundedSender<SftpEvent>,
    bytes_total: Option<u64>,
    bytes_complete: &mut u64,
) -> Result<TransferOutcome> {
    if matches!(
        control.wait_until_active().await,
        TransferControlState::Cancelled
    ) {
        return Ok(TransferOutcome::Cancelled);
    }

    tokio::fs::create_dir_all(local_root)
        .await
        .with_context(|| format!("failed to create {}", local_root.display()))?;
    emit_transfer_progress(event_sender, transfer_id, *bytes_complete, bytes_total)?;

    let mut stack = vec![(remote_root.to_string(), local_root.to_path_buf())];

    while let Some((remote_dir, local_dir)) = stack.pop() {
        if matches!(
            control.wait_until_active().await,
            TransferControlState::Cancelled
        ) {
            return Ok(TransferOutcome::Cancelled);
        }

        tokio::fs::create_dir_all(&local_dir)
            .await
            .with_context(|| format!("failed to create {}", local_dir.display()))?;

        for entry in sftp
            .read_dir(&remote_dir)
            .await
            .with_context(|| format!("failed to read remote directory {remote_dir}"))?
        {
            let filename = entry.file_name();
            let remote_child = join_remote_path(&remote_dir, &filename);
            let local_child = local_dir.join(&filename);

            if entry.metadata().is_dir() {
                stack.push((remote_child, local_child));
                continue;
            }

            if matches!(
                download_regular_file(
                    sftp,
                    transfer_id,
                    &remote_child,
                    &local_child,
                    control,
                    event_sender,
                    bytes_total,
                    bytes_complete,
                )
                .await?,
                TransferOutcome::Cancelled
            ) {
                return Ok(TransferOutcome::Cancelled);
            }
        }
    }

    Ok(TransferOutcome::Done)
}

async fn download_regular_file(
    sftp: &SftpSession,
    transfer_id: TransferId,
    remote_path: &str,
    local_path: &Path,
    control: &TransferControl,
    event_sender: &FuturesUnboundedSender<SftpEvent>,
    bytes_total: Option<u64>,
    bytes_complete: &mut u64,
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
    let mut local_file = TokioFile::create(local_path)
        .await
        .with_context(|| format!("failed to create {} for download", local_path.display()))?;

    let mut buffer = vec![0; TRANSFER_CHUNK_SIZE];
    loop {
        if matches!(
            control.wait_until_active().await,
            TransferControlState::Cancelled
        ) {
            let _ = local_file.shutdown().await;
            return Ok(TransferOutcome::Cancelled);
        }

        let read = remote_file
            .read(&mut buffer)
            .await
            .with_context(|| format!("failed to read remote file {remote_path}"))?;
        if read == 0 {
            break;
        }

        local_file
            .write_all(&buffer[..read])
            .await
            .with_context(|| {
                format!("failed to write {} while downloading", local_path.display())
            })?;

        *bytes_complete += read as u64;
        emit_transfer_progress(event_sender, transfer_id, *bytes_complete, bytes_total)?;
    }

    local_file
        .flush()
        .await
        .with_context(|| format!("failed to flush {} after download", local_path.display()))?;
    Ok(TransferOutcome::Done)
}

fn finish_transfer_task(
    event_sender: &FuturesUnboundedSender<SftpEvent>,
    transfer_id: TransferId,
    result: Result<TransferOutcome>,
) {
    match result {
        Ok(TransferOutcome::Done) => {
            let _ = emit_transfer_done(event_sender, transfer_id);
        }
        Ok(TransferOutcome::Cancelled) => {
            let _ = emit_transfer_cancelled(event_sender, transfer_id);
        }
        Err(error) => {
            let _ = emit_transfer_failed(event_sender, transfer_id, error.to_string());
        }
    }
}

pub(super) fn emit_transfer_queued(
    event_sender: &FuturesUnboundedSender<SftpEvent>,
    transfer_id: TransferId,
    direction: TransferDirection,
    source: PathBuf,
    destination: String,
) -> Result<()> {
    if event_sender
        .unbounded_send(SftpEvent::TransferQueued {
            transfer_id,
            direction,
            source,
            destination,
        })
        .is_err()
    {
        return Err(anyhow!("SFTP event receiver is closed"));
    }

    Ok(())
}

fn emit_transfer_progress(
    event_sender: &FuturesUnboundedSender<SftpEvent>,
    transfer_id: TransferId,
    bytes_complete: u64,
    bytes_total: Option<u64>,
) -> Result<()> {
    if event_sender
        .unbounded_send(SftpEvent::TransferProgress {
            transfer_id,
            bytes_complete,
            bytes_total,
        })
        .is_err()
    {
        return Err(anyhow!("SFTP event receiver is closed"));
    }

    Ok(())
}

fn emit_transfer_done(
    event_sender: &FuturesUnboundedSender<SftpEvent>,
    transfer_id: TransferId,
) -> Result<()> {
    if event_sender
        .unbounded_send(SftpEvent::TransferDone { transfer_id })
        .is_err()
    {
        return Err(anyhow!("SFTP event receiver is closed"));
    }

    Ok(())
}

pub(super) fn emit_transfer_paused(
    event_sender: &FuturesUnboundedSender<SftpEvent>,
    transfer_id: TransferId,
) -> Result<()> {
    if event_sender
        .unbounded_send(SftpEvent::TransferPaused { transfer_id })
        .is_err()
    {
        return Err(anyhow!("SFTP event receiver is closed"));
    }

    Ok(())
}

pub(super) fn emit_transfer_resumed(
    event_sender: &FuturesUnboundedSender<SftpEvent>,
    transfer_id: TransferId,
) -> Result<()> {
    if event_sender
        .unbounded_send(SftpEvent::TransferResumed { transfer_id })
        .is_err()
    {
        return Err(anyhow!("SFTP event receiver is closed"));
    }

    Ok(())
}

fn emit_transfer_cancelled(
    event_sender: &FuturesUnboundedSender<SftpEvent>,
    transfer_id: TransferId,
) -> Result<()> {
    if event_sender
        .unbounded_send(SftpEvent::TransferCancelled { transfer_id })
        .is_err()
    {
        return Err(anyhow!("SFTP event receiver is closed"));
    }

    Ok(())
}

fn emit_transfer_failed(
    event_sender: &FuturesUnboundedSender<SftpEvent>,
    transfer_id: TransferId,
    message: String,
) -> Result<()> {
    if event_sender
        .unbounded_send(SftpEvent::TransferFailed {
            transfer_id,
            message,
        })
        .is_err()
    {
        return Err(anyhow!("SFTP event receiver is closed"));
    }

    Ok(())
}

pub(super) fn emit_error(
    event_sender: &FuturesUnboundedSender<SftpEvent>,
    context: &str,
    message: String,
) -> Result<()> {
    if event_sender
        .unbounded_send(SftpEvent::Error {
            context: context.into(),
            message,
        })
        .is_err()
    {
        return Err(anyhow!("SFTP event receiver is closed"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("build current-thread runtime")
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
    fn cleanup_finished_transfers_drops_completed_handles() {
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

            cleanup_finished_transfers(&mut tasks, &mut controls);

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
    fn cancel_all_transfers_cancels_controls_and_clears_maps() {
        rt().block_on(async {
            let mut tasks: HashMap<TransferId, JoinHandle<()>> = HashMap::new();
            let mut controls: HashMap<TransferId, Arc<TransferControl>> = HashMap::new();

            let id = TransferId(7);
            let control = Arc::new(TransferControl::new());
            let task_control = control.clone();
            let handle = tokio::spawn(async move {
                let _ = task_control.wait_until_active().await;
                tokio::time::sleep(Duration::from_secs(60)).await;
            });
            tasks.insert(id, handle);
            controls.insert(id, control.clone());

            cancel_all_transfers(&mut tasks, &mut controls);

            assert!(tasks.is_empty());
            assert!(controls.is_empty());
            assert!(
                control.cancelled.load(Ordering::Relaxed),
                "underlying control was cancelled",
            );
        });
    }
}
