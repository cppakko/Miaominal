use anyhow::{Context, Result, anyhow, bail};
use futures::channel::mpsc::UnboundedSender;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::watch;
use tokio::task::JoinHandle;

const APP_INSTANCE_PROTOCOL_VERSION: u16 = 1;
const APP_INSTANCE_MAX_CONTROL_FRAME: usize = 1024;
const APP_INSTANCE_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const APP_INSTANCE_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
const APP_INSTANCE_RETRY_DELAY: Duration = Duration::from_millis(25);

trait AppInstanceIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> AppInstanceIo for T where T: AsyncRead + AsyncWrite + Unpin + Send {}
type AppInstanceStream = Box<dyn AppInstanceIo>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AppInstanceCommand {
    Activate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AppInstanceRequest {
    version: u16,
    command: AppInstanceCommand,
}

impl AppInstanceRequest {
    fn activate() -> Self {
        Self {
            version: APP_INSTANCE_PROTOCOL_VERSION,
            command: AppInstanceCommand::Activate,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AppInstanceResponse {
    version: u16,
    accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl AppInstanceResponse {
    fn accepted() -> Self {
        Self {
            version: APP_INSTANCE_PROTOCOL_VERSION,
            accepted: true,
            error: None,
        }
    }

    fn rejected(error: impl Into<String>) -> Self {
        Self {
            version: APP_INSTANCE_PROTOCOL_VERSION,
            accepted: false,
            error: Some(error.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AppInstanceEndpoint {
    #[cfg(windows)]
    WindowsNamedPipe(String),
    #[cfg(unix)]
    UnixSocket(PathBuf),
}

impl AppInstanceEndpoint {
    fn derive(instance_id: &str) -> Self {
        #[cfg(windows)]
        {
            Self::WindowsNamedPipe(format!(r"\\.\pipe\miaominal-app-instance-{instance_id}"))
        }
        #[cfg(unix)]
        {
            let uid = current_effective_uid();
            Self::UnixSocket(
                app_instance_socket_temp_dir()
                    .join(format!("miaominal-{uid}"))
                    .join(instance_id)
                    .join("app.sock"),
            )
        }
    }
}

#[cfg(all(unix, target_os = "macos"))]
fn app_instance_socket_temp_dir() -> PathBuf {
    // macOS gives applications a long per-user TMPDIR under /var/folders, while
    // sockaddr_un::sun_path only has room for 104 bytes. /tmp keeps the socket
    // address short; the UID-specific directory is still ownership-checked and
    // restricted to mode 0700 before the socket is created.
    PathBuf::from("/tmp")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn app_instance_socket_temp_dir() -> PathBuf {
    std::env::temp_dir()
}

pub(crate) enum AppInstanceDisposition {
    Primary(AppInstanceGuard),
    Secondary(AppInstanceClient),
}

pub(crate) struct AppInstanceGuard {
    _lock_file: File,
    instance_id: String,
    endpoint: AppInstanceEndpoint,
}

impl AppInstanceGuard {
    pub(crate) fn acquire(data_dir: &Path) -> Result<AppInstanceDisposition> {
        let canonical_data_dir = std::fs::canonicalize(data_dir)
            .with_context(|| format!("failed to canonicalize {}", data_dir.display()))?;
        let instance_id = instance_id_for_path(&canonical_data_dir);
        let endpoint = AppInstanceEndpoint::derive(&instance_id);
        let lock_path = canonical_data_dir.join(miaominal_paths::APP_INSTANCE_LOCK_FILE);
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock_file = options
            .open(&lock_path)
            .with_context(|| format!("failed to open instance lock {}", lock_path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            lock_file
                .set_permissions(std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("failed to secure {}", lock_path.display()))?;
        }

        match lock_file.try_lock() {
            Ok(()) => Ok(AppInstanceDisposition::Primary(Self {
                _lock_file: lock_file,
                instance_id,
                endpoint,
            })),
            Err(TryLockError::WouldBlock) => {
                Ok(AppInstanceDisposition::Secondary(AppInstanceClient {
                    endpoint,
                }))
            }
            Err(TryLockError::Error(error)) => Err(error)
                .with_context(|| format!("failed to lock instance file {}", lock_path.display())),
        }
    }

    pub(crate) fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub(crate) async fn start_server(
        &self,
        commands: UnboundedSender<AppInstanceCommand>,
    ) -> Result<AppInstanceServer> {
        let listener = PlatformListener::bind(&self.endpoint).await?;
        let (cancel, cancel_receiver) = watch::channel(false);
        let task = tokio::spawn(run_accept_loop(listener, commands, cancel_receiver));
        Ok(AppInstanceServer {
            cancel: Some(cancel),
            task: Some(task),
        })
    }
}

pub(crate) struct AppInstanceClient {
    endpoint: AppInstanceEndpoint,
}

impl AppInstanceClient {
    pub(crate) fn activate_existing_blocking(&self) -> Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .context("failed to start duplicate-instance activation runtime")?;
        runtime.block_on(self.activate_existing())
    }

    async fn activate_existing(&self) -> Result<()> {
        tokio::time::timeout(APP_INSTANCE_CONNECT_TIMEOUT, async {
            let mut stream = PlatformListener::connect(&self.endpoint).await?;
            write_control_frame(&mut stream, &AppInstanceRequest::activate()).await?;
            let response: AppInstanceResponse = read_control_frame(&mut stream).await?;
            if response.version != APP_INSTANCE_PROTOCOL_VERSION {
                bail!(
                    "unsupported app instance response version {}; expected {}",
                    response.version,
                    APP_INSTANCE_PROTOCOL_VERSION
                );
            }
            if !response.accepted {
                bail!(
                    "{}",
                    response
                        .error
                        .unwrap_or_else(|| "running instance rejected activation".into())
                );
            }
            Ok(())
        })
        .await
        .map_err(|_| anyhow!("timed out activating the running Miaominal instance"))?
    }
}

pub(crate) struct AppInstanceServer {
    cancel: Option<watch::Sender<bool>>,
    task: Option<JoinHandle<()>>,
}

impl AppInstanceServer {
    pub(crate) async fn shutdown(mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(true);
        }
        if let Some(mut task) = self.task.take()
            && tokio::time::timeout(Duration::from_secs(2), &mut task)
                .await
                .is_err()
        {
            log::debug!("timed out stopping app instance control server");
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for AppInstanceServer {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(true);
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn instance_id_for_path(path: &Path) -> String {
    let mut digest = Sha256::new();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        digest.update(path.as_os_str().as_bytes());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        for code_unit in path.as_os_str().encode_wide() {
            digest.update(code_unit.to_le_bytes());
        }
    }
    let digest = digest.finalize();
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn run_accept_loop(
    mut listener: PlatformListener,
    commands: UnboundedSender<AppInstanceCommand>,
    mut cancel: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok(stream) => {
                        let commands = commands.clone();
                        tokio::spawn(async move {
                            if let Err(error) = handle_connection(stream, commands).await {
                                log::debug!("app instance control request failed: {error:?}");
                            }
                        });
                    }
                    Err(error) => {
                        log::warn!("app instance control listener stopped: {error:?}");
                        break;
                    }
                }
            }
        }
    }
}

async fn handle_connection(
    mut stream: AppInstanceStream,
    commands: UnboundedSender<AppInstanceCommand>,
) -> Result<()> {
    let response = match tokio::time::timeout(
        APP_INSTANCE_HANDSHAKE_TIMEOUT,
        read_control_frame::<_, AppInstanceRequest>(&mut stream),
    )
    .await
    {
        Err(_) => AppInstanceResponse::rejected("app instance request timed out"),
        Ok(Err(error)) => AppInstanceResponse::rejected(format!("{error:#}")),
        Ok(Ok(request)) if request.version != APP_INSTANCE_PROTOCOL_VERSION => {
            AppInstanceResponse::rejected(format!(
                "unsupported app instance protocol version {}; expected {}",
                request.version, APP_INSTANCE_PROTOCOL_VERSION
            ))
        }
        Ok(Ok(request)) => match commands.unbounded_send(request.command) {
            Ok(()) => AppInstanceResponse::accepted(),
            Err(_) => AppInstanceResponse::rejected("application event loop is unavailable"),
        },
    };
    write_control_frame(&mut stream, &response).await?;
    stream
        .shutdown()
        .await
        .context("failed to close app instance control connection")?;
    Ok(())
}

async fn write_control_frame<W, T>(writer: &mut W, value: &T) -> Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let payload = serde_json::to_vec(value).context("failed to encode app instance frame")?;
    if payload.len() > APP_INSTANCE_MAX_CONTROL_FRAME {
        bail!(
            "app instance frame exceeds {} bytes",
            APP_INSTANCE_MAX_CONTROL_FRAME
        );
    }
    let length = u32::try_from(payload.len()).context("app instance frame is too large")?;
    writer
        .write_all(&length.to_be_bytes())
        .await
        .context("failed to write app instance frame length")?;
    writer
        .write_all(&payload)
        .await
        .context("failed to write app instance frame payload")?;
    writer
        .flush()
        .await
        .context("failed to flush app instance frame")?;
    Ok(())
}

async fn read_control_frame<R, T>(reader: &mut R) -> Result<T>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let length = reader
        .read_u32()
        .await
        .context("failed to read app instance frame length")? as usize;
    if length == 0 {
        bail!("app instance frame is empty");
    }
    if length > APP_INSTANCE_MAX_CONTROL_FRAME {
        bail!(
            "app instance frame exceeds {} bytes",
            APP_INSTANCE_MAX_CONTROL_FRAME
        );
    }
    let mut payload = vec![0; length];
    reader
        .read_exact(&mut payload)
        .await
        .context("failed to read app instance frame payload")?;
    serde_json::from_slice(&payload).context("malformed app instance frame")
}

#[cfg(windows)]
struct PlatformListener {
    pipe_name: String,
    pending: Option<tokio::net::windows::named_pipe::NamedPipeServer>,
}

#[cfg(windows)]
struct WindowsSecurityDescriptor(*mut core::ffi::c_void);

#[cfg(windows)]
impl WindowsSecurityDescriptor {
    fn current_user_only() -> Result<Self> {
        use windows_sys::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };

        let mut sddl = "D:P(A;;GA;;;OW)(A;;GA;;;SY)"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let mut descriptor = std::ptr::null_mut();
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_mut_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if converted == 0 {
            return Err(std::io::Error::last_os_error())
                .context("failed to build current-user app instance pipe ACL");
        }
        Ok(Self(descriptor))
    }
}

#[cfg(windows)]
impl Drop for WindowsSecurityDescriptor {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::LocalFree(self.0);
        }
    }
}

#[cfg(windows)]
fn create_secure_named_pipe(
    pipe_name: &str,
    first_instance: bool,
) -> Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    use tokio::net::windows::named_pipe::ServerOptions;
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;

    let descriptor = WindowsSecurityDescriptor::current_user_only()?;
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first_instance)
        .reject_remote_clients(true);
    let server = unsafe {
        options.create_with_security_attributes_raw(
            pipe_name,
            (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
        )
    }
    .with_context(|| format!("failed to bind app instance named pipe {pipe_name}"))?;
    Ok(server)
}

#[cfg(windows)]
impl PlatformListener {
    async fn bind(endpoint: &AppInstanceEndpoint) -> Result<Self> {
        let AppInstanceEndpoint::WindowsNamedPipe(pipe_name) = endpoint;
        let pending = create_secure_named_pipe(pipe_name, true)?;
        Ok(Self {
            pipe_name: pipe_name.clone(),
            pending: Some(pending),
        })
    }

    async fn accept(&mut self) -> Result<AppInstanceStream> {
        if self.pending.is_none() {
            self.pending = Some(create_secure_named_pipe(&self.pipe_name, false)?);
        }
        let server = self
            .pending
            .as_mut()
            .ok_or_else(|| anyhow!("app instance named-pipe listener is unavailable"))?;
        server
            .connect()
            .await
            .with_context(|| format!("failed to accept app instance pipe {}", self.pipe_name))?;
        let server = self
            .pending
            .take()
            .ok_or_else(|| anyhow!("app instance named-pipe listener is unavailable"))?;
        self.pending = Some(create_secure_named_pipe(&self.pipe_name, false)?);
        Ok(Box::new(server))
    }

    async fn connect(endpoint: &AppInstanceEndpoint) -> Result<AppInstanceStream> {
        use tokio::net::windows::named_pipe::ClientOptions;
        use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY};

        let AppInstanceEndpoint::WindowsNamedPipe(pipe_name) = endpoint;
        let deadline = tokio::time::Instant::now() + APP_INSTANCE_CONNECT_TIMEOUT;
        loop {
            match ClientOptions::new().open(pipe_name) {
                Ok(client) => return Ok(Box::new(client)),
                Err(error)
                    if matches!(
                        error.raw_os_error().map(|code| code as u32),
                        Some(ERROR_FILE_NOT_FOUND) | Some(ERROR_PIPE_BUSY)
                    ) && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(APP_INSTANCE_RETRY_DELAY).await;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to connect to app instance pipe {pipe_name}")
                    });
                }
            }
        }
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UnixSocketIdentity {
    device: u64,
    inode: u64,
    uid: u32,
}

#[cfg(unix)]
fn current_effective_uid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(unix)]
fn unix_socket_identity(path: &Path) -> Result<Option<UnixSocketIdentity>> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
        }
    };
    if !metadata.file_type().is_socket() {
        bail!(
            "app instance endpoint {} is not a Unix socket",
            path.display()
        );
    }
    Ok(Some(UnixSocketIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        uid: metadata.uid(),
    }))
}

#[cfg(unix)]
fn secure_unix_directory(path: &Path, current_uid: u32) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    std::fs::create_dir_all(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        bail!(
            "app instance directory {} is not a real directory",
            path.display()
        );
    }
    if metadata.uid() != current_uid {
        bail!(
            "app instance directory {} is owned by uid {}, expected {}",
            path.display(),
            metadata.uid(),
            current_uid
        );
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure {}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
struct PlatformListener {
    listener: tokio::net::UnixListener,
    socket_path: PathBuf,
    socket_identity: UnixSocketIdentity,
}

#[cfg(unix)]
impl PlatformListener {
    async fn bind(endpoint: &AppInstanceEndpoint) -> Result<Self> {
        use std::os::unix::fs::PermissionsExt;

        let AppInstanceEndpoint::UnixSocket(socket_path) = endpoint;
        let parent = socket_path
            .parent()
            .ok_or_else(|| anyhow!("app instance socket has no parent"))?;
        let user_root = parent
            .parent()
            .ok_or_else(|| anyhow!("app instance socket has no user root"))?;
        let current_uid = current_effective_uid();
        secure_unix_directory(user_root, current_uid)?;
        secure_unix_directory(parent, current_uid)?;

        if let Some(observed) = unix_socket_identity(socket_path)? {
            if observed.uid != current_uid {
                bail!(
                    "app instance socket {} is owned by uid {}, expected {}",
                    socket_path.display(),
                    observed.uid,
                    current_uid
                );
            }
            match tokio::net::UnixStream::connect(socket_path).await {
                Ok(_) => bail!(
                    "another process already owns app instance socket {}",
                    socket_path.display()
                ),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                    ) =>
                {
                    match unix_socket_identity(socket_path)? {
                        None if error.kind() == std::io::ErrorKind::NotFound => {}
                        Some(current) if current == observed => {
                            std::fs::remove_file(socket_path).with_context(|| {
                                format!(
                                    "failed to remove stale app instance socket {}",
                                    socket_path.display()
                                )
                            })?;
                        }
                        _ => bail!(
                            "app instance socket {} changed during stale inspection",
                            socket_path.display()
                        ),
                    }
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "failed to inspect app instance socket {}",
                            socket_path.display()
                        )
                    });
                }
            }
        }

        let listener = tokio::net::UnixListener::bind(socket_path)
            .with_context(|| format!("failed to bind {}", socket_path.display()))?;
        std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to secure {}", socket_path.display()))?;
        let socket_identity = unix_socket_identity(socket_path)?
            .ok_or_else(|| anyhow!("app instance socket disappeared after bind"))?;
        Ok(Self {
            listener,
            socket_path: socket_path.clone(),
            socket_identity,
        })
    }

    async fn accept(&mut self) -> Result<AppInstanceStream> {
        let (stream, _) = self.listener.accept().await.with_context(|| {
            format!(
                "failed to accept app instance socket {}",
                self.socket_path.display()
            )
        })?;
        Ok(Box::new(stream))
    }

    async fn connect(endpoint: &AppInstanceEndpoint) -> Result<AppInstanceStream> {
        let AppInstanceEndpoint::UnixSocket(socket_path) = endpoint;
        let deadline = tokio::time::Instant::now() + APP_INSTANCE_CONNECT_TIMEOUT;
        loop {
            match tokio::net::UnixStream::connect(socket_path).await {
                Ok(stream) => return Ok(Box::new(stream)),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                    ) && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(APP_INSTANCE_RETRY_DELAY).await;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "failed to connect to app instance socket {}",
                            socket_path.display()
                        )
                    });
                }
            }
        }
    }
}

#[cfg(unix)]
impl Drop for PlatformListener {
    fn drop(&mut self) {
        match unix_socket_identity(&self.socket_path) {
            Ok(Some(identity)) if identity == self.socket_identity => {
                if let Err(error) = std::fs::remove_file(&self.socket_path)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    log::debug!("failed to remove app instance socket: {error:?}");
                }
            }
            Ok(Some(_)) => {
                log::debug!("app instance socket changed before cleanup; keeping replacement");
            }
            Ok(None) => {}
            Err(error) => log::debug!("failed to inspect app instance socket: {error:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn primary(disposition: AppInstanceDisposition) -> AppInstanceGuard {
        match disposition {
            AppInstanceDisposition::Primary(guard) => guard,
            AppInstanceDisposition::Secondary(_) => panic!("expected primary instance"),
        }
    }

    fn secondary(disposition: AppInstanceDisposition) -> AppInstanceClient {
        match disposition {
            AppInstanceDisposition::Primary(_) => panic!("expected secondary instance"),
            AppInstanceDisposition::Secondary(client) => client,
        }
    }

    #[test]
    fn same_directory_has_only_one_owner_and_releases_on_drop() {
        let directory = tempfile::tempdir().unwrap();
        let first = primary(AppInstanceGuard::acquire(directory.path()).unwrap());
        let _second = secondary(AppInstanceGuard::acquire(directory.path()).unwrap());
        drop(first);
        let _replacement = primary(AppInstanceGuard::acquire(directory.path()).unwrap());
    }

    #[test]
    fn different_directories_can_have_independent_owners() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let _first = primary(AppInstanceGuard::acquire(first.path()).unwrap());
        let _second = primary(AppInstanceGuard::acquire(second.path()).unwrap());
    }

    #[test]
    fn endpoint_identity_is_stable_and_directory_specific() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let first_path = std::fs::canonicalize(first.path()).unwrap();
        let second_path = std::fs::canonicalize(second.path()).unwrap();
        let first_id = instance_id_for_path(&first_path);
        assert_eq!(first_id, instance_id_for_path(&first_path));
        assert_ne!(first_id, instance_id_for_path(&second_path));
        assert_eq!(first_id.len(), 32);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_endpoint_uses_short_tmp_socket_path() {
        use std::os::unix::ffi::OsStrExt;

        let endpoint = AppInstanceEndpoint::derive("0123456789abcdef0123456789abcdef");
        let AppInstanceEndpoint::UnixSocket(socket_path) = endpoint;

        assert!(socket_path.starts_with("/tmp"));
        assert!(socket_path.as_os_str().as_bytes().len() < 104);
    }

    #[tokio::test]
    async fn duplicate_instance_can_request_activation() {
        let directory = tempfile::tempdir().unwrap();
        let owner = primary(AppInstanceGuard::acquire(directory.path()).unwrap());
        let client = secondary(AppInstanceGuard::acquire(directory.path()).unwrap());
        let (commands, mut receiver) = futures::channel::mpsc::unbounded();
        let server = owner.start_server(commands).await.unwrap();

        client.activate_existing().await.unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), receiver.next())
                .await
                .unwrap(),
            Some(AppInstanceCommand::Activate)
        );

        server.shutdown().await;
    }

    #[tokio::test]
    async fn endpoint_conflict_prevents_instance_server_start() {
        let directory = tempfile::tempdir().unwrap();
        let owner = primary(AppInstanceGuard::acquire(directory.path()).unwrap());
        let _conflicting_listener = PlatformListener::bind(&owner.endpoint).await.unwrap();
        let (commands, _receiver) = futures::channel::mpsc::unbounded();

        assert!(owner.start_server(commands).await.is_err());
    }

    #[tokio::test]
    async fn malformed_and_oversized_frames_are_rejected() {
        let (mut writer, mut reader) = tokio::io::duplex(2048);
        writer.write_all(&5_u32.to_be_bytes()).await.unwrap();
        writer.write_all(b"nope!").await.unwrap();
        let error = read_control_frame::<_, AppInstanceRequest>(&mut reader)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("malformed"));

        let (mut writer, mut reader) = tokio::io::duplex(16);
        writer
            .write_all(&((APP_INSTANCE_MAX_CONTROL_FRAME + 1) as u32).to_be_bytes())
            .await
            .unwrap();
        let error = read_control_frame::<_, AppInstanceRequest>(&mut reader)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("exceeds"));
    }

    #[tokio::test]
    async fn unsupported_protocol_version_is_rejected_without_dispatch() {
        let (client, server) = tokio::io::duplex(2048);
        let (commands, mut receiver) = futures::channel::mpsc::unbounded();
        let task = tokio::spawn(handle_connection(Box::new(server), commands));
        let mut client: AppInstanceStream = Box::new(client);
        write_control_frame(
            &mut client,
            &AppInstanceRequest {
                version: APP_INSTANCE_PROTOCOL_VERSION + 1,
                command: AppInstanceCommand::Activate,
            },
        )
        .await
        .unwrap();
        let response: AppInstanceResponse = read_control_frame(&mut client).await.unwrap();
        assert!(!response.accepted);
        assert!(response.error.unwrap().contains("unsupported"));
        task.await.unwrap().unwrap();
        assert_eq!(receiver.next().await, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_listener_recovers_stale_socket_and_preserves_replacement() {
        use std::os::unix::net::UnixListener as StdUnixListener;

        let directory = tempfile::tempdir().unwrap();
        let instance_id = instance_id_for_path(&std::fs::canonicalize(directory.path()).unwrap());
        let endpoint = AppInstanceEndpoint::derive(&instance_id);
        let AppInstanceEndpoint::UnixSocket(socket_path) = &endpoint;
        let parent = socket_path.parent().unwrap();
        let user_root = parent.parent().unwrap();
        secure_unix_directory(user_root, current_effective_uid()).unwrap();
        secure_unix_directory(parent, current_effective_uid()).unwrap();
        drop(StdUnixListener::bind(socket_path).unwrap());

        let listener = PlatformListener::bind(&endpoint).await.unwrap();
        std::fs::remove_file(socket_path).unwrap();
        let replacement = StdUnixListener::bind(socket_path).unwrap();
        drop(listener);
        assert!(socket_path.exists());
        drop(replacement);
        std::fs::remove_file(socket_path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unix_non_utf8_paths_have_distinct_instance_ids() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let mut first = b"/tmp/miaominal-invalid-".to_vec();
        first.push(0x80);
        let mut second = b"/tmp/miaominal-invalid-".to_vec();
        second.push(0x81);
        let first = PathBuf::from(OsString::from_vec(first));
        let second = PathBuf::from(OsString::from_vec(second));

        assert_eq!(first.to_string_lossy(), second.to_string_lossy());
        assert_ne!(instance_id_for_path(&first), instance_id_for_path(&second));
    }
}
