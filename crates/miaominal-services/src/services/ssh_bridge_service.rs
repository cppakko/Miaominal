use anyhow::{Context, Result, anyhow, bail};
use miaominal_core::profile::{ProfileKind, SessionProfile};
use miaominal_core::proxy::ProxyProfile;
use miaominal_core::ssh_bridge_security::{
    BRIDGE_SYSTEM_AUTH_TIMEOUT_SECS, BridgeAuditRecord, BridgeAuthorizationDecision,
    BridgeAuthorizationOutcome, BridgeConnectionOutcome, BridgeDecisionSource,
    BridgePendingAuthorization, BridgePendingPhase, BridgeSecurityLevel, BridgeSecurityPolicy,
    BridgeSecuritySnapshot,
};
use miaominal_secrets::SecretStore;
use miaominal_settings::SshBridgeConfig;
use miaominal_ssh::{
    BridgeCredentialReadiness, ConnectedSshRoute, ProfileConnector, SshBridgeConnection,
    SshBridgeEndpoint, SshBridgeListener, SshBridgeRoute, SshBridgeRouteTable,
    SshBridgeServerIdentity, SshBridgeStatus, accept_route_request_with,
    is_bridge_vault_locked_error, run_ssh_bridge_server_with_shutdown,
};
use miaominal_storage::{BridgeAuditLog, BridgeSecuritySettingsStore, KnownHostsStore};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock, Weak};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::{Mutex, Semaphore, mpsc, watch};
use tokio::task::{JoinHandle, JoinSet};

const ACCEPT_ERROR_RETRY_INITIAL_DELAY: std::time::Duration = std::time::Duration::from_millis(100);
const ACCEPT_ERROR_RETRY_MAX_DELAY: std::time::Duration = std::time::Duration::from_secs(5);
const POLICY_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);
const MAX_PENDING_AUTHORIZATIONS: usize = 8;
const MAX_PENDING_AUTHORIZATIONS_PER_PROFILE: usize = 4;
const VAULT_UNLOCK_TIMEOUT_SECS: u64 = 60;

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
    owner_instance_id: String,
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
    security_store: Option<BridgeSecuritySettingsStore>,
    audit_log: Option<BridgeAuditLog>,
    audit_log_error: Option<String>,
    policy: RwLock<BridgeSecurityPolicy>,
    policy_consistency: StdMutex<()>,
    pending: StdMutex<HashMap<String, PendingAuthorizationEntry>>,
    credentials_generation: watch::Sender<u64>,
    audit_health_error: StdMutex<Option<String>>,
    policy_store_error: Option<String>,
    system_auth_available: AtomicBool,
    security: watch::Sender<BridgeSecuritySnapshot>,
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

struct PendingAuthorizationEntry {
    request: BridgePendingAuthorization,
    events: mpsc::UnboundedSender<PendingAuthorizationResolution>,
    decision_submitted: bool,
}

#[derive(Clone, Copy)]
enum PendingAuthorizationResolution {
    Decision(BridgeAuthorizationDecision),
    PolicyChanged,
    ProfileUnavailable,
    ServiceStopping,
    UserCancelled,
}

struct PendingAuthorizationGuard {
    inner: Weak<Inner>,
    request_id: String,
}

struct PendingAuthorizationWaiter {
    receiver: mpsc::UnboundedReceiver<PendingAuthorizationResolution>,
    _guard: PendingAuthorizationGuard,
}

impl Drop for PendingAuthorizationGuard {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            inner.remove_pending(&self.request_id);
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum AuthorizationFailure {
    Rejected,
    TimedOut,
    Unsupported,
    SystemAuthCancelled,
    Failed,
    QueueFull,
    PolicyChanged,
    PolicyStoreUnavailable,
    ProfileUnavailable,
    ServiceStopping,
}

impl AuthorizationFailure {
    fn outcome(self) -> BridgeAuthorizationOutcome {
        match self {
            Self::Rejected => BridgeAuthorizationOutcome::Rejected,
            Self::TimedOut => BridgeAuthorizationOutcome::TimedOut,
            Self::Unsupported => BridgeAuthorizationOutcome::Unsupported,
            Self::SystemAuthCancelled => BridgeAuthorizationOutcome::Cancelled,
            Self::Failed | Self::QueueFull | Self::PolicyStoreUnavailable => {
                BridgeAuthorizationOutcome::Failed
            }
            Self::PolicyChanged | Self::ProfileUnavailable | Self::ServiceStopping => {
                BridgeAuthorizationOutcome::Cancelled
            }
        }
    }

    fn error_code(self) -> &'static str {
        match self {
            Self::Rejected => "authorization_rejected",
            Self::TimedOut => "authorization_timed_out",
            Self::Unsupported => "system_auth_unsupported",
            Self::SystemAuthCancelled => "system_auth_cancelled",
            Self::Failed => "authorization_failed",
            Self::QueueFull => "authorization_queue_full",
            Self::PolicyChanged => "policy_changed",
            Self::PolicyStoreUnavailable => "policy_store_unavailable",
            Self::ProfileUnavailable => "profile_unavailable",
            Self::ServiceStopping => "service_stopping",
        }
    }
}

#[derive(Debug)]
enum BridgePreparationFailure {
    Authorization(AuthorizationFailure),
    VaultUnlockTimedOut,
    VaultUnlockCancelled,
    CredentialsUnavailable(anyhow::Error),
    Upstream(anyhow::Error),
}

impl BridgePreparationFailure {
    fn error_code(&self) -> &'static str {
        match self {
            Self::Authorization(failure) => failure.error_code(),
            Self::VaultUnlockTimedOut => "vault_unlock_timeout",
            Self::VaultUnlockCancelled => "vault_unlock_cancelled",
            Self::CredentialsUnavailable(_) => "credentials_unavailable",
            Self::Upstream(_) => "upstream_failed",
        }
    }

    fn into_error(self) -> anyhow::Error {
        match self {
            Self::Authorization(failure) => {
                anyhow!("SSH Bridge request failed: {}", failure.error_code())
            }
            Self::VaultUnlockTimedOut => anyhow!("SSH Bridge vault unlock timed out"),
            Self::VaultUnlockCancelled => anyhow!("SSH Bridge vault unlock was cancelled"),
            Self::CredentialsUnavailable(error) | Self::Upstream(error) => error,
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.cancel_all_pending(PendingAuthorizationResolution::ServiceStopping);
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
        let security_store =
            BridgeSecuritySettingsStore::open_default().map_err(|error| format!("{error:#}"));
        let audit_log = BridgeAuditLog::open_default().map_err(|error| format!("{error:#}"));
        Self::new_with_stores(
            runtime,
            endpoint,
            instance_id,
            known_hosts_path,
            config,
            secrets,
            known_hosts,
            security_store,
            audit_log,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_stores(
        runtime: TokioHandle,
        endpoint: SshBridgeEndpoint,
        instance_id: String,
        known_hosts_path: PathBuf,
        config: SshBridgeConfig,
        secrets: SecretStore,
        known_hosts: KnownHostsStore,
        security_store: std::result::Result<BridgeSecuritySettingsStore, String>,
        audit_log: std::result::Result<BridgeAuditLog, String>,
    ) -> Self {
        let (security_store, policy, policy_store_error) = match security_store {
            Ok(store) => match store.policy() {
                Ok(policy) => (Some(store), policy, None),
                Err(error) => (
                    Some(store),
                    BridgeSecurityPolicy::default(),
                    Some(format!("failed to load SSH Bridge policy: {error:#}")),
                ),
            },
            Err(error) => (None, BridgeSecurityPolicy::default(), Some(error)),
        };
        let (audit_log, audit_log_error) = match audit_log {
            Ok(log) => (Some(log), None),
            Err(error) => (None, Some(error)),
        };
        let (status, _) = watch::channel(SshBridgeStatus::Disabled);
        let (credentials_generation, _) = watch::channel(0_u64);
        let (security, _) = watch::channel(BridgeSecuritySnapshot {
            policy: policy.clone(),
            policy_store_error: policy_store_error.clone(),
            audit_health_error: audit_log_error.clone(),
            ..BridgeSecuritySnapshot::default()
        });
        let inner = Arc::new(Inner {
            runtime,
            endpoint,
            instance_id,
            owner_instance_id: uuid::Uuid::new_v4().to_string(),
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
            security_store,
            audit_log,
            audit_log_error,
            policy: RwLock::new(policy),
            policy_consistency: StdMutex::new(()),
            pending: StdMutex::new(HashMap::new()),
            credentials_generation,
            audit_health_error: StdMutex::new(None),
            policy_store_error,
            system_auth_available: AtomicBool::new(false),
            security,
        });
        if let Some(error) = &inner.audit_log_error {
            inner.set_audit_health_error(error);
        }
        Self { inner }
    }

    pub fn endpoint(&self) -> &SshBridgeEndpoint {
        &self.inner.endpoint
    }

    pub fn owner_instance_id(&self) -> &str {
        &self.inner.owner_instance_id
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

    pub fn security_snapshot(&self) -> BridgeSecuritySnapshot {
        self.inner.security.borrow().clone()
    }

    pub fn subscribe_security(&self) -> watch::Receiver<BridgeSecuritySnapshot> {
        self.inner.security.subscribe()
    }

    pub fn set_system_auth_available(&self, available: bool) {
        self.inner
            .system_auth_available
            .store(available, Ordering::Release);
        self.inner.publish_security();
    }

    pub fn security_policy(&self) -> BridgeSecurityPolicy {
        self.inner
            .policy
            .read()
            .map(|policy| policy.clone())
            .unwrap_or_default()
    }

    pub fn set_security_policy(&self, level: BridgeSecurityLevel) -> Result<BridgeSecurityPolicy> {
        let level = level.validate().map_err(anyhow::Error::msg)?;
        let _consistency = self
            .inner
            .policy_consistency
            .lock()
            .map_err(|_| anyhow!("SSH Bridge policy consistency lock is poisoned"))?;
        let store = self.inner.security_store.as_ref().ok_or_else(|| {
            anyhow!(
                self.inner
                    .policy_store_error
                    .clone()
                    .unwrap_or_else(|| { "SSH Bridge security store is unavailable".into() })
            )
        })?;
        let policy = store.set_policy(level, unix_timestamp_secs())?;
        self.inner.apply_policy_locked(policy.clone())?;
        Ok(policy)
    }

    pub fn decide_authorization(
        &self,
        request_id: &str,
        decision: BridgeAuthorizationDecision,
    ) -> Result<()> {
        self.inner.resolve_pending(request_id, decision)
    }

    pub fn cancel_pending_request(&self, request_id: &str) -> Result<()> {
        self.inner.cancel_pending_request(request_id)
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
            .collect::<HashMap<_, _>>();
        let available_profile_ids = profiles_by_id.keys().cloned().collect::<HashSet<_>>();
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
        self.inner
            .cancel_pending_for_missing_profiles(&available_profile_ids);
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
            self.inner.credentials_generation.send_modify(|generation| {
                *generation = generation.wrapping_add(1);
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
        if let Some(error) = &self.inner.policy_store_error {
            let error = anyhow!("SSH Bridge security policy store is unavailable: {error}");
            self.inner.status.send_replace(SshBridgeStatus::Error {
                endpoint: Some(self.inner.endpoint.clone()),
                message: error.to_string(),
            });
            return Err(error);
        }
        self.inner.status.send_replace(SshBridgeStatus::Starting);
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
        self.inner
            .cancel_all_pending(PendingAuthorizationResolution::ServiceStopping);
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

#[derive(Clone, Copy)]
enum BridgeAuditPhase {
    Requested,
    Finished,
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

    fn refresh_policy_from_store(&self) -> Result<BridgeSecurityPolicy> {
        let _consistency = self
            .policy_consistency
            .lock()
            .map_err(|_| anyhow!("SSH Bridge policy consistency lock is poisoned"))?;
        self.refresh_policy_from_store_locked()
    }

    fn refresh_policy_from_store_locked(&self) -> Result<BridgeSecurityPolicy> {
        let policy = self
            .security_store
            .as_ref()
            .ok_or_else(|| anyhow!("SSH Bridge security store is unavailable"))?
            .policy()
            .context("failed to refresh SSH Bridge security policy")?;
        self.apply_policy_locked(policy.clone())?;
        Ok(policy)
    }

    fn apply_policy_locked(&self, policy: BridgeSecurityPolicy) -> Result<()> {
        let current = self
            .policy
            .read()
            .map_err(|_| anyhow!("SSH Bridge policy lock is poisoned"))?
            .clone();
        if policy.generation < current.generation {
            bail!("SSH Bridge policy generation moved backwards");
        }
        if policy.generation == current.generation && policy != current {
            bail!("SSH Bridge policy changed without advancing its generation");
        }
        if policy == current {
            return Ok(());
        }
        *self
            .policy
            .write()
            .map_err(|_| anyhow!("SSH Bridge policy lock is poisoned"))? = policy.clone();
        self.cancel_pending_with_stale_generation(policy.generation);
        self.publish_security();
        Ok(())
    }

    fn ensure_policy_generation(
        &self,
        expected_generation: u64,
    ) -> std::result::Result<(), AuthorizationFailure> {
        let policy = self
            .refresh_policy_from_store()
            .map_err(|_| AuthorizationFailure::PolicyStoreUnavailable)?;
        if policy.generation != expected_generation {
            return Err(AuthorizationFailure::PolicyChanged);
        }
        Ok(())
    }

    fn set_audit_health_error(&self, error: impl std::fmt::Display) {
        if let Ok(mut health) = self.audit_health_error.lock() {
            *health = Some(error.to_string());
        }
        self.publish_security();
    }

    fn clear_audit_health_error(&self) {
        let cleared = self
            .audit_health_error
            .lock()
            .map(|mut error| error.take().is_some())
            .unwrap_or(false);
        if cleared {
            self.publish_security();
        }
    }

    fn publish_security(&self) {
        let mut pending = self
            .pending
            .lock()
            .map(|pending| {
                pending
                    .values()
                    .map(|entry| entry.request.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        pending.sort_by_key(|request| request.created_at);
        let audit_health_error = self
            .audit_health_error
            .lock()
            .ok()
            .and_then(|error| error.clone());
        self.security.send_replace(BridgeSecuritySnapshot {
            policy: self
                .policy
                .read()
                .map(|policy| policy.clone())
                .unwrap_or_default(),
            pending,
            audit_health_error,
            policy_store_error: self.policy_store_error.clone(),
            system_auth_available: self.system_auth_available.load(Ordering::Acquire),
        });
    }

    fn resolve_pending(
        &self,
        request_id: &str,
        decision: BridgeAuthorizationDecision,
    ) -> Result<()> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| anyhow!("SSH Bridge pending authorization lock is poisoned"))?;
        let request = pending
            .get_mut(request_id)
            .ok_or_else(|| anyhow!("SSH Bridge authorization request is no longer pending"))?;
        if request.decision_submitted {
            bail!("SSH Bridge authorization request has already been decided");
        }
        let valid = match request.request.phase {
            BridgePendingPhase::AwaitingApproval => matches!(
                decision,
                BridgeAuthorizationDecision::Approve | BridgeAuthorizationDecision::Reject
            ),
            BridgePendingPhase::AwaitingSystemAuth => matches!(
                decision,
                BridgeAuthorizationDecision::Reject
                    | BridgeAuthorizationDecision::SystemAuthVerified
                    | BridgeAuthorizationDecision::SystemAuthCancelled
                    | BridgeAuthorizationDecision::SystemAuthUnavailable
                    | BridgeAuthorizationDecision::SystemAuthFailed
            ),
            BridgePendingPhase::AwaitingVaultUnlock => false,
        };
        if !valid {
            bail!("authorization decision does not match the pending SSH Bridge phase");
        }
        request
            .events
            .send(PendingAuthorizationResolution::Decision(decision))
            .map_err(|_| anyhow!("SSH Bridge authorization request has already expired"))?;
        request.decision_submitted = true;
        Ok(())
    }

    fn cancel_pending_request(&self, request_id: &str) -> Result<()> {
        let entry = self
            .pending
            .lock()
            .map_err(|_| anyhow!("SSH Bridge pending request lock is poisoned"))?
            .remove(request_id)
            .ok_or_else(|| anyhow!("SSH Bridge request is no longer pending"))?;
        entry
            .events
            .send(PendingAuthorizationResolution::UserCancelled)
            .map_err(|_| anyhow!("SSH Bridge request has already expired"))?;
        self.publish_security();
        Ok(())
    }

    fn remove_pending(&self, request_id: &str) {
        let removed = self
            .pending
            .lock()
            .map(|mut pending| pending.remove(request_id).is_some())
            .unwrap_or(false);
        if removed {
            self.publish_security();
        }
    }

    fn cancel_pending_for_profile(
        &self,
        profile_id: &str,
        resolution: PendingAuthorizationResolution,
    ) {
        let entries = self
            .pending
            .lock()
            .map(|mut pending| {
                let request_ids = pending
                    .iter()
                    .filter(|(_, entry)| entry.request.profile_id == profile_id)
                    .map(|(request_id, _)| request_id.clone())
                    .collect::<Vec<_>>();
                request_ids
                    .into_iter()
                    .filter_map(|request_id| pending.remove(&request_id))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let changed = !entries.is_empty();
        for entry in entries {
            let entry_resolution = match resolution {
                PendingAuthorizationResolution::PolicyChanged => {
                    PendingAuthorizationResolution::PolicyChanged
                }
                PendingAuthorizationResolution::ProfileUnavailable => {
                    PendingAuthorizationResolution::ProfileUnavailable
                }
                PendingAuthorizationResolution::ServiceStopping => {
                    PendingAuthorizationResolution::ServiceStopping
                }
                PendingAuthorizationResolution::Decision(_)
                | PendingAuthorizationResolution::UserCancelled => continue,
            };
            let _ = entry.events.send(entry_resolution);
        }
        if changed {
            self.publish_security();
        }
    }

    fn cancel_pending_for_missing_profiles(&self, available_profile_ids: &HashSet<String>) {
        let profile_ids = self
            .pending
            .lock()
            .map(|pending| {
                pending
                    .values()
                    .filter(|entry| !available_profile_ids.contains(&entry.request.profile_id))
                    .map(|entry| entry.request.profile_id.clone())
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        for profile_id in profile_ids {
            self.cancel_pending_for_profile(
                &profile_id,
                PendingAuthorizationResolution::ProfileUnavailable,
            );
        }
    }

    fn cancel_all_pending(&self, resolution: PendingAuthorizationResolution) {
        let entries = self
            .pending
            .lock()
            .map(|mut pending| pending.drain().map(|(_, entry)| entry).collect::<Vec<_>>())
            .unwrap_or_default();
        let changed = !entries.is_empty();
        for entry in entries {
            let entry_resolution = match resolution {
                PendingAuthorizationResolution::PolicyChanged => {
                    PendingAuthorizationResolution::PolicyChanged
                }
                PendingAuthorizationResolution::ProfileUnavailable => {
                    PendingAuthorizationResolution::ProfileUnavailable
                }
                PendingAuthorizationResolution::ServiceStopping => {
                    PendingAuthorizationResolution::ServiceStopping
                }
                PendingAuthorizationResolution::Decision(_)
                | PendingAuthorizationResolution::UserCancelled => continue,
            };
            let _ = entry.events.send(entry_resolution);
        }
        if changed {
            self.publish_security();
        }
    }

    fn cancel_pending_with_stale_generation(&self, current_generation: u64) {
        let entries = self
            .pending
            .lock()
            .map(|mut pending| {
                let request_ids = pending
                    .iter()
                    .filter(|(_, entry)| entry.request.policy_generation != current_generation)
                    .map(|(request_id, _)| request_id.clone())
                    .collect::<Vec<_>>();
                request_ids
                    .into_iter()
                    .filter_map(|request_id| pending.remove(&request_id))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let changed = !entries.is_empty();
        for entry in entries {
            let _ = entry
                .events
                .send(PendingAuthorizationResolution::PolicyChanged);
        }
        if changed {
            self.publish_security();
        }
    }

    fn insert_pending(
        self: &Arc<Self>,
        request: BridgePendingAuthorization,
    ) -> std::result::Result<PendingAuthorizationWaiter, AuthorizationFailure> {
        let _consistency = self
            .policy_consistency
            .lock()
            .map_err(|_| AuthorizationFailure::Failed)?;
        let policy = self
            .refresh_policy_from_store_locked()
            .map_err(|_| AuthorizationFailure::PolicyStoreUnavailable)?;
        if policy.generation != request.policy_generation {
            return Err(AuthorizationFailure::PolicyChanged);
        }
        let (events, receiver) = mpsc::unbounded_channel();
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| AuthorizationFailure::Failed)?;
        let profile_pending = pending
            .values()
            .filter(|entry| entry.request.profile_id == request.profile_id)
            .count();
        if pending.len() >= MAX_PENDING_AUTHORIZATIONS
            || profile_pending >= MAX_PENDING_AUTHORIZATIONS_PER_PROFILE
        {
            return Err(AuthorizationFailure::QueueFull);
        }
        let request_id = request.request_id.clone();
        pending.insert(
            request_id.clone(),
            PendingAuthorizationEntry {
                request,
                events,
                decision_submitted: false,
            },
        );
        drop(pending);
        self.publish_security();
        Ok(PendingAuthorizationWaiter {
            receiver,
            _guard: PendingAuthorizationGuard {
                inner: Arc::downgrade(self),
                request_id,
            },
        })
    }

    fn transition_pending_to_vault_unlock(
        &self,
        request_id: &str,
        expires_at: i64,
    ) -> std::result::Result<(), AuthorizationFailure> {
        let now = unix_timestamp_secs();
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| AuthorizationFailure::Failed)?;
        let entry = pending
            .get_mut(request_id)
            .ok_or(AuthorizationFailure::Failed)?;
        entry.request.phase = BridgePendingPhase::AwaitingVaultUnlock;
        entry.request.phase_started_at = now;
        entry.request.expires_at = expires_at;
        drop(pending);
        self.publish_security();
        Ok(())
    }

    async fn authorize(
        self: &Arc<Self>,
        request_id: &str,
        route: &SshBridgeRoute,
        policy: BridgeSecurityPolicy,
        peer: &miaominal_core::ssh_bridge_security::BridgePeerIdentity,
    ) -> std::result::Result<
        (
            Option<BridgeDecisionSource>,
            Option<PendingAuthorizationWaiter>,
        ),
        AuthorizationFailure,
    > {
        self.ensure_policy_generation(policy.generation)?;
        let level = policy.level;
        let timeout_secs = match level {
            BridgeSecurityLevel::Standard => {
                self.ensure_policy_generation(policy.generation)?;
                return Ok((None, None));
            }
            BridgeSecurityLevel::RequireApproval { timeout_secs } => timeout_secs,
            BridgeSecurityLevel::RequireSystemAuth => {
                if !self.system_auth_available.load(Ordering::Acquire) {
                    return Err(AuthorizationFailure::Unsupported);
                }
                BRIDGE_SYSTEM_AUTH_TIMEOUT_SECS
            }
        };
        let now = unix_timestamp_secs();
        let request = BridgePendingAuthorization {
            request_id: request_id.to_string(),
            profile_id: route.profile_id.clone(),
            profile_name: route.profile_name.clone(),
            level,
            phase: match level {
                BridgeSecurityLevel::RequireApproval { .. } => BridgePendingPhase::AwaitingApproval,
                BridgeSecurityLevel::RequireSystemAuth => BridgePendingPhase::AwaitingSystemAuth,
                BridgeSecurityLevel::Standard => unreachable!(),
            },
            policy_generation: policy.generation,
            peer: peer.clone(),
            created_at: now,
            phase_started_at: now,
            expires_at: now.saturating_add(i64::from(timeout_secs)),
        };
        let mut waiter = self.insert_pending(request)?;
        self.ensure_policy_generation(policy.generation)?;
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_secs(u64::from(timeout_secs));
        let mut policy_refresh = tokio::time::interval(POLICY_REFRESH_INTERVAL);
        policy_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        policy_refresh.tick().await;
        let resolution = loop {
            tokio::select! {
                resolution = waiter.receiver.recv() => {
                    break resolution.ok_or(AuthorizationFailure::Failed)?;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    return Err(AuthorizationFailure::TimedOut);
                }
                _ = policy_refresh.tick() => {
                    self.ensure_policy_generation(policy.generation)?;
                }
            }
        };
        self.ensure_policy_generation(policy.generation)?;
        let decision = match resolution {
            PendingAuthorizationResolution::Decision(decision) => decision,
            PendingAuthorizationResolution::PolicyChanged => {
                return Err(AuthorizationFailure::PolicyChanged);
            }
            PendingAuthorizationResolution::ProfileUnavailable => {
                return Err(AuthorizationFailure::ProfileUnavailable);
            }
            PendingAuthorizationResolution::ServiceStopping => {
                return Err(AuthorizationFailure::ServiceStopping);
            }
            PendingAuthorizationResolution::UserCancelled => {
                return Err(AuthorizationFailure::Rejected);
            }
        };
        let source = match (level, decision) {
            (BridgeSecurityLevel::RequireApproval { .. }, BridgeAuthorizationDecision::Approve) => {
                Some(BridgeDecisionSource::App)
            }
            (_, BridgeAuthorizationDecision::Reject) => return Err(AuthorizationFailure::Rejected),
            (
                BridgeSecurityLevel::RequireSystemAuth,
                BridgeAuthorizationDecision::SystemAuthVerified,
            ) => Some(BridgeDecisionSource::SystemAuth),
            (
                BridgeSecurityLevel::RequireSystemAuth,
                BridgeAuthorizationDecision::SystemAuthCancelled,
            ) => return Err(AuthorizationFailure::SystemAuthCancelled),
            (
                BridgeSecurityLevel::RequireSystemAuth,
                BridgeAuthorizationDecision::SystemAuthUnavailable,
            ) => return Err(AuthorizationFailure::Unsupported),
            (
                BridgeSecurityLevel::RequireSystemAuth,
                BridgeAuthorizationDecision::SystemAuthFailed,
            ) => return Err(AuthorizationFailure::Failed),
            _ => return Err(AuthorizationFailure::Failed),
        };
        Ok((source, Some(waiter)))
    }

    async fn connect_after_authorization(
        self: &Arc<Self>,
        request_id: &str,
        requested_at: i64,
        route: &SshBridgeRoute,
        policy: &BridgeSecurityPolicy,
        peer: &miaominal_core::ssh_bridge_security::BridgePeerIdentity,
        mut waiter: Option<PendingAuthorizationWaiter>,
    ) -> std::result::Result<ConnectedSshRoute, BridgePreparationFailure> {
        let mut credentials_generation = self.credentials_generation.subscribe();
        let mut waiting_for_vault = false;
        let mut vault_deadline = None;
        let mut policy_refresh = tokio::time::interval(POLICY_REFRESH_INTERVAL);
        policy_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        policy_refresh.tick().await;

        loop {
            self.ensure_policy_generation(policy.generation)
                .map_err(BridgePreparationFailure::Authorization)?;
            let snapshot = self
                .snapshot
                .read()
                .map_err(|_| {
                    BridgePreparationFailure::CredentialsUnavailable(anyhow!(
                        "SSH Bridge snapshot lock is poisoned"
                    ))
                })?
                .clone();
            let profile = snapshot
                .profiles_by_id
                .get(&route.profile_id)
                .cloned()
                .ok_or(BridgePreparationFailure::Authorization(
                    AuthorizationFailure::ProfileUnavailable,
                ))?;
            let connector = ProfileConnector::new(
                snapshot.profiles.clone(),
                snapshot.proxies.clone(),
                snapshot.secrets.clone(),
                self.known_hosts.clone(),
            );
            let readiness = connector
                .bridge_credentials_readiness(&profile)
                .map_err(BridgePreparationFailure::CredentialsUnavailable)?;

            if readiness == BridgeCredentialReadiness::Ready {
                self.ensure_policy_generation(policy.generation)
                    .map_err(BridgePreparationFailure::Authorization)?;
                drop(waiter.take());
                waiting_for_vault = false;
                match connector.connect_bridge(profile).await {
                    Ok(connection) => return Ok(connection),
                    Err(error) if is_bridge_vault_locked_error(&error) => {}
                    Err(error) => return Err(BridgePreparationFailure::Upstream(error)),
                }
            }

            if !waiting_for_vault {
                let now = unix_timestamp_secs();
                let expires_at = now.saturating_add(VAULT_UNLOCK_TIMEOUT_SECS as i64);
                if waiter.is_some() {
                    self.transition_pending_to_vault_unlock(request_id, expires_at)
                        .map_err(BridgePreparationFailure::Authorization)?;
                } else {
                    let request = BridgePendingAuthorization {
                        request_id: request_id.to_string(),
                        profile_id: route.profile_id.clone(),
                        profile_name: route.profile_name.clone(),
                        level: policy.level,
                        phase: BridgePendingPhase::AwaitingVaultUnlock,
                        policy_generation: policy.generation,
                        peer: peer.clone(),
                        created_at: requested_at,
                        phase_started_at: now,
                        expires_at,
                    };
                    waiter = Some(
                        self.insert_pending(request)
                            .map_err(BridgePreparationFailure::Authorization)?,
                    );
                }
                waiting_for_vault = true;
                vault_deadline = Some(
                    tokio::time::Instant::now()
                        + std::time::Duration::from_secs(VAULT_UNLOCK_TIMEOUT_SECS),
                );

                // Close the race where credentials were replaced after the initial check but
                // before the pending request became visible.
                if credentials_generation.has_changed().unwrap_or(false) {
                    let _ = credentials_generation.borrow_and_update();
                    continue;
                }
            }

            let deadline = vault_deadline.expect("vault wait always has a deadline");
            let pending = waiter
                .as_mut()
                .expect("vault wait always has a pending entry");
            tokio::select! {
                event = pending.receiver.recv() => {
                    match event {
                        Some(PendingAuthorizationResolution::PolicyChanged) => {
                            return Err(BridgePreparationFailure::Authorization(
                                AuthorizationFailure::PolicyChanged,
                            ));
                        }
                        Some(PendingAuthorizationResolution::ProfileUnavailable) => {
                            return Err(BridgePreparationFailure::Authorization(
                                AuthorizationFailure::ProfileUnavailable,
                            ));
                        }
                        Some(PendingAuthorizationResolution::ServiceStopping) => {
                            return Err(BridgePreparationFailure::Authorization(
                                AuthorizationFailure::ServiceStopping,
                            ));
                        }
                        Some(PendingAuthorizationResolution::UserCancelled) => {
                            return Err(BridgePreparationFailure::VaultUnlockCancelled);
                        }
                        Some(PendingAuthorizationResolution::Decision(_)) => {
                            return Err(BridgePreparationFailure::Authorization(
                                AuthorizationFailure::Failed,
                            ));
                        }
                        None => return Err(BridgePreparationFailure::VaultUnlockCancelled),
                    }
                }
                changed = credentials_generation.changed() => {
                    if changed.is_err() {
                        return Err(BridgePreparationFailure::CredentialsUnavailable(anyhow!(
                            "SSH Bridge credential updates are unavailable"
                        )));
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    return Err(BridgePreparationFailure::VaultUnlockTimedOut);
                }
                _ = policy_refresh.tick() => {
                    self.ensure_policy_generation(policy.generation)
                        .map_err(BridgePreparationFailure::Authorization)?;
                }
            }
        }
    }

    fn append_audit(&self, audit: &Arc<StdMutex<BridgeAuditRecord>>, phase: BridgeAuditPhase) {
        let Some(log) = &self.audit_log else {
            return;
        };
        let record = audit.lock().ok().map(|record| record.clone());
        let Some(record) = record else {
            return;
        };
        let result = match phase {
            BridgeAuditPhase::Requested => log.write_requested(&record),
            BridgeAuditPhase::Finished => log.write_finished(&record),
        };
        match result {
            Ok(()) => self.clear_audit_health_error(),
            Err(error) => self.set_audit_health_error(format!("{error:#}")),
        }
    }

    fn update_audit(
        &self,
        audit: &Arc<StdMutex<BridgeAuditRecord>>,
        update: impl FnOnce(&mut BridgeAuditRecord),
    ) {
        if let Ok(mut record) = audit.lock() {
            update(&mut record);
        }
    }

    fn audit_rejected_connection(&self, connection: &SshBridgeConnection, error_code: &str) {
        let Some(log) = &self.audit_log else {
            return;
        };
        let now = unix_timestamp_secs();
        let record = BridgeAuditRecord {
            request_id: uuid::Uuid::new_v4().to_string(),
            owner_instance_id: self.owner_instance_id.clone(),
            requested_at: now,
            decision_at: Some(now),
            connected_at: None,
            finished_at: Some(now),
            profile_id: None,
            profile_name: None,
            security_level: BridgeSecurityLevel::Standard,
            peer: connection.peer.clone(),
            authorization_outcome: BridgeAuthorizationOutcome::Failed,
            connection_outcome: BridgeConnectionOutcome::Rejected,
            decision_source: None,
            error_code: Some(error_code.into()),
        };
        match log.write_finished(&record) {
            Ok(()) => self.clear_audit_health_error(),
            Err(error) => self.set_audit_health_error(format!("{error:#}")),
        }
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
                    let connection = match accepted {
                        Ok(connection) => connection,
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
                            self.audit_rejected_connection(&connection, "connection_limit_reached");
                            drop(connection);
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
                        let result = inner.handle_client(connection, identity, client_cancel).await;
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
        self: &Arc<Self>,
        mut connection: SshBridgeConnection,
        identity: Arc<SshBridgeServerIdentity>,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<()> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let requested_at = unix_timestamp_secs();
        let audit = Arc::new(StdMutex::new(BridgeAuditRecord {
            request_id: request_id.clone(),
            owner_instance_id: self.owner_instance_id.clone(),
            requested_at,
            decision_at: None,
            connected_at: None,
            finished_at: None,
            profile_id: None,
            profile_name: None,
            security_level: BridgeSecurityLevel::Standard,
            peer: connection.peer.clone(),
            authorization_outcome: BridgeAuthorizationOutcome::NotRequired,
            connection_outcome: BridgeConnectionOutcome::Pending,
            decision_source: None,
            error_code: None,
        }));
        self.append_audit(&audit, BridgeAuditPhase::Requested);
        let snapshot = self
            .snapshot
            .read()
            .map_err(|_| anyhow!("SSH Bridge snapshot lock is poisoned"))?
            .clone();
        let routes = snapshot.routes.clone();
        let inner = self.clone();
        let peer = connection.peer.clone();
        let audit_for_prepare = audit.clone();
        let prepared = tokio::select! {
            _ = wait_for_bridge_cancellation(&mut cancel) => {
                Err(anyhow!("SSH Bridge is stopping"))
            }
            route = accept_route_request_with(&mut connection.stream, &routes, move |route| {
                let inner = inner.clone();
                let peer = peer.clone();
                let audit = audit_for_prepare.clone();
                async move {
                    let policy = match inner.refresh_policy_from_store() {
                        Ok(policy) => policy,
                        Err(_) => {
                            inner.update_audit(&audit, |record| {
                                let now = unix_timestamp_secs();
                                record.profile_id = Some(route.profile_id.clone());
                                record.profile_name = Some(route.profile_name.clone());
                                record.authorization_outcome = BridgeAuthorizationOutcome::Failed;
                                record.connection_outcome = BridgeConnectionOutcome::Rejected;
                                record.decision_at = Some(now);
                                record.finished_at = Some(now);
                                record.error_code = Some("policy_store_unavailable".into());
                            });
                            bail!("SSH Bridge security policy is unavailable");
                        }
                    };
                    let level = policy.level;
                    inner.update_audit(&audit, |record| {
                        record.profile_id = Some(route.profile_id.clone());
                        record.profile_name = Some(route.profile_name.clone());
                        record.security_level = level;
                        record.authorization_outcome = if level == BridgeSecurityLevel::Standard {
                            BridgeAuthorizationOutcome::NotRequired
                        } else {
                            BridgeAuthorizationOutcome::Pending
                        };
                    });
                    let (_, waiter) = match inner
                        .authorize(&request_id, &route, policy.clone(), &peer)
                        .await
                    {
                        Ok(result) => {
                            let source = result.0;
                            inner.update_audit(&audit, |record| {
                                record.authorization_outcome = if source.is_some() {
                                    BridgeAuthorizationOutcome::Approved
                                } else {
                                    BridgeAuthorizationOutcome::NotRequired
                                };
                                record.decision_source = source;
                                record.decision_at = source.map(|_| unix_timestamp_secs());
                            });
                            result
                        }
                        Err(failure) => {
                            inner.update_audit(&audit, |record| {
                                record.authorization_outcome = failure.outcome();
                                record.connection_outcome = BridgeConnectionOutcome::Rejected;
                                record.decision_at = Some(unix_timestamp_secs());
                                record.finished_at = record.decision_at;
                                record.error_code = Some(failure.error_code().into());
                            });
                            bail!("SSH Bridge authorization failed: {}", failure.error_code());
                        }
                    };
                    if let Err(failure) = inner.ensure_policy_generation(policy.generation) {
                        inner.update_audit(&audit, |record| {
                            record.authorization_outcome = failure.outcome();
                            record.connection_outcome = BridgeConnectionOutcome::Rejected;
                            record.decision_at = Some(unix_timestamp_secs());
                            record.finished_at = record.decision_at;
                            record.error_code = Some(failure.error_code().into());
                        });
                        bail!("SSH Bridge authorization failed: {}", failure.error_code());
                    }
                    match inner
                        .connect_after_authorization(
                            &request_id,
                            requested_at,
                            &route,
                            &policy,
                            &peer,
                            waiter,
                        )
                        .await
                    {
                        Ok(connection) => Ok(connection),
                        Err(failure) => {
                            let error_code = failure.error_code();
                            inner.update_audit(&audit, |record| {
                                record.connection_outcome = match &failure {
                                    BridgePreparationFailure::Upstream(_) => {
                                        BridgeConnectionOutcome::UpstreamFailed
                                    }
                                    _ => BridgeConnectionOutcome::Rejected,
                                };
                                record.finished_at = Some(unix_timestamp_secs());
                                record.error_code = Some(error_code.into());
                            });
                            Err(failure.into_error())
                        }
                    }
                }
            }) => route,
        };
        let route = match prepared {
            Ok(route) => route,
            Err(error) => {
                let error_code = classify_bridge_error(&error);
                self.update_audit(&audit, |record| {
                    if record.authorization_outcome == BridgeAuthorizationOutcome::Pending {
                        record.authorization_outcome = BridgeAuthorizationOutcome::Cancelled;
                    }
                    if record.connection_outcome == BridgeConnectionOutcome::Pending {
                        record.connection_outcome = BridgeConnectionOutcome::Rejected;
                    }
                    record.finished_at.get_or_insert_with(unix_timestamp_secs);
                    if record.error_code.is_none() {
                        record.error_code = Some(error_code.into());
                    }
                });
                self.append_audit(&audit, BridgeAuditPhase::Finished);
                return Err(error);
            }
        };
        self.update_audit(&audit, |record| {
            record.connected_at = Some(unix_timestamp_secs());
            record.connection_outcome = BridgeConnectionOutcome::Active;
        });
        let result = run_ssh_bridge_server_with_shutdown(
            connection.stream,
            route,
            identity,
            usize::from(self.config.max_channels_per_connection.max(1)),
            async move {
                wait_for_bridge_cancellation(&mut cancel).await;
            },
        )
        .await;
        self.update_audit(&audit, |record| {
            record.finished_at = Some(unix_timestamp_secs());
            record.connection_outcome = BridgeConnectionOutcome::Disconnected;
            if result.is_err() {
                record.error_code = Some("bridge_session_failed".into());
            }
        });
        self.append_audit(&audit, BridgeAuditPhase::Finished);
        result
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

fn unix_timestamp_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

fn classify_bridge_error(error: &anyhow::Error) -> &'static str {
    let message = error.to_string();
    if message.contains("unknown SSH Bridge route token") {
        "unknown_route"
    } else if message.contains("SSH Bridge control frame timed out") {
        "control_frame_timeout"
    } else if message.contains("malformed SSH Bridge")
        || message.contains("unsupported SSH Bridge protocol")
        || message.contains("control frame")
    {
        "protocol_invalid"
    } else if message.contains("disconnected") {
        "client_disconnected"
    } else if message.contains("sent data before") {
        "client_protocol_violation"
    } else if message.contains("stopping") {
        "service_stopping"
    } else {
        "request_failed"
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
    use miaominal_ssh::{
        SshBridgeRouteRequest, connect_endpoint, request_route, write_control_frame,
    };
    use russh::keys::{Algorithm, PrivateKey};
    use russh::{Channel, ChannelId, ChannelMsg, Disconnect, client, server};
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio::sync::watch;
    use tokio::task::{JoinHandle, JoinSet};

    fn test_security_settings_store(
        directory: &std::path::Path,
        name: &str,
    ) -> std::result::Result<BridgeSecuritySettingsStore, String> {
        BridgeSecuritySettingsStore::open(&directory.join(name))
            .map_err(|error| format!("{error:#}"))
    }

    fn test_audit_log(
        directory: &std::path::Path,
        name: &str,
    ) -> std::result::Result<BridgeAuditLog, String> {
        BridgeAuditLog::open(&directory.join(name)).map_err(|error| format!("{error:#}"))
    }

    fn test_service(directory: &std::path::Path) -> SshBridgeService {
        let endpoint = SshBridgeEndpoint::derive(directory).unwrap();
        let instance_id = SshBridgeEndpoint::instance_id(directory).unwrap();
        SshBridgeService::new_with_stores(
            TokioHandle::current(),
            endpoint,
            instance_id,
            directory.join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(directory.join("upstream_known_hosts")),
            test_security_settings_store(directory, "settings.toml"),
            test_audit_log(directory, "audit.log"),
        )
    }

    fn route(profile_id: &str) -> SshBridgeRoute {
        SshBridgeRoute {
            token: format!("{profile_id:0>32}"),
            profile_id: profile_id.into(),
            profile_name: profile_id.into(),
        }
    }

    async fn wait_for_pending(service: &SshBridgeService, expected: usize) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let count = service.security_snapshot().pending.len();
            if count == expected {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "pending request count did not reach {expected}; current count is {count}"
            );
            tokio::task::yield_now().await;
        }
    }

    async fn wait_for_pending_phase(service: &SshBridgeService, phase: BridgePendingPhase) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            if service
                .security_snapshot()
                .pending
                .iter()
                .any(|request| request.phase == phase)
            {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "pending request did not reach phase {phase:?}"
            );
            tokio::task::yield_now().await;
        }
    }

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

    #[tokio::test]
    async fn approval_decision_is_single_use() {
        let directory = tempfile::tempdir().unwrap();
        let service = test_service(directory.path());
        let policy = service
            .set_security_policy(BridgeSecurityLevel::RequireApproval { timeout_secs: 30 })
            .unwrap();
        let inner = service.inner.clone();
        let route = route("approval");
        let task = tokio::spawn(async move {
            inner
                .authorize("request-approval", &route, policy, &Default::default())
                .await
        });
        wait_for_pending(&service, 1).await;
        service
            .decide_authorization("request-approval", BridgeAuthorizationDecision::Approve)
            .unwrap();
        assert!(
            service
                .decide_authorization("request-approval", BridgeAuthorizationDecision::Approve)
                .is_err()
        );
        let (source, waiter) = task.await.unwrap().unwrap();
        assert_eq!(source, Some(BridgeDecisionSource::App));
        drop(waiter);
        wait_for_pending(&service, 0).await;
    }

    #[tokio::test]
    async fn ordinary_approval_cannot_complete_system_auth() {
        let directory = tempfile::tempdir().unwrap();
        let service = test_service(directory.path());
        service.set_system_auth_available(true);
        let policy = service
            .set_security_policy(BridgeSecurityLevel::RequireSystemAuth)
            .unwrap();
        let inner = service.inner.clone();
        let route = route("system-auth");
        let task = tokio::spawn(async move {
            inner
                .authorize("request-system-auth", &route, policy, &Default::default())
                .await
        });
        wait_for_pending(&service, 1).await;
        assert!(
            service
                .decide_authorization("request-system-auth", BridgeAuthorizationDecision::Approve)
                .is_err()
        );
        assert_eq!(service.security_snapshot().pending.len(), 1);
        service
            .decide_authorization(
                "request-system-auth",
                BridgeAuthorizationDecision::SystemAuthVerified,
            )
            .unwrap();
        let (source, waiter) = task.await.unwrap().unwrap();
        assert_eq!(source, Some(BridgeDecisionSource::SystemAuth));
        drop(waiter);
    }

    #[tokio::test]
    async fn system_auth_cancelled_and_unavailable_keep_precise_failure_semantics() {
        let directory = tempfile::tempdir().unwrap();
        let service = test_service(directory.path());
        service.set_system_auth_available(true);
        let policy = service
            .set_security_policy(BridgeSecurityLevel::RequireSystemAuth)
            .unwrap();

        for (request_id, decision, expected) in [
            (
                "request-system-auth-cancelled",
                BridgeAuthorizationDecision::SystemAuthCancelled,
                AuthorizationFailure::SystemAuthCancelled,
            ),
            (
                "request-system-auth-unavailable",
                BridgeAuthorizationDecision::SystemAuthUnavailable,
                AuthorizationFailure::Unsupported,
            ),
        ] {
            let inner = service.inner.clone();
            let route = route("system-auth");
            let task_policy = policy.clone();
            let task = tokio::spawn(async move {
                inner
                    .authorize(request_id, &route, task_policy, &Default::default())
                    .await
            });
            wait_for_pending(&service, 1).await;
            service.decide_authorization(request_id, decision).unwrap();
            let failure = match task.await.unwrap() {
                Err(failure) => failure,
                Ok(_) => panic!("system authentication should fail"),
            };
            assert_eq!(failure.outcome(), expected.outcome());
            assert_eq!(failure.error_code(), expected.error_code());
        }
    }

    #[tokio::test]
    async fn unsupported_system_auth_fails_closed_without_pending_request() {
        let directory = tempfile::tempdir().unwrap();
        let service = test_service(directory.path());
        let policy = service
            .set_security_policy(BridgeSecurityLevel::RequireSystemAuth)
            .unwrap();
        let result = service
            .inner
            .authorize(
                "request-system-auth",
                &route("system-auth"),
                policy,
                &Default::default(),
            )
            .await;
        assert!(matches!(result, Err(AuthorizationFailure::Unsupported)));
        assert!(service.security_snapshot().pending.is_empty());
    }

    #[tokio::test]
    async fn global_policy_and_profile_changes_cancel_pending_authorization() {
        let directory = tempfile::tempdir().unwrap();
        let service = test_service(directory.path());
        let policy = service
            .set_security_policy(BridgeSecurityLevel::RequireApproval { timeout_secs: 30 })
            .unwrap();
        let inner = service.inner.clone();
        let task = tokio::spawn(async move {
            inner
                .authorize(
                    "request-policy",
                    &route("profile-a"),
                    policy,
                    &Default::default(),
                )
                .await
        });
        wait_for_pending(&service, 1).await;
        let policy = service
            .set_security_policy(BridgeSecurityLevel::RequireApproval { timeout_secs: 60 })
            .unwrap();
        assert!(matches!(
            task.await.unwrap(),
            Err(AuthorizationFailure::PolicyChanged)
        ));

        let mut saved = profile("profile-b");
        saved.name = "Profile B".into();
        service.refresh_routes(vec![saved], vec![]);
        let inner = service.inner.clone();
        let task = tokio::spawn(async move {
            inner
                .authorize(
                    "request-profile",
                    &route("profile-b"),
                    policy,
                    &Default::default(),
                )
                .await
        });
        wait_for_pending(&service, 1).await;
        service.refresh_routes(Vec::new(), Vec::new());
        assert!(matches!(
            task.await.unwrap(),
            Err(AuthorizationFailure::ProfileUnavailable)
        ));
    }

    #[tokio::test]
    async fn stale_policy_snapshots_are_rejected_before_standard_bypass_or_pending_insert() {
        let directory = tempfile::tempdir().unwrap();
        let running_service = test_service(directory.path());
        let settings_service = test_service(directory.path());

        settings_service
            .set_security_policy(BridgeSecurityLevel::Standard)
            .unwrap();
        let stale_standard = running_service.inner.refresh_policy_from_store().unwrap();
        assert_eq!(stale_standard.level, BridgeSecurityLevel::Standard);
        settings_service
            .set_security_policy(BridgeSecurityLevel::RequireApproval { timeout_secs: 30 })
            .unwrap();
        assert!(matches!(
            running_service
                .inner
                .authorize(
                    "stale-standard",
                    &route("profile-a"),
                    stale_standard,
                    &Default::default(),
                )
                .await,
            Err(AuthorizationFailure::PolicyChanged)
        ));
        assert!(running_service.security_snapshot().pending.is_empty());

        let stale_approval = running_service.inner.refresh_policy_from_store().unwrap();
        settings_service
            .set_security_policy(BridgeSecurityLevel::RequireSystemAuth)
            .unwrap();
        assert!(matches!(
            running_service
                .inner
                .authorize(
                    "stale-approval",
                    &route("profile-a"),
                    stale_approval,
                    &Default::default(),
                )
                .await,
            Err(AuthorizationFailure::PolicyChanged)
        ));
        assert!(running_service.security_snapshot().pending.is_empty());
    }

    #[tokio::test]
    async fn route_refresh_profile_rename_and_removal_do_not_change_global_policy() {
        let directory = tempfile::tempdir().unwrap();
        let service = test_service(directory.path());
        let level = BridgeSecurityLevel::RequireApproval { timeout_secs: 60 };
        service.set_security_policy(level).unwrap();
        let mut saved = profile("stable-id");
        saved.name = "Before Rename".into();
        service.refresh_routes(vec![saved.clone()], vec![]);
        saved.name = "After Rename".into();
        service.refresh_routes(vec![saved], vec![]);
        service.refresh_routes(Vec::new(), Vec::new());
        assert_eq!(service.security_policy().level, level);
    }

    #[test]
    fn pending_authorization_limits_are_enforced() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let service = test_service(directory.path());
        let policy = service
            .set_security_policy(BridgeSecurityLevel::RequireApproval { timeout_secs: 30 })
            .unwrap();
        let mut held = Vec::new();
        for index in 0..MAX_PENDING_AUTHORIZATIONS {
            let request = BridgePendingAuthorization {
                request_id: format!("request-{index}"),
                profile_id: format!("profile-{index}"),
                profile_name: format!("Profile {index}"),
                level: BridgeSecurityLevel::RequireApproval { timeout_secs: 30 },
                phase: BridgePendingPhase::AwaitingApproval,
                policy_generation: policy.generation,
                peer: Default::default(),
                created_at: 1,
                phase_started_at: 1,
                expires_at: 31,
            };
            held.push(service.inner.insert_pending(request).unwrap());
        }
        let overflow = service.inner.insert_pending(BridgePendingAuthorization {
            request_id: "overflow".into(),
            profile_id: "overflow".into(),
            profile_name: "Overflow".into(),
            level: BridgeSecurityLevel::RequireApproval { timeout_secs: 30 },
            phase: BridgePendingPhase::AwaitingApproval,
            policy_generation: policy.generation,
            peer: Default::default(),
            created_at: 1,
            phase_started_at: 1,
            expires_at: 31,
        });
        assert!(matches!(overflow, Err(AuthorizationFailure::QueueFull)));
        drop(held);

        let mut per_profile = Vec::new();
        for index in 0..MAX_PENDING_AUTHORIZATIONS_PER_PROFILE {
            per_profile.push(
                service
                    .inner
                    .insert_pending(BridgePendingAuthorization {
                        request_id: format!("same-profile-{index}"),
                        profile_id: "same-profile".into(),
                        profile_name: "Same Profile".into(),
                        level: BridgeSecurityLevel::RequireApproval { timeout_secs: 30 },
                        phase: BridgePendingPhase::AwaitingApproval,
                        policy_generation: policy.generation,
                        peer: Default::default(),
                        created_at: 1,
                        phase_started_at: 1,
                        expires_at: 31,
                    })
                    .unwrap(),
            );
        }
        assert!(matches!(
            service.inner.insert_pending(BridgePendingAuthorization {
                request_id: "same-profile-overflow".into(),
                profile_id: "same-profile".into(),
                profile_name: "Same Profile".into(),
                level: BridgeSecurityLevel::RequireApproval { timeout_secs: 30 },
                phase: BridgePendingPhase::AwaitingApproval,
                policy_generation: policy.generation,
                peer: Default::default(),
                created_at: 1,
                phase_started_at: 1,
                expires_at: 31,
            }),
            Err(AuthorizationFailure::QueueFull)
        ));
        drop(per_profile);
    }

    #[tokio::test]
    async fn minimum_approval_timeout_expires_request() {
        let directory = tempfile::tempdir().unwrap();
        let service = test_service(directory.path());
        let policy = service
            .set_security_policy(BridgeSecurityLevel::RequireApproval { timeout_secs: 5 })
            .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(6),
            service.inner.authorize(
                "request-timeout",
                &route("timeout"),
                policy,
                &Default::default(),
            ),
        )
        .await
        .expect("authorization timeout should complete within its deadline");
        assert!(matches!(result, Err(AuthorizationFailure::TimedOut)));
        assert!(service.security_snapshot().pending.is_empty());
    }

    #[tokio::test]
    async fn service_stop_cancels_pending_authorization() {
        let directory = tempfile::tempdir().unwrap();
        let service = test_service(directory.path());
        let policy = service
            .set_security_policy(BridgeSecurityLevel::RequireApproval { timeout_secs: 30 })
            .unwrap();
        let inner = service.inner.clone();
        let task = tokio::spawn(async move {
            inner
                .authorize("request-stop", &route("stop"), policy, &Default::default())
                .await
        });
        wait_for_pending(&service, 1).await;
        service
            .inner
            .cancel_all_pending(PendingAuthorizationResolution::ServiceStopping);
        assert!(matches!(
            task.await.unwrap(),
            Err(AuthorizationFailure::ServiceStopping)
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn service_stop_finishes_audit_before_route_handshake() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = SshBridgeEndpoint::derive(directory.path()).unwrap();
        let instance_id = SshBridgeEndpoint::instance_id(directory.path()).unwrap();
        let service = SshBridgeService::new_with_stores(
            TokioHandle::current(),
            endpoint.clone(),
            instance_id,
            directory.path().join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(directory.path().join("upstream_known_hosts")),
            test_security_settings_store(directory.path(), "settings.toml"),
            test_audit_log(directory.path(), "audit.log"),
        );
        service.enable().await.unwrap();

        let stream = connect_endpoint(&endpoint).await.unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let content = std::fs::read_to_string(directory.path().join("audit.log")).unwrap();
            let requested = content.lines().any(|line| line.contains("| requested |"));
            if requested {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline);
            tokio::task::yield_now().await;
        }

        service.disable().await;
        drop(stream);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let content = std::fs::read_to_string(directory.path().join("audit.log")).unwrap();
            let finished = content.lines().any(|line| {
                line.contains("| finished |") && line.contains("error=service_stopping")
            });
            if finished {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline);
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn unavailable_security_settings_prevent_bridge_start() {
        let directory = tempfile::tempdir().unwrap();
        let endpoint = SshBridgeEndpoint::derive(directory.path()).unwrap();
        let instance_id = SshBridgeEndpoint::instance_id(directory.path()).unwrap();
        let service = SshBridgeService::new_with_stores(
            TokioHandle::current(),
            endpoint,
            instance_id,
            directory.path().join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(directory.path().join("upstream_known_hosts")),
            Err("injected migration failure".into()),
            test_audit_log(directory.path(), "audit.log"),
        );
        assert!(service.enable().await.is_err());
        assert!(matches!(service.status(), SshBridgeStatus::Error { .. }));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn running_bridge_uses_policy_written_by_another_service_instance() {
        let upstream = spawn_counting_upstream().await;
        let directory = tempfile::tempdir().unwrap();
        let known_hosts = KnownHostsStore::with_path(directory.path().join("upstream_known_hosts"));
        known_hosts
            .learn("127.0.0.1", upstream.port, &upstream.public_key)
            .unwrap();
        let endpoint = SshBridgeEndpoint::derive(directory.path()).unwrap();
        let instance_id = SshBridgeEndpoint::instance_id(directory.path()).unwrap();
        let security_path = directory.path().join("settings.toml");
        let running_service = SshBridgeService::new_with_stores(
            TokioHandle::current(),
            endpoint.clone(),
            instance_id.clone(),
            directory.path().join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            known_hosts,
            BridgeSecuritySettingsStore::open(&security_path).map_err(|error| format!("{error:#}")),
            test_audit_log(directory.path(), "audit.log"),
        );
        let settings_service = SshBridgeService::new_with_stores(
            TokioHandle::current(),
            endpoint.clone(),
            instance_id,
            directory.path().join("settings_bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(directory.path().join("settings_upstream_known_hosts")),
            BridgeSecuritySettingsStore::open(&security_path).map_err(|error| format!("{error:#}")),
            test_audit_log(directory.path(), "settings_audit.log"),
        );
        let mut target = profile("cross-instance-policy");
        target.host = "127.0.0.1".into();
        target.port = upstream.port;
        target.username = "bridge-test".into();
        target.password = "secret".into();
        let refresh = running_service.refresh_routes(vec![target], vec![]);
        let route_token = refresh.routes[0].token.clone();
        running_service.enable().await.unwrap();

        let updated_policy = settings_service
            .set_security_policy(BridgeSecurityLevel::RequireApproval { timeout_secs: 30 })
            .unwrap();
        let client = tokio::spawn(async move {
            let mut stream = connect_endpoint(&endpoint).await?;
            request_route(&mut stream, &route_token).await?;
            Result::<_, anyhow::Error>::Ok(stream)
        });
        wait_for_pending(&running_service, 1).await;
        assert_eq!(upstream.accepted.load(Ordering::Acquire), 0);
        let snapshot = running_service.security_snapshot();
        assert_eq!(snapshot.policy, updated_policy);
        assert_eq!(
            snapshot.pending[0].policy_generation,
            updated_policy.generation
        );
        running_service
            .decide_authorization(
                &snapshot.pending[0].request_id,
                BridgeAuthorizationDecision::Approve,
            )
            .unwrap();
        let stream = tokio::time::timeout(Duration::from_secs(5), client)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        wait_for_count(&upstream.accepted, 1).await;
        drop(stream);
        running_service.disable().await;
        upstream.stop().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn upstream_connection_waits_for_approval_and_client_disconnect_wins() {
        let upstream = spawn_counting_upstream().await;
        let directory = tempfile::tempdir().unwrap();
        let known_hosts = KnownHostsStore::with_path(directory.path().join("upstream_known_hosts"));
        known_hosts
            .learn("127.0.0.1", upstream.port, &upstream.public_key)
            .unwrap();
        let endpoint = SshBridgeEndpoint::derive(directory.path()).unwrap();
        let instance_id = SshBridgeEndpoint::instance_id(directory.path()).unwrap();
        let service = SshBridgeService::new_with_stores(
            TokioHandle::current(),
            endpoint.clone(),
            instance_id,
            directory.path().join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            known_hosts,
            test_security_settings_store(directory.path(), "settings.toml"),
            test_audit_log(directory.path(), "audit.log"),
        );
        let mut target = profile("approval-target");
        target.host = "127.0.0.1".into();
        target.port = upstream.port;
        target.username = "bridge-test".into();
        target.password = "secret".into();
        let refresh = service.refresh_routes(vec![target], vec![]);
        let route_token = refresh.routes[0].token.clone();
        service
            .set_security_policy(BridgeSecurityLevel::RequireApproval { timeout_secs: 30 })
            .unwrap();
        service.enable().await.unwrap();

        let first_endpoint = endpoint.clone();
        let first_token = route_token.clone();
        let first_client = tokio::spawn(async move {
            let mut stream = connect_endpoint(&first_endpoint).await?;
            request_route(&mut stream, &first_token).await?;
            Result::<_, anyhow::Error>::Ok(stream)
        });
        wait_for_pending(&service, 1).await;
        assert_eq!(upstream.accepted.load(Ordering::Acquire), 0);
        let first_request_id = service.security_snapshot().pending[0].request_id.clone();
        service
            .decide_authorization(&first_request_id, BridgeAuthorizationDecision::Approve)
            .unwrap();
        let first_stream = tokio::time::timeout(Duration::from_secs(5), first_client)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        wait_for_count(&upstream.accepted, 1).await;
        drop(first_stream);
        wait_for_pending(&service, 0).await;

        let mut abandoned = connect_endpoint(&endpoint).await.unwrap();
        write_control_frame(
            &mut abandoned,
            &SshBridgeRouteRequest::new(route_token.clone()),
        )
        .await
        .unwrap();
        wait_for_pending(&service, 1).await;
        let abandoned_request_id = service.security_snapshot().pending[0].request_id.clone();
        drop(abandoned);
        wait_for_pending(&service, 0).await;
        assert!(
            service
                .decide_authorization(&abandoned_request_id, BridgeAuthorizationDecision::Approve,)
                .is_err()
        );
        assert_eq!(upstream.accepted.load(Ordering::Acquire), 1);

        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let content = std::fs::read_to_string(directory.path().join("audit.log")).unwrap();
            let cancelled = content.lines().any(|line| {
                line.contains(&format!("request={abandoned_request_id}"))
                    && line.contains("authorization=cancelled")
                    && line.contains("connection=rejected")
                    && line.contains("error=client_disconnected")
            });
            if cancelled {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline);
            tokio::task::yield_now().await;
        }

        service.disable().await;
        upstream.stop().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn audit_write_failure_warns_but_does_not_block_standard_connection() {
        let upstream = spawn_counting_upstream().await;
        let directory = tempfile::tempdir().unwrap();
        let known_hosts = KnownHostsStore::with_path(directory.path().join("upstream_known_hosts"));
        known_hosts
            .learn("127.0.0.1", upstream.port, &upstream.public_key)
            .unwrap();
        let endpoint = SshBridgeEndpoint::derive(directory.path()).unwrap();
        let instance_id = SshBridgeEndpoint::instance_id(directory.path()).unwrap();
        let audit_blocked_path = directory.path().join("audit_blocked");
        std::fs::write(&audit_blocked_path, "not a directory").unwrap();
        let service = SshBridgeService::new_with_stores(
            TokioHandle::current(),
            endpoint,
            instance_id,
            directory.path().join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            known_hosts,
            test_security_settings_store(directory.path(), "settings.toml"),
            BridgeAuditLog::open(&audit_blocked_path.join("audit.log"))
                .map_err(|error| format!("{error:#}")),
        );
        service
            .set_security_policy(BridgeSecurityLevel::Standard)
            .unwrap();
        let mut target = profile("standard-target");
        target.host = "127.0.0.1".into();
        target.port = upstream.port;
        target.username = "bridge-test".into();
        target.password = "secret".into();
        let refresh = service.refresh_routes(vec![target], vec![]);
        service.enable().await.unwrap();

        let client = tokio::time::timeout(
            Duration::from_secs(5),
            connect_bridge_client(&service, &refresh.routes[0].token),
        )
        .await
        .expect("audit failure must not block a Standard connection");
        wait_for_count(&upstream.accepted, 1).await;
        assert!(service.security_snapshot().audit_health_error.is_some());
        client
            .disconnect(Disconnect::ByApplication, "", "English")
            .await
            .unwrap();
        service.disable().await;
        upstream.stop().await;
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
        finish_bridge_client(stream).await
    }

    async fn finish_bridge_client(
        stream: miaominal_ssh::SshBridgeStream,
    ) -> client::Handle<AcceptAllClientHandler> {
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
        let service = SshBridgeService::new_with_stores(
            runtime.clone(),
            endpoint.clone(),
            instance_id.clone(),
            directory.path().join("known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(directory.path().join("upstream_known_hosts")),
            test_security_settings_store(directory.path(), "settings.toml"),
            test_audit_log(directory.path(), "audit.log"),
        );
        service.refresh_routes(vec![profile("one")], vec![]);
        service.enable().await.unwrap();
        service.enable().await.unwrap();
        assert!(matches!(service.status(), SshBridgeStatus::Running { .. }));
        let known_hosts_path = directory.path().join("known_hosts");
        let original_host_identity = std::fs::read(&known_hosts_path).unwrap();

        let competing = SshBridgeService::new_with_stores(
            runtime,
            endpoint,
            instance_id,
            known_hosts_path.clone(),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(directory.path().join("other_upstream_known_hosts")),
            test_security_settings_store(directory.path(), "other_settings.toml"),
            test_audit_log(directory.path(), "other_audit.log"),
        );
        assert!(competing.enable().await.is_err());
        assert!(matches!(competing.status(), SshBridgeStatus::Error { .. }));
        assert_eq!(
            std::fs::read(&known_hosts_path).unwrap(),
            original_host_identity,
            "a competing instance must not replace the running bridge host key"
        );

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
        let service = SshBridgeService::new_with_stores(
            TokioHandle::current(),
            endpoint,
            instance_id,
            known_hosts_path.clone(),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(directory.path().join("upstream_known_hosts")),
            test_security_settings_store(directory.path(), "settings.toml"),
            test_audit_log(directory.path(), "audit.log"),
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
        let service = SshBridgeService::new_with_stores(
            TokioHandle::current(),
            endpoint,
            instance_id,
            directory.path().join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(directory.path().join("upstream_known_hosts")),
            test_security_settings_store(directory.path(), "settings.toml"),
            test_audit_log(directory.path(), "audit.log"),
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
        let service = SshBridgeService::new_with_stores(
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
            test_security_settings_store(directory.path(), "settings.toml"),
            test_audit_log(directory.path(), "audit.log"),
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
        let service = SshBridgeService::new_with_stores(
            TokioHandle::current(),
            endpoint,
            instance_id,
            directory.path().join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            known_hosts,
            test_security_settings_store(directory.path(), "settings.toml"),
            test_audit_log(directory.path(), "audit.log"),
        );
        service
            .set_security_policy(BridgeSecurityLevel::Standard)
            .unwrap();
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
    async fn vault_state_is_not_exposed_until_approval_succeeds() {
        let directory = tempfile::tempdir().unwrap();
        let service = test_service(directory.path());
        service
            .set_security_policy(BridgeSecurityLevel::RequireApproval { timeout_secs: 30 })
            .unwrap();
        let mut target = profile("approval-then-vault");
        target.has_stored_password = true;
        let refresh = service.refresh_routes(vec![target], vec![]);
        let route_token = refresh.routes[0].token.clone();
        tokio::time::timeout(Duration::from_secs(5), service.enable())
            .await
            .expect("Bridge enable should complete")
            .unwrap();

        let endpoint = service.endpoint().clone();
        let request = tokio::spawn(async move {
            let mut stream = connect_endpoint(&endpoint).await.unwrap();
            request_route(&mut stream, &route_token).await
        });
        wait_for_pending_phase(&service, BridgePendingPhase::AwaitingApproval).await;
        let request_id = service.security_snapshot().pending[0].request_id.clone();
        service
            .decide_authorization(&request_id, BridgeAuthorizationDecision::Approve)
            .unwrap();
        wait_for_pending_phase(&service, BridgePendingPhase::AwaitingVaultUnlock).await;
        service.cancel_pending_request(&request_id).unwrap();
        let error = tokio::time::timeout(Duration::from_secs(5), request)
            .await
            .expect("cancelled request should finish")
            .unwrap()
            .expect_err("vault wait cancellation should fail the request");
        assert!(error.to_string().contains("vault unlock was cancelled"));
        service.disable().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn locked_request_can_be_cancelled_without_attempting_upstream() {
        let directory = tempfile::tempdir().unwrap();
        let service = test_service(directory.path());
        service
            .set_security_policy(BridgeSecurityLevel::Standard)
            .unwrap();
        let mut target = profile("cancel-locked");
        target.has_stored_password = true;
        let refresh = service.refresh_routes(vec![target], vec![]);
        let route_token = refresh.routes[0].token.clone();
        tokio::time::timeout(Duration::from_secs(5), service.enable())
            .await
            .expect("Bridge enable should complete")
            .unwrap();

        let endpoint = service.endpoint().clone();
        let request = tokio::spawn(async move {
            let mut stream = connect_endpoint(&endpoint).await.unwrap();
            request_route(&mut stream, &route_token).await
        });
        wait_for_pending(&service, 1).await;
        let pending = service.security_snapshot().pending[0].clone();
        assert_eq!(pending.phase, BridgePendingPhase::AwaitingVaultUnlock);
        service.cancel_pending_request(&pending.request_id).unwrap();

        let error = tokio::time::timeout(Duration::from_secs(5), request)
            .await
            .expect("cancelled request should finish")
            .unwrap()
            .expect_err("cancelled request should receive a failure frame");
        assert!(error.to_string().contains("vault unlock was cancelled"));
        tokio::time::timeout(Duration::from_secs(5), service.disable())
            .await
            .expect("Bridge disable should complete");
        let audit = std::fs::read_to_string(directory.path().join("audit.log")).unwrap();
        assert!(audit.contains("error=vault_unlock_cancelled"));
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
        let service = SshBridgeService::new_with_stores(
            TokioHandle::current(),
            endpoint,
            instance_id,
            directory.path().join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            known_hosts,
            test_security_settings_store(directory.path(), "settings.toml"),
            test_audit_log(directory.path(), "audit.log"),
        );
        service
            .set_security_policy(BridgeSecurityLevel::Standard)
            .unwrap();
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
        let request = tokio::spawn(async move {
            request_route(&mut locked_stream, &route_token)
                .await
                .map(|()| locked_stream)
        });
        wait_for_pending(&service, 1).await;
        assert_eq!(
            service.security_snapshot().pending[0].phase,
            BridgePendingPhase::AwaitingVaultUnlock
        );
        assert_eq!(upstream.accepted.load(Ordering::Acquire), 0);
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

        let unlocked_stream = tokio::time::timeout(Duration::from_secs(5), request)
            .await
            .expect("the original Bridge request should resume after credential refresh")
            .unwrap()
            .unwrap();
        let client = tokio::time::timeout(
            Duration::from_secs(5),
            finish_bridge_client(unlocked_stream),
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
