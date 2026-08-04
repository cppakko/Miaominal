use anyhow::{Context, Result, anyhow, bail};
use miaominal_core::profile::SessionProfile;
use miaominal_secrets::{
    APP_CREDENTIAL_SERVICE, CredentialStore, ProtectedPassphrase, SecretKind, SecretStore,
    VaultCredentialBackend, set_vault_test_parameters,
};
use miaominal_services::{OpenSshIntegrationService, SshBridgeService};
use miaominal_settings::{OpenSshIntegrationMode, SshBridgeConfig};
use miaominal_ssh::SshBridgeEndpoint;
use miaominal_storage::{BridgeAuditLog, BridgeSecurityStore, KnownHostsStore};
use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::{self, Auth};
use russh::{Channel, ChannelId, Pty};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};

const TEST_USER: &str = "bridge-test";
const TEST_PASSWORD: &str = "secret";
const TEST_ALIAS: &str = "miaominal-openssh-e2e";

#[derive(Clone)]
struct UpstreamHandler {
    events: Arc<StdMutex<Vec<String>>>,
    pty_channels: Arc<StdMutex<HashSet<ChannelId>>>,
}

impl UpstreamHandler {
    fn record(&self, event: impl Into<String>) {
        self.events.lock().unwrap().push(event.into());
    }

    fn finish(
        session: &mut server::Session,
        channel: ChannelId,
        stdout: &[u8],
        stderr: &[u8],
        status: u32,
    ) {
        if !stdout.is_empty() {
            let _ = session.data(channel, stdout.to_vec());
        }
        if !stderr.is_empty() {
            let _ = session.extended_data(channel, 1, stderr.to_vec());
        }
        let _ = session.exit_status_request(channel, status);
        let _ = session.eof(channel);
        let _ = session.close(channel);
    }
}

impl server::Handler for UpstreamHandler {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth> {
        Ok(if user == TEST_USER && password == TEST_PASSWORD {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<server::Msg>,
        _session: &mut server::Session,
    ) -> Result<bool> {
        Ok(true)
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<server::Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut server::Session,
    ) -> Result<bool> {
        self.record(format!("direct:{host_to_connect}:{port_to_connect}"));
        let target = format!("{host_to_connect}:{port_to_connect}");
        tokio::spawn(async move {
            match TcpStream::connect(target).await {
                Ok(mut target) => {
                    let mut channel = channel.into_stream();
                    let _ = copy_bidirectional(&mut channel, &mut target).await;
                }
                Err(_) => {
                    let _ = channel.close().await;
                }
            }
        });
        Ok(true)
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut server::Session,
    ) -> Result<()> {
        self.pty_channels.lock().unwrap().insert(channel);
        self.record("pty");
        let _ = session.channel_success(channel);
        Ok(())
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        variable_name: &str,
        variable_value: &str,
        session: &mut server::Session,
    ) -> Result<()> {
        self.record(format!("env:{variable_name}={variable_value}"));
        let _ = session.channel_success(channel);
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut server::Session,
    ) -> Result<()> {
        self.record("shell");
        let _ = session.channel_success(channel);
        Self::finish(session, channel, b"shell-ready\n", b"", 0);
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<()> {
        let command = String::from_utf8_lossy(data).into_owned();
        self.record(format!("exec:{command}"));
        let _ = session.channel_success(channel);
        let has_pty = self.pty_channels.lock().unwrap().remove(&channel);
        if has_pty {
            Self::finish(session, channel, b"pty-ready\n", b"", 0);
        } else if command == "exit-test" {
            Self::finish(session, channel, b"stdout", b"stderr", 7);
        } else if command.starts_with("scp ") {
            Self::finish(session, channel, b"scp-ready", b"", 0);
        } else {
            Self::finish(
                session,
                channel,
                format!("exec:{command}").as_bytes(),
                b"",
                0,
            );
        }
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut server::Session,
    ) -> Result<()> {
        self.record(format!("subsystem:{name}"));
        let _ = session.channel_success(channel);
        Self::finish(session, channel, format!("{name}-ready").as_bytes(), b"", 0);
        Ok(())
    }
}

struct TestUpstream {
    port: u16,
    public_key: PublicKey,
    events: Arc<StdMutex<Vec<String>>>,
    cancel: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl TestUpstream {
    async fn spawn() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let private_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)?;
        let public_key = private_key.public_key().clone();
        let config = Arc::new(server::Config {
            methods: russh::MethodSet::from(&[russh::MethodKind::Password][..]),
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![private_key],
            ..Default::default()
        });
        let events = Arc::new(StdMutex::new(Vec::new()));
        let pty_channels = Arc::new(StdMutex::new(HashSet::new()));
        let (cancel, mut cancelled) = watch::channel(false);
        let task_events = events.clone();
        let task = tokio::spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {
                    changed = cancelled.changed() => {
                        if changed.is_err() || *cancelled.borrow() {
                            break;
                        }
                    }
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break };
                        let config = config.clone();
                        let handler = UpstreamHandler {
                            events: task_events.clone(),
                            pty_channels: pty_channels.clone(),
                        };
                        clients.spawn(async move {
                            if let Ok(running) = server::run_stream(config, stream, handler).await {
                                let _ = running.await;
                            }
                        });
                    }
                    Some(_) = clients.join_next(), if !clients.is_empty() => {}
                }
            }
            clients.abort_all();
            while clients.join_next().await.is_some() {}
        });
        Ok(Self {
            port,
            public_key,
            events,
            cancel,
            task,
        })
    }

    async fn shutdown(self) {
        let _ = self.cancel.send(true);
        let _ = tokio::time::timeout(Duration::from_secs(5), self.task).await;
    }
}

struct EchoServer {
    port: u16,
    cancel: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl EchoServer {
    async fn spawn() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let (cancel, mut cancelled) = watch::channel(false);
        let task = tokio::spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {
                    changed = cancelled.changed() => {
                        if changed.is_err() || *cancelled.borrow() {
                            break;
                        }
                    }
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        clients.spawn(async move {
                            let mut buffer = [0_u8; 4096];
                            while let Ok(read) = stream.read(&mut buffer).await {
                                if read == 0 || stream.write_all(&buffer[..read]).await.is_err() {
                                    break;
                                }
                            }
                        });
                    }
                    Some(_) = clients.join_next(), if !clients.is_empty() => {}
                }
            }
            clients.abort_all();
            while clients.join_next().await.is_some() {}
        });
        Ok(Self { port, cancel, task })
    }

    async fn shutdown(self) {
        let _ = self.cancel.send(true);
        let _ = tokio::time::timeout(Duration::from_secs(5), self.task).await;
    }
}

struct BridgeFixture {
    _root: TempDir,
    _ssh_dir: TempDir,
    service: SshBridgeService,
    config_path: PathBuf,
    known_hosts_path: PathBuf,
    upstream: TestUpstream,
}

impl BridgeFixture {
    async fn start(helper_executable: &Path) -> Result<Self> {
        set_vault_test_parameters();
        let upstream = TestUpstream::spawn().await?;
        let root = tempfile::tempdir()?;
        let ssh_dir = tempfile::tempdir()?;
        let known_hosts = KnownHostsStore::with_path(root.path().join("upstream_known_hosts"));
        known_hosts.learn("127.0.0.1", upstream.port, &upstream.public_key)?;

        let credentials = CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new_with_path(
                root.path().join("credentials.json"),
                ProtectedPassphrase::try_from_string("bridge-e2e-passphrase".into())?,
            ),
        );
        let secrets = SecretStore::with_credentials(credentials);
        let mut profile = SessionProfile::blank("openssh-e2e", 1);
        profile.name = "OpenSSH E2E".into();
        profile.host = "127.0.0.1".into();
        profile.port = upstream.port;
        profile.username = TEST_USER.into();
        profile.has_stored_password = true;
        secrets.set(&profile.id, SecretKind::Password, TEST_PASSWORD)?;

        let endpoint = SshBridgeEndpoint::derive(root.path())?;
        let instance_id = SshBridgeEndpoint::instance_id(root.path())?;
        let bridge_known_hosts_path = ssh_dir
            .path()
            .join("miaominal")
            .join(&instance_id)
            .join("bridge_known_hosts");
        let service = SshBridgeService::new_with_stores(
            TokioHandle::current(),
            endpoint,
            instance_id.clone(),
            bridge_known_hosts_path,
            SshBridgeConfig::default(),
            secrets,
            known_hosts,
            BridgeSecurityStore::open(&root.path().join("ssh_bridge_security.db"))
                .map_err(|error| format!("{error:#}")),
            BridgeAuditLog::open(&root.path().join("ssh_bridge_audit.log"))
                .map_err(|error| format!("{error:#}")),
        );
        let integration = OpenSshIntegrationService::new_with_executable(
            service.clone(),
            ssh_dir.path().to_path_buf(),
            instance_id,
            helper_executable.to_path_buf(),
        );
        let sync = integration.sync(OpenSshIntegrationMode::Bridge, vec![profile], vec![])?;
        service.enable().await?;
        secure_for_windows_openssh(&sync.config_path)?;
        secure_for_windows_openssh(&sync.known_hosts_path)?;
        Ok(Self {
            _root: root,
            _ssh_dir: ssh_dir,
            service,
            config_path: sync.config_path,
            known_hosts_path: sync.known_hosts_path,
            upstream,
        })
    }

    async fn shutdown(self) {
        self.service.disable().await;
        self.upstream.shutdown().await;
    }
}

#[cfg(windows)]
fn secure_for_windows_openssh(path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SetFileSecurityW,
    };

    let mut sddl = "D:P(A;;FA;;;OW)(A;;FA;;;SY)"
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
        return Err(std::io::Error::last_os_error()).context("build OpenSSH test file ACL");
    }
    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let applied = unsafe {
        SetFileSecurityW(
            wide_path.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    unsafe {
        LocalFree(descriptor);
    }
    if applied == 0 {
        return Err(std::io::Error::last_os_error()).context("apply OpenSSH test file ACL");
    }
    Ok(())
}

#[cfg(not(windows))]
fn secure_for_windows_openssh(_path: &Path) -> Result<()> {
    Ok(())
}

fn system_ssh() -> Option<PathBuf> {
    let candidate = if cfg!(windows) {
        PathBuf::from(r"C:\Windows\System32\OpenSSH\ssh.exe")
    } else {
        PathBuf::from("ssh")
    };
    std::process::Command::new(&candidate)
        .arg("-V")
        .output()
        .ok()
        .map(|_| candidate)
}

fn base_ssh_command(ssh: &Path, config_path: &Path) -> Command {
    let mut command = Command::new(ssh);
    command
        .arg("-F")
        .arg(config_path)
        .arg("-o")
        .arg("LogLevel=ERROR")
        .arg("-o")
        .arg("ConnectTimeout=10");
    command
}

async fn command_output(mut command: Command) -> Result<std::process::Output> {
    tokio::time::timeout(Duration::from_secs(20), command.output())
        .await
        .context("OpenSSH command timed out")?
        .context("failed to run OpenSSH")
}

fn reserve_local_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

async fn wait_for_local_forward(port: u16) -> Result<TcpStream> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match TcpStream::connect(("127.0.0.1", port)).await {
            Ok(stream) => return Ok(stream),
            Err(error) if tokio::time::Instant::now() < deadline => {
                let _ = error;
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => return Err(error).context("OpenSSH forwarding listener did not start"),
        }
    }
}

async fn stop_child(child: &mut Child) {
    let _ = child.kill().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
}

async fn assert_echo(stream: &mut TcpStream, message: &[u8]) -> Result<()> {
    stream.write_all(message).await?;
    let mut echoed = vec![0; message.len()];
    stream.read_exact(&mut echoed).await?;
    if echoed != message {
        bail!("echo mismatch: expected {message:?}, received {echoed:?}");
    }
    Ok(())
}

async fn assert_socks5_echo(stream: &mut TcpStream, target_port: u16) -> Result<()> {
    stream.write_all(&[5, 1, 0]).await?;
    let mut method = [0_u8; 2];
    stream.read_exact(&mut method).await?;
    if method != [5, 0] {
        bail!("unexpected SOCKS5 method response: {method:?}");
    }

    let [port_high, port_low] = target_port.to_be_bytes();
    stream
        .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, port_high, port_low])
        .await?;
    let mut head = [0_u8; 4];
    stream.read_exact(&mut head).await?;
    if head[0] != 5 || head[1] != 0 {
        bail!("SOCKS5 connect failed: {head:?}");
    }
    match head[3] {
        1 => {
            let mut rest = [0_u8; 6];
            stream.read_exact(&mut rest).await?;
        }
        3 => {
            let length = stream.read_u8().await? as usize;
            let mut rest = vec![0_u8; length + 2];
            stream.read_exact(&mut rest).await?;
        }
        4 => {
            let mut rest = [0_u8; 18];
            stream.read_exact(&mut rest).await?;
        }
        address_type => bail!("unexpected SOCKS5 address type {address_type}"),
    }
    assert_echo(stream, b"dynamic-forward").await
}

async fn run_system_openssh_interoperability() -> Result<()> {
    let Some(ssh) = system_ssh() else {
        eprintln!("system OpenSSH is unavailable; skipping SSH Bridge interoperability test");
        return Ok(());
    };
    let helper = PathBuf::from(env!("CARGO_BIN_EXE_miaominal"));
    let fixture = BridgeFixture::start(&helper).await?;
    if !fixture.config_path.exists() || !fixture.known_hosts_path.exists() {
        return Err(anyhow!("Bridge projection files were not generated"));
    }

    let mut exec = base_ssh_command(&ssh, &fixture.config_path);
    exec.arg(TEST_ALIAS).arg("exit-test").stdin(Stdio::null());
    let output = command_output(exec).await?;
    if output.status.code() != Some(7)
        || output.stdout != b"stdout"
        || !String::from_utf8_lossy(&output.stderr).contains("stderr")
    {
        bail!(
            "exec relay mismatch: status={:?}, stdout={:?}, stderr={:?}",
            output.status.code(),
            output.stdout,
            output.stderr
        );
    }

    let mut shell = base_ssh_command(&ssh, &fixture.config_path);
    shell.arg("-T").arg(TEST_ALIAS).stdin(Stdio::null());
    let output = command_output(shell).await?;
    if !output.status.success() || output.stdout != b"shell-ready\n" {
        bail!("shell relay failed: {output:?}");
    }

    let mut pty = base_ssh_command(&ssh, &fixture.config_path);
    pty.arg("-tt")
        .arg(TEST_ALIAS)
        .arg("pty-test")
        .stdin(Stdio::null());
    let output = command_output(pty).await?;
    if !output.status.success() || !String::from_utf8_lossy(&output.stdout).contains("pty-ready") {
        bail!("PTY relay failed: {output:?}");
    }

    let mut subsystem = base_ssh_command(&ssh, &fixture.config_path);
    subsystem
        .arg("-s")
        .arg(TEST_ALIAS)
        .arg("sftp")
        .stdin(Stdio::null());
    let output = command_output(subsystem).await?;
    if !output.status.success() || output.stdout != b"sftp-ready" {
        bail!("subsystem relay failed: {output:?}");
    }

    let mut scp = base_ssh_command(&ssh, &fixture.config_path);
    scp.arg(TEST_ALIAS)
        .arg("scp -t /tmp/miaominal-test")
        .stdin(Stdio::null());
    let output = command_output(scp).await?;
    if !output.status.success() || output.stdout != b"scp-ready" {
        bail!("SCP exec relay failed: {output:?}");
    }

    let echo = EchoServer::spawn().await?;

    let mut stdio_forward = base_ssh_command(&ssh, &fixture.config_path);
    stdio_forward
        .arg("-W")
        .arg(format!("127.0.0.1:{}", echo.port))
        .arg(TEST_ALIAS)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = stdio_forward.spawn()?;
    let mut stdin = child.stdin.take().context("OpenSSH -W stdin")?;
    stdin.write_all(b"stdio-forward").await?;
    stdin.shutdown().await?;
    drop(stdin);
    let output = tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
        .await
        .context("OpenSSH -W timed out")??;
    if !output.status.success() || output.stdout != b"stdio-forward" {
        bail!("OpenSSH -W relay failed: {output:?}");
    }

    let local_port = reserve_local_port()?;
    let mut local_forward = base_ssh_command(&ssh, &fixture.config_path);
    local_forward
        .arg("-N")
        .arg("-L")
        .arg(format!("127.0.0.1:{local_port}:127.0.0.1:{}", echo.port))
        .arg(TEST_ALIAS)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut local_child = local_forward.spawn()?;
    let mut first = wait_for_local_forward(local_port).await?;
    let mut second = wait_for_local_forward(local_port).await?;
    assert_echo(&mut first, b"first-channel").await?;
    assert_echo(&mut second, b"second-channel").await?;
    stop_child(&mut local_child).await;

    let dynamic_port = reserve_local_port()?;
    let mut dynamic_forward = base_ssh_command(&ssh, &fixture.config_path);
    dynamic_forward
        .arg("-N")
        .arg("-D")
        .arg(format!("127.0.0.1:{dynamic_port}"))
        .arg(TEST_ALIAS)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut dynamic_child = dynamic_forward.spawn()?;
    let mut socks = wait_for_local_forward(dynamic_port).await?;
    assert_socks5_echo(&mut socks, echo.port).await?;
    stop_child(&mut dynamic_child).await;

    let events = fixture.upstream.events.lock().unwrap().clone();
    if !events.iter().any(|event| event == "shell")
        || !events.iter().any(|event| event == "pty")
        || !events.iter().any(|event| event == "subsystem:sftp")
        || !events
            .iter()
            .any(|event| event == "exec:scp -t /tmp/miaominal-test")
        || events
            .iter()
            .filter(|event| event.starts_with("direct:127.0.0.1:"))
            .count()
            < 4
    {
        bail!("missing relayed upstream events: {events:?}");
    }

    echo.shutdown().await;
    fixture.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn system_openssh_exec_subsystem_and_forwarding_through_bridge() {
    tokio::time::timeout(
        Duration::from_secs(90),
        run_system_openssh_interoperability(),
    )
    .await
    .expect("system OpenSSH interoperability test timed out")
    .expect("system OpenSSH interoperability test failed");
}
