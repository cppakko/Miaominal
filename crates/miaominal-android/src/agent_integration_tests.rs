//! Integration tests for AI CLI channel sessions (exec + loopback tunnel).
//!
//! A minimal `russh` server on `127.0.0.1` accepts password authentication and
//! answers three scripted exec commands (`emit`, `exit7`, and anything else is
//! an echo program that streams stdin back). `channel_open_direct_tcpip`
//! bridges to a real local echo listener, which exercises the full
//! local listener -> SSH channel -> remote target path.

#![cfg(test)]

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::keys::{Algorithm, PrivateKey};
use russh::server::{self, Auth, Msg, Session as ServerSession};
use russh::{Channel, ChannelId, SshId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::agent::{AgentChannelEvent, AgentHostKeyDecision, AgentSshSession};
use crate::backend::{Backend, BackendConfig};
use crate::dto::{AuthMethodDto, CredentialAction, CredentialKindDto, HostUpdate};
use crate::test_support::{business_root, cache_root, clear_test_root, shared_test_lock, test_host};

struct AgentTestServer {
    port: u16,
    public_key: russh::keys::PublicKey,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for AgentTestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl AgentTestServer {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("host key");
        let public_key = host_key.public_key().clone();
        let config = Arc::new(server::Config {
            server_id: SshId::Standard("SSH-2.0-Miaominal-Agent-Test".into()),
            methods: russh::MethodSet::from(&[russh::MethodKind::Password][..]),
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![host_key],
            ..Default::default()
        });
        let task = crate::runtime::runtime().spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let config = Arc::clone(&config);
                crate::runtime::runtime().spawn(async move {
                    let running = server::run_stream(
                        config,
                        stream,
                        AgentTestHandler {
                            echo_channels: HashSet::new(),
                        },
                    )
                    .await;
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

struct AgentTestHandler {
    echo_channels: HashSet<ChannelId>,
}

impl server::Handler for AgentTestHandler {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == "tester" && password == "secret" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        _session: &mut ServerSession,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).trim().to_string();
        let _ = session.channel_success(channel);
        match command.as_str() {
            "emit" => {
                let _ = session.data(channel, b"stdout-payload\n".to_vec());
                let _ = session.extended_data(channel, 1, b"stderr-payload\n".to_vec());
                let _ = session.exit_status_request(channel, 0);
                let _ = session.eof(channel);
                let _ = session.close(channel);
            }
            "exit7" => {
                let _ = session.exit_status_request(channel, 7);
                let _ = session.eof(channel);
                let _ = session.close(channel);
            }
            "exit7-eof-first" => {
                let _ = session.data(channel, b"last-output\n".to_vec());
                let _ = session.eof(channel);
                let _ = session.exit_status_request(channel, 7);
                let _ = session.close(channel);
            }
            "disconnect" => {
                let _ = session.disconnect(russh::Disconnect::ByApplication, "server ended", "");
            }
            "flood" => {
                for _ in 0..900 {
                    let _ = session.data(channel, vec![b'x'; 1024]);
                }
            }
            _ => {
                self.echo_channels.insert(channel);
            }
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        if self.echo_channels.contains(&channel) {
            let _ = session.data(channel, data.to_vec());
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        if self.echo_channels.remove(&channel) {
            let _ = session.exit_status_request(channel, 0);
        }
        let _ = session.eof(channel);
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        self.echo_channels.remove(&channel);
        let _ = session.close(channel);
        Ok(())
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
        let target = format!("{host_to_connect}:{port_to_connect}");
        crate::runtime::runtime().spawn(async move {
            match TcpStream::connect(&target).await {
                Ok(mut stream) => {
                    let mut forwarded = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut forwarded, &mut stream).await;
                }
                Err(_) => {
                    let _ = channel.close().await;
                }
            }
        });
        Ok(true)
    }
}

fn backend_for_test() -> Arc<Backend> {
    Backend::new(
        test_host(),
        BackendConfig {
            data_dir: business_root().to_string_lossy().into_owned(),
            cache_dir: cache_root().to_string_lossy().into_owned(),
        },
    )
}

async fn save_host(backend: &Arc<Backend>, id: &str, port: u16) {
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
}

async fn set_password(backend: &Arc<Backend>, host_id: &str) {
    backend
        .set_host_credential(
            host_id.to_string(),
            CredentialKindDto::Password,
            CredentialAction::Set {
                value: "secret".to_string(),
            },
        )
        .await
        .expect("set credential");
}

fn trust_host(server: &AgentTestServer) {
    miaominal_storage::KnownHostsStore::new()
        .expect("known hosts")
        .learn("127.0.0.1", server.port, &server.public_key)
        .expect("learn host key");
}

async fn open_agent(backend: &Arc<Backend>, host_id: &str) -> Arc<AgentSshSession> {
    backend
        .open_agent_session(host_id.to_string())
        .await
        .expect("open agent session")
}

/// Pull events until `predicate` accepts the accumulated stream.
async fn wait_events<F>(
    session: &Arc<AgentSshSession>,
    timeout: Duration,
    predicate: F,
) -> Vec<AgentChannelEvent>
where
    F: Fn(&[AgentChannelEvent]) -> bool,
{
    let deadline = Instant::now() + timeout;
    let mut events = Vec::new();
    loop {
        if predicate(&events) {
            return events;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for agent events; collected: {events:?}"
        );
        let batch = session
            .read_events(32, 200)
            .await
            .expect("read agent events");
        events.extend(batch.events);
        if batch.closed {
            assert!(
                predicate(&events),
                "agent session closed before the condition was met: {events:?}"
            );
            return events;
        }
    }
}

fn stdout_text(events: &[AgentChannelEvent]) -> String {
    let mut text = String::new();
    for event in events {
        if let AgentChannelEvent::Stdout { data, .. } = event {
            text.push_str(&String::from_utf8_lossy(data));
        }
    }
    text
}

fn stderr_text(events: &[AgentChannelEvent]) -> String {
    let mut text = String::new();
    for event in events {
        if let AgentChannelEvent::Stderr { data, .. } = event {
            text.push_str(&String::from_utf8_lossy(data));
        }
    }
    text
}

#[test]
fn exec_channel_keeps_stdout_stderr_and_exit_status_separate() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = AgentTestServer::start().await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(&backend, "host-1", server.port).await;
        set_password(&backend, "host-1").await;

        let session = open_agent(&backend, "host-1").await;
        wait_events(&session, Duration::from_secs(10), |events| {
            events
                .iter()
                .any(|event| matches!(event, AgentChannelEvent::Ready))
        })
        .await;

        session
            .open_exec(1, "emit".to_string(), Vec::new())
            .expect("open exec");
        let events = wait_events(&session, Duration::from_secs(10), |events| {
            events
                .iter()
                .any(|event| matches!(event, AgentChannelEvent::ExecExited { channel_id: 1, .. }))
        })
        .await;

        assert!(
            events.iter().any(|event| matches!(
                event,
                AgentChannelEvent::ExecOpened { channel_id: 1 }
            )),
            "expected the exec request to be accepted: {events:?}"
        );
        assert_eq!(stdout_text(&events), "stdout-payload\n");
        assert_eq!(stderr_text(&events), "stderr-payload\n");
        assert!(
            events.iter().any(|event| matches!(
                event,
                AgentChannelEvent::ExecExited {
                    channel_id: 1,
                    exit_code: Some(0),
                    ..
                }
            )),
            "expected exit status 0: {events:?}"
        );

        assert!(backend.close_agent_session(session.connection_id()).await.expect("close"));
        assert!(session.is_closed());
    });
}

#[test]
fn exec_stdin_reaches_the_remote_program() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = AgentTestServer::start().await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(&backend, "host-1", server.port).await;
        set_password(&backend, "host-1").await;

        let session = open_agent(&backend, "host-1").await;
        wait_events(&session, Duration::from_secs(10), |events| {
            events
                .iter()
                .any(|event| matches!(event, AgentChannelEvent::Ready))
        })
        .await;

        session
            .open_exec(7, "echo".to_string(), Vec::new())
            .expect("open exec");
        wait_events(&session, Duration::from_secs(10), |events| {
            events
                .iter()
                .any(|event| matches!(event, AgentChannelEvent::ExecOpened { channel_id: 7 }))
        })
        .await;
        session
            .write_stdin(7, b"ping from stdin\n".to_vec())
            .expect("write stdin");
        let events = wait_events(&session, Duration::from_secs(10), |events| {
            stdout_text(events).contains("ping from stdin")
        })
        .await;
        assert!(stdout_text(&events).contains("ping from stdin"));

        session.close_channel(7).expect("close channel");
        let events = wait_events(&session, Duration::from_secs(10), |events| {
            events
                .iter()
                .any(|event| matches!(event, AgentChannelEvent::ExecExited { channel_id: 7, .. }))
        })
        .await;
        assert!(events.iter().any(|event| matches!(
            event,
            AgentChannelEvent::ExecExited {
                channel_id: 7,
                exit_code: Some(0),
                ..
            }
        )));

        backend
            .close_agent_session(session.connection_id())
            .await
            .expect("close session");
    });
}

#[test]
fn non_zero_exit_status_is_reported() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = AgentTestServer::start().await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(&backend, "host-1", server.port).await;
        set_password(&backend, "host-1").await;

        let session = open_agent(&backend, "host-1").await;
        wait_events(&session, Duration::from_secs(10), |events| {
            events
                .iter()
                .any(|event| matches!(event, AgentChannelEvent::Ready))
        })
        .await;

        session
            .open_exec(3, "exit7".to_string(), Vec::new())
            .expect("open exec");
        let events = wait_events(&session, Duration::from_secs(10), |events| {
            events
                .iter()
                .any(|event| matches!(event, AgentChannelEvent::ExecExited { channel_id: 3, .. }))
        })
        .await;
        assert!(events.iter().any(|event| matches!(
            event,
            AgentChannelEvent::ExecExited {
                channel_id: 3,
                exit_code: Some(7),
                ..
            }
        )));

        backend
            .close_agent_session(session.connection_id())
            .await
            .expect("close session");
    });
}

#[test]
fn eof_before_exit_status_preserves_the_status_and_output() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = AgentTestServer::start().await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.unwrap();
        save_host(&backend, "host-1", server.port).await;
        set_password(&backend, "host-1").await;
        let session = open_agent(&backend, "host-1").await;
        wait_events(&session, Duration::from_secs(10), |events| {
            events
                .iter()
                .any(|event| matches!(event, AgentChannelEvent::Ready))
        })
        .await;
        session
            .open_exec(1, "exit7-eof-first".into(), vec![])
            .unwrap();
        let events = wait_events(&session, Duration::from_secs(10), |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    AgentChannelEvent::ExecExited {
                        exit_code: Some(7),
                        ..
                    }
                )
            })
        })
        .await;
        assert_eq!(stdout_text(&events), "last-output\n");
        backend
            .close_agent_session(session.connection_id())
            .await
            .unwrap();
    });
}

#[test]
fn server_disconnect_ends_the_stream_and_closes_the_handle() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = AgentTestServer::start().await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.unwrap();
        save_host(&backend, "host-1", server.port).await;
        set_password(&backend, "host-1").await;
        let session = open_agent(&backend, "host-1").await;
        wait_events(&session, Duration::from_secs(10), |events| {
            events
                .iter()
                .any(|event| matches!(event, AgentChannelEvent::Ready))
        })
        .await;
        session.open_exec(1, "disconnect".into(), vec![]).unwrap();
        wait_events(&session, Duration::from_secs(10), |events| {
            events
                .iter()
                .any(|event| matches!(event, AgentChannelEvent::Closed))
        })
        .await;
        assert!(session.is_closed());
        assert!(matches!(
            session.open_exec(2, "emit".into(), vec![]),
            Err(crate::error::BackendError::Closed)
        ));
        tokio::time::timeout(
            Duration::from_secs(2),
            backend.close_agent_session(session.connection_id()),
        )
        .await
        .unwrap()
        .unwrap();
    });
}

#[test]
fn closing_a_full_event_queue_joins_without_consuming_events() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = AgentTestServer::start().await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.unwrap();
        save_host(&backend, "host-1", server.port).await;
        set_password(&backend, "host-1").await;
        let session = open_agent(&backend, "host-1").await;
        wait_events(&session, Duration::from_secs(10), |events| {
            events
                .iter()
                .any(|event| matches!(event, AgentChannelEvent::Ready))
        })
        .await;
        session.open_exec(1, "flood".into(), vec![]).unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        tokio::time::timeout(
            Duration::from_secs(2),
            backend.close_agent_session(session.connection_id()),
        )
        .await
        .expect("close must not need a consumer")
        .unwrap();
        let first = session.read_events(1, 200).await.unwrap();
        assert!(!first.closed, "queued data must be drained before closure");
        wait_events(&session, Duration::from_secs(5), |events| {
            events
                .iter()
                .any(|event| matches!(event, AgentChannelEvent::Closed))
        })
        .await;
    });
}

#[test]
fn loopback_tunnel_forwards_to_the_remote_target() {
    assert_tunnel_connection_closes(false);
}

#[test]
fn closing_a_session_terminates_established_tunnel_connections() {
    assert_tunnel_connection_closes(true);
}

fn assert_tunnel_connection_closes(close_session: bool) {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let echo_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind echo");
        let echo_port = echo_listener.local_addr().expect("addr").port();
        let echo_task = crate::runtime::runtime().spawn(async move {
            loop {
                let Ok((mut stream, _)) = echo_listener.accept().await else {
                    break;
                };
                crate::runtime::runtime().spawn(async move {
                    let mut buffer = [0u8; 1024];
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

        let server = AgentTestServer::start().await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(&backend, "host-1", server.port).await;
        set_password(&backend, "host-1").await;

        let session = open_agent(&backend, "host-1").await;
        wait_events(&session, Duration::from_secs(10), |events| {
            events
                .iter()
                .any(|event| matches!(event, AgentChannelEvent::Ready))
        })
        .await;

        session
            .open_loopback_tunnel(9, "127.0.0.1".to_string(), echo_port)
            .expect("open tunnel");
        let events = wait_events(&session, Duration::from_secs(10), |events| {
            events
                .iter()
                .any(|event| matches!(event, AgentChannelEvent::TunnelReady { tunnel_id: 9, .. }))
        })
        .await;
        let local_port = events
            .iter()
            .find_map(|event| match event {
                AgentChannelEvent::TunnelReady { local_port, .. } => Some(*local_port),
                _ => None,
            })
            .expect("tunnel port");

        let mut client = TcpStream::connect(("127.0.0.1", local_port))
            .await
            .expect("connect tunnel");
        client.write_all(b"hello tunnel\n").await.expect("write");
        let mut buffer = vec![0u8; 64];
        let read = tokio::time::timeout(Duration::from_secs(5), client.read(&mut buffer))
            .await
            .expect("read timeout")
            .expect("read");
        assert_eq!(&buffer[..read], b"hello tunnel\n");

        if close_session {
            backend
                .close_agent_session(session.connection_id())
                .await
                .expect("close session with an active relay");
        } else {
            session.close_tunnel(9).expect("close tunnel");
            wait_events(&session, Duration::from_secs(5), |events| {
                events.iter().any(|event| matches!(
                    event,
                    AgentChannelEvent::TunnelClosed { tunnel_id: 9 }
                ))
            })
            .await;
        }
        let result = tokio::time::timeout(Duration::from_secs(2), client.read(&mut buffer))
            .await
            .expect("closing must terminate an established relay, not just its listener");
        assert!(
            matches!(result, Ok(0) | Err(_)),
            "relay remained readable: {result:?}"
        );
        backend
            .close_agent_session(session.connection_id())
            .await
            .expect("close session");
        echo_task.abort();
    });
}

#[test]
fn host_key_prompt_is_relayed_and_rejection_closes_the_session() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = AgentTestServer::start().await;
        // Deliberately do not trust the host.
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(&backend, "host-1", server.port).await;
        set_password(&backend, "host-1").await;

        let session = open_agent(&backend, "host-1").await;
        let events = wait_events(&session, Duration::from_secs(10), |events| {
            events
                .iter()
                .any(|event| matches!(event, AgentChannelEvent::HostKeyPrompt { .. }))
        })
        .await;
        let request_id = events
            .iter()
            .find_map(|event| match event {
                AgentChannelEvent::HostKeyPrompt { prompt } => Some(prompt.request_id),
                _ => None,
            })
            .expect("host key request");
        session
            .respond_host_key(request_id, AgentHostKeyDecision::Reject)
            .expect("respond");

        let events = wait_events(&session, Duration::from_secs(10), |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    AgentChannelEvent::Error { .. } | AgentChannelEvent::Closed
                )
            })
        })
        .await;
        assert!(
            events.iter().any(|event| matches!(
                event,
                AgentChannelEvent::Error { .. } | AgentChannelEvent::Closed
            )),
            "rejecting the host key must not leave the session hanging: {events:?}"
        );
        assert!(backend
            .close_agent_session(session.connection_id())
            .await
            .expect("close session"));
    });
}

#[test]
fn closing_a_session_is_idempotent_and_releases_the_worker() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = AgentTestServer::start().await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(&backend, "host-1", server.port).await;
        set_password(&backend, "host-1").await;

        let session = open_agent(&backend, "host-1").await;
        wait_events(&session, Duration::from_secs(10), |events| {
            events
                .iter()
                .any(|event| matches!(event, AgentChannelEvent::Ready))
        })
        .await;

        let connection_id = session.connection_id();
        assert!(backend
            .close_agent_session(connection_id)
            .await
            .expect("first close"));
        assert!(!backend
            .close_agent_session(connection_id)
            .await
            .expect("second close"));
        assert!(session.is_closed());

        // Explicit cancel is idempotent and the event stream ends closed.
        session.cancel();
        session.cancel();
        let batch = session.read_events(32, 200).await.expect("read after close");
        assert!(batch.closed || session.is_closed());
    });
}
