use super::bridge::{
    SSH_BRIDGE_MAX_CONTROL_FRAME, SSH_BRIDGE_PROTOCOL_VERSION, SshBridgeEndpoint, SshBridgeRoute,
};
use anyhow::{Context, Result, anyhow, bail};
use miaominal_core::ssh_bridge_security::{
    BridgePeerIdentity, BridgeProcessCaptureStatus, BridgeProcessIdentity,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::ffi::OsString;
#[cfg(any(windows, unix))]
use std::io;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const SSH_BRIDGE_ROUTE_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub trait SshBridgeIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> SshBridgeIo for T where T: AsyncRead + AsyncWrite + Unpin + Send {}
pub type SshBridgeStream = Box<dyn SshBridgeIo>;

pub struct SshBridgeConnection {
    pub stream: SshBridgeStream,
    pub peer: BridgePeerIdentity,
}

impl AsyncRead for SshBridgeConnection {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut *self.stream).poll_read(cx, buffer)
    }
}

impl AsyncWrite for SshBridgeConnection {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut *self.stream).poll_write(cx, buffer)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut *self.stream).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut *self.stream).poll_shutdown(cx)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshBridgeRouteRequest {
    pub version: u16,
    pub route: String,
}

impl SshBridgeRouteRequest {
    pub fn new(route: impl Into<String>) -> Self {
        Self {
            version: SSH_BRIDGE_PROTOCOL_VERSION,
            route: route.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshBridgeRouteResponse {
    pub version: u16,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SshBridgeRouteResponse {
    fn success() -> Self {
        Self {
            version: SSH_BRIDGE_PROTOCOL_VERSION,
            ok: true,
            error: None,
        }
    }

    fn failure(message: impl Into<String>) -> Self {
        Self {
            version: SSH_BRIDGE_PROTOCOL_VERSION,
            ok: false,
            error: Some(message.into()),
        }
    }

    fn bounded_failure(message: &str) -> Self {
        let response = Self::failure(message);
        if serde_json::to_vec(&response)
            .is_ok_and(|payload| payload.len() <= SSH_BRIDGE_MAX_CONTROL_FRAME)
        {
            return response;
        }

        const TRUNCATED_SUFFIX: &str = "…";
        let mut boundaries = message
            .char_indices()
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        boundaries.push(message.len());
        let mut low = 0;
        let mut high = boundaries.len().saturating_sub(1);
        while low < high {
            let middle = (low + high).div_ceil(2);
            let candidate = format!("{}{}", &message[..boundaries[middle]], TRUNCATED_SUFFIX);
            let fits = serde_json::to_vec(&Self::failure(candidate))
                .is_ok_and(|payload| payload.len() <= SSH_BRIDGE_MAX_CONTROL_FRAME);
            if fits {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        Self::failure(format!(
            "{}{}",
            &message[..boundaries[low]],
            TRUNCATED_SUFFIX
        ))
    }
}

#[derive(Clone, Default)]
pub struct SshBridgeRouteTable {
    routes: Arc<RwLock<HashMap<String, SshBridgeRoute>>>,
}

impl SshBridgeRouteTable {
    pub fn replace(&self, routes: impl IntoIterator<Item = SshBridgeRoute>) {
        let routes = routes
            .into_iter()
            .map(|route| (route.token.clone(), route))
            .collect();
        if let Ok(mut guard) = self.routes.write() {
            *guard = routes;
        }
    }

    pub fn resolve(&self, token: &str) -> Result<SshBridgeRoute> {
        self.routes
            .read()
            .map_err(|_| anyhow!("SSH Bridge route table lock is poisoned"))?
            .get(token)
            .cloned()
            .ok_or_else(|| anyhow!("unknown SSH Bridge route token"))
    }

    pub fn len(&self) -> usize {
        self.routes.read().map(|routes| routes.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub async fn write_control_frame<W, T>(writer: &mut W, value: &T) -> Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let payload = serde_json::to_vec(value).context("failed to encode SSH Bridge control frame")?;
    if payload.len() > SSH_BRIDGE_MAX_CONTROL_FRAME {
        bail!(
            "SSH Bridge control frame exceeds {} bytes",
            SSH_BRIDGE_MAX_CONTROL_FRAME
        );
    }
    let length = u32::try_from(payload.len()).context("SSH Bridge control frame is too large")?;
    writer
        .write_all(&length.to_be_bytes())
        .await
        .context("failed to write SSH Bridge control frame length")?;
    writer
        .write_all(&payload)
        .await
        .context("failed to write SSH Bridge control frame payload")?;
    writer
        .flush()
        .await
        .context("failed to flush SSH Bridge control frame")?;
    Ok(())
}

pub async fn read_control_frame<R, T>(reader: &mut R) -> Result<T>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let length = reader
        .read_u32()
        .await
        .context("failed to read SSH Bridge control frame length")? as usize;
    if length == 0 {
        bail!("SSH Bridge control frame is empty");
    }
    if length > SSH_BRIDGE_MAX_CONTROL_FRAME {
        bail!(
            "SSH Bridge control frame exceeds {} bytes",
            SSH_BRIDGE_MAX_CONTROL_FRAME
        );
    }
    let mut payload = vec![0; length];
    reader
        .read_exact(&mut payload)
        .await
        .context("failed to read SSH Bridge control frame payload")?;
    serde_json::from_slice(&payload).context("malformed SSH Bridge control frame")
}

pub async fn accept_route_request(
    stream: &mut SshBridgeStream,
    routes: &SshBridgeRouteTable,
) -> Result<SshBridgeRoute> {
    accept_route_request_with(stream, routes, |route| async move { Ok(route) }).await
}

pub async fn accept_route_request_with<T, F, Fut>(
    stream: &mut SshBridgeStream,
    routes: &SshBridgeRouteTable,
    prepare: F,
) -> Result<T>
where
    F: FnOnce(SshBridgeRoute) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    accept_route_request_with_timeout(stream, routes, SSH_BRIDGE_ROUTE_HANDSHAKE_TIMEOUT, prepare)
        .await
}

async fn accept_route_request_with_timeout<T, F, Fut>(
    stream: &mut SshBridgeStream,
    routes: &SshBridgeRouteTable,
    handshake_timeout: Duration,
    prepare: F,
) -> Result<T>
where
    F: FnOnce(SshBridgeRoute) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let result = async {
        let request: SshBridgeRouteRequest =
            tokio::time::timeout(handshake_timeout, read_control_frame(stream))
                .await
                .map_err(|_| anyhow!("SSH Bridge control frame timed out"))??;
        if request.version != SSH_BRIDGE_PROTOCOL_VERSION {
            bail!(
                "unsupported SSH Bridge protocol version {}; expected {}",
                request.version,
                SSH_BRIDGE_PROTOCOL_VERSION
            );
        }
        if request.route.len() != 32 || !request.route.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("malformed SSH Bridge route token");
        }
        let route = routes.resolve(&request.route)?;
        let mut unexpected = [0_u8; 1];
        tokio::select! {
            biased;
            read = stream.read(&mut unexpected) => {
                match read.context("failed while monitoring SSH Bridge client during route preparation")? {
                    0 => bail!("SSH Bridge client disconnected before route preparation completed"),
                    _ => bail!("SSH Bridge client sent data before route preparation completed"),
                }
            }
            prepared = prepare(route) => prepared,
        }
    }
    .await;

    match result {
        Ok(value) => {
            write_control_frame(stream, &SshBridgeRouteResponse::success()).await?;
            Ok(value)
        }
        Err(error) => {
            let message = format!("{error:#}");
            let response = SshBridgeRouteResponse::bounded_failure(&message);
            let _ = write_control_frame(stream, &response).await;
            #[cfg(windows)]
            {
                // The transport ends after a failure response. This byte is outside the framed
                // protocol and exists only to wake a Windows named-pipe client blocked in an
                // overlapped read. The helper consumes the bounded failure frame and exits.
                let _ = stream.write_all(&[0]).await;
                let _ = stream.flush().await;
            }
            let _ = stream.shutdown().await;
            Err(error)
        }
    }
}

pub async fn request_route(stream: &mut SshBridgeStream, route: &str) -> Result<()> {
    write_control_frame(stream, &SshBridgeRouteRequest::new(route)).await?;
    let response: SshBridgeRouteResponse = read_control_frame(stream).await?;
    if response.version != SSH_BRIDGE_PROTOCOL_VERSION {
        bail!(
            "unsupported SSH Bridge response version {}; expected {}",
            response.version,
            SSH_BRIDGE_PROTOCOL_VERSION
        );
    }
    if !response.ok {
        bail!(
            "{}",
            response
                .error
                .unwrap_or_else(|| "SSH Bridge rejected the route".into())
        );
    }
    Ok(())
}

pub struct SshBridgeListener {
    inner: PlatformListener,
}

impl SshBridgeListener {
    pub async fn bind(endpoint: &SshBridgeEndpoint) -> Result<Self> {
        Ok(Self {
            inner: PlatformListener::bind(endpoint).await?,
        })
    }

    pub async fn accept(&mut self) -> Result<SshBridgeConnection> {
        self.inner.accept().await
    }
}

pub async fn connect_endpoint(endpoint: &SshBridgeEndpoint) -> Result<SshBridgeStream> {
    PlatformListener::connect(endpoint).await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshBridgeHelperArgs {
    pub endpoint: SshBridgeEndpoint,
    pub route: String,
}

pub fn parse_ssh_bridge_helper_args(
    args: impl IntoIterator<Item = OsString>,
) -> Result<Option<SshBridgeHelperArgs>> {
    let mut args = args.into_iter();
    let _executable = args.next();
    let Some(command) = args.next() else {
        return Ok(None);
    };
    if command != "ssh-bridge-helper" {
        return Ok(None);
    }

    let mut endpoint = None;
    let mut route = None;
    while let Some(argument) = args.next() {
        let argument = argument
            .into_string()
            .map_err(|_| anyhow!("SSH Bridge helper arguments must be valid Unicode"))?;
        let value = args
            .next()
            .ok_or_else(|| anyhow!("missing value for SSH Bridge helper option {argument}"))?
            .into_string()
            .map_err(|_| anyhow!("SSH Bridge helper arguments must be valid Unicode"))?;
        match argument.as_str() {
            "--endpoint" if endpoint.is_none() => {
                endpoint = Some(SshBridgeEndpoint::from_helper_value(&value)?);
            }
            "--route" if route.is_none() => route = Some(value),
            "--endpoint" | "--route" => bail!("duplicate SSH Bridge helper option {argument}"),
            _ => bail!("unknown SSH Bridge helper option {argument}"),
        }
    }

    let endpoint = endpoint.ok_or_else(|| anyhow!("SSH Bridge helper requires --endpoint"))?;
    let route = route.ok_or_else(|| anyhow!("SSH Bridge helper requires --route"))?;
    if route.len() != 32 || !route.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("SSH Bridge helper route token is malformed");
    }
    Ok(Some(SshBridgeHelperArgs { endpoint, route }))
}

pub async fn run_ssh_bridge_helper(args: SshBridgeHelperArgs) -> Result<()> {
    let mut stream = connect_endpoint(&args.endpoint).await?;
    request_route(&mut stream, &args.route).await?;
    relay_bridge_stdio(tokio::io::stdin(), tokio::io::stdout(), stream).await
}

async fn relay_bridge_stdio<R, W>(
    mut stdin: R,
    mut stdout: W,
    stream: SshBridgeStream,
) -> Result<()>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    let (mut bridge_reader, mut bridge_writer) = tokio::io::split(stream);
    let upload = async {
        tokio::io::copy(&mut stdin, &mut bridge_writer)
            .await
            .context("failed to relay OpenSSH stdin to Miaominal")?;
        bridge_writer
            .shutdown()
            .await
            .context("failed to close SSH Bridge upload stream")?;
        Result::<()>::Ok(())
    };
    let download = async {
        tokio::io::copy(&mut bridge_reader, &mut stdout)
            .await
            .context("failed to relay Miaominal SSH bytes to OpenSSH stdout")?;
        stdout
            .flush()
            .await
            .context("failed to flush OpenSSH stdout")?;
        Result::<()>::Ok(())
    };
    tokio::try_join!(upload, download)?;
    Ok(())
}

#[cfg(windows)]
struct PlatformListener {
    pipe_name: String,
    pending: Option<tokio::net::windows::named_pipe::NamedPipeServer>,
    #[cfg(test)]
    fail_next_instance_creation: bool,
    #[cfg(test)]
    fail_next_connect: bool,
}

#[cfg(windows)]
struct WindowsSecurityDescriptor(*mut core::ffi::c_void);

#[cfg(windows)]
impl WindowsSecurityDescriptor {
    fn current_user_only() -> Result<Self> {
        use windows_sys::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };

        // Protected DACL: full access for the object owner (the current user)
        // and LocalSystem. Remote named-pipe clients are rejected separately.
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
            return Err(io::Error::last_os_error())
                .context("failed to build current-user SSH Bridge pipe ACL");
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
    .with_context(|| format!("failed to bind SSH Bridge named pipe {pipe_name}"))?;
    Ok(server)
}

#[cfg(windows)]
impl PlatformListener {
    async fn bind(endpoint: &SshBridgeEndpoint) -> Result<Self> {
        let SshBridgeEndpoint::WindowsNamedPipe(pipe_name) = endpoint else {
            bail!("expected a Windows named-pipe SSH Bridge endpoint");
        };
        let pending = create_secure_named_pipe(pipe_name, true)?;
        Ok(Self {
            pipe_name: pipe_name.clone(),
            pending: Some(pending),
            #[cfg(test)]
            fail_next_instance_creation: false,
            #[cfg(test)]
            fail_next_connect: false,
        })
    }

    async fn accept(&mut self) -> Result<SshBridgeConnection> {
        if self.pending.is_none() {
            self.pending = Some(self.create_next_instance()?);
        }
        #[cfg(test)]
        let injected_failure = std::mem::take(&mut self.fail_next_connect);
        #[cfg(not(test))]
        let injected_failure = false;
        let connect_result = if injected_failure {
            Err(std::io::Error::other(
                "injected SSH Bridge named-pipe connect failure",
            ))
        } else {
            self.pending
                .as_ref()
                .ok_or_else(|| anyhow!("SSH Bridge named-pipe listener is unavailable"))?
                .connect()
                .await
        };
        if let Err(error) = connect_result {
            if let Some(server) = self.pending.take() {
                let _ = server.disconnect();
                drop(server);
            }
            self.pending = Some(self.create_next_instance()?);
            return Err(error)
                .with_context(|| format!("failed to accept SSH Bridge pipe {}", self.pipe_name));
        }
        let server = self
            .pending
            .take()
            .ok_or_else(|| anyhow!("SSH Bridge named-pipe listener is unavailable"))?;
        let peer = capture_windows_peer_identity(&server);
        let next = self.create_next_instance()?;
        self.pending = Some(next);
        Ok(SshBridgeConnection {
            stream: Box::new(server),
            peer,
        })
    }

    fn create_next_instance(&mut self) -> Result<tokio::net::windows::named_pipe::NamedPipeServer> {
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_instance_creation) {
            bail!("injected SSH Bridge next-instance creation failure");
        }
        create_secure_named_pipe(&self.pipe_name, false)
            .with_context(|| format!("failed to create next SSH Bridge pipe {}", self.pipe_name))
    }

    async fn connect(endpoint: &SshBridgeEndpoint) -> Result<SshBridgeStream> {
        use tokio::net::windows::named_pipe::ClientOptions;

        let SshBridgeEndpoint::WindowsNamedPipe(pipe_name) = endpoint else {
            bail!("expected a Windows named-pipe SSH Bridge endpoint");
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match ClientOptions::new().open(pipe_name) {
                Ok(client) => return Ok(Box::new(client)),
                Err(error) if error.raw_os_error() == Some(231) => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(error).with_context(|| {
                            format!("timed out connecting to busy SSH Bridge pipe {pipe_name}")
                        });
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to connect to SSH Bridge pipe {pipe_name}")
                    });
                }
            }
        }
    }
}

#[cfg(windows)]
fn capture_windows_peer_identity(
    server: &tokio::net::windows::named_pipe::NamedPipeServer,
) -> BridgePeerIdentity {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId;

    let mut pid = 0_u32;
    let captured = unsafe { GetNamedPipeClientProcessId(server.as_raw_handle().cast(), &mut pid) };
    if captured == 0 {
        let error = std::io::Error::last_os_error();
        return BridgePeerIdentity {
            capture_status: Some(classify_windows_capture_error(&error)),
            ..BridgePeerIdentity::default()
        };
    }
    if pid == 0 {
        return BridgePeerIdentity {
            capture_status: Some(BridgeProcessCaptureStatus::Failed),
            ..BridgePeerIdentity::default()
        };
    }
    BridgePeerIdentity {
        pid: Some(pid),
        user_sid: windows_process_user_sid(pid),
        process_chain: windows_process_chain(pid),
        capture_status: Some(BridgeProcessCaptureStatus::Captured),
        ..BridgePeerIdentity::default()
    }
}

#[cfg(windows)]
fn windows_process_chain(start_pid: u32) -> Vec<BridgeProcessIdentity> {
    use std::collections::{HashMap, HashSet};
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return vec![capture_windows_process(start_pid, None)];
    }
    let mut parents = HashMap::new();
    let mut entry = unsafe { std::mem::zeroed::<PROCESSENTRY32W>() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut has_entry = unsafe { Process32FirstW(snapshot, &mut entry) != 0 };
    while has_entry {
        parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
        has_entry = unsafe { Process32NextW(snapshot, &mut entry) != 0 };
    }
    unsafe { CloseHandle(snapshot) };

    let mut chain = Vec::new();
    let mut current = start_pid;
    let mut visited = HashSet::new();
    while current != 0 && chain.len() < 8 && visited.insert(current) {
        let parent = parents.get(&current).copied().filter(|pid| *pid != 0);
        chain.push(capture_windows_process(current, parent));
        let Some(parent) = parent else {
            break;
        };
        current = parent;
    }
    chain
}

#[cfg(windows)]
fn capture_windows_process(pid: u32, parent_pid: Option<u32>) -> BridgeProcessIdentity {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        let error = std::io::Error::last_os_error();
        return BridgeProcessIdentity {
            pid,
            parent_pid,
            started_at: None,
            executable_path: None,
            capture_status: classify_windows_capture_error(&error),
        };
    }
    let mut path = vec![0_u16; 32_768];
    let mut path_len = path.len() as u32;
    let path_captured =
        unsafe { QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut path_len) };
    let (executable_path, path_error) = if path_captured != 0 {
        (
            Some(String::from_utf16_lossy(&path[..path_len as usize])),
            None,
        )
    } else {
        (None, Some(std::io::Error::last_os_error()))
    };
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let times_captured =
        unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
    let (started_at, times_error) = if times_captured != 0 {
        (
            windows_filetime_to_unix_seconds(
                ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64,
            ),
            None,
        )
    } else {
        (None, Some(std::io::Error::last_os_error()))
    };
    unsafe { CloseHandle(handle) };
    let capture_status = path_error
        .as_ref()
        .or(times_error.as_ref())
        .map(classify_windows_capture_error)
        .unwrap_or(BridgeProcessCaptureStatus::Captured);
    BridgeProcessIdentity {
        pid,
        parent_pid,
        started_at,
        executable_path,
        capture_status,
    }
}

#[cfg(windows)]
fn windows_filetime_to_unix_seconds(filetime: u64) -> Option<u64> {
    const WINDOWS_TO_UNIX_EPOCH_SECS: u64 = 11_644_473_600;
    filetime
        .checked_div(10_000_000)
        .and_then(|seconds| seconds.checked_sub(WINDOWS_TO_UNIX_EPOCH_SECS))
}

#[cfg(windows)]
fn windows_process_user_sid(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        unsafe { CloseHandle(process) };
        return None;
    }
    let mut size = 0_u32;
    unsafe {
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
    }
    let mut buffer = vec![0_u8; size as usize];
    let ok = size > 0
        && unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                size,
                &mut size,
            ) != 0
        };
    let sid = if ok {
        let token_user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
        let mut string_sid = std::ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut string_sid) } != 0 {
            let mut len = 0;
            while unsafe { *string_sid.add(len) } != 0 {
                len += 1;
            }
            let value =
                String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(string_sid, len) });
            unsafe { LocalFree(string_sid.cast()) };
            Some(value)
        } else {
            None
        }
    } else {
        None
    };
    unsafe {
        CloseHandle(token);
        CloseHandle(process);
    }
    sid
}

#[cfg(windows)]
fn classify_windows_capture_error(error: &std::io::Error) -> BridgeProcessCaptureStatus {
    classify_windows_capture_error_code(error.raw_os_error())
}

#[cfg(windows)]
fn classify_windows_capture_error_code(error_code: Option<i32>) -> BridgeProcessCaptureStatus {
    match error_code {
        Some(5) => BridgeProcessCaptureStatus::AccessDenied,
        Some(87) | Some(1168) => BridgeProcessCaptureStatus::Exited,
        _ => BridgeProcessCaptureStatus::Failed,
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
fn unix_socket_identity(path: &std::path::Path) -> Result<Option<UnixSocketIdentity>> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
        }
    };
    if !metadata.file_type().is_socket() {
        bail!(
            "SSH Bridge endpoint {} is not a Unix socket",
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
struct PlatformListener {
    listener: tokio::net::UnixListener,
    socket_path: std::path::PathBuf,
    socket_identity: UnixSocketIdentity,
    _lock_file: std::fs::File,
}

#[cfg(unix)]
impl PlatformListener {
    async fn bind(endpoint: &SshBridgeEndpoint) -> Result<Self> {
        use std::fs::OpenOptions;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let SshBridgeEndpoint::UnixSocket(socket_path) = endpoint else {
            bail!("expected a Unix-socket SSH Bridge endpoint");
        };
        let parent = socket_path
            .parent()
            .ok_or_else(|| anyhow!("SSH Bridge socket path has no parent"))?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("failed to secure {}", parent.display()))?;

        // Keep this advisory lock for the listener lifetime. It serializes stale-socket cleanup
        // and bind across normal Miaominal instances, closing the check/remove/bind race.
        let lock_path = socket_path.with_extension("sock.lock");
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("failed to open SSH Bridge lock {}", lock_path.display()))?;
        std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to secure {}", lock_path.display()))?;
        match lock_file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                bail!(
                    "another Miaominal instance owns SSH Bridge lock {}",
                    lock_path.display()
                );
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(error)
                    .with_context(|| format!("failed to lock {}", lock_path.display()));
            }
        }
        let lock_owner = lock_file
            .metadata()
            .with_context(|| format!("failed to inspect {}", lock_path.display()))?
            .uid();
        let current_uid = current_effective_uid();
        if lock_owner != current_uid {
            bail!(
                "SSH Bridge lock {} is owned by uid {}, expected current uid {}",
                lock_path.display(),
                lock_owner,
                current_uid
            );
        }

        if let Some(observed) = unix_socket_identity(socket_path)? {
            if observed.uid != current_uid {
                bail!(
                    "refusing to remove SSH Bridge socket {} owned by uid {}; current uid is {}",
                    socket_path.display(),
                    observed.uid,
                    current_uid
                );
            }
            match tokio::net::UnixStream::connect(socket_path).await {
                Ok(_) => bail!(
                    "another Miaominal instance already owns SSH Bridge socket {}",
                    socket_path.display()
                ),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                    ) =>
                {
                    match unix_socket_identity(socket_path)? {
                        None if error.kind() == io::ErrorKind::NotFound => {}
                        Some(current) if current == observed => {
                            std::fs::remove_file(socket_path).with_context(|| {
                                format!("failed to remove stale socket {}", socket_path.display())
                            })?;
                        }
                        _ => bail!(
                            "SSH Bridge socket {} changed during stale-socket inspection",
                            socket_path.display()
                        ),
                    }
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "failed to inspect SSH Bridge socket {}",
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
            .ok_or_else(|| anyhow!("SSH Bridge socket disappeared after bind"))?;
        if socket_identity.uid != current_uid {
            bail!(
                "SSH Bridge socket {} has unexpected owner uid {}",
                socket_path.display(),
                socket_identity.uid
            );
        }
        Ok(Self {
            listener,
            socket_path: socket_path.clone(),
            socket_identity,
            _lock_file: lock_file,
        })
    }

    async fn accept(&mut self) -> Result<SshBridgeConnection> {
        let (stream, _) = self.listener.accept().await.with_context(|| {
            format!(
                "failed to accept SSH Bridge socket {}",
                self.socket_path.display()
            )
        })?;
        let peer = capture_unix_peer_identity(&stream);
        Ok(SshBridgeConnection {
            stream: Box::new(stream),
            peer,
        })
    }

    async fn connect(endpoint: &SshBridgeEndpoint) -> Result<SshBridgeStream> {
        let SshBridgeEndpoint::UnixSocket(socket_path) = endpoint else {
            bail!("expected a Unix-socket SSH Bridge endpoint");
        };
        let stream = tokio::net::UnixStream::connect(socket_path)
            .await
            .with_context(|| format!("failed to connect to {}", socket_path.display()))?;
        Ok(Box::new(stream))
    }
}

#[cfg(unix)]
fn current_effective_uid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }

    unsafe { geteuid() }
}

#[cfg(unix)]
fn capture_unix_peer_identity(stream: &tokio::net::UnixStream) -> BridgePeerIdentity {
    let credentials = match stream.peer_cred() {
        Ok(credentials) => credentials,
        Err(_) => {
            return BridgePeerIdentity {
                capture_status: Some(BridgeProcessCaptureStatus::Failed),
                ..BridgePeerIdentity::default()
            };
        }
    };
    let pid = credentials.pid().and_then(|pid| u32::try_from(pid).ok());
    BridgePeerIdentity {
        pid,
        uid: Some(credentials.uid()),
        gid: Some(credentials.gid()),
        process_chain: pid.map(unix_process_chain).unwrap_or_default(),
        capture_status: Some(if pid.is_some() {
            BridgeProcessCaptureStatus::Captured
        } else {
            BridgeProcessCaptureStatus::Unsupported
        }),
        ..BridgePeerIdentity::default()
    }
}

#[cfg(target_os = "linux")]
fn unix_process_chain(start_pid: u32) -> Vec<BridgeProcessIdentity> {
    use std::collections::HashSet;

    let mut chain = Vec::new();
    let mut current = start_pid;
    let mut visited = HashSet::new();
    while current != 0 && chain.len() < 8 && visited.insert(current) {
        let identity = capture_linux_process(current);
        let parent = identity.parent_pid;
        chain.push(identity);
        let Some(parent) = parent else {
            break;
        };
        current = parent;
    }
    chain
}

#[cfg(target_os = "linux")]
fn capture_linux_process(pid: u32) -> BridgeProcessIdentity {
    let stat_path = format!("/proc/{pid}/stat");
    let stat = std::fs::read_to_string(&stat_path);
    let (parent_pid, started_at, capture_status) = match stat {
        Ok(stat) => {
            let fields = stat
                .rfind(')')
                .and_then(|index| stat.get(index + 1..))
                .map(|rest| rest.split_whitespace().collect::<Vec<_>>());
            match fields {
                Some(fields) if fields.len() > 19 => (
                    fields[1].parse::<u32>().ok().filter(|parent| *parent != 0),
                    fields[19]
                        .parse::<u64>()
                        .ok()
                        .and_then(linux_process_start_unix_seconds),
                    BridgeProcessCaptureStatus::Captured,
                ),
                _ => (None, None, BridgeProcessCaptureStatus::Failed),
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            (None, None, BridgeProcessCaptureStatus::Exited)
        }
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            (None, None, BridgeProcessCaptureStatus::AccessDenied)
        }
        Err(_) => (None, None, BridgeProcessCaptureStatus::Failed),
    };
    BridgeProcessIdentity {
        pid,
        parent_pid,
        started_at,
        executable_path: std::fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .map(|path| path.to_string_lossy().into_owned()),
        capture_status,
    }
}

#[cfg(target_os = "linux")]
fn linux_process_start_unix_seconds(start_ticks: u64) -> Option<u64> {
    const SC_CLK_TCK: i32 = 2;
    unsafe extern "C" {
        fn sysconf(name: i32) -> isize;
    }

    let ticks_per_second = unsafe { sysconf(SC_CLK_TCK) };
    if ticks_per_second <= 0 {
        return None;
    }
    let boot_time = std::fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("btime "))?
        .trim()
        .parse::<u64>()
        .ok()?;
    boot_time.checked_add(start_ticks / ticks_per_second as u64)
}

#[cfg(target_os = "macos")]
fn unix_process_chain(start_pid: u32) -> Vec<BridgeProcessIdentity> {
    use std::collections::HashSet;

    let mut chain = Vec::new();
    let mut current = start_pid;
    let mut visited = HashSet::new();
    while current != 0 && chain.len() < 8 && visited.insert(current) {
        let identity = capture_macos_process(current);
        let parent = identity.parent_pid;
        chain.push(identity);
        let Some(parent) = parent else {
            break;
        };
        current = parent;
    }
    chain
}

#[cfg(target_os = "macos")]
fn capture_macos_process(pid: u32) -> BridgeProcessIdentity {
    const PROC_PIDTBSDINFO: i32 = 3;
    const PROC_PIDPATHINFO_MAXSIZE: usize = 4096;

    #[repr(C)]
    struct ProcBsdInfo {
        flags: u32,
        status: u32,
        xstatus: u32,
        pid: u32,
        ppid: u32,
        uid: u32,
        gid: u32,
        ruid: u32,
        rgid: u32,
        svuid: u32,
        svgid: u32,
        reserved: u32,
        comm: [u8; 16],
        name: [u8; 32],
        nfiles: u32,
        pgid: u32,
        pjobc: u32,
        e_tdev: u32,
        e_tpgid: u32,
        nice: i32,
        start_tvsec: u64,
        start_tvusec: u64,
    }

    unsafe extern "C" {
        fn proc_pidpath(pid: i32, buffer: *mut core::ffi::c_void, buffersize: u32) -> i32;
        fn proc_pidinfo(
            pid: i32,
            flavor: i32,
            arg: u64,
            buffer: *mut core::ffi::c_void,
            buffersize: i32,
        ) -> i32;
    }

    let mut buffer = vec![0_u8; PROC_PIDPATHINFO_MAXSIZE];
    let length =
        unsafe { proc_pidpath(pid as i32, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    let executable_path = (length > 0).then(|| {
        String::from_utf8_lossy(&buffer[..length as usize])
            .trim_end_matches('\0')
            .to_string()
    });
    let mut info = unsafe { std::mem::zeroed::<ProcBsdInfo>() };
    let info_length = unsafe {
        proc_pidinfo(
            pid as i32,
            PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut ProcBsdInfo).cast(),
            std::mem::size_of::<ProcBsdInfo>() as i32,
        )
    };
    let captured_info = info_length == std::mem::size_of::<ProcBsdInfo>() as i32;
    let capture_status = if captured_info || length > 0 {
        BridgeProcessCaptureStatus::Captured
    } else {
        match std::io::Error::last_os_error().raw_os_error() {
            Some(1) | Some(13) => BridgeProcessCaptureStatus::AccessDenied,
            Some(3) => BridgeProcessCaptureStatus::Exited,
            _ => BridgeProcessCaptureStatus::Failed,
        }
    };
    BridgeProcessIdentity {
        pid,
        parent_pid: captured_info
            .then_some(info.ppid)
            .filter(|parent| *parent != 0),
        started_at: captured_info.then_some(info.start_tvsec),
        executable_path,
        capture_status,
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn unix_process_chain(start_pid: u32) -> Vec<BridgeProcessIdentity> {
    vec![BridgeProcessIdentity {
        pid: start_pid,
        parent_pid: None,
        started_at: None,
        executable_path: None,
        capture_status: BridgeProcessCaptureStatus::Unsupported,
    }]
}

#[cfg(unix)]
impl Drop for PlatformListener {
    fn drop(&mut self) {
        match unix_socket_identity(&self.socket_path) {
            Ok(Some(identity)) if identity == self.socket_identity => {
                if let Err(error) = std::fs::remove_file(&self.socket_path)
                    && error.kind() != io::ErrorKind::NotFound
                {
                    log::debug!("failed to remove SSH Bridge socket: {error:?}");
                }
            }
            Ok(Some(_)) => {
                log::debug!(
                    "SSH Bridge socket changed before listener cleanup; leaving replacement intact"
                );
            }
            Ok(None) => {}
            Err(error) => {
                log::debug!("failed to inspect SSH Bridge socket during cleanup: {error:?}")
            }
        }
    }
}

#[cfg(not(any(windows, unix)))]
struct PlatformListener;

#[cfg(not(any(windows, unix)))]
impl PlatformListener {
    async fn bind(_: &SshBridgeEndpoint) -> Result<Self> {
        bail!("SSH Bridge IPC is unsupported on this platform")
    }

    async fn accept(&mut self) -> Result<SshBridgeConnection> {
        bail!("SSH Bridge IPC is unsupported on this platform")
    }

    async fn connect(_: &SshBridgeEndpoint) -> Result<SshBridgeStream> {
        bail!("SSH Bridge IPC is unsupported on this platform")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::profile::SessionProfile;

    #[cfg(windows)]
    #[test]
    fn windows_capture_error_codes_are_classified_explicitly() {
        assert_eq!(
            classify_windows_capture_error_code(Some(5)),
            BridgeProcessCaptureStatus::AccessDenied
        );
        assert_eq!(
            classify_windows_capture_error_code(Some(87)),
            BridgeProcessCaptureStatus::Exited
        );
        assert_eq!(
            classify_windows_capture_error_code(Some(1168)),
            BridgeProcessCaptureStatus::Exited
        );
        assert_eq!(
            classify_windows_capture_error_code(None),
            BridgeProcessCaptureStatus::Failed
        );
    }

    fn route() -> SshBridgeRoute {
        SshBridgeRoute {
            token: "0123456789abcdef0123456789abcdef".into(),
            profile_id: "profile".into(),
            profile_name: "Production".into(),
        }
    }

    #[test]
    fn failure_response_truncates_to_the_control_frame_limit() {
        let message = "quoted \"error\"\n".repeat(SSH_BRIDGE_MAX_CONTROL_FRAME);
        let response = SshBridgeRouteResponse::bounded_failure(&message);
        let payload = serde_json::to_vec(&response).expect("failure response should serialize");

        assert!(payload.len() <= SSH_BRIDGE_MAX_CONTROL_FRAME);
        assert!(
            response
                .error
                .as_deref()
                .is_some_and(|error| error.ends_with('…'))
        );
    }

    #[tokio::test]
    async fn route_handshake_succeeds_and_preserves_following_raw_bytes() {
        let table = SshBridgeRouteTable::default();
        table.replace([route()]);
        let (client, server) = tokio::io::duplex(8192);
        let mut client: SshBridgeStream = Box::new(client);
        let mut server: SshBridgeStream = Box::new(server);

        let server_task = tokio::spawn(async move {
            let selected = accept_route_request(&mut server, &table)
                .await
                .expect("route should be accepted");
            assert_eq!(selected.profile_id, "profile");
            server.write_all(b"SSH-2.0-Miaominal\r\n").await.unwrap();
        });
        request_route(&mut client, &route().token)
            .await
            .expect("route request should succeed");
        let mut banner = vec![0; 19];
        client.read_exact(&mut banner).await.unwrap();
        assert_eq!(banner, b"SSH-2.0-Miaominal\r\n");
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn route_preparation_rejects_early_client_bytes() {
        let table = SshBridgeRouteTable::default();
        table.replace([route()]);
        let (client, server) = tokio::io::duplex(8192);
        let mut client: SshBridgeStream = Box::new(client);
        let mut server: SshBridgeStream = Box::new(server);
        let server_task = tokio::spawn(async move {
            accept_route_request_with(&mut server, &table, |_| async {
                std::future::pending::<Result<()>>().await
            })
            .await
            .expect_err("early bytes must cancel route preparation")
        });
        write_control_frame(&mut client, &SshBridgeRouteRequest::new(route().token))
            .await
            .unwrap();
        client.write_all(b"x").await.unwrap();
        let response: SshBridgeRouteResponse = read_control_frame(&mut client).await.unwrap();
        assert!(!response.ok);
        assert!(
            server_task
                .await
                .unwrap()
                .to_string()
                .contains("sent data before route preparation")
        );
    }

    #[tokio::test]
    async fn malformed_oversized_version_and_unknown_route_are_rejected() {
        let (mut writer, mut reader) = tokio::io::duplex(8192);
        writer
            .write_all(&((SSH_BRIDGE_MAX_CONTROL_FRAME + 1) as u32).to_be_bytes())
            .await
            .unwrap();
        let error = read_control_frame::<_, SshBridgeRouteRequest>(&mut reader)
            .await
            .expect_err("oversized frame should fail");
        assert!(error.to_string().contains("exceeds"));

        let (mut writer, mut reader) = tokio::io::duplex(64);
        writer.write_all(&[0, 0, 0, 1, b'{']).await.unwrap();
        let error = read_control_frame::<_, SshBridgeRouteRequest>(&mut reader)
            .await
            .expect_err("malformed frame should fail");
        assert!(error.to_string().contains("malformed"));

        for request in [
            SshBridgeRouteRequest {
                version: SSH_BRIDGE_PROTOCOL_VERSION + 1,
                route: route().token,
            },
            SshBridgeRouteRequest::new("ffffffffffffffffffffffffffffffff"),
        ] {
            let table = SshBridgeRouteTable::default();
            let (client, server) = tokio::io::duplex(8192);
            let mut client: SshBridgeStream = Box::new(client);
            let mut server: SshBridgeStream = Box::new(server);
            let server_task = tokio::spawn(async move {
                accept_route_request(&mut server, &table)
                    .await
                    .expect_err("request should be rejected")
            });
            write_control_frame(&mut client, &request).await.unwrap();
            let response: SshBridgeRouteResponse = read_control_frame(&mut client).await.unwrap();
            assert!(!response.ok);
            let mut trailing = Vec::new();
            client.read_to_end(&mut trailing).await.unwrap();
            if cfg!(windows) {
                assert_eq!(trailing, [0]);
            } else {
                assert!(trailing.is_empty());
            }
            server_task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn route_handshake_times_out_when_client_sends_no_control_frame() {
        let table = SshBridgeRouteTable::default();
        let (client, server) = tokio::io::duplex(8192);
        let mut client: SshBridgeStream = Box::new(client);
        let mut server: SshBridgeStream = Box::new(server);
        let server_task = tokio::spawn(async move {
            accept_route_request_with_timeout(
                &mut server,
                &table,
                Duration::from_millis(10),
                |route| async move { Ok(route) },
            )
            .await
            .expect_err("an idle route handshake must time out")
        });

        let response: SshBridgeRouteResponse = read_control_frame(&mut client).await.unwrap();
        assert!(!response.ok);
        assert!(
            response
                .error
                .as_deref()
                .is_some_and(|error| error.contains("timed out"))
        );
        assert!(server_task.await.unwrap().to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn real_listener_rejects_conflicts_and_accepts_repeated_clients() {
        let directory = tempfile::tempdir().expect("endpoint directory");
        let endpoint = SshBridgeEndpoint::derive(directory.path()).expect("endpoint");
        let mut listener = SshBridgeListener::bind(&endpoint)
            .await
            .expect("bind first listener");
        assert!(SshBridgeListener::bind(&endpoint).await.is_err());

        for byte in [1u8, 2u8] {
            let endpoint_for_client = endpoint.clone();
            let client = tokio::spawn(async move {
                let mut stream = connect_endpoint(&endpoint_for_client).await.unwrap();
                stream.write_all(&[byte]).await.unwrap();
            });
            let mut accepted = listener.accept().await.expect("accept client");
            assert!(accepted.peer.capture_status.is_some());
            #[cfg(windows)]
            assert!(accepted.peer.pid.is_some());
            #[cfg(unix)]
            {
                assert!(accepted.peer.uid.is_some());
                assert!(accepted.peer.gid.is_some());
            }
            assert_eq!(accepted.read_u8().await.unwrap(), byte);
            client.await.unwrap();
        }
    }

    #[tokio::test]
    async fn real_listener_accept_remains_available_after_cancellation() {
        let directory = tempfile::tempdir().expect("endpoint directory");
        let endpoint = SshBridgeEndpoint::derive(directory.path()).expect("endpoint");
        let mut listener = SshBridgeListener::bind(&endpoint)
            .await
            .expect("bind listener");

        assert!(
            tokio::time::timeout(Duration::from_millis(10), listener.accept())
                .await
                .is_err()
        );

        let endpoint_for_client = endpoint.clone();
        let client = tokio::spawn(async move {
            let mut stream = connect_endpoint(&endpoint_for_client).await.unwrap();
            stream.write_all(&[7]).await.unwrap();
        });
        let mut accepted = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .expect("cancelled accept should remain usable")
            .expect("accept client");
        assert_eq!(accepted.read_u8().await.unwrap(), 7);
        client.await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_listener_drop_does_not_remove_a_replacement_socket() {
        let directory = tempfile::tempdir().expect("endpoint directory");
        let endpoint = SshBridgeEndpoint::derive(directory.path()).expect("endpoint");
        let listener = SshBridgeListener::bind(&endpoint)
            .await
            .expect("bind listener");
        let SshBridgeEndpoint::UnixSocket(socket_path) = &endpoint else {
            unreachable!();
        };

        std::fs::remove_file(socket_path).expect("unlink original socket path");
        let replacement = tokio::net::UnixListener::bind(socket_path)
            .expect("bind replacement socket at the same path");
        drop(listener);

        assert!(socket_path.exists());
        drop(replacement);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn named_pipe_listener_recovers_after_next_instance_creation_failure() {
        let directory = tempfile::tempdir().expect("endpoint directory");
        let endpoint = SshBridgeEndpoint::derive(directory.path()).expect("endpoint");
        let mut listener = SshBridgeListener::bind(&endpoint)
            .await
            .expect("bind listener");
        listener.inner.fail_next_instance_creation = true;

        let first_endpoint = endpoint.clone();
        let first = tokio::spawn(async move {
            let mut stream = connect_endpoint(&first_endpoint).await.unwrap();
            let _ = stream.write_all(&[1]).await;
        });
        assert!(
            listener.accept().await.is_err(),
            "injected next-instance failure should reject the connected client"
        );
        first.await.unwrap();

        let second_endpoint = endpoint.clone();
        let second = tokio::spawn(async move {
            let mut stream = connect_endpoint(&second_endpoint).await.unwrap();
            stream.write_all(&[2]).await.unwrap();
        });
        let mut accepted = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .expect("listener recovery should not hang")
            .expect("listener should accept after recovery");
        assert_eq!(accepted.read_u8().await.unwrap(), 2);
        second.await.unwrap();
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn named_pipe_listener_rebuilds_instance_after_connect_failure() {
        let directory = tempfile::tempdir().expect("endpoint directory");
        let endpoint = SshBridgeEndpoint::derive(directory.path()).expect("endpoint");
        let mut listener = SshBridgeListener::bind(&endpoint)
            .await
            .expect("bind listener");
        listener.inner.fail_next_connect = true;

        assert!(
            listener.accept().await.is_err(),
            "injected connect failure should reject the pending instance"
        );

        let client_endpoint = endpoint.clone();
        let client = tokio::spawn(async move {
            let mut stream = connect_endpoint(&client_endpoint).await.unwrap();
            stream.write_all(&[3]).await.unwrap();
        });
        let mut accepted = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .expect("rebuilt listener should not hang")
            .expect("rebuilt listener should accept a client");
        assert_eq!(accepted.read_u8().await.unwrap(), 3);
        client.await.unwrap();
    }

    #[test]
    fn route_table_replaces_snapshots_atomically() {
        let table = SshBridgeRouteTable::default();
        table.replace([route()]);
        assert_eq!(table.len(), 1);
        assert_eq!(
            table.resolve(&route().token).unwrap().profile_name,
            "Production"
        );

        let replacement = SshBridgeRoute::derive("instance", &SessionProfile::blank("other", 1));
        table.replace([replacement.clone()]);
        assert!(table.resolve(&route().token).is_err());
        assert_eq!(table.resolve(&replacement.token).unwrap(), replacement);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_process_capture_handles_live_and_exited_processes() {
        let current = capture_linux_process(std::process::id());
        assert_eq!(current.capture_status, BridgeProcessCaptureStatus::Captured);
        assert!(current.executable_path.is_some());
        let exited = capture_linux_process(u32::MAX);
        assert_eq!(exited.capture_status, BridgeProcessCaptureStatus::Exited);
    }

    #[cfg(windows)]
    #[test]
    fn windows_process_capture_handles_live_and_exited_processes() {
        let current = capture_windows_process(std::process::id(), None);
        assert_eq!(current.capture_status, BridgeProcessCaptureStatus::Captured);
        assert!(current.executable_path.is_some());
        assert!(current.started_at.is_some());
        assert_eq!(
            windows_filetime_to_unix_seconds(11_644_473_600 * 10_000_000),
            Some(0)
        );
        let exited = capture_windows_process(u32::MAX, None);
        assert!(matches!(
            exited.capture_status,
            BridgeProcessCaptureStatus::Exited | BridgeProcessCaptureStatus::Failed
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_process_capture_resolves_current_process() {
        let current = capture_macos_process(std::process::id());
        assert_eq!(current.capture_status, BridgeProcessCaptureStatus::Captured);
        assert!(current.executable_path.is_some());
    }

    #[test]
    fn helper_arguments_are_hidden_command_specific_and_strict() {
        let endpoint =
            SshBridgeEndpoint::derive(std::path::Path::new("C:/Miaominal-test")).expect("endpoint");
        let parsed = parse_ssh_bridge_helper_args([
            "miaominal.exe".into(),
            "ssh-bridge-helper".into(),
            "--endpoint".into(),
            endpoint.helper_value().into(),
            "--route".into(),
            route().token.into(),
        ])
        .expect("helper args should parse")
        .expect("helper command should be detected");
        assert_eq!(parsed.endpoint, endpoint);

        assert!(
            parse_ssh_bridge_helper_args(["miaominal.exe".into()])
                .unwrap()
                .is_none()
        );
        assert!(
            parse_ssh_bridge_helper_args([
                "miaominal.exe".into(),
                "ssh-bridge-helper".into(),
                "--route".into(),
                "bad".into(),
            ])
            .is_err()
        );
    }

    #[tokio::test]
    async fn helper_relay_copies_inherited_stdio_in_both_directions() {
        let (mut stdin_writer, stdin_reader) = tokio::io::duplex(64);
        let (stdout_writer, mut stdout_reader) = tokio::io::duplex(64);
        let (bridge_stream, mut bridge_peer) = tokio::io::duplex(64);
        let relay = tokio::spawn(relay_bridge_stdio(
            stdin_reader,
            stdout_writer,
            Box::new(bridge_stream),
        ));

        stdin_writer.write_all(b"from-openssh").await.unwrap();
        stdin_writer.shutdown().await.unwrap();
        let mut uploaded = vec![0; 12];
        bridge_peer.read_exact(&mut uploaded).await.unwrap();
        assert_eq!(uploaded, b"from-openssh");

        bridge_peer.write_all(b"from-bridge").await.unwrap();
        bridge_peer.shutdown().await.unwrap();
        let mut downloaded = vec![0; 11];
        stdout_reader.read_exact(&mut downloaded).await.unwrap();
        assert_eq!(downloaded, b"from-bridge");
        relay.await.unwrap().unwrap();
    }
}
