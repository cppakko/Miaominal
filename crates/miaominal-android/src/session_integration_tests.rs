//! Integration tests for interactive SSH sessions against a local test server.
//!
//! The test server is a real `russh` server on `127.0.0.1` with password,
//! public-key, certificate, and multi-round keyboard-interactive authentication
//! plus a scripted shell channel. Tests hold the shared embedded-root lock for
//! their whole body and drive the process runtime explicitly.

#![cfg(test)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use russh::keys::{Algorithm, HashAlg, PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg, Session as ServerSession};
use russh::{Channel, ChannelId, Pty, Sig, SshId};
use tokio::net::TcpListener;

use crate::backend::{Backend, BackendConfig};
use crate::dto::{AuthMethodDto, CredentialAction, CredentialKindDto, HostUpdate};
use crate::error::BackendError;
use crate::session::{
    SshHostKeyDecision, SshRequestKind, SshSession, SshSessionPhase, SshSessionSnapshot,
};
use crate::test_support::{
    business_root, cache_root, clear_test_root, shared_test_lock, test_host,
};

#[derive(Clone)]
struct ServerOptions {
    password: String,
    authorized_key: Option<PublicKey>,
    accept_certificate_user: Option<String>,
    reject_pty: bool,
    reject_shell: bool,
    reject_agent_forward: bool,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            password: "secret".to_string(),
            authorized_key: None,
            accept_certificate_user: None,
            reject_pty: false,
            reject_shell: false,
            reject_agent_forward: false,
        }
    }
}

struct ServerState {
    options: ServerOptions,
    window: Mutex<(u32, u32)>,
    kbi_round: Mutex<u32>,
}

struct TestSshServer {
    port: u16,
    public_key: PublicKey,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TestSshServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TestSshServer {
    async fn start(options: ServerOptions) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let host_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("host key");
        let public_key = host_key.public_key().clone();
        let config = Arc::new(server::Config {
            server_id: SshId::Standard("SSH-2.0-Miaominal-Test".into()),
            methods: russh::MethodSet::from(
                &[
                    russh::MethodKind::Password,
                    russh::MethodKind::PublicKey,
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
            window: Mutex::new((80, 24)),
            kbi_round: Mutex::new(0),
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
                        server::run_stream(config, stream, TestServerHandler { state }).await;
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

struct TestServerHandler {
    state: Arc<ServerState>,
}

impl server::Handler for TestServerHandler {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(
            if user == "tester" && password == self.state.options.password {
                Auth::Accept
            } else {
                Auth::reject()
            },
        )
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &russh::keys::PublicKey,
    ) -> Result<Auth, Self::Error> {
        let accepted = self
            .state
            .options
            .authorized_key
            .as_ref()
            .is_some_and(|authorized| authorized.key_data() == public_key.key_data());
        Ok(if user == "tester" && accepted {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_openssh_certificate(
        &mut self,
        user: &str,
        _certificate: &russh::keys::Certificate,
    ) -> Result<Auth, Self::Error> {
        let accepted = self
            .state
            .options
            .accept_certificate_user
            .as_deref()
            .is_some_and(|expected| expected == user);
        Ok(if accepted {
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
                if answer == self.state.options.password {
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
        _channel: Channel<Msg>,
        _session: &mut ServerSession,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        *self.state.window.lock().expect("window lock") = (col_width, row_height);
        if self.state.options.reject_pty {
            let _ = session.channel_failure(channel);
        } else {
            let _ = session.channel_success(channel);
        }
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        _channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        *self.state.window.lock().expect("window lock") = (col_width, row_height);
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        if self.state.options.reject_shell {
            let _ = session.channel_failure(channel);
            return Ok(());
        }
        let _ = session.channel_success(channel);
        let _ = session.data(channel, b"$ ".to_vec());
        Ok(())
    }

    async fn signal(
        &mut self,
        _channel: ChannelId,
        _signal: Sig,
        _session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn agent_request(
        &mut self,
        channel: ChannelId,
        session: &mut ServerSession,
    ) -> Result<bool, Self::Error> {
        if self.state.options.reject_agent_forward {
            let _ = session.channel_failure(channel);
        } else {
            let _ = session.channel_success(channel);
        }
        Ok(true)
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        let _ = self;
        if data.contains(&0x03) {
            let _ = session.data(channel, b"^C\r\n$ ".to_vec());
            return Ok(());
        }
        let text = String::from_utf8_lossy(data);
        let command = text.trim_end_matches(['\r', '\n']).to_string();
        // A terminal query response must be answered by the client even when
        // the server sends nothing else: "ANSWERED" only arrives after the
        // response is written back.
        if command.starts_with("\u{1b}[?2026") {
            let _ = session.data(channel, b"ANSWERED\r\n$ ".to_vec());
            return Ok(());
        }
        let payload: Vec<u8> = match command.as_str() {
            "" => b"$ ".to_vec(),
            "query" => {
                // DECRQM for bracketed paste; the terminal answers with a
                // PtyWrite escape sequence that must reach the server.
                b"\x1b[?2026$p".to_vec()
            }
            "vim" => {
                b"\x1b[?1049h\x1b[2J\x1b[H\x1b[48;2;0;0;0m\x1b[38;2;255;120;0mFULLSCREEN\x1b[0m \xe4\xb8\xad\xe6\x96\x87 e\xcc\x81 \xe2\x94\x80\xe2\x94\x80\r\n".to_vec()
            }
            "size" => {
                let (columns, rows) = *self.state.window.lock().expect("window lock");
                format!("{columns}x{rows}\r\n$ ").into_bytes()
            }
            "flood" => {
                let mut bytes = Vec::new();
                for index in 0..300 {
                    bytes.extend_from_slice(format!("line-{index:03}\r\n").as_bytes());
                }
                bytes.push(b'$');
                bytes.push(b' ');
                bytes
            }
            other => format!("{other}\r\n$ ").into_bytes(),
        };
        let _ = session.data(channel, payload);
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        let _ = session.eof(channel);
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        let _ = session.close(channel);
        Ok(())
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

async fn save_host(
    backend: &Arc<Backend>,
    id: &str,
    port: u16,
    auth_method: AuthMethodDto,
    private_key_path: Option<PathBuf>,
) {
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
            auth_method: Some(auth_method),
            private_key_path: private_key_path.map(|path| path.to_string_lossy().into_owned()),
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

async fn set_credential(
    backend: &Arc<Backend>,
    host_id: &str,
    kind: CredentialKindDto,
    value: &str,
) {
    backend
        .set_host_credential(
            host_id.to_string(),
            kind,
            CredentialAction::Set {
                value: value.to_string(),
            },
        )
        .await
        .expect("set credential");
}

fn trust_host(server: &TestSshServer) {
    miaominal_storage::KnownHostsStore::new()
        .expect("known hosts")
        .learn("127.0.0.1", server.port, &server.public_key)
        .expect("learn host key");
}

async fn wait_phase(
    session: &Arc<SshSession>,
    phase: SshSessionPhase,
    timeout: Duration,
) -> SshSessionSnapshot {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = session.snapshot();
        if snapshot.phase == phase {
            return snapshot;
        }
        if snapshot.phase.is_terminal() && phase != snapshot.phase {
            panic!("session ended as {:?}: {snapshot:?}", snapshot.phase);
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {phase:?}; last snapshot {snapshot:?}"
        );
        let _ = session.await_update(snapshot.revision, 200).await;
    }
}

async fn wait_until_terminal(session: &Arc<SshSession>, timeout: Duration) -> SshSessionSnapshot {
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
        let _ = session.await_update(snapshot.revision, 200).await;
    }
}

fn frame_text(session: &Arc<SshSession>) -> String {
    let frame = session.render_frame(0);
    frame
        .rows
        .iter()
        .map(|row| {
            row.cells
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn wait_output(session: &Arc<SshSession>, needle: &str, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        let text = frame_text(session);
        if text.contains(needle) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {needle:?}; terminal was:\n{text}"
        );
        let revision = session.snapshot().revision;
        let _ = session.await_update(revision, 200).await;
    }
}

async fn respond_when_prompted(
    session: &Arc<SshSession>,
    kind: SshRequestKind,
    timeout: Duration,
) -> crate::session::SshSessionRequest {
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
        if snapshot.phase.is_terminal() {
            panic!("session ended before {kind:?}: {snapshot:?}");
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {kind:?}; last snapshot {snapshot:?}"
        );
        let _ = session.await_update(snapshot.revision, 200).await;
    }
}

#[test]
fn password_session_becomes_ready_and_drives_the_terminal() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = TestSshServer::start(ServerOptions::default()).await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(
            &backend,
            "host-1",
            server.port,
            AuthMethodDto::Password,
            None,
        )
        .await;
        set_credential(&backend, "host-1", CredentialKindDto::Password, "secret").await;

        let session = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("open session");
        wait_phase(&session, SshSessionPhase::Ready, Duration::from_secs(10)).await;

        // Remote output drives the grid.
        session
            .write_bytes(b"echo hello\r".to_vec())
            .expect("write");
        let text = wait_output(&session, "hello", Duration::from_secs(10)).await;
        assert!(text.contains("$ "), "expected a shell prompt in:\n{text}");

        // Ctrl-C is delivered as a control byte.
        session.write_bytes(vec![0x03]).expect("ctrl-c");
        wait_output(&session, "^C", Duration::from_secs(10)).await;

        // Resize reaches the remote PTY.
        session.resize(100, 30).expect("resize");
        session.write_bytes(b"size\r".to_vec()).expect("write size");
        let text = wait_output(&session, "100x30", Duration::from_secs(10)).await;
        assert!(text.contains("100x30"));

        // Alternate screen, true color, CJK and combining characters survive.
        session.write_bytes(b"vim\r".to_vec()).expect("write vim");
        let text = wait_output(&session, "FULLSCREEN", Duration::from_secs(10)).await;
        // Wide characters keep a spacer cell between them, so check each glyph.
        assert!(text.contains('中'), "missing CJK glyph in:\n{text}");
        assert!(text.contains('文'), "missing CJK glyph in:\n{text}");
        assert!(
            text.contains("e\u{301}"),
            "missing combining glyph in:\n{text}"
        );
        let frame = session.render_frame(0);
        assert!(frame.modes.alternate_screen);
        assert!(frame.rows.len() >= 24);

        // The rendered frame contains the true-color foreground somewhere.
        let has_truecolor = frame
            .rows
            .iter()
            .flat_map(|row| row.cells.iter())
            .any(|cell| cell.fg == 0xffff7800);
        assert!(has_truecolor, "expected the true-color cell");

        session.disconnect().await.expect("disconnect");
        let snapshot = session.snapshot();
        assert_eq!(snapshot.phase, SshSessionPhase::Closed);
    });
}

#[test]
fn unknown_host_prompts_and_accept_once_still_connects() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = TestSshServer::start(ServerOptions::default()).await;
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(
            &backend,
            "host-1",
            server.port,
            AuthMethodDto::Password,
            None,
        )
        .await;
        set_credential(&backend, "host-1", CredentialKindDto::Password, "secret").await;

        let session = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("open session");
        let request =
            respond_when_prompted(&session, SshRequestKind::HostKey, Duration::from_secs(10)).await;
        assert!(!request.key_changed);
        assert_eq!(request.fingerprint, server.fingerprint());
        assert_eq!(request.host, "127.0.0.1");
        assert_eq!(request.port, server.port);

        session
            .respond_host_key(request.request_id, SshHostKeyDecision::AcceptOnce)
            .expect("accept once");
        wait_phase(&session, SshSessionPhase::Ready, Duration::from_secs(10)).await;
        // Accepting once must not persist the key.
        let known = miaominal_storage::KnownHostsStore::new()
            .expect("known hosts")
            .list()
            .expect("list");
        assert!(
            !known
                .iter()
                .any(|entry| entry.host == "127.0.0.1" && entry.port == server.port)
        );

        session.disconnect().await.expect("disconnect");
    });
}

#[test]
fn changed_host_key_is_reported_as_a_mismatch() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = TestSshServer::start(ServerOptions::default()).await;
        // Learn a different key for the same address.
        let other = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("other key");
        miaominal_storage::KnownHostsStore::new()
            .expect("known hosts")
            .learn("127.0.0.1", server.port, &other.public_key().clone())
            .expect("learn stale key");

        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(
            &backend,
            "host-1",
            server.port,
            AuthMethodDto::Password,
            None,
        )
        .await;
        set_credential(&backend, "host-1", CredentialKindDto::Password, "secret").await;

        let session = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("open session");
        let request =
            respond_when_prompted(&session, SshRequestKind::HostKey, Duration::from_secs(10)).await;
        assert!(request.key_changed, "mismatch must be flagged");
        assert!(request.previous_fingerprint.is_some());

        // Rejecting the changed key fails the connection.
        session
            .respond_host_key(request.request_id, SshHostKeyDecision::Reject)
            .expect("reject");
        let snapshot = wait_until_terminal(&session, Duration::from_secs(10)).await;
        assert_eq!(snapshot.phase, SshSessionPhase::Failed);
    });
}

#[test]
fn saved_trust_connects_without_a_prompt() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = TestSshServer::start(ServerOptions::default()).await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(
            &backend,
            "host-1",
            server.port,
            AuthMethodDto::Password,
            None,
        )
        .await;
        set_credential(&backend, "host-1", CredentialKindDto::Password, "secret").await;

        let session = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("open session");
        let snapshot = wait_phase(&session, SshSessionPhase::Ready, Duration::from_secs(10)).await;
        assert!(snapshot.request.is_none());
        session.disconnect().await.expect("disconnect");
    });
}

#[test]
fn keyboard_interactive_answers_multiple_rounds_and_rejects_stale_ids() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = TestSshServer::start(ServerOptions::default()).await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(
            &backend,
            "host-1",
            server.port,
            AuthMethodDto::KeyboardInteractive,
            None,
        )
        .await;

        let session = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("open session");
        let first = respond_when_prompted(
            &session,
            SshRequestKind::KeyboardInteractive,
            Duration::from_secs(10),
        )
        .await;
        assert_eq!(first.prompts.len(), 1);
        assert_eq!(first.prompts[0].prompt, "Password: ");
        assert!(!first.prompts[0].echo);

        // A stale id must be rejected without disturbing the pending request.
        let stale = session.respond_keyboard_interactive(first.request_id + 100, vec!["x".into()]);
        assert!(matches!(stale, Err(BackendError::RequestExpired { .. })));
        assert!(session.snapshot().request.is_some());

        session
            .respond_keyboard_interactive(first.request_id, vec!["secret".into()])
            .expect("first answer");

        let second = respond_when_prompted(
            &session,
            SshRequestKind::KeyboardInteractive,
            Duration::from_secs(10),
        )
        .await;
        assert_ne!(second.request_id, first.request_id);
        assert_eq!(second.prompts[0].prompt, "OTP: ");
        assert!(second.prompts[0].echo);
        session
            .respond_keyboard_interactive(second.request_id, vec!["123456".into()])
            .expect("second answer");

        wait_phase(&session, SshSessionPhase::Ready, Duration::from_secs(10)).await;
        session.disconnect().await.expect("disconnect");
    });
}

fn write_openssh_key(directory: &std::path::Path, name: &str, key: &PrivateKey) -> PathBuf {
    let path = directory.join(name);
    let pem = key
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .expect("serialize key");
    std::fs::write(&path, pem.as_bytes()).expect("write key");
    path
}

#[test]
fn encrypted_private_key_authentication_succeeds() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let directory = tempfile::tempdir().expect("temp dir");
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("key");
        let public_key = key.public_key().clone();
        let encrypted = key
            .encrypt(&mut rand::rng(), "key-passphrase")
            .expect("encrypt key");
        let path = write_openssh_key(directory.path(), "id_ed25519_enc", &encrypted);

        let server = TestSshServer::start(ServerOptions {
            password: "secret".to_string(),
            authorized_key: Some(public_key),
            ..ServerOptions::default()
        })
        .await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(
            &backend,
            "host-1",
            server.port,
            AuthMethodDto::KeyFile,
            Some(path),
        )
        .await;
        set_credential(
            &backend,
            "host-1",
            CredentialKindDto::Passphrase,
            "key-passphrase",
        )
        .await;

        let session = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("open session");
        wait_phase(&session, SshSessionPhase::Ready, Duration::from_secs(10)).await;
        session.disconnect().await.expect("disconnect");
    });
}

#[test]
fn certificate_authentication_succeeds() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let directory = tempfile::tempdir().expect("temp dir");
        let ca = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("ca");
        let user = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).expect("user");
        let mut builder = russh::keys::ssh_key::certificate::Builder::new_with_random_nonce(
            &mut rand::rng(),
            user.public_key().clone(),
            0,
            u64::MAX,
        )
        .expect("builder");
        builder.serial(1).expect("serial");
        builder.key_id("miaominal-test").expect("key id");
        builder.valid_principal("tester").expect("principal");
        let certificate = builder.sign(&ca).expect("sign certificate");
        std::fs::write(
            directory.path().join("id_ed25519-cert.pub"),
            certificate.to_openssh().expect("cert pem").as_bytes(),
        )
        .expect("write cert");
        let key_path = write_openssh_key(directory.path(), "id_ed25519", &user);
        let certificate_path = directory.path().join("id_ed25519-cert.pub");

        let server = TestSshServer::start(ServerOptions {
            password: "secret".to_string(),
            accept_certificate_user: Some("tester".to_string()),
            ..ServerOptions::default()
        })
        .await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        backend
            .save_host(HostUpdate {
                id: "host-1".to_string(),
                name: Some("Cert host".to_string()),
                group: None,
                tags: None,
                kind: None,
                host: Some("127.0.0.1".to_string()),
                port: Some(server.port),
                username: Some("tester".to_string()),
                auth_method: Some(AuthMethodDto::KeyFile),
                private_key_path: Some(key_path.to_string_lossy().into_owned()),
                managed_key_id: None,
                certificate_path: Some(certificate_path.to_string_lossy().into_owned()),
                startup_command: None,
                environment_variables: None,
                shell_type: None,
                is_favorite: None,
                last_connected_at_ms: None,
            })
            .await
            .expect("save host");

        let session = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("open session");
        wait_phase(&session, SshSessionPhase::Ready, Duration::from_secs(10)).await;
        session.disconnect().await.expect("disconnect");
    });
}

#[test]
fn rejected_password_reports_a_failed_session() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = TestSshServer::start(ServerOptions::default()).await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(
            &backend,
            "host-1",
            server.port,
            AuthMethodDto::Password,
            None,
        )
        .await;
        set_credential(&backend, "host-1", CredentialKindDto::Password, "wrong").await;

        let session = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("open session");
        let snapshot = wait_until_terminal(&session, Duration::from_secs(10)).await;
        assert_eq!(snapshot.phase, SshSessionPhase::Failed);
        let failure = snapshot.failure.expect("failure text");
        assert!(
            failure.contains("authentication"),
            "unexpected failure: {failure}"
        );
    });
}

#[test]
fn cancelling_while_waiting_for_trust_closes_without_leaving_a_task() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = TestSshServer::start(ServerOptions::default()).await;
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(
            &backend,
            "host-1",
            server.port,
            AuthMethodDto::Password,
            None,
        )
        .await;
        set_credential(&backend, "host-1", CredentialKindDto::Password, "secret").await;

        let session = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("open session");
        respond_when_prompted(&session, SshRequestKind::HostKey, Duration::from_secs(10)).await;

        session.cancel();
        // Idempotent: a second cancel and close must not hang or panic.
        session.cancel();
        session.disconnect().await.expect("disconnect");
        session.disconnect().await.expect("second disconnect");
        assert_eq!(session.snapshot().phase, SshSessionPhase::Closed);

        // The registry keeps the closed session until it is explicitly removed.
        let snapshots = backend.ssh_session_snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].phase, SshSessionPhase::Closed);
        assert!(
            backend
                .close_ssh_session(snapshots[0].connection_id)
                .await
                .expect("close")
        );
        assert!(
            !backend
                .close_ssh_session(snapshots[0].connection_id)
                .await
                .expect("second close")
        );
        assert!(backend.ssh_session_snapshots().is_empty());
    });
}

#[test]
fn vault_locked_open_returns_vault_locked_without_connecting() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = TestSshServer::start(ServerOptions::default()).await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(
            &backend,
            "host-1",
            server.port,
            AuthMethodDto::Password,
            None,
        )
        .await;
        set_credential(&backend, "host-1", CredentialKindDto::Password, "secret").await;
        backend
            .enable_vault("vault-passphrase".to_string())
            .await
            .expect("enable vault");
        backend.lock_vault().await.expect("lock vault");

        let error = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .err()
            .expect("locked vault must not connect");
        assert!(matches!(error, BackendError::VaultLocked));
        assert!(backend.ssh_session_snapshots().is_empty());
    });
}

#[test]
fn multiple_sessions_for_one_host_stay_independent() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = TestSshServer::start(ServerOptions::default()).await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(
            &backend,
            "host-1",
            server.port,
            AuthMethodDto::Password,
            None,
        )
        .await;
        set_credential(&backend, "host-1", CredentialKindDto::Password, "secret").await;

        let first = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("first session");
        let second = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("second session");
        wait_phase(&first, SshSessionPhase::Ready, Duration::from_secs(10)).await;
        wait_phase(&second, SshSessionPhase::Ready, Duration::from_secs(10)).await;

        first
            .write_bytes(b"only-first\r".to_vec())
            .expect("write first");
        wait_output(&first, "only-first", Duration::from_secs(10)).await;
        let second_text = frame_text(&second);
        assert!(
            !second_text.contains("only-first"),
            "sessions must not share a terminal"
        );

        first.disconnect().await.expect("disconnect first");
        // The second session keeps working.
        second
            .write_bytes(b"still-alive\r".to_vec())
            .expect("write second");
        wait_output(&second, "still-alive", Duration::from_secs(10)).await;
        second.disconnect().await.expect("disconnect second");
    });
}

#[test]
fn fast_output_does_not_stall_the_session() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = TestSshServer::start(ServerOptions::default()).await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(
            &backend,
            "host-1",
            server.port,
            AuthMethodDto::Password,
            None,
        )
        .await;
        set_credential(&backend, "host-1", CredentialKindDto::Password, "secret").await;

        let session = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("open session");
        wait_phase(&session, SshSessionPhase::Ready, Duration::from_secs(10)).await;

        session.write_bytes(b"flood\r".to_vec()).expect("flood");
        wait_output(&session, "line-299", Duration::from_secs(20)).await;
        let frame = session.render_frame(0);
        assert!(frame.history_size >= 200, "history: {}", frame.history_size);
        session.disconnect().await.expect("disconnect");
    });
}

#[test]
fn terminal_query_response_reaches_the_server_without_further_output() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = TestSshServer::start(ServerOptions::default()).await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(
            &backend,
            "host-1",
            server.port,
            AuthMethodDto::Password,
            None,
        )
        .await;
        set_credential(&backend, "host-1", CredentialKindDto::Password, "secret").await;

        let session = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("open session");
        wait_phase(&session, SshSessionPhase::Ready, Duration::from_secs(10)).await;

        // The server sends a DECRQM query and then goes silent until the
        // client's terminal answers; a lost PtyWrite would deadlock both sides.
        session.write_bytes(b"query\r".to_vec()).expect("query");
        let text = wait_output(&session, "ANSWERED", Duration::from_secs(10)).await;
        assert!(text.contains("ANSWERED"));
        session.disconnect().await.expect("disconnect");
    });
}

/// Accepts TCP connections and never speaks SSH, so the client is stuck in the
/// handshake with no channel to poll.
async fn spawn_silent_server() -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let task = crate::runtime::runtime().spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            crate::runtime::runtime().spawn(async move {
                let _stream = stream;
                std::future::pending::<()>().await;
            });
        }
    });
    (port, task)
}

#[test]
fn disconnect_cancels_a_connection_stuck_in_the_handshake() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let (port, server_task) = spawn_silent_server().await;
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(&backend, "host-1", port, AuthMethodDto::Password, None).await;
        set_credential(&backend, "host-1", CredentialKindDto::Password, "secret").await;

        let session = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("open session");
        // Let the worker reach the stalled handshake.
        tokio::time::sleep(Duration::from_millis(300)).await;

        let started = Instant::now();
        session.disconnect().await.expect("disconnect");
        let elapsed = started.elapsed();

        // Without a real cancel the consumer wait alone would take the 8s
        // grace; a completed join also proves the connection thread ended.
        assert!(
            elapsed < Duration::from_secs(5),
            "disconnect took {elapsed:?}, the connection worker was not cancelled"
        );
        assert_eq!(session.snapshot().phase, SshSessionPhase::Closed);

        server_task.abort();
    });
}

#[test]
fn server_rejecting_pty_fails_the_session() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = TestSshServer::start(ServerOptions {
            reject_pty: true,
            ..ServerOptions::default()
        })
        .await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(
            &backend,
            "host-1",
            server.port,
            AuthMethodDto::Password,
            None,
        )
        .await;
        set_credential(&backend, "host-1", CredentialKindDto::Password, "secret").await;

        let session = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("open session");
        let snapshot = wait_until_terminal(&session, Duration::from_secs(10)).await;
        assert_eq!(snapshot.phase, SshSessionPhase::Failed);
        let failure = snapshot.failure.expect("failure text");
        assert!(failure.contains("PTY"), "unexpected failure: {failure}");
    });
}

#[test]
fn server_rejecting_shell_fails_the_session() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = TestSshServer::start(ServerOptions {
            reject_shell: true,
            ..ServerOptions::default()
        })
        .await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(
            &backend,
            "host-1",
            server.port,
            AuthMethodDto::Password,
            None,
        )
        .await;
        set_credential(&backend, "host-1", CredentialKindDto::Password, "secret").await;

        let session = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("open session");
        let snapshot = wait_until_terminal(&session, Duration::from_secs(10)).await;
        assert_eq!(snapshot.phase, SshSessionPhase::Failed);
        let failure = snapshot.failure.expect("failure text");
        assert!(
            failure.contains("interactive shell"),
            "unexpected failure: {failure}"
        );
    });
}

/// The Android UI does not expose agent forwarding yet, but desktop-synced
/// profiles can set it; the shared connector must keep the reply order intact.
fn enable_agent_forwarding(host_id: &str) {
    use miaominal_storage::config_store::store::SessionStore;
    let store = SessionStore::new().expect("session store");
    let content = store
        .read_sessions_content()
        .expect("read sessions")
        .unwrap_or_default();
    let mut profiles = store.parse_sessions(&content).expect("parse sessions");
    for profile in profiles.iter_mut() {
        if profile.id == host_id {
            profile.agent_forwarding = true;
        }
    }
    store.save(&profiles).expect("save sessions");
}

#[test]
fn agent_forwarding_reply_does_not_shift_the_shell_rejection() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = TestSshServer::start(ServerOptions {
            reject_shell: true,
            ..ServerOptions::default()
        })
        .await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(
            &backend,
            "host-1",
            server.port,
            AuthMethodDto::Password,
            None,
        )
        .await;
        set_credential(&backend, "host-1", CredentialKindDto::Password, "secret").await;
        enable_agent_forwarding("host-1");

        let session = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("open session");
        let snapshot = wait_until_terminal(&session, Duration::from_secs(10)).await;
        assert_eq!(snapshot.phase, SshSessionPhase::Failed);
        let failure = snapshot.failure.expect("failure text");
        assert!(
            failure.contains("interactive shell"),
            "the shell rejection must not be masked by the agent-forwarding reply: {failure}"
        );
    });
}

#[test]
fn refused_agent_forwarding_keeps_the_session_usable() {
    let _guard = shared_test_lock();
    clear_test_root();
    crate::runtime::runtime().block_on(async {
        let server = TestSshServer::start(ServerOptions {
            reject_agent_forward: true,
            ..ServerOptions::default()
        })
        .await;
        trust_host(&server);
        let backend = backend_for_test();
        backend.initialize().await.expect("initialize");
        save_host(
            &backend,
            "host-1",
            server.port,
            AuthMethodDto::Password,
            None,
        )
        .await;
        set_credential(&backend, "host-1", CredentialKindDto::Password, "secret").await;
        enable_agent_forwarding("host-1");

        let session = backend
            .open_ssh_session("host-1".to_string(), 80, 24)
            .await
            .expect("open session");
        wait_phase(&session, SshSessionPhase::Ready, Duration::from_secs(10)).await;

        session
            .write_bytes(b"still-works\r".to_vec())
            .expect("write");
        wait_output(&session, "still-works", Duration::from_secs(10)).await;
        session.disconnect().await.expect("disconnect");
    });
}
