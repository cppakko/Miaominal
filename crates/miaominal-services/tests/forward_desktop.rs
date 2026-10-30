//! Desktop-side integration tests for port forwarding.
//!
//! These tests drive `PortForwardManager` (the desktop service) against a real
//! `russh` test server on `127.0.0.1`, so shared forwarding changes are
//! exercised through the same code path the desktop UI uses. They cover local
//! and remote data round trips, occupied ports, unreachable targets, stopping
//! and restarting, proxy jump chains, entry-proxy authentication, node
//! failures, and chaining-loop rejection.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use miaominal_core::profile::{PortForwardKind, PortForwardRule, SessionProfile};
use miaominal_core::proxy::{ProxyAuthMode, ProxyProfile, ProxyProtocol};
use miaominal_secrets::credential_backend::{CredentialBackend, CredentialStore};
use miaominal_secrets::{SecretKind, SecretStore};
use miaominal_services::{PortForwardManager, PortForwardRuntimeSnapshot, PortForwardRuntimeState};
use miaominal_storage::KnownHostsStore;
use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg, Session as ServerSession};
use russh::{Channel, SshId};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;

// ---------------------------------------------------------------------------
// In-memory credentials (the desktop keyring must not be touched from tests)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct MemoryBackend {
    values: Mutex<HashMap<String, String>>,
}

impl CredentialBackend for MemoryBackend {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<String>> {
        Ok(self
            .values
            .lock()
            .expect("memory backend")
            .get(&format!("{service}/{account}"))
            .cloned())
    }

    fn set(&self, service: &str, account: &str, value: &str) -> Result<()> {
        self.values
            .lock()
            .expect("memory backend")
            .insert(format!("{service}/{account}"), value.to_string());
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<()> {
        self.values
            .lock()
            .expect("memory backend")
            .remove(&format!("{service}/{account}"));
        Ok(())
    }
}

fn memory_secrets() -> SecretStore {
    SecretStore::with_credentials(CredentialStore::with_backend(
        "miaominal-forward-test",
        MemoryBackend::default(),
    ))
}

// ---------------------------------------------------------------------------
// Test SSH server (password auth, direct-tcpip and tcpip-forward)
// ---------------------------------------------------------------------------

struct ServerState {
    clients: Mutex<Vec<server::Handle>>,
    authentications: std::sync::atomic::AtomicUsize,
    remote_forwards: Mutex<HashMap<(String, u32), tokio::task::JoinHandle<()>>>,
}

struct TestForwardServer {
    port: u16,
    public_key: PublicKey,
    state: Arc<ServerState>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TestForwardServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TestForwardServer {
    async fn start(runtime: &Runtime) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("host key");
        let public_key = host_key.public_key().clone();
        let config = Arc::new(server::Config {
            server_id: SshId::Standard("SSH-2.0-Miaominal-Forward-Desktop-Test".into()),
            methods: russh::MethodSet::from(&[russh::MethodKind::Password][..]),
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![host_key],
            ..Default::default()
        });
        let state = Arc::new(ServerState {
            clients: Mutex::new(Vec::new()),
            authentications: std::sync::atomic::AtomicUsize::new(0),
            remote_forwards: Mutex::new(HashMap::new()),
        });
        let server_state = state.clone();
        let runtime_handle = runtime.handle().clone();
        let task = runtime.spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let config = Arc::clone(&config);
                let state = Arc::clone(&state);
                runtime_handle.spawn(async move {
                    let running = server::run_stream(
                        config,
                        stream,
                        ForwardServerHandler {
                            state: state.clone(),
                        },
                    )
                    .await;
                    if let Ok(running) = running {
                        state.clients.lock().unwrap().push(running.handle());
                        let _ = running.await;
                    }
                });
            }
        });
        Self {
            port,
            public_key,
            state: server_state,
            task,
        }
    }
}

struct ForwardServerHandler {
    state: Arc<ServerState>,
}

impl server::Handler for ForwardServerHandler {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == "tester" && password == "secret" {
            self.state
                .authentications
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut ServerSession,
    ) -> Result<bool, Self::Error> {
        let Ok(port) = u16::try_from(port_to_connect) else {
            return Ok(false);
        };
        let target = format!("{host_to_connect}:{port}");
        let Ok(mut stream) = TcpStream::connect(&target).await else {
            return Ok(false);
        };
        tokio::spawn(async move {
            let mut forwarded = channel.into_stream();
            let _ = copy_bidirectional(&mut forwarded, &mut stream).await;
        });
        Ok(true)
    }

    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut ServerSession,
    ) -> Result<bool, Self::Error> {
        let bind = format!("{address}:{port}");
        let Ok(listener) = TcpListener::bind(&bind).await else {
            return Ok(false);
        };
        let Ok(addr) = listener.local_addr() else {
            return Ok(false);
        };
        let bound = addr.port();
        *port = u32::from(bound);
        let handle = session.handle();
        let listen_address = address.to_string();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let handle = handle.clone();
                let listen_address = listen_address.clone();
                tokio::spawn(async move {
                    if let Ok(channel) = handle
                        .channel_open_forwarded_tcpip(
                            listen_address,
                            u32::from(bound),
                            "127.0.0.1",
                            0,
                        )
                        .await
                    {
                        let mut forwarded = channel.into_stream();
                        let _ = copy_bidirectional(&mut forwarded, &mut stream).await;
                    }
                });
            }
        });
        self.state
            .remote_forwards
            .lock()
            .expect("remote forwards")
            .insert((address.to_string(), u32::from(bound)), task);
        Ok(true)
    }

    async fn cancel_tcpip_forward(
        &mut self,
        address: &str,
        port: u32,
        _session: &mut ServerSession,
    ) -> Result<bool, Self::Error> {
        let task = self
            .state
            .remote_forwards
            .lock()
            .expect("remote forwards")
            .remove(&(address.to_string(), port));
        let Some(task) = task else {
            return Ok(false);
        };
        task.abort();
        Ok(true)
    }
}

struct EchoServer {
    port: u16,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for EchoServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl EchoServer {
    async fn start(runtime: &Runtime) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind echo");
        let port = listener.local_addr().expect("addr").port();
        let task = runtime.spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buffer = [0u8; 4096];
                    loop {
                        match stream.read(&mut buffer).await {
                            Ok(0) | Err(_) => break,
                            Ok(read) => {
                                if stream.write_all(&buffer[..read]).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
            }
        });
        Self { port, task }
    }
}

/// Minimal HTTP CONNECT proxy that optionally requires Basic credentials.
struct HttpProxy {
    port: u16,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for HttpProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl HttpProxy {
    async fn start(runtime: &Runtime, expected: Option<(&'static str, &'static str)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind proxy");
        let port = listener.local_addr().expect("addr").port();
        let task = runtime.spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let expected = expected;
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    loop {
                        let mut chunk = [0u8; 512];
                        let Ok(read) = stream.read(&mut chunk).await else {
                            return;
                        };
                        if read == 0 {
                            return;
                        }
                        request.extend_from_slice(&chunk[..read]);
                        if request.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request = String::from_utf8_lossy(&request).into_owned();
                    let authorized = match expected {
                        None => true,
                        Some((user, password)) => {
                            let encoded = {
                                use base64::Engine as _;
                                base64::engine::general_purpose::STANDARD
                                    .encode(format!("{user}:{password}"))
                            };
                            request.contains(&format!("Proxy-Authorization: Basic {encoded}"))
                        }
                    };
                    if !authorized {
                        let _ = stream
                            .write_all(
                                b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n",
                            )
                            .await;
                        return;
                    }
                    let Some(authority) = request
                        .lines()
                        .next()
                        .and_then(|line| line.strip_prefix("CONNECT "))
                        .and_then(|line| line.split_whitespace().next())
                    else {
                        return;
                    };
                    let Some((host, port)) = authority.rsplit_once(':') else {
                        return;
                    };
                    let Ok(port) = port.parse::<u16>() else {
                        return;
                    };
                    let Ok(mut target) = TcpStream::connect((host, port)).await else {
                        let _ = stream
                            .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                            .await;
                        return;
                    };
                    if stream
                        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let _ = copy_bidirectional(&mut stream, &mut target).await;
                });
            }
        });
        Self { port, task }
    }
}

// ---------------------------------------------------------------------------
// Harness helpers
// ---------------------------------------------------------------------------

fn password_profile(id: &str, name: &str, port: u16) -> SessionProfile {
    let mut profile = SessionProfile::blank(id, 1);
    profile.name = name.to_string();
    profile.host = "127.0.0.1".to_string();
    profile.port = port;
    profile.username = "tester".to_string();
    profile.password = "secret".to_string();
    profile
}

fn local_rule(id: &str, listen_port: u16, target_port: u16) -> PortForwardRule {
    PortForwardRule {
        id: id.to_string(),
        label: format!("rule {id}"),
        kind: PortForwardKind::Local,
        listen_host: "127.0.0.1".to_string(),
        listen_port,
        target_host: "127.0.0.1".to_string(),
        target_port,
        enabled: true,
    }
}

fn remote_rule(id: &str, listen_port: u16, target_port: u16) -> PortForwardRule {
    PortForwardRule {
        kind: PortForwardKind::Remote,
        ..local_rule(id, listen_port, target_port)
    }
}

fn manager_for(
    runtime: &Runtime,
    secrets: SecretStore,
    root: &std::path::Path,
) -> PortForwardManager {
    PortForwardManager::new(
        runtime.handle().clone(),
        secrets,
        KnownHostsStore::with_path(root.join("known_hosts")),
    )
}

fn wait_for_state(
    runtime: &Runtime,
    manager: &PortForwardManager,
    profile_id: &str,
    rule_id: &str,
    predicate: impl Fn(&PortForwardRuntimeState) -> bool,
    timeout: Duration,
) -> PortForwardRuntimeSnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = manager
            .snapshot()
            .session(profile_id, rule_id)
            .cloned()
            .expect("forward session snapshot");
        if predicate(&snapshot.state) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a forward state; last snapshot {snapshot:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
        let _ = runtime; // the runtime must stay alive while we poll
    }
}

fn running(state: &PortForwardRuntimeState) -> bool {
    matches!(state, PortForwardRuntimeState::Running)
}

fn failed(state: &PortForwardRuntimeState) -> bool {
    matches!(state, PortForwardRuntimeState::Failed(_))
}

fn stopped(state: &PortForwardRuntimeState) -> bool {
    matches!(state, PortForwardRuntimeState::Stopped)
}

fn answer_host_key_prompts(manager: &PortForwardManager, profile_id: &str, rule_id: &str) -> bool {
    let snapshot = manager.snapshot();
    let Some(session) = snapshot.session(profile_id, rule_id) else {
        return false;
    };
    if matches!(
        session.prompt,
        Some(miaominal_services::PortForwardPrompt::HostKey(_))
    ) {
        return manager
            .respond_host_key(
                profile_id,
                rule_id,
                miaominal_ssh::HostKeyDecision::AcceptAndSave,
            )
            .is_ok();
    }
    false
}

fn round_trip(runtime: &Runtime, port: u16, payload: &[u8]) -> Vec<u8> {
    runtime.block_on(async move {
        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect to the forwarded port");
        stream.write_all(payload).await.expect("write payload");
        let mut buffer = vec![0u8; payload.len()];
        tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut buffer))
            .await
            .expect("read should not time out")
            .expect("read echoed payload");
        buffer
    })
}

fn wait_for_port_release(runtime: &Runtime, port: u16) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "port {port} was not released after stopping the forward"
        );
        runtime.block_on(tokio::time::sleep(Duration::from_millis(20)));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn local_forward_runs_relays_and_restarts() {
    let runtime = Runtime::new().expect("runtime");
    let echo = runtime.block_on(EchoServer::start(&runtime));
    let server = runtime.block_on(TestForwardServer::start(&runtime));
    let directory = tempfile::tempdir().expect("temp dir");
    let known_hosts = KnownHostsStore::with_path(directory.path().join("known_hosts"));
    known_hosts
        .learn("127.0.0.1", server.port, &server.public_key)
        .expect("learn host key");
    let manager = PortForwardManager::new(runtime.handle().clone(), memory_secrets(), known_hosts);
    let mut profile = password_profile("host-1", "Target", server.port);
    profile
        .port_forwarding_rules
        .push(local_rule("rule-1", 0, echo.port));
    manager.replace_catalogs(vec![profile], Vec::new());

    assert!(matches!(manager.start("host-1", "rule-1"), Ok(_)));
    let snapshot = wait_for_state(
        &runtime,
        &manager,
        "host-1",
        "rule-1",
        running,
        Duration::from_secs(20),
    );
    let port = snapshot.bound_port.expect("bound local port");
    assert_eq!(
        round_trip(&runtime, port, b"desktop local"),
        b"desktop local"
    );

    assert!(manager.stop("host-1", "rule-1"));
    wait_for_state(
        &runtime,
        &manager,
        "host-1",
        "rule-1",
        stopped,
        Duration::from_secs(10),
    );
    wait_for_port_release(&runtime, port);

    manager.start("host-1", "rule-1").expect("restart");
    wait_for_state(
        &runtime,
        &manager,
        "host-1",
        "rule-1",
        running,
        Duration::from_secs(20),
    );
    manager.stop_all();
    wait_for_state(
        &runtime,
        &manager,
        "host-1",
        "rule-1",
        stopped,
        Duration::from_secs(10),
    );
}

#[test]
fn remote_forward_relays_to_the_local_target() {
    let runtime = Runtime::new().expect("runtime");
    let echo = runtime.block_on(EchoServer::start(&runtime));
    let server = runtime.block_on(TestForwardServer::start(&runtime));
    let directory = tempfile::tempdir().expect("temp dir");
    let known_hosts = KnownHostsStore::with_path(directory.path().join("known_hosts"));
    known_hosts
        .learn("127.0.0.1", server.port, &server.public_key)
        .expect("learn host key");
    let manager = PortForwardManager::new(runtime.handle().clone(), memory_secrets(), known_hosts);
    let mut profile = password_profile("host-1", "Target", server.port);
    profile
        .port_forwarding_rules
        .push(remote_rule("rule-1", 0, echo.port));
    manager.replace_catalogs(vec![profile], Vec::new());

    manager.start("host-1", "rule-1").expect("start remote");
    let snapshot = wait_for_state(
        &runtime,
        &manager,
        "host-1",
        "rule-1",
        running,
        Duration::from_secs(20),
    );
    let port = snapshot.bound_port.expect("bound remote port");
    assert_ne!(port, 0);
    assert_eq!(
        round_trip(&runtime, port, b"desktop remote"),
        b"desktop remote"
    );
    manager.stop_all();
}

#[test]
fn occupied_local_port_is_reported_as_failed() {
    let runtime = Runtime::new().expect("runtime");
    let server = runtime.block_on(TestForwardServer::start(&runtime));
    let directory = tempfile::tempdir().expect("temp dir");
    let known_hosts = KnownHostsStore::with_path(directory.path().join("known_hosts"));
    known_hosts
        .learn("127.0.0.1", server.port, &server.public_key)
        .expect("learn host key");
    let manager = PortForwardManager::new(runtime.handle().clone(), memory_secrets(), known_hosts);
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").expect("occupy port");
    let occupied_port = occupied.local_addr().expect("addr").port();
    let mut profile = password_profile("host-1", "Target", server.port);
    profile
        .port_forwarding_rules
        .push(local_rule("rule-1", occupied_port, 9));
    manager.replace_catalogs(vec![profile], Vec::new());

    manager.start("host-1", "rule-1").expect("start");
    let snapshot = wait_for_state(
        &runtime,
        &manager,
        "host-1",
        "rule-1",
        failed,
        Duration::from_secs(20),
    );
    let PortForwardRuntimeState::Failed(error) = snapshot.state else {
        panic!("expected a failed state");
    };
    assert!(
        error.contains("failed to bind") || error.contains("address"),
        "unexpected failure: {error}"
    );
    assert_eq!(snapshot.bound_port, None);
    drop(occupied);
}

#[test]
fn unreachable_target_keeps_the_forward_running() {
    let runtime = Runtime::new().expect("runtime");
    let server = runtime.block_on(TestForwardServer::start(&runtime));
    let directory = tempfile::tempdir().expect("temp dir");
    let known_hosts = KnownHostsStore::with_path(directory.path().join("known_hosts"));
    known_hosts
        .learn("127.0.0.1", server.port, &server.public_key)
        .expect("learn host key");
    let manager = PortForwardManager::new(runtime.handle().clone(), memory_secrets(), known_hosts);
    let mut profile = password_profile("host-1", "Target", server.port);
    // Port 1 on loopback has no listener.
    profile
        .port_forwarding_rules
        .push(local_rule("rule-1", 0, 1));
    manager.replace_catalogs(vec![profile], Vec::new());

    manager.start("host-1", "rule-1").expect("start");
    let snapshot = wait_for_state(
        &runtime,
        &manager,
        "host-1",
        "rule-1",
        running,
        Duration::from_secs(20),
    );
    let port = snapshot.bound_port.expect("bound local port");

    let _ = runtime.block_on(TcpStream::connect(("127.0.0.1", port)));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let current = manager
            .snapshot()
            .session("host-1", "rule-1")
            .cloned()
            .expect("session");
        if matches!(current.state, PortForwardRuntimeState::Running)
            && (current
                .status_message
                .contains("could not open SSH channel")
                || current.status_message.contains("relay failed"))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the relay failure notice; last {current:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    manager.stop_all();
}

#[test]
fn multi_hop_forward_through_a_jump_host_relays_bytes() {
    let runtime = Runtime::new().expect("runtime");
    let echo = runtime.block_on(EchoServer::start(&runtime));
    let jump = runtime.block_on(TestForwardServer::start(&runtime));
    let target = runtime.block_on(TestForwardServer::start(&runtime));
    let directory = tempfile::tempdir().expect("temp dir");
    let known_hosts = KnownHostsStore::with_path(directory.path().join("known_hosts"));
    known_hosts
        .learn("127.0.0.1", jump.port, &jump.public_key)
        .expect("learn jump key");
    known_hosts
        .learn("127.0.0.1", target.port, &target.public_key)
        .expect("learn target key");
    let manager = PortForwardManager::new(runtime.handle().clone(), memory_secrets(), known_hosts);
    let jump_profile = password_profile("jump", "Jump", jump.port);
    let mut target_profile = password_profile("target", "Target", target.port);
    target_profile.proxy_jump_profile_ids = vec!["jump".to_string()];
    target_profile
        .port_forwarding_rules
        .push(local_rule("rule-1", 0, echo.port));
    manager.replace_catalogs(vec![jump_profile, target_profile], Vec::new());

    manager.start("target", "rule-1").expect("start multi-hop");
    let snapshot = wait_for_state(
        &runtime,
        &manager,
        "target",
        "rule-1",
        running,
        Duration::from_secs(30),
    );
    let port = snapshot.bound_port.expect("bound local port");
    assert_eq!(
        round_trip(&runtime, port, b"through the jump"),
        b"through the jump"
    );
    manager.stop_all();
}

#[test]
fn a_failing_jump_node_is_reported() {
    let runtime = Runtime::new().expect("runtime");
    let directory = tempfile::tempdir().expect("temp dir");
    let manager = manager_for(&runtime, memory_secrets(), directory.path());
    let mut jump_profile = password_profile("jump", "Jump", 1);
    jump_profile.host = "127.0.0.1".to_string();
    let mut target_profile = password_profile("target", "Target", 1);
    target_profile.proxy_jump_profile_ids = vec!["jump".to_string()];
    target_profile
        .port_forwarding_rules
        .push(local_rule("rule-1", 0, 9));
    manager.replace_catalogs(vec![jump_profile, target_profile], Vec::new());

    manager.start("target", "rule-1").expect("start");
    let snapshot = wait_for_state(
        &runtime,
        &manager,
        "target",
        "rule-1",
        failed,
        Duration::from_secs(20),
    );
    let PortForwardRuntimeState::Failed(error) = snapshot.state else {
        panic!("expected failure");
    };
    assert!(error.contains("127.0.0.1:1"), "unexpected failure: {error}");
}

#[test]
fn a_chaining_loop_is_rejected_before_connecting() {
    let runtime = Runtime::new().expect("runtime");
    let directory = tempfile::tempdir().expect("temp dir");
    let manager = manager_for(&runtime, memory_secrets(), directory.path());
    let mut profile = password_profile("host-1", "Target", 1);
    profile.proxy_jump_profile_ids = vec!["host-1".to_string()];
    profile
        .port_forwarding_rules
        .push(local_rule("rule-1", 0, 9));
    manager.replace_catalogs(vec![profile], Vec::new());

    manager.start("host-1", "rule-1").expect("start");
    let snapshot = wait_for_state(
        &runtime,
        &manager,
        "host-1",
        "rule-1",
        failed,
        Duration::from_secs(10),
    );
    let PortForwardRuntimeState::Failed(error) = snapshot.state else {
        panic!("expected failure");
    };
    assert!(
        error.contains("cannot reference the host being connected"),
        "unexpected failure: {error}"
    );
}

#[test]
fn entry_proxy_authentication_is_used_and_failures_are_reported() {
    let runtime = Runtime::new().expect("runtime");
    let echo = runtime.block_on(EchoServer::start(&runtime));
    let server = runtime.block_on(TestForwardServer::start(&runtime));
    let proxy = runtime.block_on(HttpProxy::start(&runtime, Some(("alice", "secret"))));
    let directory = tempfile::tempdir().expect("temp dir");
    let known_hosts = KnownHostsStore::with_path(directory.path().join("known_hosts"));
    known_hosts
        .learn("127.0.0.1", server.port, &server.public_key)
        .expect("learn host key");

    let secrets = memory_secrets();
    secrets
        .set("proxy-1", SecretKind::ProxyPassword, "secret")
        .expect("store proxy password");
    let manager = PortForwardManager::new(
        runtime.handle().clone(),
        secrets.clone(),
        known_hosts.clone(),
    );

    let mut proxy_profile = ProxyProfile::blank("proxy-1", 1);
    proxy_profile.name = "Proxy".to_string();
    proxy_profile.protocol = ProxyProtocol::HttpConnect;
    proxy_profile.host = "127.0.0.1".to_string();
    proxy_profile.port = proxy.port;
    proxy_profile.auth_mode = ProxyAuthMode::UsernamePassword;
    proxy_profile.username = "alice".to_string();
    proxy_profile.has_stored_password = true;

    let mut profile = password_profile("host-1", "Target", server.port);
    profile.entry_proxy_id = Some("proxy-1".to_string());
    profile
        .port_forwarding_rules
        .push(local_rule("rule-1", 0, echo.port));
    manager.replace_catalogs(vec![profile.clone()], vec![proxy_profile.clone()]);

    manager
        .start("host-1", "rule-1")
        .expect("start through proxy");
    let snapshot = wait_for_state(
        &runtime,
        &manager,
        "host-1",
        "rule-1",
        running,
        Duration::from_secs(20),
    );
    let port = snapshot.bound_port.expect("bound local port");
    assert_eq!(
        round_trip(&runtime, port, b"through the proxy"),
        b"through the proxy"
    );
    manager.stop_all();

    // Wrong credentials must fail the session with the proxy named.
    secrets
        .set("proxy-1", SecretKind::ProxyPassword, "wrong")
        .expect("store wrong proxy password");
    manager
        .start("host-1", "rule-1")
        .expect("restart through proxy");
    let snapshot = wait_for_state(
        &runtime,
        &manager,
        "host-1",
        "rule-1",
        failed,
        Duration::from_secs(20),
    );
    let PortForwardRuntimeState::Failed(error) = snapshot.state else {
        panic!("expected failure");
    };
    assert!(
        error.contains("authentication") || error.contains("407"),
        "unexpected failure: {error}"
    );
}

#[test]
fn host_key_prompts_carry_the_hop_node_and_expired_answers_are_rejected() {
    let runtime = Runtime::new().expect("runtime");
    let echo = runtime.block_on(EchoServer::start(&runtime));
    let server = runtime.block_on(TestForwardServer::start(&runtime));
    let directory = tempfile::tempdir().expect("temp dir");
    let manager = manager_for(&runtime, memory_secrets(), directory.path());
    let mut profile = password_profile("host-1", "Target", server.port);
    profile
        .port_forwarding_rules
        .push(local_rule("rule-1", 0, echo.port));
    manager.replace_catalogs(vec![profile], Vec::new());

    manager.start("host-1", "rule-1").expect("start");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let snapshot = manager
            .snapshot()
            .session("host-1", "rule-1")
            .cloned()
            .expect("session");
        if let Some(miaominal_services::PortForwardPrompt::HostKey(request)) = snapshot.prompt {
            assert_eq!(request.hop.label, "Target");
            assert!(
                manager
                    .respond_host_key(
                        "host-1",
                        "rule-1",
                        miaominal_ssh::HostKeyDecision::AcceptAndSave,
                    )
                    .is_ok()
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the host key prompt; last {snapshot:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let snapshot = wait_for_state(
        &runtime,
        &manager,
        "host-1",
        "rule-1",
        running,
        Duration::from_secs(20),
    );
    let port = snapshot.bound_port.expect("bound local port");
    assert_eq!(round_trip(&runtime, port, b"trusted"), b"trusted");
    manager.stop_all();
}

#[test]
fn answering_a_prompt_after_the_session_is_gone_is_harmless() {
    let runtime = Runtime::new().expect("runtime");
    let directory = tempfile::tempdir().expect("temp dir");
    let manager = manager_for(&runtime, memory_secrets(), directory.path());
    let mut profile = password_profile("host-1", "Target", 1);
    profile
        .port_forwarding_rules
        .push(local_rule("rule-1", 0, 9));
    manager.replace_catalogs(vec![profile], Vec::new());

    manager.start("host-1", "rule-1").expect("start");
    wait_for_state(
        &runtime,
        &manager,
        "host-1",
        "rule-1",
        failed,
        Duration::from_secs(20),
    );
    assert!(
        manager
            .respond_host_key(
                "host-1",
                "rule-1",
                miaominal_ssh::HostKeyDecision::AcceptOnce,
            )
            .is_err()
    );
    let _ = answer_host_key_prompts(&manager, "host-1", "rule-1");
}

async fn next_forward_event(
    connection: &mut miaominal_ssh::SessionConnection,
    predicate: impl Fn(&miaominal_ssh::SessionEvent) -> bool,
) -> miaominal_ssh::SessionEvent {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let event = connection.events.recv().await.expect("worker event");
            if predicate(&event) {
                return event;
            }
            if let miaominal_ssh::SessionEvent::Error(error) = event {
                panic!("forward failed: {error}");
            }
        }
    })
    .await
    .expect("forward event timed out")
}

#[test]
fn rdp_binding_fallback_counts_concurrent_relays_and_cancels_them_on_stop() {
    use miaominal_ssh::{
        LocalForwardBindingPolicy, SessionEvent, start_port_forward_session_with_policy,
    };
    let runtime = Runtime::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let server = runtime.block_on(TestForwardServer::start(&runtime));
    let echo = runtime.block_on(EchoServer::start(&runtime));
    let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let occupied_port = occupied.local_addr().unwrap().port();
    let known = KnownHostsStore::with_path(directory.path().join("known_hosts"));
    known
        .learn("127.0.0.1", server.port, &server.public_key)
        .unwrap();
    let mut profile = password_profile("rdp", "RDP", server.port);
    profile.port_forwarding_rules = vec![local_rule("temporary", occupied_port, echo.port)];
    let mut connection = start_port_forward_session_with_policy(
        runtime.handle(),
        profile.clone(),
        vec![profile],
        vec![],
        memory_secrets(),
        known,
        LocalForwardBindingPolicy::PreferAvailable,
    );
    let port = runtime.block_on(async {
        let SessionEvent::PortForwardActive(binding) = next_forward_event(&mut connection, |event| matches!(event, SessionEvent::PortForwardActive(_))).await else { unreachable!() };
        assert_ne!(binding.bound_port, occupied_port); let port = binding.bound_port;
        let mut first = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut second = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        next_forward_event(&mut connection, |event| matches!(event, SessionEvent::PortForwardConnections { binding_id, active: 2, .. } if *binding_id == binding.binding_id)).await;
        first.write_all(b"first").await.unwrap(); let mut bytes = [0; 5]; first.read_exact(&mut bytes).await.unwrap(); assert_eq!(&bytes, b"first");
        drop(first);
        next_forward_event(&mut connection, |event| matches!(event, SessionEvent::PortForwardConnections { active: 1, .. })).await;
        assert_eq!(server.state.authentications.load(std::sync::atomic::Ordering::Relaxed), 1);
        connection.commands.close().unwrap();
        let mut buffer = [0]; let read = tokio::time::timeout(Duration::from_secs(5), second.read(&mut buffer)).await.unwrap();
        assert!(matches!(read, Ok(0) | Err(_)), "cancel must close existing relay"); port
    });
    drop(connection.events);
    connection.thread.take().unwrap().join().unwrap();
    wait_for_port_release(&runtime, port);
}

#[test]
fn rdp_reconnect_pins_port_and_uses_new_listener_generation() {
    use miaominal_ssh::{
        LocalForwardBindingPolicy, SessionEvent, start_port_forward_session_with_policy,
    };
    let runtime = Runtime::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let server = runtime.block_on(TestForwardServer::start(&runtime));
    let known = KnownHostsStore::with_path(directory.path().join("known_hosts"));
    known
        .learn("127.0.0.1", server.port, &server.public_key)
        .unwrap();
    let dead_target = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let target_port = dead_target.local_addr().unwrap().port();
    drop(dead_target);
    let mut profile = password_profile("rdp-reconnect", "RDP", server.port);
    profile.port_forwarding_rules = vec![local_rule("temporary", 0, target_port)];
    let mut connection = start_port_forward_session_with_policy(
        runtime.handle(),
        profile.clone(),
        vec![profile],
        vec![],
        memory_secrets(),
        known,
        LocalForwardBindingPolicy::PreferAvailable,
    );
    let port = runtime.block_on(async {
        let SessionEvent::PortForwardActive(first) = next_forward_event(&mut connection, |e| matches!(e, SessionEvent::PortForwardActive(_))).await else { unreachable!() };
        let mut failed_relay = TcpStream::connect(("127.0.0.1", first.bound_port)).await.unwrap();
        let mut byte = [0]; let _ = tokio::time::timeout(Duration::from_secs(5), failed_relay.read(&mut byte)).await.unwrap();
        next_forward_event(&mut connection, |e| matches!(e, SessionEvent::PortForwardConnections { active: 0, sequence, .. } if *sequence >= 2)).await;
        let clients = std::mem::take(&mut *server.state.clients.lock().unwrap());
        for client in clients { let _ = client.disconnect(russh::Disconnect::ByApplication, "test".into(), "".into()).await; }
        next_forward_event(&mut connection, |e| matches!(e, SessionEvent::PortForwardReconnecting { .. })).await;
        let SessionEvent::PortForwardActive(second) = next_forward_event(&mut connection, |e| matches!(e, SessionEvent::PortForwardActive(_))).await else { unreachable!() };
        assert_eq!(second.bound_port, first.bound_port); assert_ne!(second.binding_id, first.binding_id);
        connection.commands.close().unwrap(); second.bound_port
    });
    drop(connection.events);
    connection.thread.take().unwrap().join().unwrap();
    wait_for_port_release(&runtime, port);
}

async fn echo_round_trip(stream: &mut TcpStream, payload: &[u8]) {
    stream.write_all(payload).await.unwrap();
    let mut bytes = vec![0; payload.len()];
    tokio::time::timeout(Duration::from_secs(5), stream.read_exact(&mut bytes))
        .await
        .expect("echo timed out")
        .unwrap();
    assert_eq!(bytes, payload);
}

async fn next_binding(
    connection: &mut miaominal_ssh::SessionConnection,
) -> miaominal_ssh::PortForwardBinding {
    match next_forward_event(connection, |event| {
        matches!(event, miaominal_ssh::SessionEvent::PortForwardActive(_))
    })
    .await
    {
        miaominal_ssh::SessionEvent::PortForwardActive(binding) => binding,
        _ => unreachable!(),
    }
}

fn start_test_forward(
    runtime: &Runtime,
    server: &TestForwardServer,
    directory: &tempfile::TempDir,
    id: &str,
    rule: PortForwardRule,
    policy: miaominal_ssh::LocalForwardBindingPolicy,
) -> miaominal_ssh::SessionConnection {
    let known = KnownHostsStore::with_path(directory.path().join("known_hosts"));
    known
        .learn("127.0.0.1", server.port, &server.public_key)
        .unwrap();
    let mut profile = password_profile(id, id, server.port);
    profile.port_forwarding_rules = vec![rule];
    miaominal_ssh::start_port_forward_session_with_policy(
        runtime.handle(),
        profile.clone(),
        vec![profile],
        vec![],
        memory_secrets(),
        known,
        policy,
    )
}

#[test]
fn rdp_listener_owns_its_relays_while_ssh_stays_connected() {
    let runtime = Runtime::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let server = runtime.block_on(TestForwardServer::start(&runtime));
    let echo = runtime.block_on(EchoServer::start(&runtime));
    let mut connection = start_test_forward(
        &runtime,
        &server,
        &directory,
        "rdp-owned",
        local_rule("temporary", 0, echo.port),
        miaominal_ssh::LocalForwardBindingPolicy::PreferAvailable,
    );
    let port = runtime.block_on(async {
        let first = next_binding(&mut connection).await;
        let mut stream = TcpStream::connect(("127.0.0.1", first.bound_port))
            .await
            .unwrap();
        echo_round_trip(&mut stream, b"before").await;
        // Replace the listener without touching the SSH connection.
        let mut changed = local_rule("temporary", first.bound_port, echo.port);
        changed.target_host = "localhost".into();
        connection
            .commands
            .sync_port_forward_rules(vec![changed])
            .unwrap();
        let second = next_binding(&mut connection).await;
        assert_ne!(second.binding_id, first.binding_id);
        let mut buffer = [0];
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer))
            .await
            .expect("relay of the replaced listener must be cancelled");
        assert!(matches!(read, Ok(0) | Err(_)));
        assert_eq!(
            server
                .state
                .authentications
                .load(std::sync::atomic::Ordering::Relaxed),
            1,
            "the SSH connection must stay up"
        );
        connection.commands.close().unwrap();
        second.bound_port
    });
    drop(connection.events);
    connection.thread.take().unwrap().join().unwrap();
    wait_for_port_release(&runtime, port);
}

#[test]
fn ordinary_forward_keeps_live_connections_when_its_rule_is_edited() {
    let runtime = Runtime::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let server = runtime.block_on(TestForwardServer::start(&runtime));
    let echo = runtime.block_on(EchoServer::start(&runtime));
    let mut connection = start_test_forward(
        &runtime,
        &server,
        &directory,
        "ordinary",
        local_rule("rule", 0, echo.port),
        miaominal_ssh::LocalForwardBindingPolicy::Strict,
    );
    let port = runtime.block_on(async {
        let first = next_binding(&mut connection).await;
        let mut stream = TcpStream::connect(("127.0.0.1", first.bound_port))
            .await
            .unwrap();
        echo_round_trip(&mut stream, b"before").await;
        let mut renamed = local_rule("rule", first.bound_port, echo.port);
        renamed.label = "renamed".into();
        connection
            .commands
            .sync_port_forward_rules(vec![renamed])
            .unwrap();
        next_binding(&mut connection).await;
        // Editing a rule restarts its listener but never drops connections in use.
        echo_round_trip(&mut stream, b"after").await;
        connection.commands.close().unwrap();
        first.bound_port
    });
    drop(connection.events);
    connection.thread.take().unwrap().join().unwrap();
    wait_for_port_release(&runtime, port);
}

#[test]
fn rdp_reconnect_falls_back_when_the_pinned_port_was_taken() {
    let runtime = Runtime::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let server = runtime.block_on(TestForwardServer::start(&runtime));
    let echo = runtime.block_on(EchoServer::start(&runtime));
    let mut connection = start_test_forward(
        &runtime,
        &server,
        &directory,
        "rdp-taken",
        local_rule("temporary", 0, echo.port),
        miaominal_ssh::LocalForwardBindingPolicy::PreferAvailable,
    );
    let (port, _occupied) = runtime.block_on(async {
        let first = next_binding(&mut connection).await;
        let clients = std::mem::take(&mut *server.state.clients.lock().unwrap());
        for client in clients {
            let _ = client
                .disconnect(russh::Disconnect::ByApplication, "test".into(), "".into())
                .await;
        }
        next_forward_event(&mut connection, |e| {
            matches!(
                e,
                miaominal_ssh::SessionEvent::PortForwardReconnecting { .. }
            )
        })
        .await;
        // Another program grabs the pinned port while the SSH connection is down.
        let deadline = Instant::now() + Duration::from_secs(5);
        let occupied = loop {
            match std::net::TcpListener::bind(("127.0.0.1", first.bound_port)) {
                Ok(listener) => break listener,
                Err(_) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(50)).await
                }
                Err(error) => panic!("pinned port was never released: {error}"),
            }
        };
        let second = next_binding(&mut connection).await;
        assert_ne!(second.bound_port, first.bound_port);
        let mut stream = TcpStream::connect(("127.0.0.1", second.bound_port))
            .await
            .unwrap();
        echo_round_trip(&mut stream, b"fallback").await;
        connection.commands.close().unwrap();
        (second.bound_port, occupied)
    });
    drop(connection.events);
    connection.thread.take().unwrap().join().unwrap();
    wait_for_port_release(&runtime, port);
}
