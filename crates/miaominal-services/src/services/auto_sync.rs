use super::sync_executor::{SyncExecutor, SyncOps};
use super::sync_service::SyncTaskResult;
use anyhow::Result;
use miaominal_paths as paths;
use miaominal_storage::SettingsStore;
use miaominal_sync::capability::{
    CapabilityError, CapabilityReason, CapabilityReport, CapabilityState, EtagKind,
    ProbeCancellation,
};
use miaominal_sync::{
    RemoteSyncState, SyncContentRelation, SyncEngine, SyncInterventionReason, SyncProvider,
    SyncStatus, classify_content_revisions,
};
use notify::{RecursiveMode, Watcher};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

pub const AUTO_SYNC_POLL_INTERVAL: Duration = Duration::from_secs(60);
pub const AUTO_SYNC_DEBOUNCE: Duration = Duration::from_secs(5);
pub const AUTO_SYNC_BACKOFF_INITIAL: Duration = Duration::from_secs(30);
pub const AUTO_SYNC_BACKOFF_MAX: Duration = Duration::from_secs(600);

const TRACKED_CONFIG_FILES: [&str; 5] = [
    "settings.toml",
    "sessions.toml",
    "proxies.toml",
    "snippets.toml",
    "managed_keys.toml",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoSyncPhase {
    CheckingCapability,
    PausedCapability,
    Disabled,
    Watching,
    Debouncing,
    Pushing,
    Pulling,
    PullRequired,
    PausedVaultLocked,
    RetryBackoff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoSyncIntervention {
    pub id: String,
    pub reason: SyncInterventionReason,
    pub remote_at: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct AutoSyncSnapshot {
    pub capability: CapabilityReport,
    pub capability_notice_id: Option<String>,
    pub updated_config: Option<miaominal_sync::SyncConfig>,
    pub revision: u64,
    pub enabled: bool,
    pub phase: AutoSyncPhase,
    pub message: Option<String>,
    pub last_result: Option<SyncTaskResult>,
    pub last_result_id: Option<u64>,
    pub dirty: bool,
    pub retry_at_unix: Option<u64>,
    pub intervention: Option<AutoSyncIntervention>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetryAction {
    Poll,
    Push,
}

enum AutoSyncCommand {
    EnableChecked(Arc<ProbeCancellation>),
    RecheckCapability(Arc<ProbeCancellation>),
    CancelCheck,
    SetEngine(SyncEngine),
    SetSettingsStore(SettingsStore),
    ReconcileManualSync {
        status: SyncStatus,
        engine: SyncEngine,
        settings_store: Box<SettingsStore>,
    },
    SetVaultLocked(bool),
    Wake,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Fingerprint {
    file_hash: String,
}

impl Fingerprint {
    fn sample(config_dir: &Path) -> Self {
        let mut hasher = Sha256::new();
        for name in TRACKED_CONFIG_FILES {
            hasher.update(name.as_bytes());
            hasher.update([0u8]);
            let path = config_dir.join(name);
            if let Ok(bytes) = fs::read(&path) {
                hasher.update(bytes);
            }
        }
        let file_hash = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self { file_hash }
    }
}

#[derive(Clone)]
pub struct AutoSyncService {
    cancellation: ProbeCancellation,
    check_requests: Arc<StdMutex<CheckRequests>>,
    runtime: TokioHandle,
    command_tx: mpsc::UnboundedSender<AutoSyncCommand>,
    state_rx: watch::Receiver<AutoSyncSnapshot>,
    task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

#[derive(Default)]
struct CheckRequests {
    pending: Option<Arc<ProbeCancellation>>,
    cancelled: bool,
}

impl CheckRequests {
    fn finish(&mut self, token: &Arc<ProbeCancellation>) {
        // A cancelled request may finish after its replacement was queued.
        // Only the owner of the current slot may release it.
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| Arc::ptr_eq(pending, token))
        {
            self.pending = None;
        }
    }
}

impl AutoSyncService {
    pub fn new(
        runtime: TokioHandle,
        executor: SyncExecutor,
        settings_store: SettingsStore,
        engine: SyncEngine,
        vault_locked: bool,
    ) -> Self {
        let config_dir = paths::config_dir().unwrap_or_else(|error| {
            log::warn!("failed to locate config directory for auto-sync: {error:?}");
            std::env::temp_dir().join(format!("miaominal-auto-sync-{}", std::process::id()))
        });
        let enabled = engine.config_store.config.auto_sync_enabled;
        let initial_phase = if !enabled {
            AutoSyncPhase::Disabled
        } else if vault_locked {
            AutoSyncPhase::PausedVaultLocked
        } else {
            AutoSyncPhase::Watching
        };
        let (state_tx, state_rx) = watch::channel(AutoSyncSnapshot {
            capability: CapabilityReport::default(),
            capability_notice_id: None,
            updated_config: None,
            revision: 0,
            enabled,
            phase: initial_phase,
            message: None,
            last_result: None,
            last_result_id: None,
            dirty: false,
            retry_at_unix: None,
            intervention: None,
        });
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let task = Arc::new(tokio::sync::Mutex::new(None));
        let service = Self {
            cancellation: ProbeCancellation::default(),
            check_requests: Arc::default(),
            runtime,
            command_tx,
            state_rx,
            task: task.clone(),
        };
        let cancellation = service.cancellation.clone();
        let check_requests = service.check_requests.clone();
        let handle = service.runtime.spawn(async move {
            run_auto_sync_with_cancellation(
                command_rx,
                state_tx,
                executor,
                settings_store,
                engine,
                config_dir,
                vault_locked,
                AUTO_SYNC_POLL_INTERVAL,
                cancellation,
                check_requests,
            )
            .await;
        });
        let mut slot = task.try_lock().expect("auto-sync task slot should lock");
        *slot = Some(handle);
        service
    }

    pub fn subscribe(&self) -> watch::Receiver<AutoSyncSnapshot> {
        self.state_rx.clone()
    }

    pub fn set_engine(&self, engine: SyncEngine) {
        self.cancellation.cancel();
        let _ = self.command_tx.send(AutoSyncCommand::SetEngine(engine));
    }

    pub fn set_settings_store(&self, settings_store: SettingsStore) {
        let _ = self
            .command_tx
            .send(AutoSyncCommand::SetSettingsStore(settings_store));
    }

    pub fn reconcile_manual_sync(
        &self,
        status: SyncStatus,
        engine: SyncEngine,
        settings_store: SettingsStore,
    ) {
        let _ = self.command_tx.send(AutoSyncCommand::ReconcileManualSync {
            status,
            engine,
            settings_store: Box::new(settings_store),
        });
    }

    pub fn set_vault_locked(&self, locked: bool) {
        if locked {
            self.cancellation.cancel();
        }
        let _ = self
            .command_tx
            .send(AutoSyncCommand::SetVaultLocked(locked));
    }

    pub fn wake(&self) {
        let _ = self.command_tx.send(AutoSyncCommand::Wake);
    }

    pub fn shutdown(&self) {
        self.cancellation.cancel();
        let _ = self.command_tx.send(AutoSyncCommand::Shutdown);
        // Do not abort: a check must finish its bounded cleanup first.
        if let Ok(mut slot) = self.task.try_lock() {
            slot.take();
        }
    }

    pub fn enable_checked(&self) -> bool {
        self.request_check(true)
    }
    pub fn recheck_capability(&self) -> bool {
        self.request_check(false)
    }
    fn request_check(&self, enable: bool) -> bool {
        let mut requests = self
            .check_requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if requests
            .pending
            .as_ref()
            .is_some_and(|token| !token.is_cancelled())
            || (requests.pending.is_none()
                && !requests.cancelled
                && self.state_rx.borrow().phase == AutoSyncPhase::CheckingCapability)
        {
            return false;
        }
        let token = Arc::new(self.cancellation.fresh());
        let command = if enable {
            AutoSyncCommand::EnableChecked(token.clone())
        } else {
            AutoSyncCommand::RecheckCapability(token.clone())
        };
        if self.command_tx.send(command).is_err() {
            return false;
        }
        requests.pending = Some(token);
        requests.cancelled = false;
        true
    }
    pub fn cancel_check(&self) {
        let mut requests = self
            .check_requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.cancellation.cancel();
        requests.cancelled = true;
        let _ = self.command_tx.send(AutoSyncCommand::CancelCheck);
    }
}

struct AutoSyncTask<S: SyncOps> {
    capability: CapabilityReport,
    capability_notice_id: Option<String>,
    capability_binding: Option<(String, String, u64)>,
    cancellation: ProbeCancellation,
    executor: S,
    settings_store: SettingsStore,
    engine: SyncEngine,
    config_dir: PathBuf,
    enabled: bool,
    vault_locked: bool,
    fingerprint: Fingerprint,
    phase: AutoSyncPhase,
    message: Option<String>,
    last_result: Option<SyncTaskResult>,
    last_result_id: Option<u64>,
    result_sequence: u64,
    dirty: bool,
    remote_missing: bool,
    pending_conflict: bool,
    intervention: Option<AutoSyncIntervention>,
    backoff_delay: Duration,
    retry_action: RetryAction,
    retry_deadline: Option<Instant>,
    retry_at_unix: Option<u64>,
    revision: u64,
    state_tx: watch::Sender<AutoSyncSnapshot>,
}

impl<S: SyncOps> AutoSyncTask<S> {
    fn refresh_capability_binding(&mut self) {
        let config = &self.engine.config_store.config;
        let binding = (
            format!("{:?}:{}", config.provider, config.webdav_url),
            config.webdav_username.clone(),
            self.engine.config_store.webdav_credential_generation(),
        );
        if self.capability_binding.as_ref() != Some(&binding) {
            self.capability_binding = Some(binding);
            // A pending cleanup still belongs to the original service endpoint.
            if self.capability.cleanup_file.is_none() {
                self.capability = CapabilityReport::default();
            }
            self.capability_notice_id = None;
        }
    }

    fn capability_blocks(&self) -> bool {
        self.engine.config_store.config.provider == SyncProvider::WebDav
            && self.capability.state != CapabilityState::Supported
    }

    fn record_capability(&mut self, report: CapabilityReport, notify: bool) {
        if notify
            && self.enabled
            && report.state != CapabilityState::Supported
            && report.reason != Some(CapabilityReason::Cancelled)
            && self.capability_notice_id.is_none()
        {
            self.capability_notice_id = Some(uuid::Uuid::new_v4().to_string());
        }
        self.capability = report;
        self.phase = if self.enabled {
            AutoSyncPhase::PausedCapability
        } else {
            AutoSyncPhase::Disabled
        };
        self.publish();
    }

    async fn ensure_capability(&mut self, explicit: bool) -> bool {
        self.engine.config_store.sync_from_disk();
        self.refresh_capability_binding();
        if self.engine.config_store.config.provider != SyncProvider::WebDav {
            return true;
        }
        if self.vault_locked {
            self.set_phase(AutoSyncPhase::PausedVaultLocked);
            return false;
        }
        if !explicit && self.capability.state == CapabilityState::Supported {
            return true;
        }
        if !explicit
            && matches!(
                self.capability.state,
                CapabilityState::Unsupported | CapabilityState::Incomplete
            )
            && (self.capability.reason != Some(CapabilityReason::Network)
                || self.retry_deadline.is_some())
        {
            self.set_phase(if self.retry_deadline.is_some() {
                AutoSyncPhase::RetryBackoff
            } else {
                AutoSyncPhase::PausedCapability
            });
            return false;
        }
        let cancellation = self.cancellation.fresh();
        let config_revision = self.engine.config_store.config.config_revision;
        let credential_generation = self.engine.config_store.webdav_credential_generation();
        self.capability.state = CapabilityState::Checking;
        self.set_phase(AutoSyncPhase::CheckingCapability);
        let mut report = self
            .executor
            .check_capability(self.engine.clone(), cancellation.clone())
            .await;
        self.engine.config_store.sync_from_disk();
        if (cancellation.is_cancelled()
            || self.engine.config_store.config.config_revision != config_revision
            || self.engine.config_store.webdav_credential_generation() != credential_generation)
            && report.cleanup_file.is_none()
        {
            report = CapabilityReport::issue(
                CapabilityReason::Cancelled,
                "cancel",
                None,
                EtagKind::Missing,
            );
        }
        if report.state == CapabilityState::Supported {
            self.capability = report;
            self.capability_notice_id = None;
            self.reset_backoff();
            self.set_phase(if self.enabled {
                AutoSyncPhase::Watching
            } else {
                AutoSyncPhase::Disabled
            });
            return true;
        }
        let retry = report.reason == Some(CapabilityReason::Network) && self.enabled;
        self.record_capability(report, !explicit && !retry);
        if retry {
            self.schedule_retry(
                RetryAction::Poll,
                anyhow::anyhow!("WebDAV connection unavailable"),
            );
        }
        false
    }

    async fn enable_checked(&mut self) {
        if self.vault_locked {
            self.set_phase(AutoSyncPhase::PausedVaultLocked);
            return;
        }
        if !matches!(self.engine.config_store.get_passphrase(), Ok(Some(value)) if !value.trim().is_empty())
        {
            self.record_capability(
                CapabilityReport::issue(
                    CapabilityReason::Configuration,
                    "encryption-passphrase",
                    None,
                    EtagKind::Missing,
                ),
                false,
            );
            return;
        }
        let generation = self.cancellation.fresh();
        if !self.ensure_capability(true).await || generation.is_cancelled() {
            return;
        }
        let revision = self.engine.config_store.config.config_revision;
        match self
            .engine
            .config_store
            .update_if_revision(revision, |config| config.auto_sync_enabled = true)
        {
            Ok(true) => {
                self.enabled = true;
                self.publish();
                self.on_tick().await;
            }
            _ => self.record_capability(
                CapabilityReport::issue(
                    CapabilityReason::Configuration,
                    "save-preference",
                    None,
                    EtagKind::Missing,
                ),
                false,
            ),
        }
    }

    fn publish(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        let _ = self.state_tx.send(AutoSyncSnapshot {
            capability: self.capability.clone(),
            capability_notice_id: self.capability_notice_id.clone(),
            updated_config: Some(self.engine.config_store.config.clone()),
            revision: self.revision,
            enabled: self.enabled,
            phase: self.phase,
            message: self.message.clone(),
            last_result: self.last_result.clone(),
            last_result_id: self.last_result_id,
            dirty: self.dirty,
            retry_at_unix: self.retry_at_unix,
            intervention: self.intervention.clone(),
        });
    }

    fn set_phase(&mut self, phase: AutoSyncPhase) {
        if self.phase != phase {
            self.phase = phase;
            self.publish();
        }
    }

    fn clear_intervention(&mut self) {
        self.pending_conflict = false;
        self.intervention = None;
    }

    fn clear_last_result(&mut self) {
        self.last_result = None;
        self.last_result_id = None;
    }

    fn set_last_result(&mut self, result: SyncTaskResult) {
        self.result_sequence = self.result_sequence.wrapping_add(1);
        if self.result_sequence == 0 {
            self.result_sequence = 1;
        }
        self.last_result = Some(result);
        self.last_result_id = Some(self.result_sequence);
    }

    fn enter_intervention(&mut self, reason: SyncInterventionReason, remote_at: Option<u64>) {
        self.pending_conflict = true;
        let intervention_changed = match &mut self.intervention {
            Some(intervention) => {
                let changed = intervention.reason != reason || intervention.remote_at != remote_at;
                intervention.reason = reason;
                intervention.remote_at = remote_at;
                changed
            }
            None => {
                self.intervention = Some(AutoSyncIntervention {
                    id: uuid::Uuid::new_v4().to_string(),
                    reason,
                    remote_at,
                });
                true
            }
        };
        if self.phase != AutoSyncPhase::PullRequired {
            self.set_phase(AutoSyncPhase::PullRequired);
        } else if intervention_changed {
            self.publish();
        }
    }

    async fn apply_engine(&mut self, engine: SyncEngine) {
        self.engine = engine;
        self.refresh_capability_binding();
        self.clear_last_result();
        self.enabled = self.engine.config_store.config.auto_sync_enabled;
        self.fingerprint = Fingerprint::sample(&self.config_dir);
        self.remote_missing = false;
        self.clear_intervention();
        self.reset_backoff();
        if self.enabled && !self.vault_locked {
            if let Err(error) = self.refresh_dirty_from_revision().await {
                self.dirty = true;
                self.schedule_retry(RetryAction::Poll, error);
                return;
            }
        } else {
            self.dirty = false;
        }
        if !self.enabled {
            self.set_phase(AutoSyncPhase::Disabled);
        } else if self.vault_locked {
            self.set_phase(AutoSyncPhase::PausedVaultLocked);
        } else if self.capability_blocks() {
            self.set_phase(AutoSyncPhase::PausedCapability);
        } else {
            self.set_phase(AutoSyncPhase::Watching);
        }
    }

    async fn reconcile_manual_sync(
        &mut self,
        status: SyncStatus,
        engine: SyncEngine,
        settings_store: SettingsStore,
    ) {
        self.engine = engine;
        self.refresh_capability_binding();
        self.settings_store = settings_store;
        self.enabled = self.engine.config_store.config.auto_sync_enabled;
        self.clear_last_result();
        self.reset_backoff();

        let previous_fingerprint = self.fingerprint.clone();
        let current_fingerprint = Fingerprint::sample(&self.config_dir);
        match status {
            SyncStatus::Pushed { .. } | SyncStatus::Pulled { .. } => {
                self.fingerprint = current_fingerprint;
                self.remote_missing = false;
                self.clear_intervention();
            }
            SyncStatus::UpToDate { .. } => {
                self.fingerprint = previous_fingerprint;
                if current_fingerprint != self.fingerprint {
                    self.dirty = true;
                }
                self.clear_intervention();
            }
            SyncStatus::PullRequired { remote_at, reason } => {
                self.dirty = true;
                self.enter_intervention(reason, remote_at);
            }
            _ => {}
        }

        if !self.pending_conflict
            && let Err(error) = self.refresh_dirty_from_revision().await
        {
            self.dirty = true;
            self.schedule_retry(RetryAction::Poll, error);
            return;
        }

        self.phase = if !self.enabled {
            AutoSyncPhase::Disabled
        } else if self.vault_locked {
            AutoSyncPhase::PausedVaultLocked
        } else if self.capability_blocks() {
            AutoSyncPhase::PausedCapability
        } else if self.pending_conflict {
            AutoSyncPhase::PullRequired
        } else {
            AutoSyncPhase::Watching
        };
        self.publish();
    }

    fn reset_backoff(&mut self) {
        self.backoff_delay = AUTO_SYNC_BACKOFF_INITIAL;
        self.retry_deadline = None;
        self.retry_at_unix = None;
        self.message = None;
    }

    fn schedule_retry(&mut self, action: RetryAction, error: anyhow::Error) {
        if let Some(capability) = error.downcast_ref::<CapabilityError>() {
            self.retry_deadline = None;
            self.retry_at_unix = None;
            self.message = None;
            self.record_capability(capability.0.clone(), true);
            return;
        }
        let delay = self.backoff_delay;
        self.backoff_delay = (delay * 2).min(AUTO_SYNC_BACKOFF_MAX);
        self.retry_action = action;
        self.retry_deadline = Some(Instant::now() + delay);
        self.retry_at_unix = Some(unix_now().saturating_add(delay.as_secs()));
        self.message = Some(error.to_string());
        self.phase = AutoSyncPhase::RetryBackoff;
        self.publish();
    }

    fn schedule_debounced_push(&mut self, debounce_deadline: &mut Option<Instant>) {
        if !self.enabled
            || self.vault_locked
            || self.retry_deadline.is_some()
            || self.capability_blocks()
        {
            return;
        }
        *debounce_deadline = Some(Instant::now() + AUTO_SYNC_DEBOUNCE);
        self.set_phase(AutoSyncPhase::Debouncing);
    }

    async fn refresh_dirty_from_revision(&mut self) -> anyhow::Result<String> {
        let revision = self
            .executor
            .local_revision(self.engine.clone(), self.settings_store.clone())
            .await?;
        self.dirty = self.remote_missing
            || self
                .engine
                .config_store
                .config
                .last_synced_local_revision
                .as_deref()
                != Some(revision.as_str());
        Ok(revision)
    }

    async fn push_if_dirty(&mut self) {
        if !self.ensure_capability(false).await {
            return;
        }
        const MAX_IMMEDIATE_PUSHES: usize = 2;
        for attempt in 0..MAX_IMMEDIATE_PUSHES {
            if self.pending_conflict {
                self.dirty = true;
                self.set_phase(AutoSyncPhase::PullRequired);
                return;
            }
            if let Err(error) = self.refresh_dirty_from_revision().await {
                self.dirty = true;
                self.schedule_retry(RetryAction::Push, error);
                return;
            }
            if !self.dirty {
                return;
            }
            self.set_phase(AutoSyncPhase::Pushing);
            let engine = self.engine.clone();
            let settings_store = self.settings_store.clone();
            match self.executor.push(engine, settings_store).await {
                Ok(result) => {
                    self.set_last_result(result.clone());
                    self.engine.config_store.config = result.updated_config;
                    match result.status {
                        SyncStatus::Pushed { .. } => {
                            self.remote_missing = false;
                            self.fingerprint = Fingerprint::sample(&self.config_dir);
                            self.clear_intervention();
                            self.reset_backoff();
                            if let Err(error) = self.refresh_dirty_from_revision().await {
                                self.dirty = true;
                                self.schedule_retry(RetryAction::Push, error);
                                return;
                            }
                            if self.dirty && attempt + 1 < MAX_IMMEDIATE_PUSHES {
                                continue;
                            }
                            self.set_phase(AutoSyncPhase::Watching);
                        }
                        SyncStatus::PullRequired { remote_at, reason } => {
                            self.dirty = true;
                            self.enter_intervention(reason, remote_at);
                        }
                        _ => {
                            self.reset_backoff();
                            self.set_phase(AutoSyncPhase::Watching);
                        }
                    }
                    return;
                }
                Err(error) => {
                    self.dirty = true;
                    let recheck = error
                        .downcast_ref::<CapabilityError>()
                        .filter(|issue| issue.0.step == "upload-412-unchanged")
                        .map(|issue| issue.0.clone());
                    self.schedule_retry(RetryAction::Push, error);
                    if let Some(report) = recheck {
                        let notice = self.capability_notice_id.clone();
                        if self.ensure_capability(true).await {
                            // A disposable file passing cannot negate a failed
                            // precondition on the actual configuration resource.
                            self.capability_notice_id = notice;
                            self.record_capability(report, true);
                        }
                    }
                    return;
                }
            }
        }
    }

    async fn poll_remote(&mut self) {
        if !self.ensure_capability(false).await {
            return;
        }
        self.set_phase(AutoSyncPhase::Pulling);
        let engine = self.engine.clone();
        match self.executor.remote_state(engine).await {
            Ok(RemoteSyncState::Disabled) => {
                self.reset_backoff();
                self.set_phase(AutoSyncPhase::Watching);
            }
            Ok(RemoteSyncState::BindingRequired(SyncProvider::GithubGist))
                if self.engine.config_store.config.gist_id.is_none() =>
            {
                self.reset_backoff();
                self.remote_missing = true;
                self.clear_intervention();
                self.dirty = true;
                self.push_if_dirty().await;
            }
            Ok(RemoteSyncState::BindingRequired(_)) => {
                self.reset_backoff();
                self.set_phase(AutoSyncPhase::Watching);
            }
            Ok(RemoteSyncState::Missing) => {
                self.reset_backoff();
                self.remote_missing = true;
                self.clear_intervention();
                self.dirty = true;
                self.push_if_dirty().await;
            }
            Ok(RemoteSyncState::NotModified | RemoteSyncState::UpToDate) => {
                self.reset_backoff();
                if self
                    .engine
                    .config_store
                    .config
                    .last_synced_local_revision
                    .is_none()
                {
                    self.dirty = true;
                    self.enter_intervention(SyncInterventionReason::MissingSyncBaseline, None);
                    return;
                }
                if let Err(error) = self.refresh_dirty_from_revision().await {
                    self.dirty = true;
                    self.schedule_retry(RetryAction::Poll, error);
                    return;
                }
                self.set_phase(if self.pending_conflict {
                    AutoSyncPhase::PullRequired
                } else {
                    AutoSyncPhase::Watching
                });
            }
            Ok(RemoteSyncState::Updated {
                synced_at,
                content_revision,
                ..
            }) => {
                let Some(content_revision) = content_revision else {
                    self.dirty = true;
                    self.enter_intervention(
                        SyncInterventionReason::MissingSyncBaseline,
                        Some(synced_at),
                    );
                    return;
                };
                let expected_local_revision = match self.refresh_dirty_from_revision().await {
                    Ok(revision) => revision,
                    Err(error) => {
                        self.dirty = true;
                        self.schedule_retry(RetryAction::Poll, error);
                        return;
                    }
                };
                match classify_content_revisions(
                    &expected_local_revision,
                    &content_revision,
                    self.engine
                        .config_store
                        .config
                        .last_synced_local_revision
                        .as_deref(),
                ) {
                    SyncContentRelation::LocalChanged => {
                        self.push_if_dirty().await;
                        return;
                    }
                    SyncContentRelation::Diverged => {
                        self.dirty = true;
                        self.enter_intervention(
                            SyncInterventionReason::BothSidesChanged,
                            Some(synced_at),
                        );
                        return;
                    }
                    SyncContentRelation::MissingBaseline => {
                        self.dirty = true;
                        self.enter_intervention(
                            SyncInterventionReason::MissingSyncBaseline,
                            Some(synced_at),
                        );
                        return;
                    }
                    SyncContentRelation::Identical | SyncContentRelation::RemoteChanged => {}
                }
                self.set_phase(AutoSyncPhase::Pulling);
                let engine = self.engine.clone();
                let settings_store = self.settings_store.clone();
                match self
                    .executor
                    .pull_if_unchanged(engine, settings_store, expected_local_revision)
                    .await
                {
                    Ok(result) => {
                        self.set_last_result(result.clone());
                        self.engine.config_store.config = result.updated_config;
                        if let Some(reload) = &result.reload
                            && let Ok(store) = &reload.settings
                        {
                            self.settings_store = store.clone();
                        }
                        match result.status {
                            SyncStatus::Pulled { .. } | SyncStatus::UpToDate { .. } => {
                                self.remote_missing = false;
                                self.fingerprint = Fingerprint::sample(&self.config_dir);
                                self.clear_intervention();
                                self.reset_backoff();
                                if let Err(error) = self.refresh_dirty_from_revision().await {
                                    self.dirty = true;
                                    self.schedule_retry(RetryAction::Poll, error);
                                    return;
                                }
                                self.set_phase(AutoSyncPhase::Watching);
                            }
                            SyncStatus::PullRequired { remote_at, reason } => {
                                self.dirty = true;
                                let remote_at = remote_at.or(Some(synced_at));
                                self.enter_intervention(reason, remote_at);
                            }
                            _ => {
                                self.reset_backoff();
                                self.set_phase(AutoSyncPhase::Watching);
                            }
                        }
                    }
                    Err(error) => {
                        self.schedule_retry(RetryAction::Poll, error);
                    }
                }
            }
            Err(error) => {
                self.schedule_retry(RetryAction::Poll, error);
            }
        }
    }

    async fn on_tick(&mut self) {
        if !self.enabled || self.vault_locked || self.retry_deadline.is_some() {
            return;
        }
        if !self.ensure_capability(false).await {
            return;
        }
        self.push_if_dirty().await;
        if self.pending_conflict {
            return;
        }
        if !matches!(
            self.phase,
            AutoSyncPhase::Pushing | AutoSyncPhase::RetryBackoff
        ) {
            self.poll_remote().await;
        }
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
async fn run_auto_sync<S: SyncOps>(
    command_rx: mpsc::UnboundedReceiver<AutoSyncCommand>,
    state_tx: watch::Sender<AutoSyncSnapshot>,
    executor: S,
    settings_store: SettingsStore,
    engine: SyncEngine,
    config_dir: PathBuf,
    vault_locked: bool,
    poll_interval: Duration,
) {
    run_auto_sync_with_cancellation(
        command_rx,
        state_tx,
        executor,
        settings_store,
        engine,
        config_dir,
        vault_locked,
        poll_interval,
        ProbeCancellation::default(),
        Arc::default(),
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn run_auto_sync_with_cancellation<S: SyncOps>(
    mut command_rx: mpsc::UnboundedReceiver<AutoSyncCommand>,
    state_tx: watch::Sender<AutoSyncSnapshot>,
    executor: S,
    settings_store: SettingsStore,
    engine: SyncEngine,
    config_dir: PathBuf,
    vault_locked: bool,
    poll_interval: Duration,
    cancellation: ProbeCancellation,
    check_requests: Arc<StdMutex<CheckRequests>>,
) {
    let enabled = engine.config_store.config.auto_sync_enabled;
    let fingerprint = Fingerprint::sample(&config_dir);
    let mut task = AutoSyncTask {
        capability: CapabilityReport::default(),
        capability_notice_id: None,
        capability_binding: None,
        cancellation,
        executor,
        settings_store,
        engine,
        config_dir: config_dir.clone(),
        enabled,
        vault_locked,
        fingerprint,
        phase: if !enabled {
            AutoSyncPhase::Disabled
        } else if vault_locked {
            AutoSyncPhase::PausedVaultLocked
        } else {
            AutoSyncPhase::Watching
        },
        message: None,
        last_result: None,
        last_result_id: None,
        result_sequence: 0,
        dirty: false,
        remote_missing: false,
        pending_conflict: false,
        intervention: None,
        backoff_delay: AUTO_SYNC_BACKOFF_INITIAL,
        retry_action: RetryAction::Poll,
        retry_deadline: None,
        retry_at_unix: None,
        revision: 0,
        state_tx,
    };
    task.publish();
    if task.enabled && !task.vault_locked {
        if task
            .engine
            .config_store
            .config
            .last_synced_local_revision
            .is_none()
        {
            task.poll_remote().await;
        } else {
            task.on_tick().await;
        }
    }

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<()>();
    let watcher = match start_file_watcher(&config_dir, event_tx) {
        Ok(watcher) => watcher,
        Err(error) => {
            log::warn!("auto-sync file watcher unavailable: {error:?}");
            None
        }
    };
    let mut watcher_active = watcher.is_some();

    let mut debounce_deadline: Option<Instant> = None;
    let mut tick = Box::pin(tokio::time::sleep(poll_interval));

    loop {
        let debounce_pending = debounce_deadline.is_some();
        let retry_pending = task.retry_deadline.is_some();
        let mut debounce = match debounce_deadline {
            Some(deadline) => Box::pin(tokio::time::sleep_until(deadline)),
            None => Box::pin(tokio::time::sleep(Duration::from_secs(3600))),
        };
        let mut retry = match task.retry_deadline {
            Some(deadline) => Box::pin(tokio::time::sleep_until(deadline)),
            None => Box::pin(tokio::time::sleep(Duration::from_secs(3600))),
        };

        tokio::select! {
            biased;
            command = command_rx.recv() => {
                let Some(command) = command else { break };
                match command {
                    AutoSyncCommand::EnableChecked(token) => {
                        if !token.is_cancelled() { task.enable_checked().await; }
                        check_requests.lock().unwrap_or_else(std::sync::PoisonError::into_inner).finish(&token);
                    }
                    AutoSyncCommand::RecheckCapability(token) => {
                        if !token.is_cancelled() && task.ensure_capability(true).await && task.enabled { task.on_tick().await; }
                        check_requests.lock().unwrap_or_else(std::sync::PoisonError::into_inner).finish(&token);
                    }
                    AutoSyncCommand::CancelCheck => {
                        task.retry_deadline = None;
                        task.retry_at_unix = None;
                        if task.capability.cleanup_file.is_none() {
                            task.record_capability(CapabilityReport::issue(CapabilityReason::Cancelled, "cancel", None, EtagKind::Missing), false);
                        }
                    }
                    AutoSyncCommand::SetEngine(engine) => {
                        task.apply_engine(engine).await;
                        if task.enabled && !task.vault_locked {
                            task.on_tick().await;
                        }
                    }
                    AutoSyncCommand::SetSettingsStore(store) => {
                        task.settings_store = store;
                    }
                    AutoSyncCommand::ReconcileManualSync {
                        status,
                        engine,
                        settings_store,
                    } => {
                        task.reconcile_manual_sync(status, engine, *settings_store)
                            .await;
                    }
                    AutoSyncCommand::SetVaultLocked(locked) => {
                        task.vault_locked = locked;
                        if locked {
                            task.set_phase(AutoSyncPhase::PausedVaultLocked);
                        } else if task.enabled {
                            if task.capability.reason == Some(CapabilityReason::Cancelled)
                                && task.capability.cleanup_file.is_none()
                            {
                                task.capability = CapabilityReport::default();
                            }
                            task.reset_backoff();
                            task.set_phase(AutoSyncPhase::Watching);
                            task.on_tick().await;
                        }
                    }
                    AutoSyncCommand::Wake => {
                        if task.enabled && !task.vault_locked {
                            task.on_tick().await;
                        }
                    }
                    AutoSyncCommand::Shutdown => break,
                }
            }
            _ = &mut retry, if retry_pending && task.enabled && !task.vault_locked => {
                task.retry_deadline = None;
                task.retry_at_unix = None;
                task.message = None;
                match task.retry_action {
                    RetryAction::Poll => task.poll_remote().await,
                    RetryAction::Push => task.push_if_dirty().await,
                }
            }
            _ = &mut debounce, if debounce_pending && task.enabled && !task.vault_locked => {
                debounce_deadline = None;
                if task.retry_deadline.is_none() {
                    task.push_if_dirty().await;
                }
            }
            _ = &mut tick, if task.enabled && !task.vault_locked => {
                tick = Box::pin(tokio::time::sleep(poll_interval));
                task.on_tick().await;
            }
            event = event_rx.recv(), if watcher_active => {
                match event {
                    Some(()) => task.schedule_debounced_push(&mut debounce_deadline),
                    None => {
                        watcher_active = false;
                        log::warn!("auto-sync file watcher stopped; continuing with periodic polling");
                    }
                }
            }
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn start_file_watcher(
    config_dir: &Path,
    event_tx: mpsc::UnboundedSender<()>,
) -> Result<Option<notify::RecommendedWatcher>> {
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(move |result| {
        let _ = tx.send(result);
    })?;
    watcher.watch(config_dir, RecursiveMode::NonRecursive)?;
    std::thread::spawn(move || {
        while let Ok(result) = rx.recv() {
            match result {
                Ok(event) => {
                    let tracked = event.paths.iter().any(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| TRACKED_CONFIG_FILES.contains(&name))
                    });
                    if tracked {
                        let _ = event_tx.send(());
                    }
                }
                Err(error) => log::warn!("auto-sync watcher event error: {error:?}"),
            }
        }
    });
    Ok(Some(watcher))
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_secrets::CredentialStore;
    use miaominal_secrets::credential_backend::CredentialBackend;
    use miaominal_sync::{SyncConfig, SyncConfigStore, SyncProvider, SyncStatus};
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn cancelled_check_accepts_replacement_before_cleanup_finishes() {
        let (command_tx, mut commands) = mpsc::unbounded_channel();
        let (state_tx, state_rx) = watch::channel(snapshot(false, AutoSyncPhase::Disabled));
        let service = AutoSyncService {
            cancellation: ProbeCancellation::default(),
            check_requests: Arc::default(),
            runtime: TokioHandle::current(),
            command_tx,
            state_rx,
            task: Arc::new(tokio::sync::Mutex::new(None)),
        };
        assert!(service.recheck_capability());
        let AutoSyncCommand::RecheckCapability(old) = commands.try_recv().unwrap() else {
            panic!("expected check")
        };
        state_tx
            .send(snapshot(false, AutoSyncPhase::CheckingCapability))
            .unwrap();
        service.cancel_check();
        assert!(matches!(
            commands.try_recv().unwrap(),
            AutoSyncCommand::CancelCheck
        ));
        assert!(service.recheck_capability());
        let AutoSyncCommand::RecheckCapability(replacement) = commands.try_recv().unwrap() else {
            panic!("expected replacement")
        };
        assert!(old.is_cancelled());
        assert!(!replacement.is_cancelled());
        service.check_requests.lock().unwrap().finish(&old);
        assert!(
            !service.clone().recheck_capability(),
            "old completion released the replacement slot"
        );
        service.check_requests.lock().unwrap().finish(&replacement);
        state_tx
            .send(snapshot(false, AutoSyncPhase::Disabled))
            .unwrap();
        assert!(service.recheck_capability());
    }

    #[tokio::test]
    async fn queued_retry_runs_after_cancelled_check_finishes_cleanup() {
        let dir = temp_config_dir("cancel-retry-cleanup");
        let mock = Arc::new(MockSyncOps::new());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        *mock.probe_gate.lock().unwrap() = Some((started_tx, finish_rx));
        let mut task = test_task(mock.clone(), &dir);
        task.engine.config_store.config.provider = SyncProvider::WebDav;
        task.engine.config_store.config.auto_sync_enabled = false;
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (state_tx, state_rx) = watch::channel(snapshot(false, AutoSyncPhase::Disabled));
        let service = AutoSyncService {
            cancellation: ProbeCancellation::default(),
            check_requests: Arc::default(),
            runtime: TokioHandle::current(),
            command_tx,
            state_rx,
            task: Arc::new(tokio::sync::Mutex::new(None)),
        };
        let run = tokio::spawn(run_auto_sync_with_cancellation(
            command_rx,
            state_tx,
            mock.clone(),
            task.settings_store,
            task.engine,
            dir,
            false,
            Duration::from_secs(3600),
            service.cancellation.clone(),
            service.check_requests.clone(),
        ));
        assert!(service.recheck_capability());
        tokio::time::timeout(Duration::from_secs(5), started_rx)
            .await
            .unwrap()
            .unwrap();
        service.cancel_check();
        assert!(service.clone().recheck_capability());
        assert!(!service.recheck_capability());
        assert_eq!(*mock.capability_calls.lock().unwrap(), 1);
        finish_tx.send(()).unwrap();
        let mut state = service.subscribe();
        tokio::time::timeout(Duration::from_secs(5), async {
            while state.borrow().capability.state != CapabilityState::Supported {
                state.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(*mock.capability_calls.lock().unwrap(), 2);
        assert_eq!(*mock.push_calls.lock().unwrap(), 0);
        service.shutdown();
        run.await.unwrap();
    }

    #[derive(Default)]
    struct MemoryCredentialBackend(Mutex<BTreeMap<String, String>>);

    impl CredentialBackend for MemoryCredentialBackend {
        fn name(&self) -> &'static str {
            "auto-sync-test-memory"
        }

        fn get(&self, service: &str, account: &str) -> anyhow::Result<Option<String>> {
            Ok(self
                .0
                .lock()
                .expect("memory backend should lock")
                .get(&format!("{service}/{account}"))
                .cloned())
        }

        fn set(&self, service: &str, account: &str, value: &str) -> anyhow::Result<()> {
            self.0
                .lock()
                .expect("memory backend should lock")
                .insert(format!("{service}/{account}"), value.to_string());
            Ok(())
        }

        fn delete(&self, service: &str, account: &str) -> anyhow::Result<()> {
            self.0
                .lock()
                .expect("memory backend should lock")
                .remove(&format!("{service}/{account}"));
            Ok(())
        }
    }

    fn memory_credentials() -> CredentialStore {
        CredentialStore::with_backend("auto-sync-test", MemoryCredentialBackend::default())
    }

    struct MockSyncOps {
        probe_gate: Mutex<
            Option<(
                tokio::sync::oneshot::Sender<()>,
                tokio::sync::oneshot::Receiver<()>,
            )>,
        >,
        capability_calls: Mutex<usize>,
        capability_result: Mutex<CapabilityReport>,
        cancel_probe: std::sync::atomic::AtomicBool,
        mutate_during_probe: Mutex<Option<&'static str>>,
        push_calls: Mutex<usize>,
        pull_calls: Mutex<usize>,
        remote_calls: Mutex<usize>,
        remote_state_result: Mutex<RemoteSyncState>,
        local_revision_result: Mutex<String>,
        revision_after_next_read: Mutex<Option<String>>,
    }
    impl MockSyncOps {
        fn new() -> Self {
            Self {
                probe_gate: Mutex::new(None),
                capability_calls: Mutex::new(0),
                capability_result: Mutex::new(CapabilityReport::supported()),
                cancel_probe: std::sync::atomic::AtomicBool::new(false),
                mutate_during_probe: Mutex::new(None),
                push_calls: Mutex::new(0),
                pull_calls: Mutex::new(0),
                remote_calls: Mutex::new(0),
                remote_state_result: Mutex::new(RemoteSyncState::UpToDate),
                local_revision_result: Mutex::new("local-revision".into()),
                revision_after_next_read: Mutex::new(None),
            }
        }

        fn pushed_result(revision: String) -> SyncTaskResult {
            SyncTaskResult {
                status: SyncStatus::Pushed { at: 1 },
                updated_config: SyncConfig {
                    last_sync_at: 1,
                    last_synced_local_revision: Some(revision),
                    ..SyncConfig::default()
                },
                reload: None,
            }
        }

        fn pulled_result(revision: String) -> SyncTaskResult {
            SyncTaskResult {
                status: SyncStatus::Pulled { at: 2 },
                updated_config: SyncConfig {
                    last_sync_at: 2,
                    last_synced_local_revision: Some(revision),
                    ..SyncConfig::default()
                },
                reload: None,
            }
        }

        fn set_local_revision(&self, revision: &str) {
            *self.local_revision_result.lock().unwrap() = revision.into();
        }

        fn change_revision_after_next_read(&self, revision: &str) {
            *self.revision_after_next_read.lock().unwrap() = Some(revision.into());
        }
    }

    impl SyncOps for Arc<MockSyncOps> {
        async fn check_capability(
            &self,
            mut engine: SyncEngine,
            cancel: ProbeCancellation,
        ) -> CapabilityReport {
            *self.capability_calls.lock().unwrap() += 1;
            let gate = self.probe_gate.lock().unwrap().take();
            if let Some((started, finish)) = gate {
                started.send(()).unwrap();
                finish.await.unwrap();
            }
            if self.cancel_probe.load(std::sync::atomic::Ordering::SeqCst) {
                cancel.cancel();
            }
            match self.mutate_during_probe.lock().unwrap().take() {
                Some("endpoint") => {
                    engine
                        .config_store
                        .update(|config| {
                            config.webdav_url = "https://other.example/sync.json".into()
                        })
                        .unwrap();
                }
                Some("disable") => {
                    // Disabling an already-off preference still cancels its
                    // pending enable request, even when no disk value changes.
                    cancel.cancel();
                    engine
                        .config_store
                        .update(|config| config.auto_sync_enabled = false)
                        .unwrap();
                }
                Some("credentials") => {
                    engine.config_store.set_webdav_password("changed").unwrap();
                }
                _ => {}
            }
            self.capability_result.lock().unwrap().clone()
        }
        async fn push(
            &self,
            _engine: SyncEngine,
            _settings_store: SettingsStore,
        ) -> anyhow::Result<SyncTaskResult> {
            *self.push_calls.lock().expect("push counter should lock") += 1;
            Ok(MockSyncOps::pushed_result(
                self.local_revision_result.lock().unwrap().clone(),
            ))
        }

        async fn pull_if_unchanged(
            &self,
            _engine: SyncEngine,
            _settings_store: SettingsStore,
            expected_local_revision: String,
        ) -> anyhow::Result<SyncTaskResult> {
            *self.pull_calls.lock().expect("pull counter should lock") += 1;
            let current_revision = self.local_revision_result.lock().unwrap().clone();
            if current_revision != expected_local_revision {
                return Ok(SyncTaskResult {
                    status: SyncStatus::PullRequired {
                        remote_at: Some(2),
                        reason: SyncInterventionReason::LocalChangedDuringPull,
                    },
                    updated_config: SyncConfig {
                        last_synced_local_revision: Some(expected_local_revision),
                        ..SyncConfig::default()
                    },
                    reload: None,
                });
            }
            Ok(MockSyncOps::pulled_result(current_revision))
        }

        async fn remote_state(&self, _engine: SyncEngine) -> anyhow::Result<RemoteSyncState> {
            *self
                .remote_calls
                .lock()
                .expect("remote counter should lock") += 1;
            Ok(self
                .remote_state_result
                .lock()
                .expect("remote result should lock")
                .clone())
        }

        async fn local_revision(
            &self,
            _engine: SyncEngine,
            _settings_store: SettingsStore,
        ) -> anyhow::Result<String> {
            let revision = self.local_revision_result.lock().unwrap().clone();
            if let Some(next) = self.revision_after_next_read.lock().unwrap().take() {
                *self.local_revision_result.lock().unwrap() = next;
            }
            Ok(revision)
        }
    }

    fn temp_config_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "miaominal-auto-sync-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("temp config dir should be created");
        dir
    }

    fn snapshot(enabled: bool, phase: AutoSyncPhase) -> AutoSyncSnapshot {
        AutoSyncSnapshot {
            capability: CapabilityReport::default(),
            capability_notice_id: None,
            updated_config: None,
            revision: 0,
            enabled,
            phase,
            message: None,
            last_result: None,
            last_result_id: None,
            dirty: false,
            retry_at_unix: None,
            intervention: None,
        }
    }

    fn test_task(mock: Arc<MockSyncOps>, config_dir: &Path) -> AutoSyncTask<Arc<MockSyncOps>> {
        let settings_store = SettingsStore::load_with_path(config_dir.join("settings.toml"))
            .expect("test settings store should load");
        let engine = SyncEngine {
            config_store: SyncConfigStore::with_credentials(
                config_dir.join("sync_config.toml"),
                SyncConfig {
                    provider: SyncProvider::GithubGist,
                    auto_sync_enabled: true,
                    last_synced_local_revision: Some("local-revision".into()),
                    ..SyncConfig::default()
                },
                memory_credentials(),
            ),
        };
        let fingerprint = Fingerprint::sample(config_dir);
        engine
            .config_store
            .set_passphrase("test-passphrase")
            .unwrap();
        let (state_tx, _state_rx) = watch::channel(snapshot(true, AutoSyncPhase::Watching));
        AutoSyncTask {
            capability: CapabilityReport::default(),
            capability_notice_id: None,
            capability_binding: None,
            cancellation: ProbeCancellation::default(),
            executor: mock,
            settings_store,
            engine,
            config_dir: config_dir.to_path_buf(),
            enabled: true,
            vault_locked: false,
            fingerprint,
            phase: AutoSyncPhase::Watching,
            message: None,
            last_result: None,
            last_result_id: None,
            result_sequence: 0,
            dirty: false,
            remote_missing: false,
            pending_conflict: false,
            intervention: None,
            backoff_delay: AUTO_SYNC_BACKOFF_INITIAL,
            retry_action: RetryAction::Poll,
            retry_deadline: None,
            retry_at_unix: None,
            revision: 0,
            state_tx,
        }
    }

    #[tokio::test]
    async fn unsupported_webdav_stops_polling_and_manual_success_does_not_resume_it() {
        let dir = temp_config_dir("capability-pause");
        let mock = Arc::new(MockSyncOps::new());
        *mock.capability_result.lock().unwrap() = CapabilityReport::issue(
            CapabilityReason::VersionUnavailable,
            "poll",
            Some(200),
            EtagKind::Weak,
        );
        let mut task = test_task(mock.clone(), &dir);
        task.engine.config_store.config.provider = SyncProvider::WebDav;
        for _ in 0..3 {
            task.on_tick().await;
        }
        assert_eq!(*mock.capability_calls.lock().unwrap(), 1);
        assert_eq!(*mock.remote_calls.lock().unwrap(), 0);
        assert_eq!(*mock.push_calls.lock().unwrap(), 0);
        assert_eq!(task.phase, AutoSyncPhase::PausedCapability);
        let notice = task.capability_notice_id.clone();
        assert!(notice.is_some());
        task.reconcile_manual_sync(
            SyncStatus::Pushed { at: 42 },
            task.engine.clone(),
            task.settings_store.clone(),
        )
        .await;
        task.on_tick().await;
        assert_eq!(task.phase, AutoSyncPhase::PausedCapability);
        assert_eq!(task.capability_notice_id, notice);
        assert_eq!(*mock.capability_calls.lock().unwrap(), 1);
        assert_eq!(*mock.remote_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn explicit_enable_is_persisted_only_after_a_successful_check() {
        let dir = temp_config_dir("capability-enable");
        let mock = Arc::new(MockSyncOps::new());
        *mock.capability_result.lock().unwrap() = CapabilityReport::issue(
            CapabilityReason::Permission,
            "create",
            Some(403),
            EtagKind::Missing,
        );
        let mut task = test_task(mock.clone(), &dir);
        task.engine.config_store.config.provider = SyncProvider::WebDav;
        task.engine.config_store.config.auto_sync_enabled = false;
        task.enabled = false;
        task.enable_checked().await;
        assert!(!task.engine.config_store.config.auto_sync_enabled);
        assert!(!task.enabled);
        assert!(task.capability_notice_id.is_none());
        assert_eq!(task.capability.state, CapabilityState::Incomplete);
        assert_eq!(*mock.push_calls.lock().unwrap(), 0);
        *mock.capability_result.lock().unwrap() = CapabilityReport::supported();
        task.enable_checked().await;
        assert!(task.enabled);
        assert!(task.engine.config_store.config.auto_sync_enabled);
        assert_eq!(*mock.capability_calls.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn cancelled_probe_cannot_enable_auto_sync() {
        let dir = temp_config_dir("capability-cancel");
        let mock = Arc::new(MockSyncOps::new());
        mock.cancel_probe
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let mut task = test_task(mock.clone(), &dir);
        task.engine.config_store.config.provider = SyncProvider::WebDav;
        task.engine.config_store.config.auto_sync_enabled = false;
        task.enabled = false;
        task.enable_checked().await;
        assert!(!task.enabled);
        assert_eq!(task.capability.reason, Some(CapabilityReason::Cancelled));
        assert_eq!(*mock.push_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn missing_encryption_configuration_does_not_enable_or_probe() {
        let dir = temp_config_dir("capability-no-passphrase");
        let mock = Arc::new(MockSyncOps::new());
        let mut task = test_task(mock.clone(), &dir);
        task.engine.config_store.config.provider = SyncProvider::WebDav;
        task.engine.config_store.config.auto_sync_enabled = false;
        task.engine.config_store.delete_passphrase().unwrap();
        task.enabled = false;
        task.enable_checked().await;
        assert!(!task.enabled);
        assert_eq!(
            task.capability.reason,
            Some(CapabilityReason::Configuration)
        );
        assert_eq!(*mock.capability_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn stale_checks_cannot_enable_after_another_window_changes_configuration() {
        for mutation in ["endpoint", "disable", "credentials"] {
            let dir = temp_config_dir(mutation);
            let mock = Arc::new(MockSyncOps::new());
            *mock.mutate_during_probe.lock().unwrap() = Some(mutation);
            let mut task = test_task(mock.clone(), &dir);
            task.engine.config_store.config.provider = SyncProvider::WebDav;
            task.engine.config_store.config.auto_sync_enabled = false;
            task.engine.config_store.update(|_| {}).unwrap();
            task.enabled = false;
            task.enable_checked().await;
            assert!(!task.enabled, "{mutation}");
            assert!(!task.engine.config_store.config.auto_sync_enabled);
            assert_eq!(task.capability.reason, Some(CapabilityReason::Cancelled));
            assert_eq!(*mock.push_calls.lock().unwrap(), 0);
            assert_eq!(*mock.remote_calls.lock().unwrap(), 0);
        }
    }

    #[tokio::test]
    async fn capability_cache_tracks_endpoint_and_credentials_but_not_baseline() {
        let dir = temp_config_dir("capability-binding");
        let mock = Arc::new(MockSyncOps::new());
        let mut task = test_task(mock.clone(), &dir);
        task.engine.config_store.config.provider = SyncProvider::WebDav;
        assert!(task.ensure_capability(false).await);
        task.engine.config_store.config.last_sync_at = 42;
        assert!(task.ensure_capability(false).await);
        assert_eq!(*mock.capability_calls.lock().unwrap(), 1);
        task.engine.config_store.config.webdav_url = "https://example.com/new.json".into();
        assert!(task.ensure_capability(false).await);
        task.engine
            .config_store
            .set_webdav_password("new-password")
            .unwrap();
        assert!(task.ensure_capability(false).await);
        assert_eq!(*mock.capability_calls.lock().unwrap(), 3);
    }

    #[tokio::test]
    async fn network_probe_failures_back_off_without_polling_payloads() {
        let dir = temp_config_dir("capability-network");
        let mock = Arc::new(MockSyncOps::new());
        *mock.capability_result.lock().unwrap() = CapabilityReport::issue(
            CapabilityReason::Network,
            "read-resource",
            None,
            EtagKind::Missing,
        );
        let mut task = test_task(mock.clone(), &dir);
        task.engine.config_store.config.provider = SyncProvider::WebDav;
        task.on_tick().await;
        assert_eq!(task.phase, AutoSyncPhase::RetryBackoff);
        assert!(task.retry_deadline.is_some());
        task.on_tick().await;
        assert_eq!(*mock.capability_calls.lock().unwrap(), 1);
        assert_eq!(*mock.remote_calls.lock().unwrap(), 0);
        assert_eq!(*mock.push_calls.lock().unwrap(), 0);
    }

    #[test]
    fn fingerprint_tracks_files_but_not_sync_config_or_credentials() {
        let dir = temp_config_dir("fingerprint");
        let first = Fingerprint::sample(&dir);
        std::fs::write(dir.join("sessions.toml"), b"[[sessions]]")
            .expect("sessions file should be writable");
        let second = Fingerprint::sample(&dir);
        assert_ne!(
            first, second,
            "tracked file changes must change the fingerprint"
        );

        std::fs::write(dir.join("sync_config.toml"), b"provider = \"webdav\"")
            .expect("sync config should be writable");
        let third = Fingerprint::sample(&dir);
        assert_eq!(
            second, third,
            "sync_config.toml must not participate in the fingerprint"
        );

        memory_credentials()
            .set("account", "value")
            .expect("credential set should succeed");
        let fourth = Fingerprint::sample(&dir);
        assert_eq!(
            third, fourth,
            "credential writes are detected by payload revision, not a global counter"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn result_ids_are_stable_across_snapshots_and_advance_for_new_results() {
        let dir = temp_config_dir("result-id");
        let mock = Arc::new(MockSyncOps::new());
        let mut task = test_task(mock, &dir);

        task.set_last_result(MockSyncOps::pushed_result("first".into()));
        let first_id = task.last_result_id;
        task.publish();
        assert_eq!(task.last_result_id, first_id);

        task.set_last_result(MockSyncOps::pulled_result("second".into()));
        assert_ne!(task.last_result_id, first_id);
        task.clear_last_result();
        assert!(task.last_result.is_none());
        assert!(task.last_result_id.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn push_if_dirty_skips_when_nothing_changed() {
        let dir = temp_config_dir("push-clean");
        let mock = Arc::new(MockSyncOps::new());
        let mut task = test_task(mock.clone(), &dir);
        task.push_if_dirty().await;
        assert_eq!(*mock.push_calls.lock().unwrap(), 0);
        assert_eq!(task.phase, AutoSyncPhase::Watching);
        assert!(!task.dirty);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn push_if_dirty_pushes_changes_and_resamples_fingerprint() {
        let dir = temp_config_dir("push-dirty");
        let mock = Arc::new(MockSyncOps::new());
        let mut task = test_task(mock.clone(), &dir);
        mock.set_local_revision("changed");
        std::fs::write(dir.join("sessions.toml"), b"changed")
            .expect("tracked file should be writable");
        task.push_if_dirty().await;
        assert_eq!(*mock.push_calls.lock().unwrap(), 1);
        assert_eq!(task.phase, AutoSyncPhase::Watching);
        assert!(!task.dirty);
        assert_eq!(
            task.fingerprint,
            Fingerprint::sample(&dir),
            "fingerprint must be resampled after a successful push"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn push_if_dirty_does_not_override_pending_conflict() {
        let dir = temp_config_dir("push-conflict");
        let mock = Arc::new(MockSyncOps::new());
        let mut task = test_task(mock.clone(), &dir);
        task.pending_conflict = true;
        std::fs::write(dir.join("settings.toml"), b"key = 1")
            .expect("tracked file should be writable");
        task.push_if_dirty().await;
        assert_eq!(*mock.push_calls.lock().unwrap(), 0);
        assert_eq!(task.phase, AutoSyncPhase::PullRequired);
        assert!(task.dirty);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn poll_remote_pulls_when_remote_updated_and_local_clean() {
        let dir = temp_config_dir("pull-clean");
        let mock = Arc::new(MockSyncOps::new());
        *mock.remote_state_result.lock().unwrap() = RemoteSyncState::Updated {
            synced_at: 2,
            etag: Some("\"pull-etag\"".into()),
            payload_id: Some("payload-2".into()),
            content_revision: Some("remote-revision".into()),
        };
        let mut task = test_task(mock.clone(), &dir);
        task.poll_remote().await;
        assert_eq!(*mock.remote_calls.lock().unwrap(), 1);
        assert_eq!(*mock.pull_calls.lock().unwrap(), 1);
        assert_eq!(task.phase, AutoSyncPhase::Watching);
        assert!(!task.dirty);
        assert_eq!(task.engine.config_store.config.last_sync_at, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn poll_remote_acknowledges_matching_content_instead_of_reporting_conflict() {
        let dir = temp_config_dir("matching-content");
        let mock = Arc::new(MockSyncOps::new());
        mock.set_local_revision("converged-revision");
        *mock.remote_state_result.lock().unwrap() = RemoteSyncState::Updated {
            synced_at: 2,
            etag: Some("\"matching-etag\"".into()),
            payload_id: Some("different-payload-id".into()),
            content_revision: Some("converged-revision".into()),
        };
        let mut task = test_task(mock.clone(), &dir);

        task.poll_remote().await;

        assert_eq!(*mock.pull_calls.lock().unwrap(), 1);
        assert_eq!(*mock.push_calls.lock().unwrap(), 0);
        assert_eq!(task.phase, AutoSyncPhase::Watching);
        assert!(!task.pending_conflict);
        assert!(!task.dirty);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn poll_remote_pushes_when_only_local_content_changed() {
        let dir = temp_config_dir("local-only-change");
        let mock = Arc::new(MockSyncOps::new());
        mock.set_local_revision("local-change");
        *mock.remote_state_result.lock().unwrap() = RemoteSyncState::Updated {
            synced_at: 2,
            etag: Some("\"rewrapped-etag\"".into()),
            payload_id: Some("rewrapped-payload".into()),
            content_revision: Some("local-revision".into()),
        };
        let mut task = test_task(mock.clone(), &dir);

        task.poll_remote().await;

        assert_eq!(*mock.push_calls.lock().unwrap(), 1);
        assert_eq!(*mock.pull_calls.lock().unwrap(), 0);
        assert_eq!(task.phase, AutoSyncPhase::Watching);
        assert!(!task.pending_conflict);
        assert!(!task.dirty);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn poll_remote_refuses_pull_when_local_changes_after_clean_check() {
        let dir = temp_config_dir("pull-race");
        let mock = Arc::new(MockSyncOps::new());
        *mock.remote_state_result.lock().unwrap() = RemoteSyncState::Updated {
            synced_at: 2,
            etag: Some("\"pull-etag\"".into()),
            payload_id: Some("payload-2".into()),
            content_revision: Some("remote-revision".into()),
        };
        mock.change_revision_after_next_read("saved-during-pull");
        let mut task = test_task(mock.clone(), &dir);

        task.poll_remote().await;

        assert_eq!(*mock.pull_calls.lock().unwrap(), 1);
        assert_eq!(task.phase, AutoSyncPhase::PullRequired);
        assert!(task.dirty);
        assert_eq!(
            task.intervention.as_ref().map(|item| &item.reason),
            Some(&SyncInterventionReason::LocalChangedDuringPull)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn poll_remote_pauses_when_remote_updated_and_local_dirty() {
        let dir = temp_config_dir("pull-dirty");
        let mock = Arc::new(MockSyncOps::new());
        *mock.remote_state_result.lock().unwrap() = RemoteSyncState::Updated {
            synced_at: 2,
            etag: None,
            payload_id: Some("payload-2".into()),
            content_revision: Some("remote-revision".into()),
        };
        let mut task = test_task(mock.clone(), &dir);
        mock.set_local_revision("changed");
        std::fs::write(dir.join("proxies.toml"), b"changed")
            .expect("tracked file should be writable");
        task.poll_remote().await;
        assert_eq!(*mock.remote_calls.lock().unwrap(), 1);
        assert_eq!(
            *mock.pull_calls.lock().unwrap(),
            0,
            "auto-pull must be paused while local changes are pending"
        );
        assert_eq!(task.phase, AutoSyncPhase::PullRequired);
        assert!(task.dirty);
        assert!(task.pending_conflict);
        let intervention = task
            .intervention
            .as_ref()
            .expect("dirty remote update should create an intervention");
        assert_eq!(
            intervention.reason,
            SyncInterventionReason::BothSidesChanged
        );
        assert_eq!(intervention.remote_at, Some(2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn intervention_id_is_stable_until_conflict_is_cleared() {
        let dir = temp_config_dir("intervention-lifecycle");
        let mock = Arc::new(MockSyncOps::new());
        *mock.remote_state_result.lock().unwrap() = RemoteSyncState::Updated {
            synced_at: 2,
            etag: Some("\"etag-2\"".into()),
            payload_id: Some("payload-2".into()),
            content_revision: Some("remote-revision".into()),
        };
        let mut task = test_task(mock.clone(), &dir);
        mock.set_local_revision("changed");

        task.poll_remote().await;
        let first_id = task
            .intervention
            .as_ref()
            .expect("first conflict should create an intervention")
            .id
            .clone();
        task.poll_remote().await;
        assert_eq!(
            task.intervention.as_ref().map(|item| item.id.as_str()),
            Some(first_id.as_str()),
            "the same unresolved conflict must retain its event id"
        );

        task.reconcile_manual_sync(
            SyncStatus::Pulled { at: 2 },
            task.engine.clone(),
            task.settings_store.clone(),
        )
        .await;
        assert!(task.intervention.is_none());

        mock.set_local_revision("changed-again");
        task.poll_remote().await;
        let second_id = &task
            .intervention
            .as_ref()
            .expect("a later conflict should create a new intervention")
            .id;
        assert_ne!(second_id, &first_id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn missing_baseline_creates_structured_intervention() {
        let dir = temp_config_dir("missing-baseline");
        let mock = Arc::new(MockSyncOps::new());
        let mut task = test_task(mock, &dir);
        task.engine.config_store.config.last_synced_local_revision = None;

        task.poll_remote().await;

        assert_eq!(task.phase, AutoSyncPhase::PullRequired);
        assert_eq!(
            task.intervention.as_ref().map(|item| &item.reason),
            Some(&SyncInterventionReason::MissingSyncBaseline)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn on_tick_is_inert_when_disabled_or_vault_locked() {
        let dir = temp_config_dir("inert");
        let mock = Arc::new(MockSyncOps::new());
        let mut task = test_task(mock.clone(), &dir);
        std::fs::write(dir.join("sessions.toml"), b"changed")
            .expect("tracked file should be writable");

        task.enabled = false;
        task.on_tick().await;
        assert_eq!(*mock.push_calls.lock().unwrap(), 0);
        assert_eq!(*mock.remote_calls.lock().unwrap(), 0);

        task.enabled = true;
        task.vault_locked = true;
        task.on_tick().await;
        assert_eq!(*mock.push_calls.lock().unwrap(), 0);
        assert_eq!(*mock.remote_calls.lock().unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn on_tick_is_inert_while_manual_intervention_is_pending() {
        let dir = temp_config_dir("pending-intervention");
        let mock = Arc::new(MockSyncOps::new());
        let mut task = test_task(mock.clone(), &dir);
        task.enter_intervention(SyncInterventionReason::BothSidesChanged, Some(2));
        let intervention_id = task
            .intervention
            .as_ref()
            .expect("intervention should exist")
            .id
            .clone();

        task.on_tick().await;

        assert_eq!(*mock.push_calls.lock().unwrap(), 0);
        assert_eq!(*mock.remote_calls.lock().unwrap(), 0);
        assert_eq!(task.phase, AutoSyncPhase::PullRequired);
        assert_eq!(
            task.intervention.as_ref().map(|item| item.id.as_str()),
            Some(intervention_id.as_str())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn poll_tick_and_file_events_respect_retry_backoff() {
        let dir = temp_config_dir("backoff-events");
        let mock = Arc::new(MockSyncOps::new());
        let mut task = test_task(mock.clone(), &dir);
        mock.set_local_revision("changed");
        task.schedule_retry(RetryAction::Push, anyhow::anyhow!("temporary failure"));
        let retry_deadline = task.retry_deadline;
        let mut debounce_deadline = None;

        task.schedule_debounced_push(&mut debounce_deadline);
        task.on_tick().await;

        assert!(debounce_deadline.is_none());
        assert_eq!(task.retry_deadline, retry_deadline);
        assert_eq!(task.phase, AutoSyncPhase::RetryBackoff);
        assert_eq!(*mock.push_calls.lock().unwrap(), 0);
        assert_eq!(*mock.remote_calls.lock().unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn schedule_retry_backoff_doubles_then_caps() {
        let dir = temp_config_dir("backoff");
        let mock = Arc::new(MockSyncOps::new());
        let mut task = test_task(mock, &dir);
        assert_eq!(task.backoff_delay.as_secs(), 30);
        let expected = [30u64, 60, 120, 240, 480, 600, 600];
        for (index, seconds) in expected.iter().enumerate() {
            task.schedule_retry(RetryAction::Poll, anyhow::anyhow!("failure {index}"));
            let next = expected.get(index + 1).copied().unwrap_or(600);
            assert_eq!(
                task.backoff_delay.as_secs(),
                next,
                "scheduled delay {seconds}s must double to {next}s after failure {index}"
            );
            assert_eq!(task.phase, AutoSyncPhase::RetryBackoff);
            assert!(task.retry_at_unix.is_some());
        }
        task.reset_backoff();
        assert!(task.message.is_none());
        assert!(task.retry_at_unix.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn apply_engine_toggles_enabled_and_phase() {
        let dir = temp_config_dir("apply-engine");
        let mock = Arc::new(MockSyncOps::new());
        let mut task = test_task(mock, &dir);

        let disabled_engine = SyncEngine {
            config_store: SyncConfigStore::with_credentials(
                dir.join("sync_config.toml"),
                SyncConfig {
                    auto_sync_enabled: false,
                    ..SyncConfig::default()
                },
                memory_credentials(),
            ),
        };
        task.apply_engine(disabled_engine).await;
        assert!(!task.enabled);
        assert_eq!(task.phase, AutoSyncPhase::Disabled);

        let enabled_engine = SyncEngine {
            config_store: SyncConfigStore::with_credentials(
                dir.join("sync_config.toml"),
                SyncConfig {
                    auto_sync_enabled: true,
                    ..SyncConfig::default()
                },
                memory_credentials(),
            ),
        };
        task.apply_engine(enabled_engine).await;
        assert!(task.enabled);
        assert_eq!(task.phase, AutoSyncPhase::Watching);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn apply_engine_resets_pending_conflict_and_resamples_fingerprint() {
        let dir = temp_config_dir("apply-engine-conflict");
        let mock = Arc::new(MockSyncOps::new());
        let mut task = test_task(mock, &dir);
        task.pending_conflict = true;
        task.dirty = true;
        let fingerprint = task.fingerprint.clone();
        task.executor.set_local_revision("changed");
        std::fs::write(dir.join("sessions.toml"), b"changed")
            .expect("tracked file should be writable");

        let engine = SyncEngine {
            config_store: SyncConfigStore::with_credentials(
                dir.join("sync_config.toml"),
                SyncConfig {
                    auto_sync_enabled: true,
                    ..SyncConfig::default()
                },
                memory_credentials(),
            ),
        };
        task.apply_engine(engine).await;

        assert!(!task.pending_conflict);
        assert!(task.intervention.is_none());
        assert!(task.dirty);
        assert_ne!(task.fingerprint, fingerprint);
        assert_eq!(task.phase, AutoSyncPhase::Watching);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn manual_sync_reconciliation_clears_conflict_after_pull() {
        let dir = temp_config_dir("manual-sync-pulled");
        let mock = Arc::new(MockSyncOps::new());
        let mut task = test_task(mock, &dir);
        task.pending_conflict = true;
        task.dirty = true;
        task.schedule_retry(RetryAction::Poll, anyhow::anyhow!("temporary failure"));
        std::fs::write(dir.join("sessions.toml"), b"pulled")
            .expect("tracked file should be writable");

        task.reconcile_manual_sync(
            SyncStatus::Pulled { at: 2 },
            task.engine.clone(),
            task.settings_store.clone(),
        )
        .await;

        assert!(!task.pending_conflict);
        assert!(task.intervention.is_none());
        assert!(!task.dirty);
        assert!(task.message.is_none());
        assert_eq!(task.fingerprint, Fingerprint::sample(&dir));
        assert_eq!(task.phase, AutoSyncPhase::Watching);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn manual_up_to_date_clears_conflict_but_keeps_local_changes_dirty() {
        let dir = temp_config_dir("manual-sync-up-to-date");
        let mock = Arc::new(MockSyncOps::new());
        let mut task = test_task(mock, &dir);
        task.pending_conflict = true;
        task.dirty = true;
        task.executor.set_local_revision("changed");
        std::fs::write(dir.join("settings.toml"), b"local-change")
            .expect("tracked file should be writable");

        task.reconcile_manual_sync(
            SyncStatus::UpToDate { at: 1 },
            task.engine.clone(),
            task.settings_store.clone(),
        )
        .await;

        assert!(!task.pending_conflict);
        assert!(task.dirty);
        assert_eq!(task.phase, AutoSyncPhase::Watching);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn run_auto_sync_without_baseline_checks_remote_before_pushing() {
        let dir = temp_config_dir("loop");
        let mock = Arc::new(MockSyncOps::new());
        *mock.remote_state_result.lock().unwrap() = RemoteSyncState::Updated {
            synced_at: 2,
            etag: Some("\"loop-etag\"".into()),
            payload_id: Some("payload-2".into()),
            content_revision: Some("remote-revision".into()),
        };
        let settings_store = SettingsStore::load_with_path(dir.join("settings.toml"))
            .expect("test settings store should load");
        let engine = SyncEngine {
            config_store: SyncConfigStore::with_credentials(
                dir.join("sync_config.toml"),
                SyncConfig {
                    provider: SyncProvider::GithubGist,
                    auto_sync_enabled: true,
                    ..SyncConfig::default()
                },
                memory_credentials(),
            ),
        };
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (state_tx, _state_rx) = watch::channel(snapshot(true, AutoSyncPhase::Watching));
        let handle = tokio::spawn(run_auto_sync(
            command_rx,
            state_tx,
            mock.clone(),
            settings_store,
            engine,
            dir.clone(),
            false,
            AUTO_SYNC_POLL_INTERVAL,
        ));

        for _ in 0..100 {
            if *mock.remote_calls.lock().unwrap() >= 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            *mock.remote_calls.lock().unwrap() >= 1,
            "startup without a local baseline must inspect remote state first"
        );
        assert_eq!(
            *mock.push_calls.lock().unwrap(),
            0,
            "an existing remote must not be overwritten without a local baseline"
        );
        assert_eq!(
            *mock.pull_calls.lock().unwrap(),
            0,
            "an existing remote and missing local baseline require an explicit direction"
        );

        command_tx
            .send(AutoSyncCommand::Shutdown)
            .expect("shutdown command should send");
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn watcher_start_failure_does_not_starve_poll_timer() {
        let dir = temp_config_dir("watcher-failure");
        let settings_store = SettingsStore::load_with_path(dir.join("settings.toml"))
            .expect("test settings store should load");
        let engine = SyncEngine {
            config_store: SyncConfigStore::with_credentials(
                dir.join("sync_config.toml"),
                SyncConfig {
                    provider: SyncProvider::GithubGist,
                    auto_sync_enabled: true,
                    last_synced_local_revision: Some("local-revision".into()),
                    ..SyncConfig::default()
                },
                memory_credentials(),
            ),
        };
        std::fs::remove_dir_all(&dir).expect("watch directory should be removed");
        let mock = Arc::new(MockSyncOps::new());
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (state_tx, _state_rx) = watch::channel(snapshot(true, AutoSyncPhase::Watching));
        let handle = tokio::spawn(run_auto_sync(
            command_rx,
            state_tx,
            mock.clone(),
            settings_store,
            engine,
            dir,
            false,
            Duration::from_millis(20),
        ));

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if *mock.remote_calls.lock().unwrap() >= 2 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("periodic polling should continue without a watcher");

        command_tx
            .send(AutoSyncCommand::Shutdown)
            .expect("shutdown command should send");
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("auto-sync task should stop")
            .expect("auto-sync task should not panic");
    }
}
