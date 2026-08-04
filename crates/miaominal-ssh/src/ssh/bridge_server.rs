use super::bridge_ipc::SshBridgeStream;
use super::profile_connector::ConnectedSshRoute;
use anyhow::{Context, Result, anyhow};
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{self, Auth};
use russh::{
    Channel, ChannelId, ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Disconnect, Pty, Sig, client,
};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;

const BRIDGE_SERVER_WINDOW_SIZE: u32 = 1024 * 1024;
const BRIDGE_SERVER_MAXIMUM_PACKET_SIZE: u32 = 32 * 1024;
const BRIDGE_SERVER_CHANNEL_BUFFER_SIZE: usize = 64;
const BRIDGE_SERVER_EVENT_BUFFER_SIZE: usize = 64;
const BRIDGE_SERVER_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const BRIDGE_SERVER_MAX_CONNECTION_LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Clone)]
pub struct SshBridgeServerIdentity {
    private_key: PrivateKey,
    host_key_alias: String,
    known_hosts_path: PathBuf,
}

impl SshBridgeServerIdentity {
    pub fn generate(instance_id: &str, known_hosts_path: impl Into<PathBuf>) -> Result<Self> {
        let private_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .context("failed to generate in-memory SSH Bridge host key")?;
        let identity = Self {
            private_key,
            host_key_alias: format!("miaominal-bridge-{instance_id}"),
            known_hosts_path: known_hosts_path.into(),
        };
        identity.write_known_hosts_sidecar()?;
        Ok(identity)
    }

    pub fn host_key_alias(&self) -> &str {
        &self.host_key_alias
    }

    pub fn known_hosts_path(&self) -> &Path {
        &self.known_hosts_path
    }

    pub fn write_known_hosts_sidecar(&self) -> Result<()> {
        let public_key = self
            .private_key
            .public_key()
            .to_openssh()
            .context("failed to encode SSH Bridge public host key")?;
        let contents = format!("{} {}\n", self.host_key_alias, public_key.trim());
        miaominal_paths::atomic_write(&self.known_hosts_path, contents.as_bytes()).with_context(
            || {
                format!(
                    "failed to write SSH Bridge known-hosts sidecar {}",
                    self.known_hosts_path.display()
                )
            },
        )
    }
}

pub async fn run_ssh_bridge_server(
    stream: SshBridgeStream,
    route: ConnectedSshRoute,
    identity: Arc<SshBridgeServerIdentity>,
    max_channels: usize,
) -> Result<()> {
    run_ssh_bridge_server_with_shutdown(
        stream,
        route,
        identity,
        max_channels,
        std::future::pending(),
    )
    .await
}

pub async fn run_ssh_bridge_server_with_shutdown<F>(
    stream: SshBridgeStream,
    route: ConnectedSshRoute,
    identity: Arc<SshBridgeServerIdentity>,
    max_channels: usize,
    shutdown: F,
) -> Result<()>
where
    F: Future<Output = ()> + Send,
{
    let route = Arc::new(route);
    let relay_tasks = Arc::new(Mutex::new(JoinSet::new()));
    let handler = SshBridgeServerHandler {
        route: route.clone(),
        channels: Arc::new(Mutex::new(HashMap::new())),
        channel_slots: Arc::new(Semaphore::new(max_channels.max(1))),
        relay_tasks: relay_tasks.clone(),
    };
    let config = Arc::new(server_config(&identity));
    let running = server::run_stream(config, stream, handler)
        .await
        .context("failed to start local SSH Bridge server")?;
    let server_handle = running.handle();
    tokio::pin!(running);
    tokio::pin!(shutdown);
    let lifetime = tokio::time::sleep(BRIDGE_SERVER_MAX_CONNECTION_LIFETIME);
    tokio::pin!(lifetime);
    let result = tokio::select! {
        result = &mut running => result.context("local SSH Bridge server failed"),
        () = &mut shutdown => {
            let _ = server_handle
                .disconnect(
                    Disconnect::ByApplication,
                    "SSH Bridge is stopping".into(),
                    "English".into(),
                )
                .await;
            match tokio::time::timeout(Duration::from_secs(5), &mut running).await {
                Ok(result) => result.context("local SSH Bridge server failed during shutdown"),
                Err(_) => Err(anyhow!("local SSH Bridge server shutdown timed out")),
            }
        }
        () = &mut lifetime => {
            let _ = server_handle
                .disconnect(
                    Disconnect::ByApplication,
                    "SSH Bridge connection lifetime exceeded".into(),
                    "English".into(),
                )
                .await;
            Err(anyhow!("SSH Bridge connection lifetime exceeded"))
        }
    };

    let mut tasks = relay_tasks.lock().await;
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    drop(tasks);
    route.disconnect().await;
    result
}

fn server_config(identity: &SshBridgeServerIdentity) -> server::Config {
    server::Config {
        server_id: russh::SshId::Standard("SSH-2.0-Miaominal-Bridge".into()),
        methods: russh::MethodSet::from(&[russh::MethodKind::None][..]),
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![identity.private_key.clone()],
        window_size: BRIDGE_SERVER_WINDOW_SIZE,
        maximum_packet_size: BRIDGE_SERVER_MAXIMUM_PACKET_SIZE,
        channel_buffer_size: BRIDGE_SERVER_CHANNEL_BUFFER_SIZE,
        event_buffer_size: BRIDGE_SERVER_EVENT_BUFFER_SIZE,
        inactivity_timeout: Some(BRIDGE_SERVER_INACTIVITY_TIMEOUT),
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    }
}

struct UpstreamChannel {
    writer: Arc<ChannelWriteHalf<client::Msg>>,
    _permit: OwnedSemaphorePermit,
}

struct SshBridgeServerHandler {
    route: Arc<ConnectedSshRoute>,
    channels: Arc<Mutex<HashMap<ChannelId, UpstreamChannel>>>,
    channel_slots: Arc<Semaphore>,
    relay_tasks: Arc<Mutex<JoinSet<()>>>,
}

impl SshBridgeServerHandler {
    async fn register_channel(
        &self,
        local_channel: Channel<server::Msg>,
        upstream_channel: Channel<client::Msg>,
        permit: OwnedSemaphorePermit,
        local_handle: server::Handle,
    ) {
        let channel_id = local_channel.id();
        let (upstream_reader, upstream_writer) = upstream_channel.split();
        self.channels.lock().await.insert(
            channel_id,
            UpstreamChannel {
                writer: Arc::new(upstream_writer),
                _permit: permit,
            },
        );
        let channels = self.channels.clone();
        let mut relay_tasks = self.relay_tasks.lock().await;
        reap_completed_relay_tasks(&mut relay_tasks);
        relay_tasks.spawn(async move {
            relay_upstream_channel(upstream_reader, local_handle, channel_id).await;
            channels.lock().await.remove(&channel_id);
        });
    }

    async fn fail_channel(&self, channel: ChannelId, session: &mut server::Session) {
        let _ = session.channel_failure(channel);
        let _ = session.close(channel);
        self.channels.lock().await.remove(&channel);
    }

    async fn send_request<F>(&self, channel: ChannelId, session: &mut server::Session, request: F)
    where
        F: std::future::Future<Output = std::result::Result<(), russh::Error>>,
    {
        if request.await.is_err() {
            self.fail_channel(channel, session).await;
        }
    }
}

fn reap_completed_relay_tasks(tasks: &mut JoinSet<()>) {
    while let Some(result) = tasks.try_join_next() {
        if let Err(error) = result
            && !error.is_cancelled()
        {
            log::debug!("SSH Bridge channel relay task failed: {error}");
        }
    }
}

impl server::Handler for SshBridgeServerHandler {
    type Error = anyhow::Error;

    async fn auth_none(&mut self, user: &str) -> Result<Auth, Self::Error> {
        Ok(if user == "miaominal" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<server::Msg>,
        session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        let Ok(permit) = self.channel_slots.clone().try_acquire_owned() else {
            return Ok(false);
        };
        let upstream = match self.route.open_session().await {
            Ok(channel) => channel,
            Err(error) => {
                log::warn!("SSH Bridge could not open upstream session channel: {error:#}");
                return Ok(false);
            }
        };
        self.register_channel(channel, upstream, permit, session.handle())
            .await;
        Ok(true)
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<server::Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        originator_address: &str,
        originator_port: u32,
        session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        let Ok(permit) = self.channel_slots.clone().try_acquire_owned() else {
            return Ok(false);
        };
        let upstream = match self
            .route
            .open_direct_tcpip(
                host_to_connect.to_string(),
                port_to_connect,
                originator_address.to_string(),
                originator_port,
            )
            .await
        {
            Ok(channel) => channel,
            Err(error) => {
                log::warn!("SSH Bridge upstream direct-tcpip request failed: {error:#}");
                return Ok(false);
            }
        };
        self.register_channel(channel, upstream, permit, session.handle())
            .await;
        Ok(true)
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let writer = self
            .channels
            .lock()
            .await
            .get(&channel)
            .map(|upstream| upstream.writer.clone());
        if let Some(writer) = writer {
            let _ = writer.eof().await;
        }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if let Some(upstream) = self.channels.lock().await.remove(&channel) {
            let _ = upstream.writer.close().await;
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let writer = self
            .channels
            .lock()
            .await
            .get(&channel)
            .map(|upstream| upstream.writer.clone());
        let Some(writer) = writer else {
            let _ = session.close(channel);
            return Ok(());
        };
        if writer.data_bytes(data.to_vec()).await.is_err() {
            self.fail_channel(channel, session).await;
        }
        Ok(())
    }

    async fn extended_data(
        &mut self,
        channel: ChannelId,
        code: u32,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let writer = self
            .channels
            .lock()
            .await
            .get(&channel)
            .map(|upstream| upstream.writer.clone());
        let Some(writer) = writer else {
            let _ = session.close(channel);
            return Ok(());
        };
        if writer
            .extended_data_bytes(code, data.to_vec())
            .await
            .is_err()
        {
            self.fail_channel(channel, session).await;
        }
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        col_width: u32,
        row_height: u32,
        pix_width: u32,
        pix_height: u32,
        modes: &[(Pty, u32)],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let writer = self
            .channels
            .lock()
            .await
            .get(&channel)
            .map(|upstream| upstream.writer.clone());
        let Some(writer) = writer else {
            self.fail_channel(channel, session).await;
            return Ok(());
        };
        let request = writer.request_pty(
            true, term, col_width, row_height, pix_width, pix_height, modes,
        );
        self.send_request(channel, session, request).await;
        Ok(())
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        variable_name: &str,
        variable_value: &str,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let writer = self
            .channels
            .lock()
            .await
            .get(&channel)
            .map(|upstream| upstream.writer.clone());
        let Some(writer) = writer else {
            self.fail_channel(channel, session).await;
            return Ok(());
        };
        self.send_request(
            channel,
            session,
            writer.set_env(true, variable_name.to_string(), variable_value.to_string()),
        )
        .await;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let writer = self
            .channels
            .lock()
            .await
            .get(&channel)
            .map(|upstream| upstream.writer.clone());
        let Some(writer) = writer else {
            self.fail_channel(channel, session).await;
            return Ok(());
        };
        self.send_request(channel, session, writer.request_shell(true))
            .await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let writer = self
            .channels
            .lock()
            .await
            .get(&channel)
            .map(|upstream| upstream.writer.clone());
        let Some(writer) = writer else {
            self.fail_channel(channel, session).await;
            return Ok(());
        };
        self.send_request(channel, session, writer.exec(true, data.to_vec()))
            .await;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let writer = self
            .channels
            .lock()
            .await
            .get(&channel)
            .map(|upstream| upstream.writer.clone());
        let Some(writer) = writer else {
            self.fail_channel(channel, session).await;
            return Ok(());
        };
        self.send_request(
            channel,
            session,
            writer.request_subsystem(true, name.to_string()),
        )
        .await;
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        col_width: u32,
        row_height: u32,
        pix_width: u32,
        pix_height: u32,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let writer = self
            .channels
            .lock()
            .await
            .get(&channel)
            .map(|upstream| upstream.writer.clone());
        if let Some(writer) = writer {
            let _ = writer
                .window_change(col_width, row_height, pix_width, pix_height)
                .await;
        }
        Ok(())
    }

    async fn signal(
        &mut self,
        channel: ChannelId,
        signal: Sig,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let writer = self
            .channels
            .lock()
            .await
            .get(&channel)
            .map(|upstream| upstream.writer.clone());
        if let Some(writer) = writer {
            let _ = writer.signal(signal).await;
        }
        Ok(())
    }

    async fn x11_request(
        &mut self,
        channel: ChannelId,
        _single_connection: bool,
        _x11_auth_protocol: &str,
        _x11_auth_cookie: &str,
        _x11_screen_number: u32,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let _ = session.channel_failure(channel);
        Ok(())
    }

    async fn agent_request(
        &mut self,
        channel: ChannelId,
        session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        let _ = session.channel_failure(channel);
        Ok(false)
    }

    async fn tcpip_forward(
        &mut self,
        _address: &str,
        _port: &mut u32,
        _session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn cancel_tcpip_forward(
        &mut self,
        _address: &str,
        _port: u32,
        _session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn streamlocal_forward(
        &mut self,
        _socket_path: &str,
        _session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn cancel_streamlocal_forward(
        &mut self,
        _socket_path: &str,
        _session: &mut server::Session,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }
}

async fn relay_upstream_channel(
    mut upstream: ChannelReadHalf,
    local: server::Handle,
    channel: ChannelId,
) {
    while let Some(message) = upstream.wait().await {
        let keep_open = match message {
            ChannelMsg::Data { data } => local.data(channel, data).await.is_ok(),
            ChannelMsg::ExtendedData { data, ext } => {
                local.extended_data(channel, ext, data).await.is_ok()
            }
            ChannelMsg::Eof => local.eof(channel).await.is_ok(),
            ChannelMsg::Close => {
                let _ = local.close(channel).await;
                false
            }
            ChannelMsg::ExitStatus { exit_status } => local
                .exit_status_request(channel, exit_status)
                .await
                .is_ok(),
            ChannelMsg::ExitSignal {
                signal_name,
                core_dumped,
                error_message,
                lang_tag,
            } => local
                .exit_signal_request(channel, signal_name, core_dumped, error_message, lang_tag)
                .await
                .is_ok(),
            ChannelMsg::Success => local.channel_success(channel).await.is_ok(),
            ChannelMsg::Failure | ChannelMsg::OpenFailure(_) => {
                local.channel_failure(channel).await.is_ok()
            }
            ChannelMsg::XonXoff { client_can_do } => {
                local.xon_xoff_request(channel, client_can_do).await.is_ok()
            }
            ChannelMsg::WindowAdjusted { .. } | ChannelMsg::Open { .. } => true,
            _ => true,
        };
        if !keep_open {
            return;
        }
    }
    let _ = local.close(channel).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProfileConnector, connection};
    use miaominal_core::profile::{AuthMethod, SessionProfile};
    use miaominal_secrets::SecretStore;
    use miaominal_storage::KnownHostsStore;
    use std::sync::Mutex as StdMutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::task::JoinHandle;

    struct AcceptAllClientHandler;

    impl client::Handler for AcceptAllClientHandler {
        type Error = anyhow::Error;

        async fn check_server_key(
            &mut self,
            _server_public_key: &russh::keys::PublicKey,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }
    }

    struct TestUpstreamHandler {
        events: Arc<StdMutex<Vec<String>>>,
    }

    impl TestUpstreamHandler {
        fn record(&self, event: impl Into<String>) {
            if let Ok(mut events) = self.events.lock() {
                events.push(event.into());
            }
        }
    }

    impl server::Handler for TestUpstreamHandler {
        type Error = anyhow::Error;

        async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth> {
            Ok(if user == "bridge-test" && password == "secret" {
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
            Ok(())
        }

        async fn exec_request(
            &mut self,
            channel: ChannelId,
            data: &[u8],
            session: &mut server::Session,
        ) -> Result<()> {
            self.record(format!("exec:{}", String::from_utf8_lossy(data)));
            let _ = session.channel_success(channel);
            let _ = session.data(channel, b"stdout".to_vec());
            let _ = session.extended_data(channel, 1, b"stderr".to_vec());
            let _ = session.exit_status_request(channel, 7);
            let _ = session.eof(channel);
            let _ = session.close(channel);
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
            let _ = session.data(channel, format!("{name}-ready").into_bytes());
            let _ = session.exit_status_request(channel, 0);
            let _ = session.close(channel);
            Ok(())
        }

        async fn window_change_request(
            &mut self,
            _channel: ChannelId,
            col_width: u32,
            row_height: u32,
            _pix_width: u32,
            _pix_height: u32,
            _session: &mut server::Session,
        ) -> Result<()> {
            self.record(format!("window:{col_width}x{row_height}"));
            Ok(())
        }

        async fn signal(
            &mut self,
            _channel: ChannelId,
            signal: Sig,
            _session: &mut server::Session,
        ) -> Result<()> {
            self.record(format!("signal:{signal:?}"));
            Ok(())
        }

        async fn data(
            &mut self,
            channel: ChannelId,
            data: &[u8],
            session: &mut server::Session,
        ) -> Result<()> {
            let _ = session.data(channel, data.to_vec());
            Ok(())
        }

        async fn channel_eof(
            &mut self,
            channel: ChannelId,
            session: &mut server::Session,
        ) -> Result<()> {
            let _ = session.eof(channel);
            Ok(())
        }

        async fn channel_close(
            &mut self,
            channel: ChannelId,
            session: &mut server::Session,
        ) -> Result<()> {
            let _ = session.close(channel);
            Ok(())
        }
    }

    async fn spawn_test_upstream(
        events: Arc<StdMutex<Vec<String>>>,
    ) -> (u16, russh::keys::PublicKey, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let private_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let public_key = private_key.public_key().clone();
        let config = Arc::new(server::Config {
            methods: russh::MethodSet::from(&[russh::MethodKind::Password][..]),
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![private_key],
            ..Default::default()
        });
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let running = server::run_stream(config, stream, TestUpstreamHandler { events })
                .await
                .unwrap();
            let _ = running.await;
        });
        (port, public_key, task)
    }

    async fn expect_success(channel: &mut Channel<client::Msg>) {
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => return,
                Some(ChannelMsg::Failure) => panic!("request unexpectedly failed"),
                Some(_) => {}
                None => panic!("channel closed before request reply"),
            }
        }
    }

    #[test]
    fn generated_identity_writes_only_public_known_host_material() {
        let directory = tempfile::tempdir().expect("identity directory");
        let path = directory.path().join("bridge_known_hosts");
        let identity =
            SshBridgeServerIdentity::generate("instance", &path).expect("identity should generate");
        let contents = std::fs::read_to_string(&path).expect("known-hosts sidecar");

        assert!(contents.starts_with("miaominal-bridge-instance ssh-ed25519 "));
        assert!(!contents.contains("PRIVATE KEY"));
        assert_eq!(identity.known_hosts_path(), path);
    }

    #[test]
    fn local_server_configuration_is_bounded_and_none_only() {
        let directory = tempfile::tempdir().expect("identity directory");
        let identity = SshBridgeServerIdentity::generate(
            "instance",
            directory.path().join("bridge_known_hosts"),
        )
        .expect("identity should generate");
        let config = server_config(&identity);

        assert_eq!(config.window_size, BRIDGE_SERVER_WINDOW_SIZE);
        assert_eq!(
            config.maximum_packet_size,
            BRIDGE_SERVER_MAXIMUM_PACKET_SIZE
        );
        assert_eq!(
            config.channel_buffer_size,
            BRIDGE_SERVER_CHANNEL_BUFFER_SIZE
        );
        assert_eq!(config.event_buffer_size, BRIDGE_SERVER_EVENT_BUFFER_SIZE);
        assert_eq!(&*config.methods, &[russh::MethodKind::None]);
        assert_eq!(config.keys.len(), 1);
    }

    #[tokio::test]
    async fn local_server_relays_sessions_requests_and_direct_tcpip() {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let (upstream_port, upstream_key, upstream_task) =
            spawn_test_upstream(events.clone()).await;
        let directory = tempfile::tempdir().expect("bridge test directory");
        let known_hosts = KnownHostsStore::with_path(directory.path().join("upstream_known_hosts"));
        known_hosts
            .learn("127.0.0.1", upstream_port, &upstream_key)
            .unwrap();

        let mut profile = SessionProfile::blank("target", 1);
        profile.host = "127.0.0.1".into();
        profile.port = upstream_port;
        profile.username = "bridge-test".into();
        profile.password = "secret".into();
        profile.auth_method = Some(AuthMethod::Password);
        let route = ProfileConnector::new(
            vec![profile.clone()],
            Vec::new(),
            SecretStore::new_locked_vault(),
            known_hosts,
        )
        .connect_bridge(profile)
        .await
        .expect("connect upstream route");

        let identity = Arc::new(
            SshBridgeServerIdentity::generate("test", directory.path().join("bridge_known_hosts"))
                .unwrap(),
        );
        let (local_client_stream, local_server_stream) = tokio::io::duplex(1024 * 1024);
        let bridge_task = tokio::spawn(run_ssh_bridge_server(
            Box::new(local_server_stream),
            route,
            identity,
            8,
        ));

        let mut local = client::connect_stream(
            connection::default_client_config(),
            local_client_stream,
            AcceptAllClientHandler,
        )
        .await
        .expect("connect local bridge server");
        assert!(
            local
                .authenticate_none("miaominal")
                .await
                .unwrap()
                .success()
        );

        let mut exec = local.channel_open_session().await.unwrap();
        exec.request_pty(true, "xterm", 80, 24, 0, 0, &[])
            .await
            .unwrap();
        expect_success(&mut exec).await;
        exec.set_env(true, "LANG", "C.UTF-8").await.unwrap();
        expect_success(&mut exec).await;
        exec.window_change(120, 40, 0, 0).await.unwrap();
        exec.signal(Sig::TERM).await.unwrap();
        exec.exec(true, b"printf bridge".to_vec()).await.unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_status = None;
        while let Some(message) = exec.wait().await {
            match message {
                ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
                ChannelMsg::ExtendedData { data, ext: 1 } => stderr.extend_from_slice(&data),
                ChannelMsg::ExitStatus {
                    exit_status: status,
                } => exit_status = Some(status),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        assert_eq!(stdout, b"stdout");
        assert_eq!(stderr, b"stderr");
        assert_eq!(exit_status, Some(7));

        let mut subsystem = local.channel_open_session().await.unwrap();
        subsystem.request_subsystem(true, "sftp").await.unwrap();
        expect_success(&mut subsystem).await;
        assert!(matches!(
            subsystem.wait().await,
            Some(ChannelMsg::Data { data }) if data == b"sftp-ready".as_slice()
        ));

        let echo_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let echo_port = echo_listener.local_addr().unwrap().port();
        let echo_task = tokio::spawn(async move {
            let (mut stream, _) = echo_listener.accept().await.unwrap();
            let (mut reader, mut writer) = stream.split();
            tokio::io::copy(&mut reader, &mut writer).await.unwrap();
        });
        let direct = local
            .channel_open_direct_tcpip("127.0.0.1", u32::from(echo_port), "127.0.0.1", 12345)
            .await
            .expect("direct-tcpip should open");
        let mut direct = direct.into_stream();
        direct.write_all(b"forwarded").await.unwrap();
        let mut echoed = vec![0; 9];
        direct.read_exact(&mut echoed).await.unwrap();
        assert_eq!(echoed, b"forwarded");
        direct.shutdown().await.unwrap();
        echo_task.await.unwrap();

        assert!(local.tcpip_forward("127.0.0.1", 0).await.is_err());
        assert!(local.streamlocal_forward("bridge.sock").await.is_err());
        assert!(
            local
                .channel_open_direct_streamlocal("bridge.sock")
                .await
                .is_err()
        );
        let mut rejected = local.channel_open_session().await.unwrap();
        rejected.agent_forward(true).await.unwrap();
        assert!(matches!(rejected.wait().await, Some(ChannelMsg::Failure)));

        local
            .disconnect(Disconnect::ByApplication, "", "English")
            .await
            .unwrap();
        bridge_task.await.unwrap().unwrap();
        upstream_task.await.unwrap();

        let events = events.lock().unwrap().clone();
        assert!(events.iter().any(|event| event == "pty"));
        assert!(events.iter().any(|event| event == "env:LANG=C.UTF-8"));
        assert!(events.iter().any(|event| event == "window:120x40"));
        assert!(events.iter().any(|event| event == "signal:TERM"));
        assert!(events.iter().any(|event| event == "exec:printf bridge"));
        assert!(events.iter().any(|event| event == "subsystem:sftp"));
    }

    #[tokio::test]
    async fn local_server_enforces_channel_bounds_and_releases_slots() {
        let events = Arc::new(StdMutex::new(Vec::new()));
        let (upstream_port, upstream_key, upstream_task) = spawn_test_upstream(events).await;
        let directory = tempfile::tempdir().expect("bridge test directory");
        let known_hosts = KnownHostsStore::with_path(directory.path().join("upstream_known_hosts"));
        known_hosts
            .learn("127.0.0.1", upstream_port, &upstream_key)
            .unwrap();
        let mut profile = SessionProfile::blank("target", 1);
        profile.host = "127.0.0.1".into();
        profile.port = upstream_port;
        profile.username = "bridge-test".into();
        profile.password = "secret".into();
        let route = ProfileConnector::new(
            vec![profile.clone()],
            Vec::new(),
            SecretStore::new_locked_vault(),
            known_hosts,
        )
        .connect_bridge(profile)
        .await
        .unwrap();
        let identity = Arc::new(
            SshBridgeServerIdentity::generate(
                "bounded",
                directory.path().join("bridge_known_hosts"),
            )
            .unwrap(),
        );
        let (client_stream, server_stream) = tokio::io::duplex(1024 * 1024);
        let bridge_task = tokio::spawn(run_ssh_bridge_server(
            Box::new(server_stream),
            route,
            identity,
            1,
        ));
        let mut local = client::connect_stream(
            connection::default_client_config(),
            client_stream,
            AcceptAllClientHandler,
        )
        .await
        .unwrap();
        assert!(
            local
                .authenticate_none("miaominal")
                .await
                .unwrap()
                .success()
        );

        let mut first = local.channel_open_session().await.unwrap();
        first.request_shell(true).await.unwrap();
        expect_success(&mut first).await;
        assert!(local.channel_open_session().await.is_err());

        first.close().await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(mut replacement) = local.channel_open_session().await {
                    replacement.request_shell(true).await.unwrap();
                    expect_success(&mut replacement).await;
                    replacement.close().await.unwrap();
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("channel slot should be released");

        local
            .disconnect(Disconnect::ByApplication, "", "English")
            .await
            .unwrap();
        bridge_task.await.unwrap().unwrap();
        upstream_task.await.unwrap();
    }

    #[tokio::test]
    async fn completed_relay_tasks_are_reaped_before_more_channels_accumulate() {
        let mut tasks = JoinSet::new();
        for _ in 0..128 {
            tasks.spawn(async {});
        }
        while !tasks.is_empty() {
            tokio::task::yield_now().await;
            reap_completed_relay_tasks(&mut tasks);
        }
        assert!(tasks.is_empty());
    }
}
