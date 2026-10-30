//! Runtime-only RDP sessions. No profile rules, credentials, or runtime state are persisted.
use crate::PortForwardPrompt;
use anyhow::{Context, Result, anyhow, bail};
use miaominal_core::{
    profile::{AuthMethod, PortForwardKind, PortForwardRule, ProfileKind, SessionProfile},
    proxy::ProxyProfile,
};
use miaominal_rdp::{ClientProcessState, LaunchedClient, PasswordDelivery, RdpEndpoint};
use miaominal_secrets::{SecretKind, SecretStore};
use miaominal_settings::RdpClientSettings;
use miaominal_ssh::{
    HostKeyDecision, LocalForwardBindingPolicy, SessionCommandSender, SessionEvent,
};
use miaominal_storage::KnownHostsStore;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};
use tokio::{
    runtime::Handle,
    sync::{mpsc, watch},
    time::Instant,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RdpConnectionMode {
    Direct,
    #[default]
    SshTunnel,
}

#[derive(Clone, Debug)]
pub struct RdpLaunchOptions {
    mode: RdpConnectionMode,
    reuse_password: bool,
    port: u16,
    client: RdpClientSettings,
}
impl RdpLaunchOptions {
    /// Standard RDP port, used unless the launch overrides it.
    pub const DEFAULT_PORT: u16 = 3389;
    pub fn new(mode: RdpConnectionMode, client: RdpClientSettings) -> Self {
        Self {
            mode,
            reuse_password: false,
            port: Self::DEFAULT_PORT,
            client,
        }
    }
    pub fn with_reuse_ssh_password(mut self, reuse: bool) -> Self {
        self.reuse_password = reuse;
        self
    }
    /// RDP port on the remote host: `host:port` when direct, `127.0.0.1:port` as seen
    /// from the SSH server when tunnelled. Zero is ignored.
    pub fn with_port(mut self, port: u16) -> Self {
        if port != 0 {
            self.port = port;
        }
        self
    }
    pub fn mode(&self) -> RdpConnectionMode {
        self.mode
    }
    pub fn port(&self) -> u16 {
        self.port
    }
    pub fn reuses_ssh_password(&self) -> bool {
        self.reuse_password
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RdpRuntimeState {
    Starting,
    WaitingForClient,
    ClientRunning,
    Reconnecting,
    Stopped,
    Failed(String),
}
impl RdpRuntimeState {
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Stopped | Self::Failed(_))
    }
}

#[derive(Clone, Debug)]
pub struct RdpSessionSnapshot {
    id: String,
    profile_id: String,
    profile_name: String,
    mode: RdpConnectionMode,
    state: RdpRuntimeState,
    local_port: Option<u16>,
    /// Loopback address of the tunnel listener; empty for direct connections.
    local_host: String,
    /// RDP port on the remote side (see [`RdpLaunchOptions::with_port`]).
    remote_port: u16,
    active_connections: usize,
    keep_tunnel: bool,
    release_in: Option<u64>,
    reopen_required: bool,
    prompt: Option<PortForwardPrompt>,
    log: Vec<String>,
    revision: u64,
}
impl RdpSessionSnapshot {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }
    pub fn profile_name(&self) -> &str {
        &self.profile_name
    }
    pub fn mode(&self) -> RdpConnectionMode {
        self.mode
    }
    pub fn state(&self) -> &RdpRuntimeState {
        &self.state
    }
    pub fn local_port(&self) -> Option<u16> {
        self.local_port
    }
    pub fn local_host(&self) -> &str {
        &self.local_host
    }
    pub fn remote_port(&self) -> u16 {
        self.remote_port
    }
    pub fn active_connections(&self) -> usize {
        self.active_connections
    }
    pub fn keep_tunnel(&self) -> bool {
        self.keep_tunnel
    }
    pub fn release_in(&self) -> Option<u64> {
        self.release_in
    }
    pub fn reopen_required(&self) -> bool {
        self.reopen_required
    }
    pub fn prompt(&self) -> Option<&PortForwardPrompt> {
        self.prompt.as_ref()
    }
    pub fn log(&self) -> &[String] {
        &self.log
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Clone, Debug, Default)]
pub struct RdpSnapshot {
    sessions: Vec<RdpSessionSnapshot>,
}
impl RdpSnapshot {
    pub fn sessions(&self) -> &[RdpSessionSnapshot] {
        &self.sessions
    }
    pub fn session(&self, id: &str) -> Option<&RdpSessionSnapshot> {
        self.sessions.iter().find(|s| s.id == id)
    }
}

/// All automatic release durations, separate from the two-second process polling cadence.
#[derive(Clone, Debug)]
pub(crate) struct RdpTunnelPolicy {
    exited: Duration,
    untracked_idle: Duration,
    initial: Duration,
    running_idle: Duration,
}
impl Default for RdpTunnelPolicy {
    fn default() -> Self {
        Self {
            exited: Duration::from_secs(15),
            untracked_idle: Duration::from_secs(60),
            initial: Duration::from_secs(120),
            running_idle: Duration::from_secs(600),
        }
    }
}

struct Activity {
    ready: Option<Instant>,
    idle_since: Option<Instant>,
    exited_at: Option<Instant>,
    process: ClientProcessState,
    seen: bool,
    active: usize,
    hold: bool,
    reconnecting: bool,
    binding_id: u64,
    sequence: u64,
}
impl Default for Activity {
    fn default() -> Self {
        Self {
            ready: None,
            idle_since: None,
            exited_at: None,
            process: ClientProcessState::Untracked,
            seen: false,
            active: 0,
            hold: false,
            reconnecting: false,
            binding_id: 0,
            sequence: 0,
        }
    }
}
impl Activity {
    fn ready(&mut self, binding_id: u64, now: Instant) {
        self.binding_id = binding_id;
        self.sequence = 0;
        self.active = 0;
        self.reconnecting = false;
        self.ready = Some(now);
        self.idle_since = Some(now);
        if self.process == ClientProcessState::Exited {
            self.exited_at = Some(now);
        }
    }
    fn connections(&mut self, binding_id: u64, sequence: u64, active: usize, now: Instant) -> bool {
        if self.binding_id != binding_id || sequence <= self.sequence || self.reconnecting {
            return false;
        }
        self.sequence = sequence;
        self.active = active;
        self.seen = true;
        self.idle_since = (active == 0).then_some(now);
        true
    }
    fn process(&mut self, state: ClientProcessState, now: Instant) {
        if state != self.process {
            self.exited_at = (state == ClientProcessState::Exited).then_some(now);
        }
        self.process = state;
    }
    fn hold(&mut self, hold: bool, now: Instant) {
        if self.hold && !hold {
            if self.ready.is_some() {
                self.ready = Some(now);
            }
            if self.active == 0 {
                self.idle_since = Some(now);
            }
            if self.process == ClientProcessState::Exited {
                self.exited_at = Some(now);
            }
        }
        self.hold = hold;
    }
    fn reopened(&mut self, now: Instant) {
        // Relaunching the UI does not replace the TCP listener or its live connections.
        self.process(ClientProcessState::Running, now);
        self.ready = Some(now);
        if self.active == 0 {
            self.idle_since = Some(now);
        }
    }
    fn deadline(&self, policy: &RdpTunnelPolicy) -> Option<Instant> {
        if self.hold || self.active > 0 || self.reconnecting {
            return None;
        }
        let ready = self.ready?;
        if self.process == ClientProcessState::Exited {
            return Some(
                self.idle_since
                    .unwrap_or(ready)
                    .max(self.exited_at.unwrap_or(ready))
                    + policy.exited,
            );
        }
        if !self.seen {
            return Some(
                ready
                    + if self.process == ClientProcessState::Running {
                        policy.running_idle
                    } else {
                        policy.initial
                    },
            );
        }
        Some(
            self.idle_since?
                + if self.process == ClientProcessState::Running {
                    policy.running_idle
                } else {
                    policy.untracked_idle
                },
        )
    }
}

enum Control {
    Stop,
    Reopen,
    Hold(bool),
}
struct Entry {
    snapshot: RdpSessionSnapshot,
    generation: u64,
    last_bound_port: Option<u16>,
    /// Loopback address the tunnel listens on; unique among active sessions on Windows.
    listen_host: String,
    options: RdpLaunchOptions,
    identity: Vec<String>,
    control: mpsc::UnboundedSender<Control>,
    commands: Option<SessionCommandSender>,
    forget: bool,
}
#[derive(Default)]
struct Catalog {
    profiles: Vec<SessionProfile>,
    proxies: Vec<ProxyProfile>,
}
struct Inner {
    runtime: Handle,
    secrets: RwLock<SecretStore>,
    known_hosts: KnownHostsStore,
    catalog: RwLock<Catalog>,
    entries: Mutex<HashMap<String, Entry>>,
    snapshots: watch::Sender<RdpSnapshot>,
    policy: RdpTunnelPolicy,
}
#[derive(Clone)]
pub struct RdpService {
    inner: Arc<Inner>,
}

impl RdpService {
    pub fn new(runtime: Handle, secrets: SecretStore, known_hosts: KnownHostsStore) -> Self {
        let (snapshots, _) = watch::channel(RdpSnapshot::default());
        Self {
            inner: Arc::new(Inner {
                runtime,
                secrets: RwLock::new(secrets),
                known_hosts,
                catalog: RwLock::new(Catalog::default()),
                entries: Mutex::new(HashMap::new()),
                snapshots,
                policy: RdpTunnelPolicy::default(),
            }),
        }
    }
    pub fn replace_secrets(&self, secrets: SecretStore) {
        if let Ok(mut current) = self.inner.secrets.write() {
            *current = secrets;
        }
    }
    pub fn replace_catalogs(&self, profiles: Vec<SessionProfile>, proxies: Vec<ProxyProfile>) {
        let next = Catalog { profiles, proxies };
        // Same lock order as `start` (catalog, then entries): a launch either registers
        // against the old catalog before this check, or sees the new catalog.
        let Ok(mut catalog) = self.inner.catalog.write() else {
            return;
        };
        if let Ok(entries) = self.inner.entries.lock() {
            for entry in entries.values() {
                if entry.snapshot.mode == RdpConnectionMode::SshTunnel
                    && entry.snapshot.state.is_active()
                    && route_identity(&entry.snapshot.profile_id, &next).as_ref()
                        != Some(&entry.identity)
                {
                    let _ = entry.control.send(Control::Stop);
                }
            }
        }
        *catalog = next;
    }
    pub fn snapshot(&self) -> RdpSnapshot {
        self.inner.snapshots.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<RdpSnapshot> {
        self.inner.snapshots.subscribe()
    }
    pub fn launch(
        &self,
        profile_id: &str,
        options: RdpLaunchOptions,
    ) -> Result<RdpSessionSnapshot> {
        self.start(uuid::Uuid::new_v4().to_string(), profile_id, options)
    }
    pub fn restart(&self, id: &str, client: RdpClientSettings) -> Result<RdpSessionSnapshot> {
        let (profile_id, mut options) = {
            let entries = self
                .inner
                .entries
                .lock()
                .map_err(|_| anyhow!("RDP state unavailable"))?;
            let entry = entries.get(id).context("RDP session not found")?;
            if entry.snapshot.state.is_active() {
                bail!("RDP session is already active");
            }
            (entry.snapshot.profile_id.clone(), entry.options.clone())
        };
        options.client = client;
        self.start(id.to_string(), &profile_id, options)
    }
    fn start(
        &self,
        id: String,
        profile_id: &str,
        options: RdpLaunchOptions,
    ) -> Result<RdpSessionSnapshot> {
        let catalog = self
            .inner
            .catalog
            .read()
            .map_err(|_| anyhow!("RDP catalog unavailable"))?;
        let profile = catalog
            .profiles
            .iter()
            .find(|p| p.id == profile_id && p.kind == ProfileKind::Ssh)
            .cloned()
            .context("RDP requires an existing SSH profile")?;
        RdpEndpoint::new(&profile.host, options.port)?;
        let identity = if options.mode == RdpConnectionMode::SshTunnel {
            route_identity(profile_id, &catalog).context("SSH route is unavailable")?
        } else {
            Vec::new()
        };
        let profiles = catalog.profiles.clone();
        let proxies = catalog.proxies.clone();
        let mut snapshot = RdpSessionSnapshot {
            id: id.clone(),
            profile_id: profile_id.to_string(),
            profile_name: profile.connection_label(),
            mode: options.mode,
            state: RdpRuntimeState::Starting,
            local_port: None,
            local_host: String::new(),
            remote_port: options.port,
            active_connections: 0,
            keep_tunnel: false,
            release_in: None,
            reopen_required: false,
            prompt: None,
            log: Vec::new(),
            revision: 1,
        };
        let (control, receiver) = mpsc::unbounded_channel();
        let generation;
        let requested_port;
        let listen_host;
        {
            let mut entries = self
                .inner
                .entries
                .lock()
                .map_err(|_| anyhow!("RDP state unavailable"))?;
            if entries
                .get(&id)
                .is_some_and(|e| e.snapshot.state.is_active())
            {
                bail!("RDP session is already active");
            }
            generation = entries.get(&id).map_or(1, |entry| entry.generation + 1);
            let last_bound_port = entries.get(&id).and_then(|entry| entry.last_bound_port);
            requested_port = last_bound_port.unwrap_or_else(|| preferred_port(profile_id));
            listen_host = if options.mode == RdpConnectionMode::SshTunnel {
                tunnel_listen_host(profile_id, &id, &entries)
            } else {
                String::new()
            };
            snapshot.local_host = listen_host.clone();
            snapshot.revision = entries
                .get(&id)
                .map_or(1, |entry| entry.snapshot.revision + 1);
            entries.insert(
                id.clone(),
                Entry {
                    snapshot: snapshot.clone(),
                    generation,
                    last_bound_port,
                    listen_host: listen_host.clone(),
                    identity,
                    options: options.clone(),
                    control,
                    commands: None,
                    forget: false,
                },
            );
            self.publish(&entries);
        }
        drop(catalog);
        let service = self.clone();
        self.inner.runtime.spawn(async move {
            // Run in its own task so a panic is observed as a failure instead of leaving
            // the entry "active" forever (which would block restart and stop).
            let worker = {
                let service = service.clone();
                let id = id.clone();
                service.inner.runtime.clone().spawn(async move {
                    service
                        .run(
                            &id,
                            generation,
                            requested_port,
                            listen_host,
                            profile,
                            profiles,
                            proxies,
                            options,
                            receiver,
                        )
                        .await
                })
            };
            let result = match worker.await {
                Ok(result) => result,
                Err(error) => Err(anyhow!("RDP session task ended unexpectedly: {error}")),
            };
            service.update(&id, generation, |entry| {
                entry.commands = None;
                entry.snapshot.prompt = None;
                entry.snapshot.active_connections = 0;
                entry.snapshot.release_in = None;
                entry.snapshot.local_port = None;
                match result {
                    Err(error) => {
                        entry.snapshot.state = RdpRuntimeState::Failed(error.to_string());
                        append_log(&mut entry.snapshot.log, error.to_string());
                    }
                    Ok(()) if !matches!(entry.snapshot.state, RdpRuntimeState::Failed(_)) => {
                        entry.snapshot.state = RdpRuntimeState::Stopped
                    }
                    _ => {}
                }
            });
            if let Ok(mut entries) = service.inner.entries.lock() {
                if entries.get(&id).is_some_and(|entry| {
                    entry.generation == generation
                        && (entry.forget
                            || entry.snapshot.mode == RdpConnectionMode::Direct
                                && !matches!(entry.snapshot.state, RdpRuntimeState::Failed(_)))
                }) {
                    entries.remove(&id);
                    service.publish(&entries);
                }
            }
        });
        Ok(snapshot)
    }
    pub fn relaunch_client(&self, id: &str) -> Result<()> {
        self.control(id, Control::Reopen)
    }
    pub fn set_keep_tunnel(&self, id: &str, keep: bool) -> Result<()> {
        self.control(id, Control::Hold(keep))
    }
    pub fn stop(&self, id: &str) -> bool {
        self.control(id, Control::Stop).is_ok()
    }
    pub fn stop_all(&self) {
        if let Ok(entries) = self.inner.entries.lock() {
            for entry in entries.values() {
                let _ = entry.control.send(Control::Stop);
            }
        }
    }
    pub fn forget(&self, id: &str) {
        if let Ok(mut entries) = self.inner.entries.lock() {
            if let Some(entry) = entries.get_mut(id) {
                entry.forget = true;
                let _ = entry.control.send(Control::Stop);
            }
            if entries
                .get(id)
                .is_some_and(|e| !e.snapshot.state.is_active())
            {
                entries.remove(id);
            }
            self.publish(&entries);
        }
    }
    fn control(&self, id: &str, control: Control) -> Result<()> {
        let entries = self
            .inner
            .entries
            .lock()
            .map_err(|_| anyhow!("RDP state unavailable"))?;
        let entry = entries.get(id).context("RDP session not found")?;
        if !entry.snapshot.state.is_active() {
            bail!("RDP session has stopped");
        }
        entry
            .control
            .send(control)
            .map_err(|_| anyhow!("RDP session has stopped"))
    }
    pub fn respond_host_key(
        &self,
        id: &str,
        request_id: u64,
        decision: HostKeyDecision,
    ) -> Result<()> {
        self.respond(
            id,
            request_id,
            |prompt| matches!(prompt, PortForwardPrompt::HostKey(_)),
            |commands| commands.respond_host_key(request_id, decision),
        )
    }
    pub fn respond_keyboard_interactive(
        &self,
        id: &str,
        request_id: u64,
        responses: Vec<String>,
    ) -> Result<()> {
        self.respond(
            id,
            request_id,
            |prompt| matches!(prompt, PortForwardPrompt::KeyboardInteractive(_)),
            |commands| commands.respond_keyboard_interactive(request_id, responses),
        )
    }
    fn respond(
        &self,
        id: &str,
        request_id: u64,
        kind: impl FnOnce(&PortForwardPrompt) -> bool,
        answer: impl FnOnce(&SessionCommandSender) -> Result<()>,
    ) -> Result<()> {
        let mut entries = self
            .inner
            .entries
            .lock()
            .map_err(|_| anyhow!("RDP state unavailable"))?;
        let entry = entries.get_mut(id).context("RDP session not found")?;
        let prompt = entry
            .snapshot
            .prompt
            .as_ref()
            .context("authentication request expired")?;
        let current_id = match prompt {
            PortForwardPrompt::HostKey(p) => p.request_id,
            PortForwardPrompt::KeyboardInteractive(p) => p.request_id,
        };
        if current_id != request_id || !kind(prompt) {
            bail!("authentication request expired");
        }
        answer(
            entry
                .commands
                .as_ref()
                .context("SSH connection unavailable")?,
        )?;
        entry.snapshot.prompt = None;
        entry.snapshot.revision += 1;
        self.publish(&entries);
        Ok(())
    }
    fn published_release_in(&self, id: &str, generation: u64) -> Option<Option<u64>> {
        let entries = self.inner.entries.lock().ok()?;
        entries
            .get(id)
            .filter(|entry| entry.generation == generation)
            .map(|entry| entry.snapshot.release_in)
    }
    fn update(&self, id: &str, generation: u64, update: impl FnOnce(&mut Entry)) {
        if let Ok(mut entries) = self.inner.entries.lock() {
            if let Some(entry) = entries
                .get_mut(id)
                .filter(|entry| entry.generation == generation)
            {
                update(entry);
                entry.snapshot.revision += 1;
                self.publish(&entries);
            }
        }
    }
    fn publish(&self, entries: &HashMap<String, Entry>) {
        let mut sessions = entries
            .values()
            .map(|e| e.snapshot.clone())
            .collect::<Vec<_>>();
        sessions.sort_by(|a, b| a.id.cmp(&b.id));
        self.inner.snapshots.send_replace(RdpSnapshot { sessions });
    }
    async fn open_client(
        &self,
        profile: &SessionProfile,
        options: &RdpLaunchOptions,
        endpoint: RdpEndpoint,
    ) -> Result<LaunchedClient> {
        let settings = options.client.clone();
        let client = self
            .inner
            .runtime
            .spawn_blocking(move || miaominal_rdp::resolve_client(&settings))
            .await??;
        let username = profile.username.clone();
        let reuse = options.reuse_password
            && client.supports_password()
            && profile.effective_auth_method() == AuthMethod::Password;
        let entered = (reuse && !profile.password.is_empty())
            .then(|| zeroize::Zeroizing::new(profile.password.clone()));
        let stored = reuse && entered.is_none() && profile.has_stored_password;
        let profile_id = profile.id.clone();
        let secrets = self
            .inner
            .secrets
            .read()
            .map_err(|_| anyhow!("credential store unavailable"))?
            .clone();
        self.inner
            .runtime
            .spawn_blocking(move || {
                // Keyring access may block; it stays off the async workers.
                let password = match entered {
                    Some(password) => Some(password),
                    None if stored => match secrets.get(&profile_id, SecretKind::Password) {
                        Ok(password) => password.map(zeroize::Zeroizing::new),
                        Err(error) if SecretStore::is_locked_error(&error) => None,
                        Err(error) => return Err(error),
                    },
                    None => None,
                };
                // The borrowed password lives only until launch returns.
                miaominal_rdp::launch(
                    &client,
                    &endpoint,
                    &username,
                    password.as_ref().map(|p| p.as_str()),
                )
            })
            .await
            .context("RDP launcher task failed")?
    }
    #[allow(clippy::too_many_arguments)]
    async fn run(
        &self,
        id: &str,
        generation: u64,
        requested_port: u16,
        listen_host: String,
        profile: SessionProfile,
        profiles: Vec<SessionProfile>,
        proxies: Vec<ProxyProfile>,
        options: RdpLaunchOptions,
        mut controls: mpsc::UnboundedReceiver<Control>,
    ) -> Result<()> {
        // Detect before authenticating so missing clients do not leave unnecessary tunnels.
        let client_settings = options.client.clone();
        self.inner
            .runtime
            .spawn_blocking(move || miaominal_rdp::resolve_client(&client_settings))
            .await??;
        let mut connection = if options.mode == RdpConnectionMode::SshTunnel {
            let mut tunnel_profile = profile.clone();
            tunnel_profile.port_forwarding_rules = vec![PortForwardRule {
                id: id.to_string(),
                label: "Remote desktop".into(),
                kind: PortForwardKind::Local,
                listen_host: listen_host.clone(),
                listen_port: requested_port,
                target_host: "127.0.0.1".into(),
                target_port: options.port,
                enabled: true,
            }];
            let secrets = self
                .inner
                .secrets
                .read()
                .map_err(|_| anyhow!("credential store unavailable"))?
                .clone();
            let connection = miaominal_ssh::start_port_forward_session_with_policy(
                &self.inner.runtime,
                tunnel_profile,
                profiles,
                proxies,
                secrets,
                self.inner.known_hosts.clone(),
                LocalForwardBindingPolicy::PreferAvailable,
            );
            self.update(id, generation, |entry| {
                entry.commands = Some(connection.commands.clone())
            });
            Some(connection)
        } else {
            None
        };
        // Cancellation on every return, including failed process launch or worker cancellation.
        let _close = CloseConnection(connection.as_ref().map(|c| c.commands.clone()));
        let result = async {
            let mut clients = Vec::new();
            let mut activity = Activity::default();
            let mut endpoint = None;
            if options.mode == RdpConnectionMode::Direct {
                let direct = RdpEndpoint::new(&profile.host, options.port)?;
                let client = self.open_client(&profile, &options, direct).await?;
                let message = launch_message(&options, client.password_delivery());
                clients.push(client);
                self.update(id, generation, |entry| {
                    entry.snapshot.state = RdpRuntimeState::ClientRunning;
                    append_log(&mut entry.snapshot.log, message.into());
                });
            }
            let mut ticker = tokio::time::interval(Duration::from_secs(2));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let direct_started = Instant::now();
            loop {
                tokio::select! {
                biased;
                command = controls.recv() => match command {
                    Some(Control::Stop) | None => break Ok(()),
                    Some(Control::Hold(hold)) => {
                        activity.hold(hold, Instant::now());
                        self.update(id, generation, |entry| {
                            entry.snapshot.keep_tunnel = hold;
                            entry.snapshot.release_in = None;
                        });
                    }
                    Some(Control::Reopen) => {
                        if activity.reconnecting { continue; }
                        if let Some(endpoint) = endpoint.clone() {
                            match self.open_client(&profile, &options, endpoint).await {
                                Ok(client) => {
                                    let state = client.initial_process_state();
                                    let message = launch_message(&options, client.password_delivery());
                                    clients.push(client);
                                    activity.reopened(Instant::now());
                                    activity.process(state, Instant::now());
                                    self.update(id, generation, |entry| {
                                        entry.snapshot.reopen_required = false;
                                        entry.snapshot.release_in = None;
                                        append_log(&mut entry.snapshot.log, message.into());
                                    });
                                }
                                Err(error) => self.update(id, generation, |entry| {
                                    append_log(&mut entry.snapshot.log, error.to_string());
                                }),
                            }
                        }
                    }
                },
                event = async {
                    match &mut connection {
                        Some(connection) => connection.events.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    let Some(event) = event else { break Ok(()); };
                    match event {
                        SessionEvent::PortForwardActive(binding) if binding.rule_id == id => {
                            let previous = endpoint.as_ref().map(RdpEndpoint::port);
                            let next = RdpEndpoint::new(&binding.listen_host, binding.bound_port)?;
                            activity.ready(binding.binding_id, Instant::now());
                            if clients.is_empty() {
                                let client = self.open_client(&profile, &options, next.clone()).await?;
                                let message = launch_message(&options, client.password_delivery());
                                clients.push(client);
                                self.update(id, generation, |entry| {
                                    append_log(&mut entry.snapshot.log, message.into());
                                });
                            }
                            endpoint = Some(next);
                            self.update(id, generation, |entry| {
                                entry.last_bound_port = Some(binding.bound_port);
                                entry.snapshot.local_port = Some(binding.bound_port);
                                entry.snapshot.active_connections = 0;
                                entry.snapshot.state = RdpRuntimeState::WaitingForClient;
                                entry.snapshot.prompt = None;
                                entry.snapshot.reopen_required = previous.is_some_and(|port| port != binding.bound_port);
                            });
                        }
                        SessionEvent::PortForwardConnections { rule_id, binding_id, sequence, active } if rule_id == id => {
                            if activity.connections(binding_id, sequence, active, Instant::now()) {
                                self.update(id, generation, |entry| {
                                    entry.snapshot.active_connections = active;
                                    if active > 0 {
                                        entry.snapshot.release_in = None;
                                    }
                                    entry.snapshot.state = if active > 0 {
                                        RdpRuntimeState::ClientRunning
                                    } else {
                                        RdpRuntimeState::WaitingForClient
                                    };
                                });
                            }
                        }
                        SessionEvent::PortForwardReconnecting { error, .. } => {
                            activity.reconnecting = true;
                            activity.active = 0;
                            self.update(id, generation, |entry| {
                                entry.snapshot.state = RdpRuntimeState::Reconnecting;
                                entry.snapshot.prompt = None;
                                entry.snapshot.local_port = None;
                                entry.snapshot.active_connections = 0;
                                entry.snapshot.release_in = None;
                                append_log(&mut entry.snapshot.log, error);
                            });
                        }
                        SessionEvent::HostKeyPrompt(request) => self.update(id, generation, |entry| {
                            entry.snapshot.prompt = Some(PortForwardPrompt::HostKey(request));
                        }),
                        SessionEvent::KeyboardInteractivePrompt(request) => self.update(id, generation, |entry| {
                            entry.snapshot.prompt = Some(PortForwardPrompt::KeyboardInteractive(request));
                        }),
                        SessionEvent::RequestExpired { request_id } => self.update(id, generation, |entry| {
                            if entry.snapshot.prompt.as_ref().is_some_and(|prompt| match prompt {
                                PortForwardPrompt::HostKey(prompt) => prompt.request_id == request_id,
                                PortForwardPrompt::KeyboardInteractive(prompt) => prompt.request_id == request_id,
                            }) {
                                entry.snapshot.prompt = None;
                            }
                        }),
                        SessionEvent::Status(message) | SessionEvent::PortForwardNotice(message) => {
                            self.update(id, generation, |entry| append_log(&mut entry.snapshot.log, message));
                        }
                        SessionEvent::PortForwardFailed(failure) => {
                            self.update(id, generation, |entry| append_log(&mut entry.snapshot.log, failure.message));
                        }
                        SessionEvent::Error(error) => break Err(anyhow!(error)),
                        SessionEvent::Closed => break Ok(()),
                        _ => {}
                    }
                },
                _ = ticker.tick() => {
                    let mut running = false;
                    let mut untracked = false;
                    for client in &mut clients {
                        // Process tracking is auxiliary. Losing a process handle must not
                        // terminate a tunnel that still carries live TCP connections.
                        match client.poll().unwrap_or(ClientProcessState::Untracked) {
                            ClientProcessState::Running => running = true,
                            ClientProcessState::Untracked => untracked = true,
                            ClientProcessState::Exited => {}
                        }
                    }
                    let state = if running {
                        ClientProcessState::Running
                    } else if untracked {
                        ClientProcessState::Untracked
                    } else {
                        ClientProcessState::Exited
                    };
                    activity.process(state, Instant::now());
                    if options.mode == RdpConnectionMode::Direct {
                        if (!running && !untracked && direct_started.elapsed() >= Duration::from_secs(30))
                            || direct_started.elapsed() >= Duration::from_secs(600)
                        {
                            break Ok(());
                        }
                    } else {
                        let deadline = activity.deadline(&self.inner.policy);
                        if deadline.is_some_and(|deadline| deadline <= Instant::now()) {
                            self.update(id, generation, |entry| append_log(&mut entry.snapshot.log, "Idle tunnel released".into()));
                            break Ok(());
                        }
                        let release_in = deadline.map(|deadline| {
                            deadline.saturating_duration_since(Instant::now()).as_secs().saturating_add(1)
                        });
                        // Only publish when the visible countdown changes; idle ticks are silent.
                        if self.published_release_in(id, generation) != Some(release_in) {
                            self.update(id, generation, |entry| {
                                entry.snapshot.release_in = release_in;
                            });
                        }
                    }
                }
            }
            }
        }
        .await;
        if let Some(mut connection) = connection {
            let _ = connection.commands.close();
            drop(connection.events);
            if let Some(thread) = connection.thread.take() {
                let _ = self
                    .inner
                    .runtime
                    .spawn_blocking(move || thread.join())
                    .await;
            }
        }
        result
    }
}

struct CloseConnection(Option<SessionCommandSender>);
impl Drop for CloseConnection {
    fn drop(&mut self) {
        if let Some(commands) = &self.0 {
            let _ = commands.close();
        }
    }
}
fn append_log(log: &mut Vec<String>, message: String) {
    if log.len() >= 200 {
        log.remove(0);
    }
    log.push(message);
}
fn launch_message(options: &RdpLaunchOptions, delivery: PasswordDelivery) -> &'static str {
    match delivery {
        PasswordDelivery::Stdin => "RDP client started; the SSH password was provided",
        PasswordDelivery::WindowsCredential => {
            "RDP client started; a temporary Windows credential was provided (Windows may still ask, e.g. when Credential Guard blocks saved credentials)"
        }
        PasswordDelivery::Prompt if options.reuse_password => {
            "RDP client started; the password will be requested by the client"
        }
        PasswordDelivery::Prompt => "RDP client started",
    }
}
fn fnv1a(value: &str) -> u32 {
    // Fixed FNV-1a, independent of Rust's randomized/default hash implementation.
    value.bytes().fold(2_166_136_261_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
    })
}
fn preferred_port(profile_id: &str) -> u16 {
    33890 + (fnv1a(profile_id) % 1000) as u16
}
/// Number of distinct 127.77.x.y addresses (x, y in 1..=254).
const LOOPBACK_SLOTS: u32 = 254 * 254;
fn loopback_slot(slot: u32) -> String {
    format!("127.77.{}.{}", slot / 254 + 1, slot % 254 + 1)
}
/// Loopback address for a tunnel listener.
///
/// On Windows every tunnel gets its own 127.77.x.y address (all of 127/8 is loopback
/// there). mstsc may look up saved credentials by host name without the port, so a
/// per-session host keeps temporary credentials of concurrent tunnels apart. The
/// address is derived from the profile, keeping the client's certificate trust stable.
/// Other platforms only guarantee 127.0.0.1 and do not use credential leases.
fn tunnel_listen_host(profile_id: &str, id: &str, entries: &HashMap<String, Entry>) -> String {
    if !cfg!(windows) {
        return "127.0.0.1".into();
    }
    let in_use = entries
        .iter()
        .filter(|(key, entry)| key.as_str() != id && entry.snapshot.state.is_active())
        .map(|(_, entry)| entry.listen_host.as_str())
        .collect::<std::collections::HashSet<_>>();
    let start = fnv1a(profile_id) % LOOPBACK_SLOTS;
    (0..LOOPBACK_SLOTS)
        .map(|offset| loopback_slot((start + offset) % LOOPBACK_SLOTS))
        .find(|host| !in_use.contains(host.as_str()))
        .unwrap_or_else(|| "127.0.0.1".into())
}
fn route_identity(profile_id: &str, catalog: &Catalog) -> Option<Vec<String>> {
    let profile = catalog
        .profiles
        .iter()
        .find(|p| p.id == profile_id && p.kind == ProfileKind::Ssh)?;
    let mut route =
        miaominal_ssh::connection::resolve_proxy_jump_profiles(profile, &catalog.profiles).ok()?;
    route.push(profile.clone());
    let mut identity = Vec::new();
    for profile in route {
        identity.push(format!(
            "{:?}",
            (
                &profile.id,
                profile.kind,
                &profile.host,
                profile.port,
                &profile.username,
                profile.auth_method,
                &profile.private_key_path,
                &profile.managed_key_id,
                &profile.certificate_path,
                &profile.agent_identity,
                profile.agent_forwarding
            )
        ));
        if let Some(id) = profile.entry_proxy_id {
            let proxy = catalog.proxies.iter().find(|proxy| proxy.id == id)?;
            identity.push(format!(
                "{:?}",
                (
                    &proxy.id,
                    proxy.protocol,
                    &proxy.host,
                    proxy.port,
                    proxy.auth_mode,
                    &proxy.username,
                    proxy.resolve_dns_through_proxy
                )
            ));
        }
    }
    Some(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn reopen_preserves_connections_and_resets_only_idle_grace() {
        let policy = RdpTunnelPolicy::default();
        let now = Instant::now();
        let mut activity = Activity::default();
        activity.ready(42, now);
        activity.connections(42, 1, 1, now);
        tokio::time::advance(Duration::from_secs(600)).await;
        activity.reopened(Instant::now());
        assert_eq!(activity.active, 1);
        assert_eq!(activity.binding_id, 42);
        assert_eq!(activity.sequence, 1);
        assert!(activity.deadline(&policy).is_none());
        activity.connections(42, 2, 0, Instant::now());
        assert_eq!(
            activity.deadline(&policy),
            Some(Instant::now() + Duration::from_secs(600))
        );
        activity.reconnecting = true;
        tokio::time::advance(Duration::from_secs(1000)).await;
        assert!(activity.deadline(&policy).is_none());
        activity.ready(43, Instant::now());
        assert_eq!(
            activity.deadline(&policy),
            Some(Instant::now() + Duration::from_secs(600))
        );
    }
    #[tokio::test(start_paused = true)]
    async fn initial_wait_is_bounded_and_each_hold_release_grants_fresh_time() {
        let policy = RdpTunnelPolicy::default();
        for (state, seconds) in [
            (ClientProcessState::Untracked, 120),
            (ClientProcessState::Running, 600),
            (ClientProcessState::Exited, 15),
        ] {
            let now = Instant::now();
            let mut activity = Activity::default();
            activity.ready(1, now);
            activity.process(state, now);
            tokio::time::advance(Duration::from_secs(seconds - 1)).await;
            assert!(activity.deadline(&policy).unwrap() > Instant::now());
            tokio::time::advance(Duration::from_secs(1)).await;
            assert_eq!(activity.deadline(&policy), Some(Instant::now()));
            activity.hold(true, Instant::now());
            tokio::time::advance(Duration::from_secs(900)).await;
            assert!(activity.deadline(&policy).is_none());
            activity.hold(false, Instant::now());
            assert_eq!(
                activity.deadline(&policy),
                Some(Instant::now() + Duration::from_secs(seconds))
            );
        }
    }
    #[tokio::test]
    async fn authentication_answers_are_single_use_and_bound_to_request_and_generation() {
        let directory = tempfile::tempdir().unwrap();
        let service = RdpService::new(
            Handle::current(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(directory.path().join("known_hosts")),
        );
        let mut channels = miaominal_ssh::session_channels();
        let profile = SessionProfile::blank("profile", 1);
        let catalog = Catalog {
            profiles: vec![profile.clone()],
            proxies: vec![],
        };
        let snapshot = RdpSessionSnapshot {
            id: "session".into(),
            profile_id: profile.id.clone(),
            profile_name: "Profile".into(),
            mode: RdpConnectionMode::SshTunnel,
            state: RdpRuntimeState::Starting,
            local_port: None,
            local_host: "127.0.0.1".into(),
            remote_port: RdpLaunchOptions::DEFAULT_PORT,
            active_connections: 0,
            keep_tunnel: false,
            release_in: None,
            reopen_required: false,
            prompt: Some(PortForwardPrompt::HostKey(miaominal_ssh::HostKeyRequest {
                request_id: 99,
                hop: miaominal_ssh::SessionHop::new("host", 1, 1),
                prompt: miaominal_ssh::HostKeyPrompt {
                    host: "test.invalid".into(),
                    port: 22,
                    algorithm: "ed25519".into(),
                    fingerprint: "test".into(),
                    previous_fingerprint: None,
                },
            })),
            log: vec![],
            revision: 1,
        };
        let (control, mut controls) = mpsc::unbounded_channel();
        service.inner.entries.lock().unwrap().insert(
            "session".into(),
            Entry {
                snapshot,
                generation: 2,
                last_bound_port: None,
                listen_host: "127.0.0.1".into(),
                options: RdpLaunchOptions::new(
                    RdpConnectionMode::SshTunnel,
                    RdpClientSettings::default(),
                ),
                identity: route_identity(&profile.id, &catalog).unwrap(),
                control,
                commands: Some(channels.connection.commands.clone()),
                forget: false,
            },
        );
        assert!(
            service
                .respond_host_key("session", 98, HostKeyDecision::AcceptOnce)
                .is_err()
        );
        assert!(channels.commands.try_recv().is_err());
        service
            .respond_host_key("session", 99, HostKeyDecision::AcceptOnce)
            .unwrap();
        assert!(matches!(
            channels.commands.try_recv().unwrap(),
            miaominal_ssh::SessionCommand::HostKeyDecision { request_id: 99, .. }
        ));
        assert!(
            service
                .respond_host_key("session", 99, HostKeyDecision::AcceptOnce)
                .is_err()
        );
        service.update("session", 1, |entry| {
            entry.snapshot.state = RdpRuntimeState::Stopped
        });
        assert!(
            service
                .snapshot()
                .session("session")
                .unwrap()
                .state()
                .is_active()
        );
        service.replace_secrets(SecretStore::new_locked_vault());
        assert!(controls.try_recv().is_err());
        service.replace_catalogs(vec![profile.clone()], vec![]);
        assert!(controls.try_recv().is_err());
        let mut renamed = profile;
        renamed.name = "Renamed".into();
        renamed.tags.push("new".into());
        service.replace_catalogs(vec![renamed.clone()], vec![]);
        assert!(controls.try_recv().is_err());
        renamed.host = "different.invalid".into();
        service.replace_catalogs(vec![renamed], vec![]);
        assert!(matches!(controls.try_recv().unwrap(), Control::Stop));
    }
    #[tokio::test(start_paused = true)]
    async fn idle_deadlines_cover_initial_running_untracked_and_exited() {
        let policy = RdpTunnelPolicy::default();
        let now = Instant::now();
        let mut a = Activity::default();
        assert!(a.deadline(&policy).is_none());
        a.ready(1, now);
        assert_eq!(a.deadline(&policy), Some(now + Duration::from_secs(120)));
        a.process(ClientProcessState::Running, now);
        assert_eq!(a.deadline(&policy), Some(now + Duration::from_secs(600)));
        assert!(a.connections(1, 1, 1, now));
        assert!(a.deadline(&policy).is_none());
        tokio::time::advance(Duration::from_secs(20)).await;
        let idle = Instant::now();
        assert!(a.connections(1, 2, 0, idle));
        assert_eq!(a.deadline(&policy), Some(idle + Duration::from_secs(600)));
        a.process(ClientProcessState::Untracked, idle);
        assert_eq!(a.deadline(&policy), Some(idle + Duration::from_secs(60)));
        a.process(ClientProcessState::Exited, idle);
        assert_eq!(a.deadline(&policy), Some(idle + Duration::from_secs(15)));
        tokio::time::advance(Duration::from_secs(14)).await;
        assert!(a.deadline(&policy).unwrap() > Instant::now());
        assert!(a.connections(1, 3, 1, Instant::now()));
        assert!(a.deadline(&policy).is_none());
    }
    #[tokio::test(start_paused = true)]
    async fn reconnect_hold_and_coalesced_counts_are_safe() {
        let policy = RdpTunnelPolicy::default();
        let now = Instant::now();
        let mut a = Activity::default();
        a.ready(1, now);
        a.connections(1, 2, 0, now); // One connection started and ended before the notification was consumed.
        assert!(a.seen);
        assert_eq!(a.deadline(&policy), Some(now + Duration::from_secs(60)));
        a.hold(true, now);
        tokio::time::advance(Duration::from_secs(900)).await;
        assert!(a.deadline(&policy).is_none());
        a.hold(false, Instant::now());
        assert_eq!(
            a.deadline(&policy),
            Some(Instant::now() + Duration::from_secs(60))
        );
        a.reconnecting = true;
        assert!(a.deadline(&policy).is_none());
        assert!(!a.connections(1, 3, 1, Instant::now()));
        a.ready(2, Instant::now());
        assert!(!a.connections(1, 99, 2, Instant::now()));
        assert!(!a.connections(2, 0, 1, Instant::now()));
        assert!(a.connections(2, 1, 1, Instant::now()));
        assert!(!a.connections(2, 1, 0, Instant::now()));
    }
    #[test]
    fn route_identity_ignores_presentation_but_tracks_connection_changes() {
        let mut profile = SessionProfile::blank("host", 1);
        profile.host = "host".into();
        let mut catalog = Catalog {
            profiles: vec![profile],
            proxies: vec![],
        };
        let identity = route_identity("host", &catalog);
        catalog.profiles[0].name = "Renamed".into();
        catalog.profiles[0].tags.push("tag".into());
        assert_eq!(identity, route_identity("host", &catalog));
        catalog.profiles[0].port = 2222;
        assert_ne!(identity, route_identity("host", &catalog));
        catalog.profiles.clear();
        assert!(route_identity("host", &catalog).is_none());
    }
    fn test_entry(id: &str, listen_host: &str, state: RdpRuntimeState) -> Entry {
        let (control, _) = mpsc::unbounded_channel();
        Entry {
            snapshot: RdpSessionSnapshot {
                id: id.into(),
                profile_id: "profile".into(),
                profile_name: "Profile".into(),
                mode: RdpConnectionMode::SshTunnel,
                state,
                local_port: None,
                local_host: listen_host.into(),
                remote_port: RdpLaunchOptions::DEFAULT_PORT,
                active_connections: 0,
                keep_tunnel: false,
                release_in: None,
                reopen_required: false,
                prompt: None,
                log: vec![],
                revision: 1,
            },
            generation: 1,
            last_bound_port: None,
            listen_host: listen_host.into(),
            options: RdpLaunchOptions::new(
                RdpConnectionMode::SshTunnel,
                RdpClientSettings::default(),
            ),
            identity: vec![],
            control,
            commands: None,
            forget: false,
        }
    }
    #[test]
    fn tunnel_hosts_are_stable_per_profile_and_unique_among_active_sessions() {
        let mut entries = HashMap::new();
        let first = tunnel_listen_host("profile", "a", &entries);
        assert_eq!(first, tunnel_listen_host("profile", "a", &entries));
        if !cfg!(windows) {
            assert_eq!(first, "127.0.0.1");
            return;
        }
        assert!(first.starts_with("127.77."));
        entries.insert(
            "a".into(),
            test_entry("a", &first, RdpRuntimeState::WaitingForClient),
        );
        // The same session keeps its address on restart.
        assert_eq!(first, tunnel_listen_host("profile", "a", &entries));
        // A concurrent session of the same profile must not share the credential host.
        let second = tunnel_listen_host("profile", "b", &entries);
        assert_ne!(first, second);
        assert!(second.starts_with("127.77."));
        // Stopped sessions release their address.
        entries.insert(
            "a".into(),
            test_entry("a", &first, RdpRuntimeState::Stopped),
        );
        assert_eq!(first, tunnel_listen_host("profile", "b", &entries));
        for slot in [0, 253, 254, LOOPBACK_SLOTS - 1] {
            let host = loopback_slot(slot);
            assert!(host.parse::<std::net::Ipv4Addr>().unwrap().is_loopback());
            assert!(!host.ends_with(".0") && !host.ends_with(".255"));
        }
    }
    #[test]
    fn launch_port_defaults_to_3389_and_ignores_zero() {
        let options = RdpLaunchOptions::new(RdpConnectionMode::Direct, Default::default());
        assert_eq!(options.port(), 3389);
        assert_eq!(options.clone().with_port(13389).port(), 13389);
        assert_eq!(options.with_port(0).port(), 3389);
    }
    #[test]
    fn launch_messages_never_claim_an_unconfirmed_autofill() {
        let reuse = RdpLaunchOptions::new(RdpConnectionMode::SshTunnel, Default::default())
            .with_reuse_ssh_password(true);
        assert!(launch_message(&reuse, PasswordDelivery::Prompt).contains("requested"));
        assert!(launch_message(&reuse, PasswordDelivery::WindowsCredential).contains("may"));
        let ask = RdpLaunchOptions::new(RdpConnectionMode::Direct, Default::default());
        assert_eq!(
            launch_message(&ask, PasswordDelivery::Prompt),
            "RDP client started"
        );
    }
}
