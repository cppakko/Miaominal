//! Telnet terminal sessions.
//!
//! A Telnet profile opens a TCP connection (directly or through the profile's
//! entry proxy) and speaks the same session protocol as an SSH session, so the
//! desktop workspace reuses terminal rendering, split panes, search, clipboard
//! and reconnect paths. Protocol handling lives in `miaominal_core::telnet`.

use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use miaominal_core::profile::SessionProfile;
use miaominal_core::proxy::ProxyProfile;
use miaominal_core::telnet::{DEFAULT_TELNET_TERMINAL_TYPE, TelnetAutoLogin, TelnetCodec};
use miaominal_core::terminal::MIN_TERMINAL_COLUMNS;
use miaominal_secrets::SecretStore;
use miaominal_ssh::transport::{BoxedTransport, connect_via_proxy, resolve_entry_proxy};
use miaominal_ssh::{
    SessionChannels, SessionCommand, SessionConnection, SessionEvent, SessionHop, SessionPhase,
    SessionPhaseChange, session_channels,
};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::mpsc::{Sender, UnboundedReceiver};

const DIRECT_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_BUFFER_SIZE: usize = 16 * 1024;

/// Starts a Telnet session and returns the connection handed to the workspace,
/// matching [`miaominal_ssh::start_session`].
pub(crate) fn start_telnet_session(
    runtime: &TokioHandle,
    profile: SessionProfile,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    columns: usize,
    lines: usize,
) -> SessionConnection {
    let SessionChannels {
        mut connection,
        commands,
        events,
    } = session_channels();
    let runtime = runtime.clone();
    let thread = std::thread::Builder::new()
        .name(format!("telnet-session-{}", profile.id))
        .spawn(move || {
            runtime.block_on(run_telnet_session(
                profile,
                all_proxies,
                secrets,
                (columns, lines),
                commands,
                events,
            ));
        })
        .expect("failed to spawn Telnet session thread");
    connection.thread = Some(thread);
    connection
}

fn terminal_size(columns: usize, lines: usize) -> (usize, usize) {
    (columns.max(MIN_TERMINAL_COLUMNS), lines.max(1))
}

async fn run_telnet_session(
    profile: SessionProfile,
    all_proxies: Vec<ProxyProfile>,
    secrets: SecretStore,
    (columns, lines): (usize, usize),
    mut commands: UnboundedReceiver<SessionCommand>,
    events: Sender<SessionEvent>,
) {
    let mut size = terminal_size(columns, lines);
    let _ = events
        .send(SessionEvent::Phase(SessionPhaseChange {
            phase: SessionPhase::Connecting,
            hop: Some(SessionHop::new(profile.connection_label(), 1, 1)),
        }))
        .await;

    let connect = connect_transport(&profile, &all_proxies, &secrets);
    tokio::pin!(connect);
    let transport = loop {
        tokio::select! {
            result = &mut connect => match result {
                Ok(transport) => break transport,
                Err(error) => {
                    let _ = events.send(SessionEvent::Error(format!("{error:#}"))).await;
                    let _ = events.send(SessionEvent::Closed).await;
                    return;
                }
            },
            // Input typed before the connection exists is dropped, like SSH.
            command = commands.recv() => match command {
                None | Some(SessionCommand::Close) => {
                    let _ = events.send(SessionEvent::Closed).await;
                    return;
                }
                Some(SessionCommand::Resize { columns, lines }) => {
                    size = terminal_size(columns, lines);
                }
                Some(_) => {}
            },
        }
    };

    if let Err(error) = drive_telnet_session(&profile, transport, size, commands, &events).await {
        let _ = events.send(SessionEvent::Error(format!("{error:#}"))).await;
    }
    let _ = events.send(SessionEvent::Closed).await;
}

async fn connect_transport(
    profile: &SessionProfile,
    all_proxies: &[ProxyProfile],
    secrets: &SecretStore,
) -> Result<BoxedTransport> {
    let host = profile.host.trim();
    if host.is_empty() {
        return Err(anyhow!("Telnet profile {} has no host", profile.name));
    }
    let port = profile.port;
    if let Some(proxy) = resolve_entry_proxy(profile.entry_proxy_id.as_deref(), all_proxies)? {
        return connect_via_proxy(proxy, host, port, secrets).await;
    }
    let unbracketed = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    let stream = tokio::time::timeout(
        DIRECT_CONNECT_TIMEOUT,
        tokio::net::TcpStream::connect((unbracketed, port)),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "TCP connection to {host}:{port} timed out after {} seconds",
            DIRECT_CONNECT_TIMEOUT.as_secs()
        )
    })?
    .with_context(|| format!("failed to establish TCP connection to {host}:{port}"))?;
    let _ = stream.set_nodelay(true);
    Ok(Box::new(stream))
}

/// Typed once the login is over; see [`TelnetAutoLogin::ready_for_startup`].
fn startup_command_line(startup_command: &str) -> Option<Vec<u8>> {
    let command = startup_command.trim();
    (!command.is_empty()).then(|| format!("{command}\r").into_bytes())
}

async fn drive_telnet_session<T>(
    profile: &SessionProfile,
    transport: T,
    (columns, lines): (usize, usize),
    mut commands: UnboundedReceiver<SessionCommand>,
    events: &Sender<SessionEvent>,
) -> Result<()>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let (mut reader, mut writer) = tokio::io::split(transport);
    let mut codec = TelnetCodec::new(DEFAULT_TELNET_TERMINAL_TYPE, columns, lines);
    let mut login = TelnetAutoLogin::new(&profile.username, &profile.password);
    let mut startup = startup_command_line(&profile.startup_command);

    writer
        .write_all(&codec.initial_negotiation())
        .await
        .context("failed to start Telnet negotiation")?;
    writer.flush().await?;
    if events
        .send(SessionEvent::Connected(profile.connection_label()))
        .await
        .is_err()
    {
        return Ok(());
    }

    let mut buffer = vec![0u8; READ_BUFFER_SIZE];
    loop {
        tokio::select! {
            read = reader.read(&mut buffer) => {
                let read = read.context("failed to read from the Telnet server")?;
                if read == 0 {
                    return Ok(());
                }
                let received = codec.receive(&buffer[..read]);
                if !received.reply.is_empty() {
                    writer.write_all(&received.reply).await?;
                }
                if !received.data.is_empty() {
                    let answer = login.observe(&received.data);
                    if events.send(SessionEvent::Output(received.data)).await.is_err() {
                        return Ok(());
                    }
                    if let Some(answer) = answer {
                        writer.write_all(&codec.encode_input(answer.as_bytes())).await?;
                    }
                    if login.ready_for_startup()
                        && let Some(command) = startup.take()
                    {
                        writer.write_all(&codec.encode_input(&command)).await?;
                    }
                }
                writer.flush().await?;
            }
            command = commands.recv() => match command {
                None | Some(SessionCommand::Close) => {
                    let _ = writer.shutdown().await;
                    return Ok(());
                }
                Some(SessionCommand::Send(bytes)) => {
                    login.note_user_input(&bytes);
                    writer
                        .write_all(&codec.encode_input(&bytes))
                        .await
                        .context("failed to write to the Telnet server")?;
                    writer.flush().await?;
                }
                Some(SessionCommand::Resize { columns, lines }) => {
                    let (columns, lines) = terminal_size(columns, lines);
                    let update = codec.resize(columns, lines);
                    if !update.is_empty() {
                        writer.write_all(&update).await?;
                        writer.flush().await?;
                    }
                }
                // Monitoring, host keys, keyboard-interactive and forwarding
                // are SSH features with no Telnet equivalent.
                Some(_) => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::profile::ProfileKind;
    use miaominal_core::telnet::{DO, IAC, OPTION_ECHO, OPTION_NAWS, SB, SE, WILL};
    use tokio::io::DuplexStream;

    fn telnet_profile() -> SessionProfile {
        let mut profile = SessionProfile::blank("telnet-test", 1);
        profile.kind = ProfileKind::Telnet;
        profile.host = "127.0.0.1".into();
        profile.port = 23;
        profile.auth_method = None;
        profile
    }

    /// Server side of the in-memory link; keeps unread bytes between waits so
    /// several client writes landing in one read are never lost.
    struct Server {
        stream: DuplexStream,
        pending: Vec<u8>,
    }

    impl Server {
        async fn expect(&mut self, needle: &[u8]) {
            let mut buffer = [0u8; 512];
            loop {
                if let Some(position) = self
                    .pending
                    .windows(needle.len())
                    .position(|window| window == needle)
                {
                    self.pending.drain(..position + needle.len());
                    return;
                }
                let read =
                    tokio::time::timeout(Duration::from_secs(5), self.stream.read(&mut buffer))
                        .await
                        .unwrap_or_else(|_| {
                            panic!("timed out waiting for {needle:?}, have {:?}", self.pending)
                        })
                        .expect("server read");
                assert!(read > 0, "client closed before sending {needle:?}");
                self.pending.extend_from_slice(&buffer[..read]);
            }
        }

        async fn send(&mut self, bytes: &[u8]) {
            self.stream.write_all(bytes).await.expect("server write");
        }
    }

    struct Harness {
        server: Server,
        commands: tokio::sync::mpsc::UnboundedSender<SessionCommand>,
        events: tokio::sync::mpsc::Receiver<SessionEvent>,
        session: tokio::task::JoinHandle<Result<()>>,
    }

    fn start(profile: SessionProfile) -> Harness {
        let (client, server) = tokio::io::duplex(4096);
        let (commands, command_receiver) = tokio::sync::mpsc::unbounded_channel();
        let (event_sender, events) = tokio::sync::mpsc::channel(64);
        let session = tokio::spawn(async move {
            drive_telnet_session(&profile, client, (80, 24), command_receiver, &event_sender).await
        });
        Harness {
            server: Server {
                stream: server,
                pending: Vec::new(),
            },
            commands,
            events,
            session,
        }
    }

    async fn collect_output(
        events: &mut tokio::sync::mpsc::Receiver<SessionEvent>,
        until: &[u8],
    ) -> Vec<u8> {
        let mut output = Vec::new();
        while !output.ends_with(until) {
            match events
                .recv()
                .await
                .expect("session should keep emitting events")
            {
                SessionEvent::Output(bytes) => output.extend(bytes),
                SessionEvent::Error(error) => panic!("telnet session failed: {error}"),
                _ => {}
            }
        }
        output
    }

    #[tokio::test]
    async fn session_negotiates_logs_in_and_runs_the_startup_command() {
        let mut profile = telnet_profile();
        profile.username = "admin".into();
        profile.password = "hunter2".into();
        profile.startup_command = "show version".into();
        let mut harness = start(profile);
        let server = &mut harness.server;

        server.expect(&[IAC, WILL, OPTION_NAWS]).await;
        server
            .send(&[IAC, DO, OPTION_NAWS, IAC, WILL, OPTION_ECHO])
            .await;
        server
            .expect(&[IAC, SB, OPTION_NAWS, 0, 80, 0, 24, IAC, SE])
            .await;

        server.send(b"\r\nswitch login: ").await;
        server.expect(b"admin\r\0").await;
        server.send(b"Password: ").await;
        server.expect(b"hunter2\r\0").await;
        server.expect(b"show version\r\0").await;

        let output = collect_output(&mut harness.events, b"Password: ").await;
        assert_eq!(output, b"\r\nswitch login: Password: ");

        harness
            .commands
            .send(SessionCommand::Resize {
                columns: 120,
                lines: 40,
            })
            .unwrap();
        harness
            .server
            .expect(&[IAC, SB, OPTION_NAWS, 0, 120, 0, 40, IAC, SE])
            .await;

        harness
            .commands
            .send(SessionCommand::Send(vec![b'q', IAC, b'\r']))
            .unwrap();
        harness.server.expect(&[b'q', IAC, IAC, b'\r', 0]).await;

        harness.commands.send(SessionCommand::Close).unwrap();
        harness
            .session
            .await
            .unwrap()
            .expect("an explicit close ends the session cleanly");
    }

    #[tokio::test]
    async fn startup_command_waits_for_a_manually_typed_password() {
        let mut profile = telnet_profile();
        profile.username = "admin".into();
        profile.startup_command = "enable".into();
        let mut harness = start(profile);

        harness.server.send(b"login: ").await;
        harness.server.expect(b"admin\r\0").await;
        harness.server.send(b"Password: ").await;
        collect_output(&mut harness.events, b"Password: ").await;

        harness
            .commands
            .send(SessionCommand::Send(b"secret\r".to_vec()))
            .unwrap();
        harness.server.expect(b"secret\r\0").await;
        assert!(
            !harness.server.pending.windows(6).any(|w| w == b"enable"),
            "startup command must not be typed into the password prompt"
        );

        harness.server.send(b"\r\nrouter> ").await;
        harness.server.expect(b"enable\r\0").await;

        harness.commands.send(SessionCommand::Close).unwrap();
        harness.session.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn remote_close_ends_the_session() {
        let mut harness = start(telnet_profile());
        harness.server.expect(&[IAC, WILL, OPTION_NAWS]).await;
        drop(harness.server);
        harness
            .session
            .await
            .unwrap()
            .expect("remote EOF is an orderly end");
        let mut connected = false;
        while let Some(event) = harness.events.recv().await {
            connected |= matches!(event, SessionEvent::Connected(_));
        }
        assert!(connected);
    }
}
