//! Integration tests for port forwarding against a local test SSH server.
//!
//! The server is a real `russh` server on `127.0.0.1` that supports
//! `direct-tcpip` (local forwarding) and `tcpip-forward`/`cancel-tcpip-forward`
//! (remote forwarding), plus an echo TCP service as the forwarding target.

#![cfg(test)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg, Session as ServerSession};
use russh::{Channel, SshId};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, copy_bidirectional};
use tokio::net::{TcpListener, TcpStream};

use crate::backend::{Backend, BackendConfig};
use crate::dto::{
    AuthMethodDto, CredentialAction, CredentialKindDto, ForwardKindDto, ForwardRuleDto, HostUpdate,
};
use crate::error::BackendError;
use crate::forward::{ForwardFailureKind, ForwardPhase, ForwardSession, ForwardSnapshot};
use crate::platform::PlatformHost;
use crate::session::SshHostKeyDecision;
use crate::test_support::{
    business_root, cache_root, clear_test_root, shared_test_lock, test_host,
};

#[derive(Clone, Default)]
struct ForwardTestOptions {
    /// Never answer password authentication (cancel-during-auth tests).
    hang_auth: bool,
    /// Refuse every `tcpip-forward` request (remote bind rejection).
    reject_remote_forward: bool,
}

struct ServerState {
    options: ForwardTestOptions,
    remote_forwards: Mutex<HashMap<(String, u32), tokio::task::JoinHandle<()>>>,
}

struct TestForwardServer {
    port: u16,
    public_key: PublicKey,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TestForwardServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TestForwardServer {
    async fn start(options: ForwardTestOptions) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("host key");
        let public_key = host_key.public_key().clone();
        let config = Arc::new(server::Config {
            server_id: SshId::Standard("SSH-2.0-Miaominal-Forward-Test".into()),
            methods: russh::MethodSet::from(&[russh::MethodKind::Password][..]),
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![host_key],
            ..Default::default()
        });
        let state = Arc::new(ServerState {
            options,
            remote_forwards: Mutex::new(HashMap::new()),
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
                        server::run_stream(config, stream, ForwardServerHandler { state }).await;
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
}

struct ForwardServerHandler {
    state: Arc<ServerState>,
}

impl server::Handler for ForwardServerHandler {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        if self.state.options.hang_auth {
            tokio::time::sleep(Duration::from_secs(30)).await;
            return Ok(Auth::reject());
        }
        Ok(if user == "tester" && password == "secret" {
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
        let stream = match TcpStream::connect(&target).await {
            Ok(stream) => stream,
            Err(_) => return Ok(false),
        };
        crate::runtime::runtime().spawn(async move {
            let mut stream = stream;
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
        if self.state.options.reject_remote_forward {
            return Ok(false);
        }
        let bind = format!("{address}:{port}");
        let listener = match TcpListener::bind(&bind).await {
            Ok(listener) => listener,
            Err(_) => return Ok(false),
        };
        let bound = match listener.local_addr() {
            Ok(addr) => addr.port(),
            Err(_) => return Ok(false),
        };
        *port = u32::from(bound);
        let handle = session.handle();
        let listen_address = address.to_string();
        let task = crate::runtime::runtime().spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let handle = handle.clone();
                let listen_address = listen_address.clone();
                crate::runtime::runtime().spawn(async move {
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

/// Echo TCP service used as the forwarding target.
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
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind echo");
        let port = listener.local_addr().expect("addr").port();
        let task = crate::runtime::runtime().spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                crate::runtime::runtime().spawn(async move {
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

fn trust_host(server: &TestForwardServer) {
    miaominal_storage::KnownHostsStore::new()
        .expect("known hosts")
        .learn("127.0.0.1", server.port, &server.public_key)
        .expect("learn host key");
}

#[allow(clippy::too_many_arguments)]
async fn save_local_rule(
    backend: &Arc<Backend>,
    profile_id: &str,
    rule_id: &str,
    listen_port: u16,
    target_host: &str,
    target_port: u16,
) {
    backend
        .upsert_forward(ForwardRuleDto {
            id: rule_id.to_string(),
            profile_id: profile_id.to_string(),
            label: format!("rule {rule_id}"),
            kind: ForwardKindDto::Local,
            listen_host: "127.0.0.1".to_string(),
            listen_port,
            target_host: target_host.to_string(),
            target_port,
            enabled: false,
        })
        .await
        .expect("upsert local rule");
}

async fn save_remote_rule(
    backend: &Arc<Backend>,
    profile_id: &str,
    rule_id: &str,
    listen_port: u16,
    target_host: &str,
    target_port: u16,
) {
    backend
        .upsert_forward(ForwardRuleDto {
            id: rule_id.to_string(),
            profile_id: profile_id.to_string(),
            label: format!("rule {rule_id}"),
            kind: ForwardKindDto::Remote,
            listen_host: "127.0.0.1".to_string(),
            listen_port,
            target_host: target_host.to_string(),
            target_port,
            enabled: true,
        })
        .await
        .expect("upsert remote rule");
}

async fn wait_for_phase(
    session: &Arc<ForwardSession>,
    phase: ForwardPhase,
    timeout: Duration,
) -> ForwardSnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = session.snapshot();
        if snapshot.phase == phase {
            return snapshot;
        }
        assert!(
            !snapshot.phase.is_terminal() || snapshot.phase == phase,
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

async fn wait_for_terminal(session: &Arc<ForwardSession>, timeout: Duration) -> ForwardSnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = session.snapshot();
        if snapshot.phase.is_terminal() {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a terminal phase; last snapshot {snapshot:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_for_relay_failure(
    session: &Arc<ForwardSession>,
    timeout: Duration,
) -> ForwardSnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = session.snapshot();
        if snapshot.relay_failure.is_some() {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a relay failure; last snapshot {snapshot:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_until(predicate: impl Fn() -> bool, timeout: Duration, what: &str) {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn round_trip(port: u16, payload: &[u8]) -> Vec<u8> {
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
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn local_forward_relays_bytes_through_the_ssh_route() {
    let _guard = shared_test_lock();
    clear_test_root();
    let echo = EchoServer::start().await;
    let server = TestForwardServer::start(ForwardTestOptions::default()).await;
    trust_host(&server);
    let (_host, backend) = backend_for_test();
    backend.initialize().await.expect("initialize backend");
    save_password_host(&backend, "host-1", server.port).await;
    save_local_rule(&backend, "host-1", "rule-1", 0, "127.0.0.1", echo.port).await;

    let session = backend
        .start_forward_session("host-1".to_string(), "rule-1".to_string())
        .await
        .expect("start forward");
    let snapshot = wait_for_phase(&session, ForwardPhase::Listening, Duration::from_secs(20)).await;
    let port = snapshot.bound_port.expect("bound local port");
    assert_eq!(snapshot.label, "rule rule-1");

    let echoed = round_trip(port, b"hello through the tunnel").await;
    assert_eq!(echoed, b"hello through the tunnel");

    assert!(
        backend
            .close_forward_session("host-1".to_string(), "rule-1".to_string())
            .await
            .expect("close forward")
    );
    backend.shutdown();
}

#[tokio::test]
async fn remote_forward_relays_bytes_back_to_the_local_target() {
    let _guard = shared_test_lock();
    clear_test_root();
    let echo = EchoServer::start().await;
    let server = TestForwardServer::start(ForwardTestOptions::default()).await;
    trust_host(&server);
    let (_host, backend) = backend_for_test();
    backend.initialize().await.expect("initialize backend");
    save_password_host(&backend, "host-1", server.port).await;
    save_remote_rule(&backend, "host-1", "rule-1", 0, "127.0.0.1", echo.port).await;

    let session = backend
        .start_forward_session("host-1".to_string(), "rule-1".to_string())
        .await
        .expect("start forward");
    let snapshot = wait_for_phase(&session, ForwardPhase::Connected, Duration::from_secs(20)).await;
    let port = snapshot.bound_port.expect("bound remote port");
    assert_ne!(port, 0);

    let echoed = round_trip(port, b"remote forwarding works").await;
    assert_eq!(echoed, b"remote forwarding works");

    backend.shutdown();
}

#[tokio::test]
async fn occupied_local_port_fails_with_a_typed_failure() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = TestForwardServer::start(ForwardTestOptions::default()).await;
    trust_host(&server);
    let occupied = TcpListener::bind("127.0.0.1:0").await.expect("occupy port");
    let occupied_port = occupied.local_addr().expect("addr").port();
    let (_host, backend) = backend_for_test();
    backend.initialize().await.expect("initialize backend");
    save_password_host(&backend, "host-1", server.port).await;
    save_local_rule(&backend, "host-1", "rule-1", occupied_port, "127.0.0.1", 9).await;

    let session = backend
        .start_forward_session("host-1".to_string(), "rule-1".to_string())
        .await
        .expect("start forward");
    let snapshot = wait_for_terminal(&session, Duration::from_secs(20)).await;

    assert_eq!(snapshot.phase, ForwardPhase::Failed);
    let failure = snapshot.failure.expect("typed failure");
    assert_eq!(failure.kind, ForwardFailureKind::PortOccupied);
    assert_eq!(failure.node_label, "Test host-1");
    assert_eq!(failure.hop_index, 1);
    assert_eq!(failure.hop_total, 1);
    drop(occupied);
    backend.shutdown();
}

#[tokio::test]
async fn refused_remote_bind_is_reported_as_remote_refused() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = TestForwardServer::start(ForwardTestOptions {
        reject_remote_forward: true,
        ..ForwardTestOptions::default()
    })
    .await;
    trust_host(&server);
    let (_host, backend) = backend_for_test();
    backend.initialize().await.expect("initialize backend");
    save_password_host(&backend, "host-1", server.port).await;
    save_remote_rule(&backend, "host-1", "rule-1", 9000, "127.0.0.1", 9).await;

    let session = backend
        .start_forward_session("host-1".to_string(), "rule-1".to_string())
        .await
        .expect("start forward");
    let snapshot = wait_for_terminal(&session, Duration::from_secs(20)).await;

    assert_eq!(snapshot.phase, ForwardPhase::Failed);
    assert_eq!(
        snapshot.failure.expect("typed failure").kind,
        ForwardFailureKind::RemoteRefused
    );
    backend.shutdown();
}

#[tokio::test]
async fn an_unreachable_target_is_a_relay_failure_not_a_session_failure() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = TestForwardServer::start(ForwardTestOptions::default()).await;
    trust_host(&server);
    let (_host, backend) = backend_for_test();
    backend.initialize().await.expect("initialize backend");
    save_password_host(&backend, "host-1", server.port).await;
    // Port 1 on loopback has no listener: the SSH server refuses the direct-tcpip.
    save_local_rule(&backend, "host-1", "rule-1", 0, "127.0.0.1", 1).await;

    let session = backend
        .start_forward_session("host-1".to_string(), "rule-1".to_string())
        .await
        .expect("start forward");
    let snapshot = wait_for_phase(&session, ForwardPhase::Listening, Duration::from_secs(20)).await;
    let port = snapshot.bound_port.expect("bound local port");

    let _ = TcpStream::connect(("127.0.0.1", port)).await;
    let snapshot = wait_for_relay_failure(&session, Duration::from_secs(10)).await;
    assert_eq!(snapshot.phase, ForwardPhase::Listening);
    assert!(snapshot.failure.is_none());
    assert_eq!(
        snapshot.relay_failure.expect("relay failure").kind,
        ForwardFailureKind::TargetUnreachable
    );
    backend.shutdown();
}

#[tokio::test]
async fn host_key_prompt_binds_the_node_and_rejects_stale_answers() {
    let _guard = shared_test_lock();
    clear_test_root();
    let echo = EchoServer::start().await;
    let server = TestForwardServer::start(ForwardTestOptions::default()).await;
    let (_host, backend) = backend_for_test();
    backend.initialize().await.expect("initialize backend");
    save_password_host(&backend, "host-1", server.port).await;
    save_local_rule(&backend, "host-1", "rule-1", 0, "127.0.0.1", echo.port).await;

    let session = backend
        .start_forward_session("host-1".to_string(), "rule-1".to_string())
        .await
        .expect("start forward");
    let snapshot = wait_for_phase(
        &session,
        ForwardPhase::WaitingForTrust,
        Duration::from_secs(20),
    )
    .await;
    let request = snapshot.request.expect("host key request");
    assert_eq!(request.hop_label, "Test host-1");
    assert!(!request.key_changed);

    let error = session
        .respond_host_key(
            request.request_id.wrapping_add(999),
            SshHostKeyDecision::AcceptOnce,
        )
        .expect_err("a stale answer must be rejected");
    assert!(matches!(error, BackendError::RequestExpired { .. }));

    session
        .respond_host_key(request.request_id, SshHostKeyDecision::AcceptAndSave)
        .expect("answer the host key prompt");
    let snapshot = wait_for_phase(&session, ForwardPhase::Listening, Duration::from_secs(20)).await;
    let port = snapshot.bound_port.expect("bound local port");
    assert_eq!(round_trip(port, b"trusted").await, b"trusted");
    backend.shutdown();
}

#[tokio::test]
async fn cancelling_during_authentication_stops_the_session() {
    let _guard = shared_test_lock();
    clear_test_root();
    let server = TestForwardServer::start(ForwardTestOptions {
        hang_auth: true,
        ..ForwardTestOptions::default()
    })
    .await;
    trust_host(&server);
    let (_host, backend) = backend_for_test();
    backend.initialize().await.expect("initialize backend");
    save_password_host(&backend, "host-1", server.port).await;
    save_local_rule(&backend, "host-1", "rule-1", 0, "127.0.0.1", 9).await;

    let session = backend
        .start_forward_session("host-1".to_string(), "rule-1".to_string())
        .await
        .expect("start forward");
    wait_for_phase(
        &session,
        ForwardPhase::WaitingForAuthentication,
        Duration::from_secs(20),
    )
    .await;

    let started = Instant::now();
    session.cancel();
    let snapshot = wait_for_terminal(&session, Duration::from_secs(8)).await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "cancel must interrupt authentication, took {:?}",
        started.elapsed()
    );
    assert_eq!(snapshot.phase, ForwardPhase::Stopped);
    assert!(snapshot.request.is_none());
    backend.shutdown();
}

#[tokio::test]
async fn a_stopped_rule_can_be_started_again_with_a_fresh_connection() {
    let _guard = shared_test_lock();
    clear_test_root();
    let echo = EchoServer::start().await;
    let server = TestForwardServer::start(ForwardTestOptions::default()).await;
    trust_host(&server);
    let (_host, backend) = backend_for_test();
    backend.initialize().await.expect("initialize backend");
    save_password_host(&backend, "host-1", server.port).await;
    save_local_rule(&backend, "host-1", "rule-1", 0, "127.0.0.1", echo.port).await;

    let first = backend
        .start_forward_session("host-1".to_string(), "rule-1".to_string())
        .await
        .expect("first start");
    let first_snapshot =
        wait_for_phase(&first, ForwardPhase::Listening, Duration::from_secs(20)).await;
    let first_port = first_snapshot.bound_port.expect("first port");
    let first_id = first_snapshot.connection_id;

    assert!(
        backend
            .close_forward_session("host-1".to_string(), "rule-1".to_string())
            .await
            .expect("stop forward")
    );
    assert!(
        !backend
            .close_forward_session("host-1".to_string(), "rule-1".to_string())
            .await
            .expect("repeated stop is idempotent")
    );
    wait_until(
        || std::net::TcpListener::bind(("127.0.0.1", first_port)).is_ok(),
        Duration::from_secs(5),
        "the stopped local listener to release its port",
    )
    .await;

    let second = backend
        .start_forward_session("host-1".to_string(), "rule-1".to_string())
        .await
        .expect("second start");
    let second_snapshot =
        wait_for_phase(&second, ForwardPhase::Listening, Duration::from_secs(20)).await;
    assert_ne!(second_snapshot.connection_id, first_id);
    assert_eq!(
        round_trip(second_snapshot.bound_port.expect("second port"), b"again").await,
        b"again"
    );
    backend.shutdown();
}

#[tokio::test]
async fn starting_an_active_rule_again_reuses_the_session() {
    let _guard = shared_test_lock();
    clear_test_root();
    let echo = EchoServer::start().await;
    let server = TestForwardServer::start(ForwardTestOptions::default()).await;
    trust_host(&server);
    let (_host, backend) = backend_for_test();
    backend.initialize().await.expect("initialize backend");
    save_password_host(&backend, "host-1", server.port).await;
    save_local_rule(&backend, "host-1", "rule-1", 0, "127.0.0.1", echo.port).await;

    let first = backend
        .start_forward_session("host-1".to_string(), "rule-1".to_string())
        .await
        .expect("first start");
    let first_snapshot =
        wait_for_phase(&first, ForwardPhase::Listening, Duration::from_secs(20)).await;

    let second = backend
        .start_forward_session("host-1".to_string(), "rule-1".to_string())
        .await
        .expect("second start");
    assert_eq!(
        second.snapshot().connection_id,
        first_snapshot.connection_id,
        "starting an active rule must reuse the running session"
    );
    backend.shutdown();
}

#[tokio::test]
async fn concurrent_starts_of_the_same_rule_share_one_session() {
    let _guard = shared_test_lock();
    clear_test_root();
    let echo = EchoServer::start().await;
    let server = TestForwardServer::start(ForwardTestOptions::default()).await;
    trust_host(&server);
    let (_host, backend) = backend_for_test();
    backend.initialize().await.expect("initialize backend");
    save_password_host(&backend, "host-1", server.port).await;
    save_local_rule(&backend, "host-1", "rule-1", 0, "127.0.0.1", echo.port).await;

    for round in 0..5 {
        let starts = (0..8)
            .map(|_| backend.start_forward_session("host-1".to_string(), "rule-1".to_string()))
            .collect::<Vec<_>>();
        let sessions = futures::future::join_all(starts).await;
        let connection_ids = sessions
            .iter()
            .map(|session| {
                session
                    .as_ref()
                    .expect("every concurrent start must succeed")
                    .snapshot()
                    .connection_id
            })
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            connection_ids.len(),
            1,
            "round {round}: concurrent starts created {} connections for one rule",
            connection_ids.len()
        );
        assert_eq!(backend.forward_session_snapshots().len(), 1);

        if round == 0 {
            let snapshot = wait_for_phase(
                &sessions[0].as_ref().unwrap(),
                ForwardPhase::Listening,
                Duration::from_secs(20),
            )
            .await;
            let port = snapshot.bound_port.expect("bound local port");
            assert_eq!(
                round_trip(port, b"single connection").await,
                b"single connection"
            );
        }

        assert!(
            backend
                .close_forward_session("host-1".to_string(), "rule-1".to_string())
                .await
                .expect("stop forward")
        );
        assert!(backend.forward_session_snapshots().is_empty());
    }
    backend.shutdown();
}

#[tokio::test]
async fn shutdown_stops_every_forward_session_and_releases_ports() {
    let _guard = shared_test_lock();
    clear_test_root();
    let echo = EchoServer::start().await;
    let server = TestForwardServer::start(ForwardTestOptions::default()).await;
    trust_host(&server);
    let (_host, backend) = backend_for_test();
    backend.initialize().await.expect("initialize backend");
    save_password_host(&backend, "host-1", server.port).await;
    save_local_rule(&backend, "host-1", "rule-1", 0, "127.0.0.1", echo.port).await;
    save_local_rule(&backend, "host-1", "rule-2", 0, "127.0.0.1", echo.port).await;

    let first = backend
        .start_forward_session("host-1".to_string(), "rule-1".to_string())
        .await
        .expect("start first");
    let first_snapshot =
        wait_for_phase(&first, ForwardPhase::Listening, Duration::from_secs(20)).await;
    let second = backend
        .start_forward_session("host-1".to_string(), "rule-2".to_string())
        .await
        .expect("start second");
    let second_snapshot =
        wait_for_phase(&second, ForwardPhase::Listening, Duration::from_secs(20)).await;

    backend.shutdown();

    assert!(backend.forward_session_snapshots().is_empty());
    assert!(first.snapshot().phase.is_terminal());
    assert!(second.snapshot().phase.is_terminal());
    for port in [
        first_snapshot.bound_port.expect("first port"),
        second_snapshot.bound_port.expect("second port"),
    ] {
        wait_until(
            || std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(),
            Duration::from_secs(5),
            "the shutdown listener to release its port",
        )
        .await;
    }
}

#[tokio::test]
async fn a_missing_rule_is_rejected_before_connecting() {
    let _guard = shared_test_lock();
    clear_test_root();
    let (_host, backend) = backend_for_test();
    backend.initialize().await.expect("initialize backend");
    save_password_host(&backend, "host-1", 65000).await;

    let result = backend
        .start_forward_session("host-1".to_string(), "missing".to_string())
        .await;
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("a missing rule must be rejected"),
    };
    assert!(matches!(error, BackendError::InvalidArgument { .. }));
    assert!(backend.forward_session_snapshots().is_empty());
    backend.shutdown();
}
