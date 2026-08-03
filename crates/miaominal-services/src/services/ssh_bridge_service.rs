use anyhow::{Context, Result, anyhow, bail};
use miaominal_core::profile::{ProfileKind, SessionProfile};
use miaominal_core::proxy::ProxyProfile;
use miaominal_secrets::SecretStore;
use miaominal_settings::SshBridgeConfig;
use miaominal_ssh::{
    ProfileConnector, SshBridgeEndpoint, SshBridgeListener, SshBridgeRoute, SshBridgeRouteTable,
    SshBridgeServerIdentity, SshBridgeStatus, accept_route_request_with,
    run_ssh_bridge_server_with_shutdown,
};
use miaominal_storage::KnownHostsStore;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::{Mutex, Semaphore, watch};
use tokio::task::{JoinHandle, JoinSet};

const ACCEPT_ERROR_RETRY_INITIAL_DELAY: std::time::Duration = std::time::Duration::from_millis(100);
const ACCEPT_ERROR_RETRY_MAX_DELAY: std::time::Duration = std::time::Duration::from_secs(5);

fn next_accept_error_retry_delay(current: std::time::Duration) -> std::time::Duration {
    current.saturating_mul(2).min(ACCEPT_ERROR_RETRY_MAX_DELAY)
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SshBridgeRouteRefresh {
    pub routes: Vec<SshBridgeRoute>,
    pub diagnostics: Vec<String>,
}

impl SshBridgeRouteRefresh {
    pub fn exported_profile_count(&self) -> usize {
        self.routes.len()
    }

    pub fn skipped_profile_count(&self) -> usize {
        self.diagnostics.len()
    }
}

#[derive(Clone)]
pub struct SshBridgeService {
    inner: Arc<Inner>,
}

struct Inner {
    runtime: TokioHandle,
    endpoint: SshBridgeEndpoint,
    instance_id: String,
    known_hosts_path: PathBuf,
    known_hosts: KnownHostsStore,
    config: SshBridgeConfig,
    snapshot: RwLock<Arc<BridgeSnapshot>>,
    refresh: RwLock<SshBridgeRouteRefresh>,
    desired_enabled: AtomicBool,
    active_connections: AtomicUsize,
    last_error: StdMutex<Option<String>>,
    status: watch::Sender<SshBridgeStatus>,
    control: StdMutex<ServiceControl>,
    operation: Mutex<()>,
}

#[derive(Clone)]
struct BridgeSnapshot {
    profiles: Vec<SessionProfile>,
    proxies: Vec<ProxyProfile>,
    profiles_by_id: HashMap<String, SessionProfile>,
    secrets: SecretStore,
    routes: SshBridgeRouteTable,
}

struct ServiceControl {
    cancel: Option<watch::Sender<bool>>,
    task: Option<JoinHandle<()>>,
    identity: Option<Arc<SshBridgeServerIdentity>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Ok(mut control) = self.control.lock() {
            if let Some(cancel) = control.cancel.take() {
                let _ = cancel.send(true);
            }
            if let Some(task) = control.task.take() {
                task.abort();
            }
        }
    }
}

impl SshBridgeService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        runtime: TokioHandle,
        endpoint: SshBridgeEndpoint,
        instance_id: String,
        known_hosts_path: PathBuf,
        config: SshBridgeConfig,
        secrets: SecretStore,
        known_hosts: KnownHostsStore,
    ) -> Self {
        let (status, _) = watch::channel(SshBridgeStatus::Disabled);
        Self {
            inner: Arc::new(Inner {
                runtime,
                endpoint,
                instance_id,
                known_hosts_path,
                known_hosts,
                config,
                snapshot: RwLock::new(Arc::new(BridgeSnapshot {
                    profiles: Vec::new(),
                    proxies: Vec::new(),
                    profiles_by_id: HashMap::new(),
                    secrets,
                    routes: SshBridgeRouteTable::default(),
                })),
                refresh: RwLock::new(SshBridgeRouteRefresh::default()),
                desired_enabled: AtomicBool::new(false),
                active_connections: AtomicUsize::new(0),
                last_error: StdMutex::new(None),
                status,
                control: StdMutex::new(ServiceControl {
                    cancel: None,
                    task: None,
                    identity: None,
                }),
                operation: Mutex::new(()),
            }),
        }
    }

    pub fn endpoint(&self) -> &SshBridgeEndpoint {
        &self.inner.endpoint
    }

    pub fn known_hosts_path(&self) -> &std::path::Path {
        &self.inner.known_hosts_path
    }

    pub fn ensure_known_hosts_sidecar(&self) -> Result<bool> {
        let identity = self
            .inner
            .control
            .lock()
            .map_err(|_| anyhow!("SSH Bridge service control lock is poisoned"))?
            .identity
            .clone();
        let Some(identity) = identity else {
            return Ok(false);
        };
        identity
            .write_known_hosts_sidecar()
            .context("failed to restore SSH Bridge known-hosts sidecar")?;
        Ok(true)
    }

    pub fn status(&self) -> SshBridgeStatus {
        self.inner.status.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<SshBridgeStatus> {
        self.inner.status.subscribe()
    }

    pub fn refresh_routes(
        &self,
        profiles: Vec<SessionProfile>,
        proxies: Vec<ProxyProfile>,
    ) -> SshBridgeRouteRefresh {
        let refresh = validate_and_build_routes(&self.inner.instance_id, &profiles, &proxies);
        let routes = SshBridgeRouteTable::default();
        routes.replace(refresh.routes.clone());
        let secrets = self
            .inner
            .snapshot
            .read()
            .map(|snapshot| snapshot.secrets.clone())
            .unwrap_or_else(|_| SecretStore::new_locked_vault());
        let profiles_by_id = profiles
            .iter()
            .cloned()
            .map(|profile| (profile.id.clone(), profile))
            .collect();
        if let Ok(mut snapshot) = self.inner.snapshot.write() {
            *snapshot = Arc::new(BridgeSnapshot {
                profiles,
                proxies,
                profiles_by_id,
                secrets,
                routes,
            });
        }
        if let Ok(mut current) = self.inner.refresh.write() {
            *current = refresh.clone();
        }
        self.inner.publish_running();
        refresh
    }

    pub fn replace_secrets(&self, secrets: SecretStore) {
        if let Ok(mut snapshot) = self.inner.snapshot.write() {
            let current = snapshot.as_ref();
            *snapshot = Arc::new(BridgeSnapshot {
                profiles: current.profiles.clone(),
                proxies: current.proxies.clone(),
                profiles_by_id: current.profiles_by_id.clone(),
                secrets,
                routes: current.routes.clone(),
            });
        }
    }

    pub fn route_refresh(&self) -> SshBridgeRouteRefresh {
        self.inner
            .refresh
            .read()
            .map(|refresh| refresh.clone())
            .unwrap_or_default()
    }

    pub fn set_desired_enabled(&self, enabled: bool) {
        self.inner.desired_enabled.store(enabled, Ordering::Release);
    }

    pub async fn reconcile_desired_state(&self) -> Result<()> {
        let _operation = self.inner.operation.lock().await;
        loop {
            let desired_enabled = self.inner.desired_enabled.load(Ordering::Acquire);
            let result = if desired_enabled {
                self.start_locked().await
            } else {
                self.stop_locked().await;
                Ok(())
            };
            if let Err(error) = result
                && self.inner.desired_enabled.load(Ordering::Acquire) == desired_enabled
            {
                return Err(error);
            }
            if self.inner.desired_enabled.load(Ordering::Acquire) == desired_enabled {
                return Ok(());
            }
        }
    }

    pub async fn enable(&self) -> Result<()> {
        self.set_desired_enabled(true);
        self.reconcile_desired_state().await
    }

    async fn start_locked(&self) -> Result<()> {
        if matches!(
            self.status(),
            SshBridgeStatus::Starting | SshBridgeStatus::Running { .. }
        ) {
            return Ok(());
        }
        self.inner.status.send_replace(SshBridgeStatus::Starting);
        let identity = match SshBridgeServerIdentity::generate(
            &self.inner.instance_id,
            self.inner.known_hosts_path.clone(),
        )
        .context("failed to initialize SSH Bridge host identity")
        {
            Ok(identity) => Arc::new(identity),
            Err(error) => {
                self.inner.status.send_replace(SshBridgeStatus::Error {
                    endpoint: Some(self.inner.endpoint.clone()),
                    message: error.to_string(),
                });
                return Err(error);
            }
        };
        let listener = match SshBridgeListener::bind(&self.inner.endpoint).await {
            Ok(listener) => listener,
            Err(error) => {
                self.inner.status.send_replace(SshBridgeStatus::Error {
                    endpoint: Some(self.inner.endpoint.clone()),
                    message: error.to_string(),
                });
                return Err(error);
            }
        };
        let (cancel, cancel_receiver) = watch::channel(false);
        let inner = self.inner.clone();
        let task_identity = identity.clone();
        let task = self.inner.runtime.spawn(async move {
            inner
                .accept_loop(listener, task_identity, cancel_receiver)
                .await;
        });
        if let Ok(mut control) = self.inner.control.lock() {
            control.cancel = Some(cancel);
            control.task = Some(task);
            control.identity = Some(identity);
        }
        self.inner.publish_running();
        Ok(())
    }

    pub async fn disable(&self) {
        self.set_desired_enabled(false);
        let _ = self.reconcile_desired_state().await;
    }

    async fn stop_locked(&self) {
        if matches!(self.status(), SshBridgeStatus::Disabled) {
            return;
        }
        self.inner.status.send_replace(SshBridgeStatus::Stopping);
        let (cancel, task) = self
            .inner
            .control
            .lock()
            .map(|mut control| {
                control.identity = None;
                (control.cancel.take(), control.task.take())
            })
            .unwrap_or((None, None));
        if let Some(cancel) = cancel {
            let _ = cancel.send(true);
        }
        if let Some(task) = task {
            let _ = task.await;
        }
        self.inner.active_connections.store(0, Ordering::Release);
        self.inner.status.send_replace(SshBridgeStatus::Disabled);
    }

    pub async fn shutdown(&self) {
        self.disable().await;
    }
}

impl Inner {
    fn refresh_counts(&self) -> (usize, usize) {
        self.refresh
            .read()
            .map(|refresh| {
                (
                    refresh.exported_profile_count(),
                    refresh.skipped_profile_count(),
                )
            })
            .unwrap_or((0, 0))
    }

    fn publish_running(&self) {
        if !matches!(
            *self.status.borrow(),
            SshBridgeStatus::Running { .. } | SshBridgeStatus::Starting
        ) {
            return;
        }
        let (exported_profile_count, skipped_profile_count) = self.refresh_counts();
        let last_error = self.last_error.lock().ok().and_then(|error| error.clone());
        self.status.send_replace(SshBridgeStatus::Running {
            endpoint: self.endpoint.clone(),
            exported_profile_count,
            skipped_profile_count,
            active_connection_count: self.active_connections.load(Ordering::Acquire),
            last_error,
        });
    }

    fn record_error(&self, error: impl Into<String>) {
        if let Ok(mut last_error) = self.last_error.lock() {
            *last_error = Some(error.into());
        }
        self.publish_running();
    }

    async fn accept_loop(
        self: Arc<Self>,
        mut listener: SshBridgeListener,
        identity: Arc<SshBridgeServerIdentity>,
        mut cancel: watch::Receiver<bool>,
    ) {
        let connection_slots = Arc::new(Semaphore::new(usize::from(
            self.config.max_connections.max(1),
        )));
        let mut clients = JoinSet::new();
        let mut accept_error_retry_delay = ACCEPT_ERROR_RETRY_INITIAL_DELAY;
        loop {
            tokio::select! {
                changed = cancel.changed() => {
                    if changed.is_err() || *cancel.borrow() {
                        break;
                    }
                }
                Some(result) = clients.join_next(), if !clients.is_empty() => {
                    if let Err(error) = result {
                        self.record_error(format!("SSH Bridge client task failed: {error}"));
                    }
                }
                accepted = listener.accept() => {
                    let stream = match accepted {
                        Ok(stream) => stream,
                        Err(error) => {
                            self.record_error(error.to_string());
                            tokio::select! {
                                _ = tokio::time::sleep(accept_error_retry_delay) => {}
                                _ = wait_for_bridge_cancellation(&mut cancel) => break,
                            }
                            accept_error_retry_delay =
                                next_accept_error_retry_delay(accept_error_retry_delay);
                            continue;
                        }
                    };
                    accept_error_retry_delay = ACCEPT_ERROR_RETRY_INITIAL_DELAY;
                    let permit = match connection_slots.clone().try_acquire_owned() {
                        Ok(permit) => permit,
                        Err(_) => {
                            drop(stream);
                            self.record_error("SSH Bridge connection limit reached");
                            continue;
                        }
                    };
                    let inner = self.clone();
                    let identity = identity.clone();
                    let client_cancel = cancel.clone();
                    clients.spawn(async move {
                        let _permit = permit;
                        inner.active_connections.fetch_add(1, Ordering::AcqRel);
                        inner.publish_running();
                        let result = inner.handle_client(stream, identity, client_cancel).await;
                        inner.active_connections.fetch_sub(1, Ordering::AcqRel);
                        if let Err(error) = result {
                            inner.record_error(format!("{error:#}"));
                        } else {
                            inner.publish_running();
                        }
                    });
                }
            }
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(7);
        while !clients.is_empty() {
            match tokio::time::timeout_at(deadline, clients.join_next()).await {
                Ok(Some(result)) => {
                    if let Err(error) = result {
                        self.record_error(format!("SSH Bridge client task failed: {error}"));
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    self.record_error("SSH Bridge client shutdown timed out");
                    clients.abort_all();
                    while clients.join_next().await.is_some() {}
                    break;
                }
            }
        }
    }

    async fn handle_client(
        &self,
        mut stream: miaominal_ssh::SshBridgeStream,
        identity: Arc<SshBridgeServerIdentity>,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<()> {
        let snapshot = self
            .snapshot
            .read()
            .map_err(|_| anyhow!("SSH Bridge snapshot lock is poisoned"))?
            .clone();
        let routes = snapshot.routes.clone();
        let known_hosts = self.known_hosts.clone();
        let route = tokio::select! {
            _ = wait_for_bridge_cancellation(&mut cancel) => {
                bail!("SSH Bridge is stopping")
            }
            route = accept_route_request_with(&mut stream, &routes, move |route| {
                let snapshot = snapshot.clone();
                async move {
                    let profile = snapshot
                        .profiles_by_id
                        .get(&route.profile_id)
                        .cloned()
                        .ok_or_else(|| anyhow!("SSH Bridge profile is no longer available"))?;
                    ProfileConnector::new(
                        snapshot.profiles.clone(),
                        snapshot.proxies.clone(),
                        snapshot.secrets.clone(),
                        known_hosts,
                    )
                    .connect_bridge(profile)
                    .await
                }
            }) => route?,
        };
        run_ssh_bridge_server_with_shutdown(
            stream,
            route,
            identity,
            usize::from(self.config.max_channels_per_connection.max(1)),
            async move {
                wait_for_bridge_cancellation(&mut cancel).await;
            },
        )
        .await
    }
}

async fn wait_for_bridge_cancellation(cancel: &mut watch::Receiver<bool>) {
    if *cancel.borrow() {
        return;
    }
    while cancel.changed().await.is_ok() {
        if *cancel.borrow() {
            return;
        }
    }
}

fn validate_and_build_routes(
    instance_id: &str,
    profiles: &[SessionProfile],
    proxies: &[ProxyProfile],
) -> SshBridgeRouteRefresh {
    let by_id = profiles
        .iter()
        .map(|profile| (profile.id.as_str(), profile))
        .collect::<HashMap<_, _>>();
    let proxy_ids = proxies
        .iter()
        .map(|proxy| proxy.id.as_str())
        .collect::<HashSet<_>>();
    let mut memo = HashMap::<String, std::result::Result<(), String>>::new();
    let mut routes = Vec::new();
    let mut diagnostics = Vec::new();

    for profile in profiles {
        let mut stack = Vec::new();
        match validate_profile_topology(profile, &by_id, &proxy_ids, &mut memo, &mut stack) {
            Ok(()) => routes.push(SshBridgeRoute::derive(instance_id, profile)),
            Err(error) => diagnostics.push(format!("{}: {error}", profile.connection_label())),
        }
    }
    SshBridgeRouteRefresh {
        routes,
        diagnostics,
    }
}

fn validate_profile_topology(
    profile: &SessionProfile,
    profiles: &HashMap<&str, &SessionProfile>,
    proxy_ids: &HashSet<&str>,
    memo: &mut HashMap<String, std::result::Result<(), String>>,
    stack: &mut Vec<String>,
) -> std::result::Result<(), String> {
    if let Some(result) = memo.get(&profile.id) {
        return result.clone();
    }
    if stack.iter().any(|id| id == &profile.id) {
        return Err(format!("jump dependency cycle includes {}", profile.id));
    }
    stack.push(profile.id.clone());
    let result = (|| {
        if profile.kind != ProfileKind::Ssh {
            return Err("profile is not SSH".into());
        }
        if profile.host.trim().is_empty() || profile.username.trim().is_empty() {
            return Err("host and username are required".into());
        }
        if let Some(proxy_id) = profile.entry_proxy_id.as_deref()
            && !proxy_ids.contains(proxy_id)
        {
            return Err(format!("entry proxy {proxy_id} is missing"));
        }
        let mut jumps = HashSet::new();
        for jump_id in &profile.proxy_jump_profile_ids {
            if jump_id == &profile.id {
                return Err("profile references itself as a jump host".into());
            }
            if !jumps.insert(jump_id.as_str()) {
                return Err(format!("jump host {jump_id} is repeated"));
            }
            let jump = profiles
                .get(jump_id.as_str())
                .ok_or_else(|| format!("jump host {jump_id} is missing"))?;
            validate_profile_topology(jump, profiles, proxy_ids, memo, stack)?;
        }
        Ok(())
    })();
    stack.pop();
    memo.insert(profile.id.clone(), result.clone());
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_secrets::{
        APP_CREDENTIAL_SERVICE, CredentialStore, ProtectedPassphrase, SecretKind,
        VaultCredentialBackend, set_vault_test_parameters,
    };
    use miaominal_settings::SshBridgeConfig;
    use miaominal_ssh::{connect_endpoint, request_route};
    use russh::keys::{Algorithm, PrivateKey};
    use russh::{Channel, ChannelId, ChannelMsg, Disconnect, client, server};
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio::sync::watch;
    use tokio::task::{JoinHandle, JoinSet};

    #[test]
    fn accept_error_retry_delay_grows_and_is_capped() {
        let mut delay = ACCEPT_ERROR_RETRY_INITIAL_DELAY;
        for _ in 0..16 {
            delay = next_accept_error_retry_delay(delay);
        }
        assert_eq!(delay, ACCEPT_ERROR_RETRY_MAX_DELAY);
        assert_eq!(
            next_accept_error_retry_delay(ACCEPT_ERROR_RETRY_MAX_DELAY),
            ACCEPT_ERROR_RETRY_MAX_DELAY
        );
    }

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

    struct CountingUpstreamHandler {
        active: Arc<AtomicUsize>,
    }

    impl CountingUpstreamHandler {
        fn new(active: Arc<AtomicUsize>) -> Self {
            active.fetch_add(1, Ordering::AcqRel);
            Self { active }
        }
    }

    impl Drop for CountingUpstreamHandler {
        fn drop(&mut self) {
            self.active.fetch_sub(1, Ordering::AcqRel);
        }
    }

    impl server::Handler for CountingUpstreamHandler {
        type Error = anyhow::Error;

        async fn auth_password(&mut self, user: &str, password: &str) -> Result<server::Auth> {
            Ok(if user == "bridge-test" && password == "secret" {
                server::Auth::Accept
            } else {
                server::Auth::reject()
            })
        }

        async fn channel_open_session(
            &mut self,
            _channel: Channel<server::Msg>,
            _session: &mut server::Session,
        ) -> Result<bool> {
            Ok(true)
        }

        async fn shell_request(
            &mut self,
            channel: ChannelId,
            session: &mut server::Session,
        ) -> Result<()> {
            let _ = session.channel_success(channel);
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
    }

    struct TestUpstream {
        port: u16,
        public_key: russh::keys::PublicKey,
        accepted: Arc<AtomicUsize>,
        active: Arc<AtomicUsize>,
        cancel: watch::Sender<bool>,
        task: JoinHandle<()>,
    }

    impl TestUpstream {
        async fn stop(self) {
            let _ = self.cancel.send(true);
            let _ = self.task.await;
        }
    }

    async fn spawn_counting_upstream() -> TestUpstream {
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
        let accepted = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let (cancel, mut cancelled) = watch::channel(false);
        let accepted_for_task = accepted.clone();
        let active_for_task = active.clone();
        let task = tokio::spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {
                    changed = cancelled.changed() => {
                        if changed.is_err() || *cancelled.borrow() {
                            break;
                        }
                    }
                    Some(_) = clients.join_next(), if !clients.is_empty() => {}
                    accepted_stream = listener.accept() => {
                        let Ok((stream, _)) = accepted_stream else { break };
                        accepted_for_task.fetch_add(1, Ordering::AcqRel);
                        let config = config.clone();
                        let handler = CountingUpstreamHandler::new(active_for_task.clone());
                        clients.spawn(async move {
                            if let Ok(running) = server::run_stream(config, stream, handler).await {
                                let _ = running.await;
                            }
                        });
                    }
                }
            }
            clients.abort_all();
            while clients.join_next().await.is_some() {}
        });
        TestUpstream {
            port,
            public_key,
            accepted,
            active,
            cancel,
            task,
        }
    }

    async fn connect_bridge_client(
        service: &SshBridgeService,
        route_token: &str,
    ) -> client::Handle<AcceptAllClientHandler> {
        let mut stream = connect_endpoint(service.endpoint()).await.unwrap();
        request_route(&mut stream, route_token).await.unwrap();
        let mut client = client::connect_stream(
            Arc::new(client::Config::default()),
            stream,
            AcceptAllClientHandler,
        )
        .await
        .unwrap();
        assert!(
            client
                .authenticate_none("miaominal")
                .await
                .unwrap()
                .success()
        );
        client
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

    async fn assert_echo(client: &client::Handle<AcceptAllClientHandler>, value: &[u8]) {
        let mut channel = client.channel_open_session().await.unwrap();
        channel.request_shell(true).await.unwrap();
        expect_success(&mut channel).await;
        channel.data(value).await.unwrap();
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Data { data }) if data == value => break,
                Some(_) => {}
                None => panic!("echo channel closed before data arrived"),
            }
        }
        channel.close().await.unwrap();
    }

    async fn wait_for_count(value: &AtomicUsize, expected: usize) {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let current = value.load(Ordering::Acquire);
            if current == expected {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "count did not reach {expected}; current value is {current}"
            );
            tokio::task::yield_now().await;
        }
    }

    fn profile(id: &str) -> SessionProfile {
        let mut profile = SessionProfile::blank(id, 1);
        profile.name = id.into();
        profile.host = "example.com".into();
        profile.username = "akko".into();
        profile
    }

    #[test]
    fn topology_validation_skips_missing_repeated_and_cyclic_dependencies() {
        let valid = profile("valid");
        let mut missing = profile("missing");
        missing.proxy_jump_profile_ids = vec!["gone".into()];
        let mut repeated = profile("repeated");
        repeated.proxy_jump_profile_ids = vec!["valid".into(), "valid".into()];
        let mut cycle_a = profile("cycle-a");
        cycle_a.proxy_jump_profile_ids = vec!["cycle-b".into()];
        let mut cycle_b = profile("cycle-b");
        cycle_b.proxy_jump_profile_ids = vec!["cycle-a".into()];

        let refresh = validate_and_build_routes(
            "instance",
            &[valid, missing, repeated, cycle_a, cycle_b],
            &[],
        );
        assert_eq!(refresh.exported_profile_count(), 1);
        assert_eq!(refresh.skipped_profile_count(), 4);
        assert!(
            refresh
                .diagnostics
                .iter()
                .any(|error| error.contains("missing"))
        );
        assert!(
            refresh
                .diagnostics
                .iter()
                .any(|error| error.contains("repeated"))
        );
        assert!(
            refresh
                .diagnostics
                .iter()
                .any(|error| error.contains("cycle"))
        );
    }

    #[tokio::test]
    async fn service_enable_disable_is_idempotent_and_reports_endpoint_conflicts() {
        let runtime = TokioHandle::current();
        let directory = tempfile::tempdir().unwrap();
        let endpoint = SshBridgeEndpoint::derive(directory.path()).unwrap();
        let instance_id = SshBridgeEndpoint::instance_id(directory.path()).unwrap();
        let service = SshBridgeService::new(
            runtime.clone(),
            endpoint.clone(),
            instance_id.clone(),
            directory.path().join("known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(directory.path().join("upstream_known_hosts")),
        );
        service.refresh_routes(vec![profile("one")], vec![]);
        service.enable().await.unwrap();
        service.enable().await.unwrap();
        assert!(matches!(service.status(), SshBridgeStatus::Running { .. }));

        let competing = SshBridgeService::new(
            runtime,
            endpoint,
            instance_id,
            directory.path().join("other_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(directory.path().join("other_upstream_known_hosts")),
        );
        assert!(competing.enable().await.is_err());
        assert!(matches!(competing.status(), SshBridgeStatus::Error { .. }));

        service.disable().await;
        service.disable().await;
        assert_eq!(service.status(), SshBridgeStatus::Disabled);
    }

    #[tokio::test]
    async fn host_identity_failure_enters_error_and_enable_retries_initialization() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = SshBridgeEndpoint::derive(directory.path()).unwrap();
        let instance_id = SshBridgeEndpoint::instance_id(directory.path()).unwrap();
        let known_hosts_path = directory.path().join("bridge_known_hosts");
        std::fs::create_dir(&known_hosts_path).unwrap();
        let service = SshBridgeService::new(
            TokioHandle::current(),
            endpoint,
            instance_id,
            known_hosts_path.clone(),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(directory.path().join("upstream_known_hosts")),
        );

        service
            .enable()
            .await
            .expect_err("a directory at the known-hosts path must fail identity setup");
        assert!(matches!(service.status(), SshBridgeStatus::Error { .. }));

        std::fs::remove_dir(&known_hosts_path).unwrap();
        service
            .enable()
            .await
            .expect("enable should retry identity generation after the path is repaired");
        assert!(matches!(service.status(), SshBridgeStatus::Running { .. }));
        service.disable().await;
    }

    #[tokio::test]
    async fn delayed_reconcile_uses_latest_desired_state() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = SshBridgeEndpoint::derive(directory.path()).unwrap();
        let instance_id = SshBridgeEndpoint::instance_id(directory.path()).unwrap();
        let service = SshBridgeService::new(
            TokioHandle::current(),
            endpoint,
            instance_id,
            directory.path().join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(directory.path().join("upstream_known_hosts")),
        );
        let (release, delayed) = tokio::sync::oneshot::channel();

        service.set_desired_enabled(true);
        let older_service = service.clone();
        let older = tokio::spawn(async move {
            let _ = delayed.await;
            older_service.reconcile_desired_state().await
        });
        service.set_desired_enabled(false);
        service.reconcile_desired_state().await.unwrap();
        let _ = release.send(());
        older.await.unwrap().unwrap();

        assert_eq!(service.status(), SshBridgeStatus::Disabled);
        assert!(connect_endpoint(service.endpoint()).await.is_err());
    }

    #[tokio::test]
    async fn excess_connections_are_closed_before_a_client_task_is_created() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = SshBridgeEndpoint::derive(directory.path()).unwrap();
        let instance_id = SshBridgeEndpoint::instance_id(directory.path()).unwrap();
        let service = SshBridgeService::new(
            TokioHandle::current(),
            endpoint,
            instance_id,
            directory.path().join("bridge_known_hosts"),
            SshBridgeConfig {
                max_connections: 1,
                ..SshBridgeConfig::default()
            },
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(directory.path().join("upstream_known_hosts")),
        );
        let refresh = service.refresh_routes(vec![profile("limited")], vec![]);
        let route_token = refresh.routes[0].token.clone();
        service.enable().await.unwrap();

        let held = connect_endpoint(service.endpoint()).await.unwrap();
        wait_for_count(&service.inner.active_connections, 1).await;
        for _ in 0..12 {
            let connected =
                tokio::time::timeout(Duration::from_secs(1), connect_endpoint(service.endpoint()))
                    .await
                    .expect("an excess connection attempt must finish promptly");
            if let Ok(mut stream) = connected {
                let result = tokio::time::timeout(
                    Duration::from_secs(1),
                    request_route(&mut stream, &route_token),
                )
                .await
                .expect("an accepted excess stream must be closed promptly");
                assert!(result.is_err());
            }
        }
        assert_eq!(service.inner.active_connections.load(Ordering::Acquire), 1);

        drop(held);
        wait_for_count(&service.inner.active_connections, 0).await;
        service.disable().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_clients_keep_isolated_routes_across_profile_refresh_and_disable_cleanup() {
        let upstream_a = spawn_counting_upstream().await;
        let upstream_b = spawn_counting_upstream().await;
        let directory = tempfile::tempdir().unwrap();
        let known_hosts = KnownHostsStore::with_path(directory.path().join("upstream_known_hosts"));
        known_hosts
            .learn("127.0.0.1", upstream_a.port, &upstream_a.public_key)
            .unwrap();
        known_hosts
            .learn("127.0.0.1", upstream_b.port, &upstream_b.public_key)
            .unwrap();
        let endpoint = SshBridgeEndpoint::derive(directory.path()).unwrap();
        let instance_id = SshBridgeEndpoint::instance_id(directory.path()).unwrap();
        let service = SshBridgeService::new(
            TokioHandle::current(),
            endpoint,
            instance_id,
            directory.path().join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            known_hosts,
        );
        let mut target = profile("target");
        target.host = "127.0.0.1".into();
        target.port = upstream_a.port;
        target.username = "bridge-test".into();
        target.password = "secret".into();
        let first_refresh = service.refresh_routes(vec![target.clone()], vec![]);
        let route_token = first_refresh.routes[0].token.clone();
        tokio::time::timeout(Duration::from_secs(5), service.enable())
            .await
            .expect("Bridge enable should complete")
            .unwrap();

        let first = tokio::time::timeout(
            Duration::from_secs(5),
            connect_bridge_client(&service, &route_token),
        )
        .await
        .expect("first Bridge client should connect");
        wait_for_count(&upstream_a.accepted, 1).await;
        tokio::time::timeout(
            Duration::from_secs(5),
            assert_echo(&first, b"first-before-refresh"),
        )
        .await
        .expect("first client should echo before refresh");

        target.port = upstream_b.port;
        let second_refresh = service.refresh_routes(vec![target], vec![]);
        assert_eq!(second_refresh.routes[0].token, route_token);
        let second = tokio::time::timeout(
            Duration::from_secs(5),
            connect_bridge_client(&service, &route_token),
        )
        .await
        .expect("second Bridge client should connect");
        wait_for_count(&upstream_b.accepted, 1).await;
        wait_for_count(&upstream_a.active, 1).await;
        wait_for_count(&upstream_b.active, 1).await;
        tokio::time::timeout(
            Duration::from_secs(5),
            assert_echo(&first, b"first-after-refresh"),
        )
        .await
        .expect("first client should retain its active route snapshot");
        tokio::time::timeout(
            Duration::from_secs(5),
            assert_echo(&second, b"second-after-refresh"),
        )
        .await
        .expect("second client should use the refreshed route snapshot");
        assert!(matches!(
            service.status(),
            SshBridgeStatus::Running {
                active_connection_count: 2,
                ..
            }
        ));

        tokio::time::timeout(
            Duration::from_secs(5),
            first.disconnect(Disconnect::ByApplication, "", "English"),
        )
        .await
        .expect("first client disconnect should complete")
        .unwrap();
        wait_for_count(&upstream_a.active, 0).await;
        tokio::time::timeout(
            Duration::from_secs(5),
            assert_echo(&second, b"second-remains-isolated"),
        )
        .await
        .expect("second client should remain isolated after first disconnects");

        tokio::time::timeout(Duration::from_secs(5), service.disable())
            .await
            .expect("Bridge disable should cancel active clients");
        assert_eq!(service.status(), SshBridgeStatus::Disabled);
        wait_for_count(&upstream_b.active, 0).await;
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), second.channel_open_session()).await,
            Ok(Err(_))
        ));

        tokio::time::timeout(Duration::from_secs(5), upstream_a.stop())
            .await
            .expect("first upstream should stop");
        tokio::time::timeout(Duration::from_secs(5), upstream_b.stop())
            .await
            .expect("second upstream should stop");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn locked_credentials_fail_without_removing_route_and_succeed_after_secret_refresh() {
        set_vault_test_parameters();
        let upstream = spawn_counting_upstream().await;
        let directory = tempfile::tempdir().unwrap();
        let known_hosts = KnownHostsStore::with_path(directory.path().join("upstream_known_hosts"));
        known_hosts
            .learn("127.0.0.1", upstream.port, &upstream.public_key)
            .unwrap();
        let endpoint = SshBridgeEndpoint::derive(directory.path()).unwrap();
        let instance_id = SshBridgeEndpoint::instance_id(directory.path()).unwrap();
        let service = SshBridgeService::new(
            TokioHandle::current(),
            endpoint,
            instance_id,
            directory.path().join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            known_hosts,
        );
        let mut target = profile("stored-password");
        target.host = "127.0.0.1".into();
        target.port = upstream.port;
        target.username = "bridge-test".into();
        target.has_stored_password = true;
        let refresh = service.refresh_routes(vec![target], vec![]);
        let route_token = refresh.routes[0].token.clone();
        tokio::time::timeout(Duration::from_secs(5), service.enable())
            .await
            .expect("Bridge enable should complete")
            .unwrap();

        let mut locked_stream = connect_endpoint(service.endpoint()).await.unwrap();
        let locked_error = tokio::time::timeout(
            Duration::from_secs(5),
            request_route(&mut locked_stream, &route_token),
        )
        .await
        .expect("locked route request should receive an error response")
        .expect_err("locked vault should reject a new route");
        assert!(locked_error.to_string().contains("local vault is locked"));
        assert_eq!(service.route_refresh().routes.len(), 1);

        let credentials = CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new_with_path(
                directory.path().join("test_vault.json"),
                ProtectedPassphrase::try_from_string("bridge-test-passphrase".to_string()).unwrap(),
            ),
        );
        let secrets = SecretStore::with_credentials(credentials);
        secrets
            .set("stored-password", SecretKind::Password, "secret")
            .unwrap();
        service.replace_secrets(secrets);

        let client = tokio::time::timeout(
            Duration::from_secs(5),
            connect_bridge_client(&service, &route_token),
        )
        .await
        .expect("Bridge client should connect after credential refresh");
        tokio::time::timeout(Duration::from_secs(5), assert_echo(&client, b"unlocked"))
            .await
            .expect("unlocked Bridge client should echo");
        tokio::time::timeout(Duration::from_secs(5), service.disable())
            .await
            .expect("Bridge disable should complete");
        wait_for_count(&upstream.active, 0).await;
        tokio::time::timeout(Duration::from_secs(5), upstream.stop())
            .await
            .expect("upstream should stop");
    }
}
