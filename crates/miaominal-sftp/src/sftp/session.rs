use super::paths::{
    canonical_remote_path, emit_directory_listing, emit_subdirectory_listing,
    list_directory_entries,
};
use super::transfer::{
    TransferControl, cancel_all_transfers, cleanup_finished_transfers, emit_error,
    emit_error_with_path, emit_transfer_paused, emit_transfer_queued, emit_transfer_resumed,
    spawn_download_task, spawn_upload_task,
};
use anyhow::{Context, Result, anyhow, bail};
use futures::SinkExt as _;
use futures::channel::mpsc::{
    Receiver as FuturesReceiver, Sender as FuturesSender, channel as futures_channel,
};
use miaominal_core::known_host::HostKeyCheck;
use miaominal_core::profile::SessionProfile;
use miaominal_core::sftp::{SftpEntry, TransferDirection, TransferId};
use miaominal_secrets::SecretStore;
use miaominal_ssh as ssh;
use miaominal_storage::KnownHostsStore;
use russh::{Disconnect, client};
use russh_sftp::{
    client::{SftpSession, error::Error as SftpClientError},
    protocol::{FileType, StatusCode},
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::Mutex;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

const SFTP_EVENT_CHANNEL_CAPACITY: usize = 256;

#[derive(Debug)]
enum SftpCommand {
    ListDirectory {
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
    RemoveDirectory {
        path: String,
    },
    Rename {
        from: String,
        to: String,
    },
    Upload {
        transfer_id: TransferId,
        local_path: PathBuf,
        remote_path: String,
    },
    Download {
        transfer_id: TransferId,
        remote_path: String,
        local_path: PathBuf,
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

#[derive(Debug, Clone)]
pub enum SftpEvent {
    Status(String),
    DirectoryListing {
        path: String,
        entries: Vec<SftpEntry>,
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
    TransferProgress {
        transfer_id: TransferId,
        bytes_complete: u64,
        bytes_total: Option<u64>,
        child: Option<SftpTransferChildUpdate>,
    },
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

#[derive(Clone, Debug)]
pub struct SftpCommandSender {
    sender: UnboundedSender<SftpCommand>,
    next_transfer_id: Arc<AtomicU64>,
}

impl SftpCommandSender {
    pub fn list_directory(&self, path: impl Into<String>) -> Result<()> {
        self.send_command(SftpCommand::ListDirectory { path: path.into() })
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
        self.send_command(SftpCommand::RemoveDirectory { path: path.into() })
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
        let transfer_id = self.next_transfer_id();
        self.send_command(SftpCommand::Upload {
            transfer_id,
            local_path,
            remote_path: remote_path.into(),
        })?;
        Ok(transfer_id)
    }

    pub fn queue_download(
        &self,
        remote_path: impl Into<String>,
        local_path: PathBuf,
    ) -> Result<TransferId> {
        let transfer_id = self.next_transfer_id();
        self.send_command(SftpCommand::Download {
            transfer_id,
            remote_path: remote_path.into(),
            local_path,
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
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    paths: Vec<String>,
) -> Result<Vec<ResolvedRemotePath>> {
    let (event_sender, _event_receiver) = sftp_event_channel();
    let connected_session =
        connect_authenticated_session(profile, all_profiles, secrets, known_hosts, &event_sender)
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
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
) -> SftpConnection {
    let (event_sender, event_receiver) = sftp_event_channel();
    let (command_sender, command_receiver) = unbounded_channel();
    let runtime = runtime.clone();
    let next_transfer_id = Arc::new(AtomicU64::new(1));

    std::thread::Builder::new()
        .name(format!("sftp-session-{}", profile.id))
        .spawn(move || {
            let result = runtime.block_on(run_session(
                profile,
                all_profiles,
                secrets,
                known_hosts,
                command_receiver,
                event_sender.clone(),
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
            next_transfer_id,
        },
        events: event_receiver,
    }
}

async fn run_session(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    mut command_receiver: UnboundedReceiver<SftpCommand>,
    event_sender: SftpEventSender,
) -> Result<()> {
    let remote = format!("{}@{}:{}", profile.username, profile.host, profile.port);
    let connected_session = connect_authenticated_session(
        profile.clone(),
        all_profiles,
        secrets,
        known_hosts,
        &event_sender,
    )
    .await?;

    let session_result: Result<()> = async {
        let sftp = Arc::new(open_sftp_session(&connected_session).await?);

        let mut transfer_controls = HashMap::new();
        let mut transfer_tasks = HashMap::new();

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
                recover_operation_result(&event_sender, "list_directory", Some("."), initial_result)
                    .await?
            {
                emit_directory_listing(&event_sender, initial_path, entries).await?;
            }

            while let Some(command) = command_receiver.recv().await {
                cleanup_finished_transfers(&mut transfer_tasks, &mut transfer_controls);

                match command {
                    SftpCommand::ListDirectory { path } => {
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
                            "list_directory",
                            Some(&path),
                            result,
                        )
                        .await?
                        {
                            emit_directory_listing(&event_sender, canonical_path, entries).await?;
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
                            "list_subdirectory",
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
                        if recover_operation_result(&event_sender, "create_directory", None, result)
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
                        if recover_operation_result(&event_sender, "remove_file", None, result)
                            .await?
                            .is_some()
                        {
                            emit_status(&event_sender, format!("Removed remote file {path}"))
                                .await?;
                        }
                    }
                    SftpCommand::RemoveDirectory { path } => {
                        let result = sftp
                            .remove_dir(path.clone())
                            .await
                            .with_context(|| format!("failed to remove remote directory {path}"));
                        if recover_operation_result(&event_sender, "remove_directory", None, result)
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
                        if recover_operation_result(&event_sender, "rename", None, result)
                            .await?
                            .is_some()
                        {
                            emit_status(&event_sender, format!("Renamed {from} to {to}")).await?;
                        }
                    }
                    SftpCommand::Upload {
                        transfer_id,
                        local_path,
                        remote_path,
                    } => {
                        emit_transfer_queued(
                            &event_sender,
                            transfer_id,
                            TransferDirection::Upload,
                            local_path.clone(),
                            remote_path.clone(),
                        )
                        .await?;

                        let control = Arc::new(TransferControl::new());
                        let task = spawn_upload_task(
                            sftp.clone(),
                            event_sender.clone(),
                            transfer_id,
                            local_path,
                            remote_path,
                            control.clone(),
                        );
                        transfer_controls.insert(transfer_id, control);
                        transfer_tasks.insert(transfer_id, task);
                    }
                    SftpCommand::Download {
                        transfer_id,
                        remote_path,
                        local_path,
                    } => {
                        emit_transfer_queued(
                            &event_sender,
                            transfer_id,
                            TransferDirection::Download,
                            local_path.clone(),
                            remote_path.clone(),
                        )
                        .await?;

                        let control = Arc::new(TransferControl::new());
                        let task = spawn_download_task(
                            sftp.clone(),
                            event_sender.clone(),
                            transfer_id,
                            remote_path,
                            local_path,
                            control.clone(),
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
    .await;

    connected_session.disconnect().await;

    session_result
}

async fn recover_operation_result<T>(
    event_sender: &SftpEventSender,
    context: &str,
    path: Option<&str>,
    result: Result<T>,
) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if is_recoverable_operation_error(&error) => {
            emit_error_with_path(
                event_sender,
                context,
                path.map(str::to_owned),
                format!("{error:#}"),
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
}

impl client::Handler for SftpClientHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        match self
            .known_hosts
            .check(&self.host, self.port, server_public_key)?
        {
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
        }
    }
}

async fn connect_authenticated_session(
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    event_sender: &SftpEventSender,
) -> Result<SftpConnectedSession> {
    let profile = ssh::hydrate_profile_from_secrets(profile, &secrets);
    let remote = format!("{}@{}:{}", profile.username, profile.host, profile.port);
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

        let mut current_session =
            connect_profile_session(first_hop, config.clone(), known_hosts.clone()).await?;
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
        let mut session = connect_profile_session(&profile, config, known_hosts).await?;
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
) -> Result<client::Handle<SftpClientHandler>> {
    let handler = SftpClientHandler {
        known_hosts,
        host: profile.host.clone(),
        port: profile.port,
    };
    ssh::connection::connect_profile_session(profile, config, handler).await
}

async fn connect_profile_stream<R>(
    profile: &SessionProfile,
    transport: R,
    config: Arc<client::Config>,
    known_hosts: KnownHostsStore,
) -> Result<client::Handle<SftpClientHandler>>
where
    R: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let handler = SftpClientHandler {
        known_hosts,
        host: profile.host.clone(),
        port: profile.port,
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
    use russh_sftp::protocol::Status;
    use std::time::Duration;

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
    fn subdirectory_failure_emits_requested_path_and_is_recoverable() {
        let (event_sender, mut event_receiver) = sftp_event_channel();
        let operation_result: Result<()> = Err(status_error(
            StatusCode::PermissionDenied,
            "permission denied",
        ))
        .context("failed to read remote directory /protected");

        let recovered = futures::executor::block_on(recover_operation_result(
            &event_sender,
            "list_subdirectory",
            Some("/protected"),
            operation_result,
        ))
        .expect("operation failure should be reported without becoming fatal");

        assert!(recovered.is_none());
        let event = futures::executor::block_on(event_receiver.next())
            .expect("operation failure should emit an event");
        match event {
            SftpEvent::Error {
                context,
                path,
                message,
            } => {
                assert_eq!(context, "list_subdirectory");
                assert_eq!(path.as_deref(), Some("/protected"));
                assert_eq!(
                    message,
                    "failed to read remote directory /protected: Permission denied: permission denied"
                );
            }
            other => panic!("expected operation error event, got {other:?}"),
        }
    }

    #[test]
    fn operation_failure_is_fatal_when_error_event_cannot_be_delivered() {
        let (event_sender, event_receiver) = sftp_event_channel();
        drop(event_receiver);

        let error = futures::executor::block_on(recover_operation_result::<()>(
            &event_sender,
            "list_directory",
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
            "list_directory",
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
                "list_directory",
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
