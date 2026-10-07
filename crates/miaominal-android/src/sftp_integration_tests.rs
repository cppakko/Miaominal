//! Integration tests for the interactive SFTP client against a real local
//! SFTP server.
//!
//! The server is a `russh` SSH server on `127.0.0.1` that serves the `sftp`
//! subsystem from a temporary directory (plus optional virtual files for
//! large-file and failure scenarios). Transfers always move real bytes through
//! the duplicated-descriptor path, because the SAF stand-in on the test host
//! opens real files and hands their descriptors to Rust.

#![cfg(test)]

use std::collections::HashMap;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use russh::keys::{Algorithm, HashAlg, PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg, Session as ServerSession};
use russh::{Channel, ChannelId, SshId};
use russh_sftp::protocol::{
    Attrs, Data, File as SftpFile, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode,
    Version,
};
use russh_sftp::server::{Handler as SftpHandler, StatusReply};
use tokio::net::TcpListener;

use crate::backend::{Backend, BackendConfig};
use crate::dto::{AuthMethodDto, CredentialAction, CredentialKindDto, HostUpdate};
use crate::error::BackendError;
use crate::platform::PlatformHost;
use crate::session::SshHostKeyDecision;
use crate::sftp::{
    SftpRequestKind, SftpSession, SftpSessionPhase, SftpSessionSnapshot, SftpTransfer,
    SftpTransferFailureKind, SftpTransferSnapshot, SftpTransferState,
};
use crate::test_support::{business_root, cache_root, clear_test_root, shared_test_lock, test_host};

#[derive(Clone, Default)]
struct SftpTestOptions {
    /// Paths (POSIX-style, absolute) whose write/remove/mkdir operations are
    /// rejected with `PermissionDenied`.
    denied_paths: Vec<String>,
    /// Paths whose writes fail with a "No space left on device" failure.
    disk_full_paths: Vec<String>,
    /// Virtual files: path -> size. Reads return zero bytes.
    virtual_files: Vec<(String, u64)>,
    /// Artificial delay before every read reply, to make transfers observable.
    read_delay: Duration,
    /// Abort the channel after this many bytes were served for one file.
    drop_after_bytes: Option<u64>,
    /// Include a `..` entry in directory listings (path traversal probe).
    inject_parent_name: bool,
    /// Never answer password authentication (cancel-during-auth tests).
    hang_auth: bool,
    /// Never answer the SFTP subsystem request (cancel-during-setup tests).
    hang_subsystem: bool,
}

struct ServerState {
    options: SftpTestOptions,
    root: PathBuf,
    channels: Mutex<HashMap<ChannelId, Channel<Msg>>>,
    kbi_round: Mutex<u32>,
    bytes_served: AtomicU64,
}

struct TestSftpServer {
    port: u16,
    public_key: PublicKey,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TestSftpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TestSftpServer {
    async fn start(options: SftpTestOptions, root: PathBuf) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("host key");
        let public_key = host_key.public_key().clone();
        let config = Arc::new(server::Config {
            server_id: SshId::Standard("SSH-2.0-Miaominal-Sftp-Test".into()),
            methods: russh::MethodSet::from(
                &[
                    russh::MethodKind::Password,
                    russh::MethodKind::KeyboardInteractive,
                ][..],
            ),
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![host_key],
            ..Default::default()
        });
        let state = Arc::new(ServerState {
            options,
            root,
            channels: Mutex::new(HashMap::new()),
            kbi_round: Mutex::new(0),
            bytes_served: AtomicU64::new(0),
        });
        let task = crate::runtime::runtime().spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let config = Arc::clone(&config);
                let state = Arc::clone(&state);
                crate::runtime::runtime().spawn(async move {
                    let running =
                        server::run_stream(config, stream, SftpServerHandler { state }).await;
                    if let Ok(running) = running {
                        let _ = running.await;
                    }
                });
            }
        });
        Self {
            port,
            public_key,
            task,
        }
    }

    fn fingerprint(&self) -> String {
        self.public_key.fingerprint(HashAlg::Sha256).to_string()
    }
}

struct SftpServerHandler {
    state: Arc<ServerState>,
}

impl server::Handler for SftpServerHandler {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        if self.state.options.hang_auth {
            // A server that never answers authentication must not be able to
            // keep a cancelled session alive.
            tokio::time::sleep(Duration::from_secs(30)).await;
            return Ok(Auth::reject());
        }
        Ok(if user == "tester" && password == "secret" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<server::Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        if user != "tester" {
            return Ok(Auth::reject());
        }
        let round = {
            let mut round = self.state.kbi_round.lock().expect("kbi lock");
            *round += 1;
            *round
        };
        match (round, response) {
            (1, _) => Ok(server::Auth::Partial {
                name: "Two-factor".into(),
                instructions: "Enter the password".into(),
                prompts: vec![("Password: ".into(), false)].into(),
            }),
            (2, Some(mut response)) => {
                let answer = response
                    .next()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    .unwrap_or_default();
                if answer == "secret" {
                    Ok(server::Auth::Partial {
                        name: "Two-factor".into(),
                        instructions: "Enter the one-time code".into(),
                        prompts: vec![("OTP: ".into(), true)].into(),
                    })
                } else {
                    Ok(Auth::reject())
                }
            }
            (_, Some(mut response)) => {
                let answer = response
                    .next()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    .unwrap_or_default();
                Ok(if answer == "123456" {
                    Auth::Accept
                } else {
                    Auth::reject()
                })
            }
            (_, None) => Ok(Auth::reject()),
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        _session: &mut ServerSession,
    ) -> Result<bool, Self::Error> {
        self.state
            .channels
            .lock()
            .expect("channel lock")
            .insert(channel.id(), channel);
        Ok(true)
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        if name != "sftp" {
            session.channel_failure(channel_id)?;
            return Ok(());
        }
        if self.state.options.hang_subsystem {
            // Never acknowledge the subsystem request.
            return Ok(());
        }
        let Some(channel) = self
            .state
            .channels
            .lock()
            .expect("channel lock")
            .remove(&channel_id)
        else {
            session.channel_failure(channel_id)?;
            return Ok(());
        };
        session.channel_success(channel_id)?;
        let handler = SftpFsHandler {
            state: Arc::clone(&self.state),
            handles: HashMap::new(),
            next_handle: 1,
        };
        russh_sftp::server::run(channel.into_stream(), handler).await;
        Ok(())
    }
}

enum FsHandle {
    File {
        file: Option<std::fs::File>,
        virtual_size: Option<u64>,
        bytes_written: u64,
    },
    Dir {
        entries: Vec<(String, bool, u64)>,
        position: usize,
    },
}

struct SftpFsHandler {
    state: Arc<ServerState>,
    handles: HashMap<String, FsHandle>,
    next_handle: u64,
}

impl SftpFsHandler {
    fn normalize(path: &str) -> String {
        let trimmed = path.trim_start_matches('/');
        if trimmed.is_empty() {
            "/".to_string()
        } else {
            format!("/{trimmed}")
        }
    }

    fn real_path(&self, path: &str) -> PathBuf {
        let normalized = Self::normalize(path);
        let relative = normalized.trim_start_matches('/');
        if relative.is_empty() {
            self.state.root.clone()
        } else {
            self.state.root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR))
        }
    }

    fn virtual_size(&self, path: &str) -> Option<u64> {
        let normalized = Self::normalize(path);
        self.state
            .options
            .virtual_files
            .iter()
            .find(|(candidate, _)| Self::normalize(candidate) == normalized)
            .map(|(_, size)| *size)
    }

    fn is_denied(&self, path: &str) -> bool {
        let normalized = Self::normalize(path);
        self.state
            .options
            .denied_paths
            .iter()
            .any(|candidate| Self::normalize(candidate) == normalized)
    }

    fn is_disk_full(&self, path: &str) -> bool {
        let normalized = Self::normalize(path);
        self.state
            .options
            .disk_full_paths
            .iter()
            .any(|candidate| Self::normalize(candidate) == normalized)
    }

    fn ok(id: u32) -> Status {
        Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "Ok".to_string(),
            language_tag: "en-US".to_string(),
        }
    }

    fn attrs_for(&self, path: &str) -> Result<FileAttributes, StatusReply> {
        if let Some(size) = self.virtual_size(path) {
            return Ok(FileAttributes {
                size: Some(size),
                permissions: Some(0o100644),
                ..FileAttributes::default()
            });
        }
        let metadata = std::fs::symlink_metadata(self.real_path(path))
            .map_err(|_| StatusCode::NoSuchFile)?;
        Ok(FileAttributes {
            size: Some(if metadata.is_dir() { 0 } else { metadata.len() }),
            permissions: Some(if metadata.is_dir() { 0o040755 } else { 0o100644 }),
            mtime: metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs() as u32),
            ..FileAttributes::default()
        })
    }

    fn handle_mut(&mut self, handle: &str) -> Result<&mut FsHandle, StatusReply> {
        self.handles
            .get_mut(handle)
            .ok_or_else(|| StatusCode::Failure.with_message("unknown file handle"))
    }

    fn open_handle(&mut self, handle: FsHandle) -> Handle {
        let id = format!("handle-{}", self.next_handle);
        self.next_handle += 1;
        self.handles.insert(id.clone(), handle);
        Handle {
            id: 0,
            handle: id,
        }
    }
}

impl SftpHandler for SftpFsHandler {
    type Error = StatusReply;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported.into()
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        Ok(Name {
            id,
            files: vec![SftpFile::dummy(Self::normalize(&path))],
        })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        Ok(Attrs {
            id,
            attrs: self.attrs_for(&path)?,
        })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        Ok(Attrs {
            id,
            attrs: self.attrs_for(&path)?,
        })
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        if self.is_denied(&filename) {
            return Err(StatusCode::PermissionDenied.with_message("permission denied"));
        }
        if self.is_disk_full(&filename) && pflags.contains(OpenFlags::WRITE) {
            return Err(StatusCode::Failure.with_message("No space left on device"));
        }
        if let Some(size) = self.virtual_size(&filename) {
            if pflags.contains(OpenFlags::WRITE) {
                return Err(StatusCode::PermissionDenied.with_message("read-only virtual file"));
            }
            let mut handle = self.open_handle(FsHandle::File {
                file: None,
                virtual_size: Some(size),
                bytes_written: 0,
            });
            handle.id = id;
            return Ok(handle);
        }
        let path = self.real_path(&filename);
        let mut options = std::fs::OpenOptions::new();
        options.read(pflags.contains(OpenFlags::READ) || !pflags.contains(OpenFlags::WRITE));
        options.write(pflags.contains(OpenFlags::WRITE));
        options.create(pflags.contains(OpenFlags::CREATE));
        options.truncate(pflags.contains(OpenFlags::TRUNCATE));
        options.append(pflags.contains(OpenFlags::APPEND));
        let file = options
            .open(&path)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => StatusCode::NoSuchFile.into(),
                std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied.into(),
                _ => StatusCode::Failure.with_message(error.to_string()),
            })?;
        let mut handle = self.open_handle(FsHandle::File {
            file: Some(file),
            virtual_size: None,
            bytes_written: 0,
        });
        handle.id = id;
        Ok(handle)
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.handles.remove(&handle);
        Ok(Self::ok(id))
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        if !self.state.options.read_delay.is_zero() {
            tokio::time::sleep(self.state.options.read_delay).await;
        }
        let fs_handle = self.handle_mut(&handle)?;
        match fs_handle {
            FsHandle::File {
                file,
                virtual_size,
                ..
            } => {
                if let Some(size) = *virtual_size {
                    if offset >= size {
                        return Err(StatusCode::Eof.into());
                    }
                    let remaining = size - offset;
                    let length = u64::from(len).min(remaining) as usize;
                    let served = self.state.bytes_served.fetch_add(length as u64, Ordering::SeqCst)
                        + length as u64;
                    if let Some(limit) = self.state.options.drop_after_bytes
                        && served > limit
                    {
                        return Err(StatusCode::ConnectionLost
                            .with_message("the test server dropped the connection"));
                    }
                    return Ok(Data {
                        id,
                        data: vec![0u8; length],
                    });
                }
                let file = file.as_mut().ok_or(StatusReply::from(StatusCode::Failure))?;
                file.seek(SeekFrom::Start(offset))
                    .map_err(|error| StatusCode::Failure.with_message(error.to_string()))?;
                let mut buffer = vec![0u8; len as usize];
                let read = file
                    .read(&mut buffer)
                    .map_err(|error| StatusCode::Failure.with_message(error.to_string()))?;
                if read == 0 {
                    return Err(StatusCode::Eof.into());
                }
                buffer.truncate(read);
                Ok(Data { id, data: buffer })
            }
            FsHandle::Dir { .. } => Err(StatusCode::Failure.into()),
        }
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let served = {
            let fs_handle = self.handle_mut(&handle)?;
            let FsHandle::File { file, bytes_written, .. } = fs_handle else {
                return Err(StatusCode::Failure.into());
            };
            let file = file.as_mut().ok_or(StatusReply::from(StatusCode::Failure))?;
            file.seek(SeekFrom::Start(offset))
                .map_err(|error| StatusCode::Failure.with_message(error.to_string()))?;
            file.write_all(&data)
                .map_err(|error| StatusCode::Failure.with_message(error.to_string()))?;
            *bytes_written += data.len() as u64;
            *bytes_written
        };
        let total = self
            .state
            .bytes_served
            .fetch_add(data.len() as u64, Ordering::SeqCst)
            + data.len() as u64;
        if let Some(limit) = self.state.options.drop_after_bytes
            && total > limit
        {
            return Err(StatusCode::ConnectionLost
                .with_message("the test server dropped the connection"));
        }
        let _ = served;
        Ok(Self::ok(id))
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        if self.is_denied(&path) {
            return Err(StatusCode::PermissionDenied.with_message("permission denied"));
        }
        let mut entries = Vec::new();
        if self.state.options.inject_parent_name && Self::normalize(&path) == "/" {
            // A hostile entry name that tries to escape the transfer root.
            entries.push(("../escape.txt".to_string(), false, 5));
        }
        let real = self.real_path(&path);
        let reader = std::fs::read_dir(&real).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => StatusReply::from(StatusCode::NoSuchFile),
            std::io::ErrorKind::PermissionDenied => StatusReply::from(StatusCode::PermissionDenied),
            _ => StatusCode::Failure.with_message(error.to_string()),
        })?;
        for entry in reader {
            let entry = entry.map_err(|error| StatusCode::Failure.with_message(error.to_string()))?;
            let metadata = std::fs::symlink_metadata(entry.path())
                .map_err(|error| StatusCode::Failure.with_message(error.to_string()))?;
            let is_dir = metadata.is_dir();
            entries.push((
                entry.file_name().to_string_lossy().into_owned(),
                is_dir,
                if is_dir { 0 } else { metadata.len() },
            ));
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        let mut handle = self.open_handle(FsHandle::Dir {
            entries,
            position: 0,
        });
        handle.id = id;
        Ok(handle)
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        let fs_handle = self.handle_mut(&handle)?;
        let FsHandle::Dir { entries, position } = fs_handle else {
            return Err(StatusCode::Failure.into());
        };
        if *position >= entries.len() {
            return Err(StatusCode::Eof.into());
        }
        let chunk: Vec<_> = entries
            .iter()
            .skip(*position)
            .take(32)
            .cloned()
            .collect();
        *position += chunk.len();
        Ok(Name {
            id,
            files: chunk
                .into_iter()
                .map(|(name, is_dir, size)| SftpFile {
                    filename: name,
                    longname: String::new(),
                    attrs: FileAttributes {
                        size: Some(size),
                        permissions: Some(if is_dir { 0o040755 } else { 0o100644 }),
                        ..FileAttributes::default()
                    },
                })
                .collect(),
        })
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        if self.is_denied(&filename) {
            return Err(StatusCode::PermissionDenied.with_message("permission denied"));
        }
        std::fs::remove_file(self.real_path(&filename))
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => StatusReply::from(StatusCode::NoSuchFile),
                std::io::ErrorKind::PermissionDenied => StatusReply::from(StatusCode::PermissionDenied),
                _ => StatusCode::Failure.with_message(error.to_string()),
            })?;
        Ok(Self::ok(id))
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        if self.is_denied(&path) {
            return Err(StatusCode::PermissionDenied.with_message("permission denied"));
        }
        std::fs::create_dir(self.real_path(&path)).map_err(|error| match error.kind() {
            std::io::ErrorKind::AlreadyExists => StatusReply::from(StatusCode::Failure),
            _ => StatusCode::Failure.with_message(error.to_string()),
        })?;
        Ok(Self::ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        std::fs::remove_dir(self.real_path(&path))
            .map_err(|error| StatusCode::Failure.with_message(error.to_string()))?;
        Ok(Self::ok(id))
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        if self.is_denied(&newpath) {
            return Err(StatusCode::PermissionDenied.with_message("permission denied"));
        }
        if self.is_disk_full(&newpath) {
            return Err(StatusCode::Failure.with_message("No space left on device"));
        }
        std::fs::rename(self.real_path(&oldpath), self.real_path(&newpath))
            .map_err(|error| StatusCode::Failure.with_message(error.to_string()))?;
        Ok(Self::ok(id))
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let fs_handle = self.handle_mut(&handle)?;
        match fs_handle {
            FsHandle::File {
                file,
                virtual_size,
                ..
            } => {
                let len = virtual_size.unwrap_or_else(|| {
                    file.as_ref()
                        .and_then(|file| file.metadata().ok())
                        .map(|metadata| metadata.len())
                        .unwrap_or(0)
                });
                Ok(Attrs {
                    id,
                    attrs: FileAttributes {
                        size: Some(len),
                        ..FileAttributes::default()
                    },
                })
            }
            FsHandle::Dir { .. } => Ok(Attrs {
                id,
                attrs: FileAttributes::default(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

fn backend_for_test() -> (Arc<crate::test_support::TestHost>, Arc<Backend>) {
    let host = test_host();
    let backend = Backend::new(
        Arc::clone(&host) as Arc<dyn PlatformHost>,
        BackendConfig {
            data_dir: business_root().to_string_lossy().into_owned(),
            cache_dir: cache_root().to_string_lossy().into_owned(),
        },
    );
    (host, backend)
}

async fn save_password_host(backend: &Arc<Backend>, id: &str, port: u16) {
    backend
        .save_host(HostUpdate {
            id: id.to_string(),
            name: Some(format!("Test {id}")),
            group: None,
            tags: None,
            kind: None,
            host: Some("127.0.0.1".to_string()),
            port: Some(port),
            username: Some("tester".to_string()),
            auth_method: Some(AuthMethodDto::Password),
            private_key_path: None,
            managed_key_id: None,
            certificate_path: None,
            startup_command: None,
            environment_variables: None,
            shell_type: None,
            is_favorite: None,
            last_connected_at_ms: None,
        })
        .await
        .expect("save host");
    backend
        .set_host_credential(
            id.to_string(),
            CredentialKindDto::Password,
            CredentialAction::Set {
                value: "secret".to_string(),
            },
        )
        .await
        .expect("set password credential");
}

fn trust_host(server: &TestSftpServer) {
    miaominal_storage::KnownHostsStore::new()
        .expect("known hosts")
        .learn("127.0.0.1", server.port, &server.public_key)
        .expect("learn host key");
}

async fn wait_for_phase(
    session: &Arc<SftpSession>,
    phase: SftpSessionPhase,
    timeout: Duration,
) -> SftpSessionSnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = session.snapshot();
        if snapshot.phase == phase {
            return snapshot;
        }
        assert!(
            !snapshot.phase.into_terminal() || snapshot.phase == phase,
            "session ended as {:?}: {snapshot:?}",
            snapshot.phase
        );
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {phase:?}; last snapshot {snapshot:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

trait TerminalPhase {
    fn into_terminal(self) -> bool;
}

impl TerminalPhase for SftpSessionPhase {
    fn into_terminal(self) -> bool {
        matches!(self, SftpSessionPhase::Closed | SftpSessionPhase::Failed)
    }
}

async fn open_ready_session(backend: &Arc<Backend>, host_id: &str) -> Arc<SftpSession> {
    let session = backend
        .open_sftp_session(host_id.to_string())
        .await
        .expect("open SFTP session");
    wait_for_phase(&session, SftpSessionPhase::Ready, Duration::from_secs(20)).await;
    session
}

async fn wait_for_request(
    session: &Arc<SftpSession>,
    kind: SftpRequestKind,
    timeout: Duration,
) -> crate::sftp::SftpRequest {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = session.snapshot();
        if let Some(request) = snapshot
            .request
            .as_ref()
            .filter(|request| request.kind == kind)
        {
            return request.clone();
        }
        assert!(
            !snapshot.phase.into_terminal(),
            "session ended before a {kind:?} request: {snapshot:?}"
        );
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {kind:?}; last snapshot {snapshot:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_for_transfer(
    transfer: &Arc<SftpTransfer>,
    predicate: impl Fn(&SftpTransferSnapshot) -> bool,
    timeout: Duration,
) -> SftpTransferSnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = transfer.snapshot();
        if let Some(snapshot) = snapshot.as_ref().filter(|snapshot| predicate(snapshot)) {
            return snapshot.clone();
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a transfer state; last snapshot {snapshot:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_until(
    predicate: impl Fn() -> bool,
    timeout: Duration,
    what: &str,
) {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn finished(snapshot: &SftpTransferSnapshot) -> bool {
    matches!(
        snapshot.state,
        SftpTransferState::Done | SftpTransferState::Cancelled | SftpTransferState::Failed
    )
}

fn write_real_file(root: &std::path::Path, relative: &str, contents: &[u8]) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent directory");
    }
    std::fs::write(path, contents).expect("write test file");
}

fn read_real_file(root: &std::path::Path, relative: &str) -> Vec<u8> {
    std::fs::read(root.join(relative)).expect("read test file")
}

// ---------------------------------------------------------------------------
// File and directory transfers
// ---------------------------------------------------------------------------

#[test]
fn file_upload_and_download_round_trip() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(SftpTestOptions::default(), server_root.path().into())
            .await;
        trust_host(&server);
        let (host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;

        let session = open_ready_session(&backend, "host-1").await;
        let listing = session.list_directory("/".to_string()).expect("list root");
        assert!(listing > 0);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(session.directory_snapshot().error, None);

        // Upload from a registered descriptor.
        let upload_source = tempfile::tempdir().expect("upload source");
        let upload_path = upload_source.path().join("报告.txt");
        std::fs::write(&upload_path, "héllo 世界".repeat(1000)).expect("write upload source");
        let fd = crate::local_file::open_test_fd(&upload_path).expect("open test fd");
        let handle = backend
            .register_local_file(fd, "报告.txt".to_string())
            .expect("register local file");
        assert!(
            handle.try_clone_file().is_ok(),
            "the duplicate must be readable right after registration"
        );
        let transfer = session
            .upload_file(Arc::clone(&handle), "/报告.txt".to_string())
            .await
            .expect("queue upload");
        let snapshot = wait_for_transfer(
            &transfer,
            |snapshot| snapshot.state == SftpTransferState::Done,
            Duration::from_secs(30),
        )
        .await;
        assert_eq!(snapshot.bytes_total, Some(("héllo 世界".repeat(1000)).len() as u64));
        assert_eq!(snapshot.bytes_complete, snapshot.bytes_total.unwrap());
        assert!(handle.is_closed(), "the session owns and closes the handle");

        let uploaded = read_real_file(server_root.path(), "报告.txt");
        assert_eq!(uploaded, "héllo 世界".repeat(1000).as_bytes());

        // Download into an authorized SAF tree (the test host backs it with a
        // real directory and real descriptors).
        let download_root = tempfile::tempdir().expect("download root");
        host.set_saf_root(download_root.path().to_path_buf());
        let transfer = session
            .download(
                "/报告.txt".to_string(),
                "content://test-tree".to_string(),
                "sub/报告-copy.txt".to_string(),
            )
            .await
            .expect("queue download");
        let snapshot = wait_for_transfer(
            &transfer,
            |snapshot| snapshot.state == SftpTransferState::Done,
            Duration::from_secs(30),
        )
        .await;
        assert_eq!(snapshot.direction, crate::sftp::SftpTransferDirection::Download);
        assert_eq!(
            read_real_file(download_root.path(), "sub/报告-copy.txt"),
            "héllo 世界".repeat(1000).as_bytes()
        );

        assert!(session.disconnect().await.is_ok());
        backend.shutdown();
    });
}

#[test]
fn empty_file_round_trips() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        write_real_file(server_root.path(), "empty.bin", b"");
        let server = TestSftpServer::start(SftpTestOptions::default(), server_root.path().into())
            .await;
        trust_host(&server);
        let (host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        let source = tempfile::tempdir().expect("source");
        let path = source.path().join("empty.bin");
        std::fs::write(&path, b"").expect("write empty file");
        let fd = crate::local_file::open_test_fd(&path).expect("open test fd");
        let handle = backend
            .register_local_file(fd, "empty.bin".to_string())
            .expect("register");
        let transfer = session
            .upload_file(handle.clone(), "/overwrite-empty.bin".to_string())
            .await
            .expect("queue upload");
        let snapshot = wait_for_transfer(
            &transfer,
            |snapshot| snapshot.state == SftpTransferState::Done,
            Duration::from_secs(20),
        )
        .await;
        assert_eq!(snapshot.bytes_complete, 0);
        assert_eq!(snapshot.bytes_total, Some(0));
        assert_eq!(read_real_file(server_root.path(), "overwrite-empty.bin"), b"");
        tokio::time::timeout(Duration::from_secs(2), async {
            while !handle.is_closed() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("completed empty upload must release its registered descriptor");

        let download_root = tempfile::tempdir().expect("download root");
        host.set_saf_root(download_root.path().to_path_buf());
        let transfer = session
            .download(
                "/empty.bin".to_string(),
                "content://test-tree".to_string(),
                "empty-copy.bin".to_string(),
            )
            .await
            .expect("queue download");
        wait_for_transfer(
            &transfer,
            |snapshot| snapshot.state == SftpTransferState::Done,
            Duration::from_secs(20),
        )
        .await;
        assert_eq!(read_real_file(download_root.path(), "empty-copy.bin"), b"");

        backend.shutdown();
    });
}

#[test]
fn directory_transfers_are_recursive_and_skip_nothing() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(SftpTestOptions::default(), server_root.path().into())
            .await;
        trust_host(&server);
        let (host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        // Upload a directory tree through the SAF stand-in.
        let upload_root = tempfile::tempdir().expect("upload root");
        host.set_saf_root(upload_root.path().to_path_buf());
        write_real_file(upload_root.path(), "tree/a.txt", b"alpha");
        write_real_file(upload_root.path(), "tree/nested/中文.txt", "内容".as_bytes());
        write_real_file(upload_root.path(), "tree/nested/deep/data.bin", &[7u8; 4096]);
        let transfer = session
            .upload_directory(
                "content://test-tree".to_string(),
                "tree".to_string(),
                "tree".to_string(),
                "/uploaded-tree".to_string(),
            )
            .await
            .expect("queue directory upload");
        let snapshot = wait_for_transfer(
            &transfer,
            |snapshot| snapshot.state == SftpTransferState::Done,
            Duration::from_secs(30),
        )
        .await;
        assert_eq!(snapshot.child_count, 3, "all files must be children");
        assert_eq!(snapshot.children_done, 3);
        assert_eq!(read_real_file(server_root.path(), "uploaded-tree/a.txt"), b"alpha");
        assert_eq!(
            read_real_file(server_root.path(), "uploaded-tree/nested/中文.txt"),
            "内容".as_bytes()
        );

        // Download the same tree into another SAF root.
        let download_root = tempfile::tempdir().expect("download root");
        host.set_saf_root(download_root.path().to_path_buf());
        let transfer = session
            .download(
                "/uploaded-tree".to_string(),
                "content://test-tree".to_string(),
                "restored".to_string(),
            )
            .await
            .expect("queue directory download");
        let snapshot = wait_for_transfer(
            &transfer,
            |snapshot| snapshot.state == SftpTransferState::Done,
            Duration::from_secs(30),
        )
        .await;
        assert_eq!(snapshot.children_done, 3);
        assert_eq!(read_real_file(download_root.path(), "restored/a.txt"), b"alpha");
        assert_eq!(
            read_real_file(download_root.path(), "restored/nested/deep/data.bin"),
            vec![7u8; 4096]
        );

        backend.shutdown();
    });
}

#[test]
fn uploads_replace_existing_remote_files() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        write_real_file(server_root.path(), "existing.txt", b"old contents");
        let server = TestSftpServer::start(SftpTestOptions::default(), server_root.path().into())
            .await;
        trust_host(&server);
        let (_host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        let source = tempfile::tempdir().expect("source");
        let path = source.path().join("existing.txt");
        std::fs::write(&path, b"new contents").expect("write source");
        let fd = crate::local_file::open_test_fd(&path).expect("open test fd");
        let handle = backend
            .register_local_file(fd, "existing.txt".to_string())
            .expect("register");
        let transfer = session
            .upload_file(handle, "/existing.txt".to_string())
            .await
            .expect("queue upload");
        wait_for_transfer(
            &transfer,
            |snapshot| snapshot.state == SftpTransferState::Done,
            Duration::from_secs(20),
        )
        .await;
        assert_eq!(read_real_file(server_root.path(), "existing.txt"), b"new contents");
        let leftovers: Vec<_> = std::fs::read_dir(server_root.path())
            .expect("list server root")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "no temporary or backup files may remain: {leftovers:?}");

        backend.shutdown();
    });
}

#[test]
fn downloads_replace_existing_documents() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        write_real_file(server_root.path(), "replace.txt", b"remote data");
        let server = TestSftpServer::start(SftpTestOptions::default(), server_root.path().into())
            .await;
        trust_host(&server);
        let (host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        let download_root = tempfile::tempdir().expect("download root");
        write_real_file(download_root.path(), "replace.txt", b"local data");
        host.set_saf_root(download_root.path().to_path_buf());
        let transfer = session
            .download(
                "/replace.txt".to_string(),
                "content://test-tree".to_string(),
                "replace.txt".to_string(),
            )
            .await
            .expect("queue download");
        wait_for_transfer(
            &transfer,
            |snapshot| snapshot.state == SftpTransferState::Done,
            Duration::from_secs(20),
        )
        .await;
        assert_eq!(read_real_file(download_root.path(), "replace.txt"), b"remote data");

        backend.shutdown();
    });
}

#[test]
fn a_refused_commit_keeps_the_original_and_removes_the_staging_document() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        write_real_file(server_root.path(), "report.txt", b"remote contents");
        let server = TestSftpServer::start(SftpTestOptions::default(), server_root.path().into())
            .await;
        trust_host(&server);
        let (host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        let download_root = tempfile::tempdir().expect("download root");
        write_real_file(download_root.path(), "report.txt", b"original contents");
        host.set_saf_root(download_root.path().to_path_buf());
        host.set_fail_commit_paths(vec!["report.txt".to_string()]);

        let transfer = session
            .download(
                "/report.txt".to_string(),
                "content://test-tree".to_string(),
                "report.txt".to_string(),
            )
            .await
            .expect("queue download");
        let snapshot = wait_for_transfer(
            &transfer,
            |snapshot| snapshot.state == SftpTransferState::Failed,
            Duration::from_secs(20),
        )
        .await;
        assert_eq!(
            snapshot.failure_kind,
            SftpTransferFailureKind::ProviderRefused,
            "a refused replace must be classified as provider refusal: {snapshot:?}"
        );
        assert_eq!(
            read_real_file(download_root.path(), "report.txt"),
            b"original contents",
            "the original document must survive a refused commit"
        );
        let leftovers: Vec<_> = std::fs::read_dir(download_root.path())
            .expect("list download root")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            leftovers,
            vec!["report.txt".to_string()],
            "the staging document must be removed: {leftovers:?}"
        );

        backend.shutdown();
    });
}

#[test]
fn cancel_while_replacing_keeps_the_original_and_removes_the_staging_document() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(
            SftpTestOptions {
                virtual_files: vec![("/big.bin".to_string(), 24 * 1024 * 1024)],
                read_delay: Duration::from_millis(4),
                ..SftpTestOptions::default()
            },
            server_root.path().into(),
        )
        .await;
        trust_host(&server);
        let (host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        let download_root = tempfile::tempdir().expect("download root");
        write_real_file(download_root.path(), "big.bin", b"original contents");
        host.set_saf_root(download_root.path().to_path_buf());
        let transfer = session
            .download(
                "/big.bin".to_string(),
                "content://test-tree".to_string(),
                "big.bin".to_string(),
            )
            .await
            .expect("queue download");
        wait_for_transfer(
            &transfer,
            |snapshot| snapshot.bytes_complete > 512 * 1024,
            Duration::from_secs(20),
        )
        .await;
        transfer.cancel().expect("cancel");
        let snapshot = wait_for_transfer(&transfer, finished, Duration::from_secs(20)).await;
        assert_eq!(snapshot.state, SftpTransferState::Cancelled);

        assert_eq!(
            read_real_file(download_root.path(), "big.bin"),
            b"original contents",
            "a cancelled replacement must leave the original untouched"
        );
        let leftovers: Vec<_> = std::fs::read_dir(download_root.path())
            .expect("list download root")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            leftovers,
            vec!["big.bin".to_string()],
            "the staging document must be removed: {leftovers:?}"
        );

        backend.shutdown();
    });
}

#[test]
fn remove_file_and_directory_delete_remote_entries() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        write_real_file(server_root.path(), "remove-me.txt", b"data");
        write_real_file(server_root.path(), "tree/child.txt", b"data");
        let server = TestSftpServer::start(SftpTestOptions::default(), server_root.path().into())
            .await;
        trust_host(&server);
        let (_host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        session
            .remove_file("/remove-me.txt".to_string())
            .expect("remove file");
        wait_until(
            || !server_root.path().join("remove-me.txt").exists(),
            Duration::from_secs(10),
            "the remote file to be removed",
        )
        .await;

        session
            .remove_directory("/tree".to_string())
            .expect("remove directory");
        wait_until(
            || !server_root.path().join("tree").exists(),
            Duration::from_secs(10),
            "the remote directory to be removed",
        )
        .await;

        backend.shutdown();
    });
}

#[test]
fn directory_creation_and_rename_are_applied() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        write_real_file(server_root.path(), "before.txt", b"data");
        let server = TestSftpServer::start(SftpTestOptions::default(), server_root.path().into())
            .await;
        trust_host(&server);
        let (_host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        session
            .create_directory("/created".to_string())
            .expect("create directory");
        wait_until(
            || server_root.path().join("created").is_dir(),
            Duration::from_secs(10),
            "the remote directory to be created",
        )
        .await;
        session
            .rename("/before.txt".to_string(), "/after.txt".to_string())
            .expect("rename");
        wait_until(
            || server_root.path().join("after.txt").exists(),
            Duration::from_secs(10),
            "the remote rename to be applied",
        )
        .await;

        backend.shutdown();
    });
}

// ---------------------------------------------------------------------------
// Failures: permission, disk, provider, disconnects, invalid handles
// ---------------------------------------------------------------------------

#[test]
fn permission_denied_is_reported_for_uploads_and_listings() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(
            SftpTestOptions {
                denied_paths: vec!["/locked".to_string(), "/locked.txt".to_string()],
                ..SftpTestOptions::default()
            },
            server_root.path().into(),
        )
        .await;
        trust_host(&server);
        let (_host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        let source = tempfile::tempdir().expect("source");
        let path = source.path().join("locked.txt");
        std::fs::write(&path, b"data").expect("write source");
        let fd = crate::local_file::open_test_fd(&path).expect("open test fd");
        let handle = backend
            .register_local_file(fd, "locked.txt".to_string())
            .expect("register");
        let transfer = session
            .upload_file(handle, "/locked.txt".to_string())
            .await
            .expect("queue upload");
        let snapshot = wait_for_transfer(
            &transfer,
            |snapshot| snapshot.state == SftpTransferState::Failed,
            Duration::from_secs(20),
        )
        .await;
        assert_eq!(snapshot.failure_kind, SftpTransferFailureKind::Permission);
        assert!(!server_root.path().join("locked.txt").exists());

        session.list_directory("/locked".to_string()).expect("queue listing");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let directory = session.directory_snapshot();
            if directory.error.is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "listing failure was not reported");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        // Queue-only operations (remove/rename/create) report their failure
        // through the typed snapshot instead of losing it.
        session
            .remove_file("/locked.txt".to_string())
            .expect("queue remove");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let snapshot = session.snapshot();
            if let Some(error) = snapshot
                .operation_error
                .as_ref()
                .filter(|error| error.operation == crate::sftp::SftpOperation::RemoveFile)
            {
                assert!(error.message.contains("permission denied"), "unexpected: {error:?}");
                assert_eq!(error.path.as_deref(), Some("/locked.txt"));
                assert!(error.sequence > 0);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "remove failure was not reported: {snapshot:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let first_sequence = session
            .snapshot()
            .operation_error
            .expect("remove failure")
            .sequence;

        session
            .create_directory("/locked".to_string())
            .expect("queue create");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let snapshot = session.snapshot();
            if let Some(error) = snapshot
                .operation_error
                .as_ref()
                .filter(|error| error.operation == crate::sftp::SftpOperation::CreateDirectory)
            {
                assert!(error.sequence > first_sequence, "sequences must increase");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "create failure was not reported: {snapshot:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        backend.shutdown();
    });
}

#[test]
fn disk_full_upload_fails_with_a_typed_kind() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(
            SftpTestOptions {
                disk_full_paths: vec!["/full.txt".to_string()],
                ..SftpTestOptions::default()
            },
            server_root.path().into(),
        )
        .await;
        trust_host(&server);
        let (_host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        let source = tempfile::tempdir().expect("source");
        let path = source.path().join("full.txt");
        std::fs::write(&path, vec![1u8; 1024]).expect("write source");
        let fd = crate::local_file::open_test_fd(&path).expect("open test fd");
        let handle = backend
            .register_local_file(fd, "full.txt".to_string())
            .expect("register");
        let transfer = session
            .upload_file(handle, "/full.txt".to_string())
            .await
            .expect("queue upload");
        let snapshot = wait_for_transfer(
            &transfer,
            |snapshot| snapshot.state == SftpTransferState::Failed,
            Duration::from_secs(20),
        )
        .await;
        assert_eq!(snapshot.failure_kind, SftpTransferFailureKind::NoSpace);
        assert!(
            snapshot.error.as_deref().is_some_and(|error| error.contains("space")),
            "error must explain the failure: {snapshot:?}"
        );
        assert!(!server_root.path().join("full.txt").exists());

        backend.shutdown();
    });
}

#[test]
fn a_server_that_drops_the_channel_fails_the_transfer() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(
            SftpTestOptions {
                virtual_files: vec![("/large.bin".to_string(), 4 * 1024 * 1024)],
                drop_after_bytes: Some(1024 * 1024),
                ..SftpTestOptions::default()
            },
            server_root.path().into(),
        )
        .await;
        trust_host(&server);
        let (host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        let download_root = tempfile::tempdir().expect("download root");
        host.set_saf_root(download_root.path().to_path_buf());
        let transfer = session
            .download(
                "/large.bin".to_string(),
                "content://test-tree".to_string(),
                "large.bin".to_string(),
            )
            .await
            .expect("queue download");
        let snapshot = wait_for_transfer(
            &transfer,
            |snapshot| snapshot.state == SftpTransferState::Failed,
            Duration::from_secs(30),
        )
        .await;
        assert!(
            matches!(
                snapshot.failure_kind,
                SftpTransferFailureKind::Network | SftpTransferFailureKind::Other
            ),
            "unexpected failure kind: {snapshot:?}"
        );
        assert!(
            !download_root.path().join("large.bin").exists(),
            "an incomplete download must not be left behind"
        );

        backend.shutdown();
    });
}

#[test]
fn invalid_and_closed_descriptors_are_handled() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(SftpTestOptions::default(), server_root.path().into())
            .await;
        trust_host(&server);
        let (_host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        // A descriptor that was never valid is rejected at registration.
        let error = backend
            .register_local_file(-1, "nope.bin".to_string())
            .expect_err("negative descriptors must be rejected");
        assert!(matches!(
            error,
            BackendError::InvalidArgument { .. } | BackendError::Storage { .. }
        ));

        // A handle closed before the transfer starts fails without a leak.
        let source = tempfile::tempdir().expect("source");
        let path = source.path().join("gone.bin");
        std::fs::write(&path, b"data").expect("write source");
        let fd = crate::local_file::open_test_fd(&path).expect("open test fd");
        let handle = backend
            .register_local_file(fd, "gone.bin".to_string())
            .expect("register");
        assert!(handle.release());
        let transfer = session
            .upload_file(handle, "/gone.bin".to_string())
            .await
            .expect("queue upload");
        let snapshot = wait_for_transfer(
            &transfer,
            finished,
            Duration::from_secs(20),
        )
        .await;
        assert_eq!(snapshot.state, SftpTransferState::Failed);
        assert!(!server_root.path().join("gone.bin").exists());

        backend.shutdown();
    });
}

// ---------------------------------------------------------------------------
// Pause, resume, cancel, large files
// ---------------------------------------------------------------------------

#[test]
fn pause_and_resume_finish_the_transfer_from_its_position() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(
            SftpTestOptions {
                virtual_files: vec![("/big.bin".to_string(), 24 * 1024 * 1024)],
                read_delay: Duration::from_millis(4),
                ..SftpTestOptions::default()
            },
            server_root.path().into(),
        )
        .await;
        trust_host(&server);
        let (host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        let download_root = tempfile::tempdir().expect("download root");
        host.set_saf_root(download_root.path().to_path_buf());
        let transfer = session
            .download(
                "/big.bin".to_string(),
                "content://test-tree".to_string(),
                "big.bin".to_string(),
            )
            .await
            .expect("queue download");

        // Wait until some progress accumulated, then pause.
        let running = wait_for_transfer(
            &transfer,
            |snapshot| snapshot.bytes_complete > 1024 * 1024,
            Duration::from_secs(20),
        )
        .await;
        transfer.pause().expect("pause");
        let paused = wait_for_transfer(
            &transfer,
            |snapshot| snapshot.state == SftpTransferState::Paused,
            Duration::from_secs(10),
        )
        .await;
        let paused_bytes = paused.bytes_complete.max(running.bytes_complete);
        tokio::time::sleep(Duration::from_millis(150)).await;
        let still_paused = transfer.snapshot().expect("transfer snapshot");
        assert_eq!(still_paused.state, SftpTransferState::Paused);
        assert_eq!(
            still_paused.bytes_complete, paused_bytes,
            "a paused transfer must not advance"
        );

        transfer.resume().expect("resume");
        let done = wait_for_transfer(
            &transfer,
            |snapshot| snapshot.state == SftpTransferState::Done,
            Duration::from_secs(60),
        )
        .await;
        assert_eq!(done.bytes_complete, 24 * 1024 * 1024);
        assert_eq!(
            std::fs::metadata(download_root.path().join("big.bin"))
                .expect("downloaded file")
                .len(),
            24 * 1024 * 1024
        );

        backend.shutdown();
    });
}

#[test]
fn cancel_stops_the_transfer_and_cleans_up() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(
            SftpTestOptions {
                virtual_files: vec![("/big.bin".to_string(), 24 * 1024 * 1024)],
                read_delay: Duration::from_millis(4),
                ..SftpTestOptions::default()
            },
            server_root.path().into(),
        )
        .await;
        trust_host(&server);
        let (host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        let download_root = tempfile::tempdir().expect("download root");
        host.set_saf_root(download_root.path().to_path_buf());
        let transfer = session
            .download(
                "/big.bin".to_string(),
                "content://test-tree".to_string(),
                "big.bin".to_string(),
            )
            .await
            .expect("queue download");
        wait_for_transfer(
            &transfer,
            |snapshot| snapshot.bytes_complete > 512 * 1024,
            Duration::from_secs(20),
        )
        .await;
        transfer.cancel().expect("cancel");
        let snapshot = wait_for_transfer(&transfer, finished, Duration::from_secs(20)).await;
        assert_eq!(snapshot.state, SftpTransferState::Cancelled);
        assert!(
            !download_root.path().join("big.bin").exists(),
            "a cancelled download must not leave an incomplete document"
        );

        backend.shutdown();
    });
}

#[test]
fn progress_beyond_four_gibibytes_does_not_overflow() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let five_gib = 5u64 * 1024 * 1024 * 1024;
        let drop_after = 4 * 1024 * 1024 * 1024 + 64 * 1024 * 1024;
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(
            SftpTestOptions {
                virtual_files: vec![("/giant.bin".to_string(), five_gib)],
                // The server drops the channel once more than 4 GiB + 64 MiB
                // has been served, so the test observes a u64 progress value
                // beyond the 32-bit boundary without writing 5 GiB to disk.
                drop_after_bytes: Some(drop_after),
                ..SftpTestOptions::default()
            },
            server_root.path().into(),
        )
        .await;
        trust_host(&server);
        let (host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        host.set_saf_null_writes(true);
        let transfer = session
            .download(
                "/giant.bin".to_string(),
                "content://test-tree".to_string(),
                "giant.bin".to_string(),
            )
            .await
            .expect("queue download");
        let finished = wait_for_transfer(&transfer, finished, Duration::from_secs(600)).await;

        assert_eq!(
            finished.bytes_total,
            Some(five_gib),
            "a 64-bit length must survive the protocol: {finished:?}"
        );
        assert!(
            finished.bytes_complete > u64::from(u32::MAX),
            "progress must not truncate at 32 bits: {finished:?}"
        );
        assert!(
            matches!(
                finished.state,
                SftpTransferState::Failed | SftpTransferState::Cancelled
            ),
            "the dropped connection must not report success: {finished:?}"
        );

        backend.shutdown();
    });
}

#[test]
fn recursive_transfers_reject_parent_path_entries() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        write_real_file(server_root.path(), "secret.txt", b"do not escape");
        let server = TestSftpServer::start(
            SftpTestOptions {
                inject_parent_name: true,
                ..SftpTestOptions::default()
            },
            server_root.path().into(),
        )
        .await;
        trust_host(&server);
        let (host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        // A malicious entry name must fail the recursive download instead of
        // writing outside the destination.
        let download_root = tempfile::tempdir().expect("download root");
        host.set_saf_root(download_root.path().to_path_buf());
        let transfer = session
            .download(
                "/".to_string(),
                "content://test-tree".to_string(),
                "tree".to_string(),
            )
            .await
            .expect("queue download");
        let snapshot = wait_for_transfer(
            &transfer,
            |snapshot| snapshot.state == SftpTransferState::Failed,
            Duration::from_secs(20),
        )
        .await;
        assert!(
            snapshot
                .error
                .as_deref()
                .is_some_and(|error| error.contains("not valid")),
            "the parent entry must be rejected: {snapshot:?}"
        );
        assert!(
            !download_root.path().join("secret.txt").exists()
                && !download_root.path().parent().expect("parent").join("secret.txt").exists(),
            "nothing may be written outside the destination"
        );

        // The queue API also rejects escapes before the transfer starts.
        let error = session
            .download(
                "/secret.txt".to_string(),
                "content://test-tree".to_string(),
                "../escape.txt".to_string(),
            )
            .await
            .expect_err("escapes must be rejected at queue time");
        assert!(matches!(error, BackendError::InvalidArgument { .. }));

        backend.shutdown();
    });
}

#[test]
fn at_most_two_transfers_run_concurrently() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(
            SftpTestOptions {
                virtual_files: vec![
                    ("/one.bin".to_string(), 4 * 1024 * 1024),
                    ("/two.bin".to_string(), 4 * 1024 * 1024),
                    ("/three.bin".to_string(), 4 * 1024 * 1024),
                ],
                read_delay: Duration::from_millis(15),
                ..SftpTestOptions::default()
            },
            server_root.path().into(),
        )
        .await;
        trust_host(&server);
        let (host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        // All three write into the same tree; the null sink avoids disk churn.
        host.set_saf_null_writes(true);
        let mut transfers = Vec::new();
        for (index, name) in ["one.bin", "two.bin", "three.bin"].iter().enumerate() {
            let remote = format!("/{name}");
            let transfer = session
                .download(
                    remote,
                    "content://test-tree".to_string(),
                    format!("copy-{index}.bin"),
                )
                .await
                .expect("queue download");
            transfers.push(transfer);
        }

        // Wait until the third transfer is visibly queued behind the limit.
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut observed_queued_behind_limit = false;
        loop {
            // A record appears on the transfer as soon as the queued event is
            // applied; before that the transfer simply is not snapshotable yet.
            let snapshots: Vec<_> = transfers
                .iter()
                .filter_map(|transfer| transfer.snapshot())
                .collect();
            if snapshots.len() < transfers.len() {
                assert!(Instant::now() < deadline, "transfers never appeared");
                tokio::time::sleep(Duration::from_millis(20)).await;
                continue;
            }
            let running = snapshots
                .iter()
                .filter(|snapshot| {
                    snapshot.state == SftpTransferState::Running
                        || (snapshot.state == SftpTransferState::Queued
                            && snapshot.bytes_complete > 0)
                })
                .count();
            let queued_idle = snapshots
                .iter()
                .filter(|snapshot| {
                    snapshot.state == SftpTransferState::Queued && snapshot.bytes_complete == 0
                })
                .count();
            if running >= 1 && queued_idle >= 1 {
                observed_queued_behind_limit = true;
            }
            if snapshots.iter().all(|snapshot| finished(snapshot)) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "transfers did not finish: {snapshots:?}"
            );
            assert!(
                snapshots
                    .iter()
                    .filter(|snapshot| snapshot.bytes_complete > 0 && !finished(snapshot))
                    .count()
                    <= 2,
                "no more than two transfers may run concurrently: {snapshots:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        assert!(
            observed_queued_behind_limit,
            "the third transfer must have waited for a slot"
        );
        for transfer in &transfers {
            assert_eq!(
                transfer.snapshot().expect("snapshot").state,
                SftpTransferState::Done
            );
        }

        backend.shutdown();
    });
}

// ---------------------------------------------------------------------------
// Interactive authentication and request ids
// ---------------------------------------------------------------------------

#[test]
fn unknown_host_key_prompts_and_accept_and_save_connects() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(SftpTestOptions::default(), server_root.path().into())
            .await;
        let (_host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;

        let session = backend
            .open_sftp_session("host-1".to_string())
            .await
            .expect("open SFTP session");
        let request =
            wait_for_request(&session, SftpRequestKind::HostKey, Duration::from_secs(20)).await;
        assert_eq!(request.host, "127.0.0.1");
        assert_eq!(request.fingerprint, server.fingerprint());
        assert!(request.previous_fingerprint.is_none());

        session
            .answer_host_key(request.request_id, SshHostKeyDecision::AcceptAndSave)
            .expect("answer host key");
        wait_for_phase(&session, SftpSessionPhase::Ready, Duration::from_secs(20)).await;
        assert!(
            miaominal_storage::KnownHostsStore::new()
                .expect("known hosts")
                .check("127.0.0.1", server.port, &server.public_key)
                .is_ok()
        );

        backend.shutdown();
    });
}

#[test]
fn keyboard_interactive_answers_multiple_rounds_and_rejects_stale_ids() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(SftpTestOptions::default(), server_root.path().into())
            .await;
        trust_host(&server);
        let (_host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        backend
            .save_host(HostUpdate {
                id: "host-1".to_string(),
                name: Some("Test host-1".to_string()),
                group: None,
                tags: None,
                kind: None,
                host: Some("127.0.0.1".to_string()),
                port: Some(server.port),
                username: Some("tester".to_string()),
                auth_method: Some(AuthMethodDto::KeyboardInteractive),
                private_key_path: None,
                managed_key_id: None,
                certificate_path: None,
                startup_command: None,
                environment_variables: None,
                shell_type: None,
                is_favorite: None,
                last_connected_at_ms: None,
            })
            .await
            .expect("save host");

        let session = backend
            .open_sftp_session("host-1".to_string())
            .await
            .expect("open SFTP session");
        let first = wait_for_request(
            &session,
            SftpRequestKind::KeyboardInteractive,
            Duration::from_secs(20),
        )
        .await;
        assert_eq!(first.prompts.len(), 1);
        // A stale request id is rejected without disturbing the pending one.
        let error = session
            .answer_keyboard_interactive(first.request_id + 100, vec!["secret".to_string()])
            .expect_err("stale answers must be rejected");
        assert!(matches!(error, BackendError::RequestExpired { .. }));
        session
            .answer_keyboard_interactive(first.request_id, vec!["secret".to_string()])
            .expect("answer first round");

        let second = wait_for_request(
            &session,
            SftpRequestKind::KeyboardInteractive,
            Duration::from_secs(20),
        )
        .await;
        assert!(second.request_id > first.request_id);
        session
            .answer_keyboard_interactive(second.request_id, vec!["123456".to_string()])
            .expect("answer second round");
        wait_for_phase(&session, SftpSessionPhase::Ready, Duration::from_secs(20)).await;

        backend.shutdown();
    });
}

#[test]
fn an_unreachable_host_fails_the_session_and_close_is_idempotent() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let (_host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        backend
            .save_host(HostUpdate {
                id: "host-unreachable".to_string(),
                name: Some("Unreachable".to_string()),
                group: None,
                tags: None,
                kind: None,
                host: Some("127.0.0.1".to_string()),
                port: Some(1),
                username: Some("tester".to_string()),
                auth_method: Some(AuthMethodDto::Password),
                private_key_path: None,
                managed_key_id: None,
                certificate_path: None,
                startup_command: None,
                environment_variables: None,
                shell_type: None,
                is_favorite: None,
                last_connected_at_ms: None,
            })
            .await
            .expect("save host");
        backend
            .set_host_credential(
                "host-unreachable".to_string(),
                CredentialKindDto::Password,
                CredentialAction::Set {
                    value: "secret".to_string(),
                },
            )
            .await
            .expect("credential");

        let session = backend
            .open_sftp_session("host-unreachable".to_string())
            .await
            .expect("open session");
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let snapshot = session.snapshot();
            if snapshot.phase == SftpSessionPhase::Failed {
                assert!(snapshot.failure.is_some(), "a failure must explain itself");
                break;
            }
            assert!(
                snapshot.phase != SftpSessionPhase::Closed,
                "a failed connection must not be reported as a clean close: {snapshot:?}"
            );
            assert!(Instant::now() < deadline, "timed out: {snapshot:?}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        let connection_id = session.connection_id();
        assert!(backend.close_sftp_session(connection_id).await.expect("close"));
        assert!(!backend.close_sftp_session(connection_id).await.expect("close twice"));
        backend.shutdown();
    });
}

#[test]
fn closing_while_authentication_hangs_stops_the_session() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(
            SftpTestOptions {
                hang_auth: true,
                ..SftpTestOptions::default()
            },
            server_root.path().into(),
        )
        .await;
        trust_host(&server);
        let (_host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;

        let session = backend
            .open_sftp_session("host-1".to_string())
            .await
            .expect("open session");
        // Wait until the connector is inside authentication.
        wait_for_phase(
            &session,
            SftpSessionPhase::WaitingForAuthentication,
            Duration::from_secs(20),
        )
        .await;

        let started = Instant::now();
        session.disconnect().await.expect("disconnect");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "a hanging authentication must not keep the session alive ({elapsed:?})"
        );
        assert_eq!(session.snapshot().phase, SftpSessionPhase::Closed);
        assert!(backend.close_sftp_session(session.connection_id()).await.expect("close"));

        backend.shutdown();
    });
}

#[test]
fn closing_while_the_subsystem_hangs_stops_the_session() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(
            SftpTestOptions {
                hang_subsystem: true,
                ..SftpTestOptions::default()
            },
            server_root.path().into(),
        )
        .await;
        trust_host(&server);
        let (_host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;

        let session = backend
            .open_sftp_session("host-1".to_string())
            .await
            .expect("open session");
        wait_for_phase(
            &session,
            SftpSessionPhase::OpeningSubsystem,
            Duration::from_secs(20),
        )
        .await;

        let started = Instant::now();
        session.disconnect().await.expect("disconnect");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "a hanging subsystem request must not keep the session alive ({elapsed:?})"
        );
        assert_eq!(session.snapshot().phase, SftpSessionPhase::Closed);
        assert!(backend.close_sftp_session(session.connection_id()).await.expect("close"));

        backend.shutdown();
    });
}

#[test]
fn registry_keeps_sessions_until_they_are_closed() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(SftpTestOptions::default(), server_root.path().into())
            .await;
        trust_host(&server);
        let (_host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;

        let session = open_ready_session(&backend, "host-1").await;
        assert_eq!(backend.sftp_session_snapshots().len(), 1);
        let connection_id = session.connection_id();
        session.disconnect().await.expect("disconnect");
        assert_eq!(backend.sftp_session_snapshots().len(), 1, "closed sessions are retained");
        assert!(backend
            .close_sftp_session(connection_id)
            .await
            .expect("close session"));
        assert!(backend.sftp_session_snapshots().is_empty());
        assert!(
            !backend
                .close_sftp_session(connection_id)
                .await
                .expect("idempotent close"),
            "closing twice returns false"
        );

        backend.shutdown();
    });
}

#[test]
fn shutdown_cancels_running_sftp_transfers() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server_root = tempfile::tempdir().expect("server root");
        let server = TestSftpServer::start(
            SftpTestOptions {
                virtual_files: vec![("/big.bin".to_string(), 64 * 1024 * 1024)],
                read_delay: Duration::from_millis(4),
                ..SftpTestOptions::default()
            },
            server_root.path().into(),
        )
        .await;
        trust_host(&server);
        let (host, backend) = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_password_host(&backend, "host-1", server.port).await;
        let session = open_ready_session(&backend, "host-1").await;

        let download_root = tempfile::tempdir().expect("download root");
        host.set_saf_root(download_root.path().to_path_buf());
        let transfer = session
            .download(
                "/big.bin".to_string(),
                "content://test-tree".to_string(),
                "big.bin".to_string(),
            )
            .await
            .expect("queue download");
        wait_for_transfer(
            &transfer,
            |snapshot| snapshot.bytes_complete > 512 * 1024,
            Duration::from_secs(20),
        )
        .await;

        backend.shutdown();
        backend.shutdown();

        // The old handle is closed and no session survives into the next
        // generation; the incomplete download is not a completed document.
        assert!(matches!(
            backend.open_sftp_session("host-1".to_string()).await,
            Err(BackendError::Closed)
        ));
        assert!(!download_root.path().join("big.bin").exists());

        let restarted = backend_for_test().1;
        restarted.initialize().await.expect("reinitialize");
        assert!(
            restarted.sftp_session_snapshots().is_empty(),
            "a restarted backend starts without sessions (they show as interrupted)"
        );
        restarted.shutdown();
    });
}
