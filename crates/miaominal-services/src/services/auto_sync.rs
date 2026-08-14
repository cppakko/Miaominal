use super::sync_executor::{SyncExecutor, SyncOps};
use super::sync_service::SyncTaskResult;
use anyhow::Result;
use miaominal_paths as paths;
use miaominal_storage::SettingsStore;
use miaominal_sync::{
    RemoteSyncState, SyncEngine, SyncInterventionReason, SyncProvider, SyncStatus,
};
use notify::{RecursiveMode, Watcher};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
    SetEngine(SyncEngine),
    SetSettingsStore(SettingsStore),
    ReconcileManualSync {
        status: SyncStatus,
        engine: SyncEngine,
        settings_store: SettingsStore,
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
    runtime: TokioHandle,
    command_tx: mpsc::UnboundedSender<AutoSyncCommand>,
    state_rx: watch::Receiver<AutoSyncSnapshot>,
    task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
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
            runtime,
            command_tx,
            state_rx,
            task,
        };
        service.spawn_task(
            command_rx,
            state_tx,
            executor,
            settings_store,
            engine,
            config_dir,
            vault_locked,
        );
        service
    }

    fn spawn_task<S: SyncOps>(
        &self,
        command_rx: mpsc::UnboundedReceiver<AutoSyncCommand>,
        state_tx: watch::Sender<AutoSyncSnapshot>,
        executor: S,
        settings_store: SettingsStore,
        engine: SyncEngine,
        config_dir: PathBuf,
        vault_locked: bool,
    ) {
        let runtime = self.runtime.clone();
        let task = self.task.clone();
        let handle = runtime.spawn(async move {
            run_auto_sync(
                command_rx,
                state_tx,
                executor,
                settings_store,
                engine,
                config_dir,
                vault_locked,
            )
            .await;
        });
        let mut slot = task.try_lock().expect("auto-sync task slot should lock");
        if let Some(previous) = slot.take() {
            previous.abort();
        }
        *slot = Some(handle);
    }

    pub fn subscribe(&self) -> watch::Receiver<AutoSyncSnapshot> {
        self.state_rx.clone()
    }

    pub fn set_engine(&self, engine: SyncEngine) {
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
            settings_store,
        });
    }

    pub fn set_vault_locked(&self, locked: bool) {
        let _ = self
            .command_tx
            .send(AutoSyncCommand::SetVaultLocked(locked));
    }

    pub fn wake(&self) {
        let _ = self.command_tx.send(AutoSyncCommand::Wake);
    }

    pub fn shutdown(&self) {
        let _ = self.command_tx.send(AutoSyncCommand::Shutdown);
        if let Ok(mut slot) = self.task.try_lock() {
            if let Some(handle) = slot.take() {
                handle.abort();
            }
        }
    }
}

struct AutoSyncTask<S: SyncOps> {
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
    fn publish(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        let _ = self.state_tx.send(AutoSyncSnapshot {
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
        let delay = self.backoff_delay;
        self.backoff_delay = (delay * 2).min(AUTO_SYNC_BACKOFF_MAX);
        self.retry_action = action;
        self.retry_deadline = Some(Instant::now() + delay);
        self.retry_at_unix = Some(unix_now().saturating_add(delay.as_secs()));
        self.message = Some(error.to_string());
        self.phase = AutoSyncPhase::RetryBackoff;
        self.publish();
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
                    self.schedule_retry(RetryAction::Push, error);
                    return;
                }
            }
        }
    }

    async fn poll_remote(&mut self) {
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
            Ok(RemoteSyncState::Updated { synced_at, .. }) => {
                if self
                    .engine
                    .config_store
                    .config
                    .last_synced_local_revision
                    .is_none()
                {
                    self.dirty = true;
                    self.enter_intervention(
                        SyncInterventionReason::MissingSyncBaseline,
                        Some(synced_at),
                    );
                    return;
                }
                if let Err(error) = self.refresh_dirty_from_revision().await {
                    self.dirty = true;
                    self.schedule_retry(RetryAction::Poll, error);
                    return;
                }
                if self.dirty {
                    self.dirty = true;
                    self.enter_intervention(
                        SyncInterventionReason::BothSidesChanged,
                        Some(synced_at),
                    );
                    return;
                }
                self.set_phase(AutoSyncPhase::Pulling);
                let engine = self.engine.clone();
                let settings_store = self.settings_store.clone();
                match self.executor.pull(engine, settings_store).await {
                    Ok(result) => {
                        self.set_last_result(result.clone());
                        self.engine.config_store.config = result.updated_config;
                        if let Some(reload) = &result.reload
                            && let Ok(store) = &reload.settings
                        {
                            self.settings_store = store.clone();
                        }
                        match result.status {
                            SyncStatus::Pulled { .. } => {
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
        if !self.enabled || self.vault_locked {
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

async fn run_auto_sync<S: SyncOps>(
    mut command_rx: mpsc::UnboundedReceiver<AutoSyncCommand>,
    state_tx: watch::Sender<AutoSyncSnapshot>,
    executor: S,
    settings_store: SettingsStore,
    engine: SyncEngine,
    config_dir: PathBuf,
    vault_locked: bool,
) {
    let enabled = engine.config_store.config.auto_sync_enabled;
    let fingerprint = Fingerprint::sample(&config_dir);
    let mut task = AutoSyncTask {
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
    let _watcher = match start_file_watcher(&config_dir, event_tx) {
        Ok(watcher) => watcher,
        Err(error) => {
            log::warn!("auto-sync file watcher unavailable: {error:?}");
            None
        }
    };

    let mut debounce_deadline: Option<Instant> = None;
    let mut tick = Box::pin(tokio::time::sleep(AUTO_SYNC_POLL_INTERVAL));

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
                        task.reconcile_manual_sync(status, engine, settings_store).await;
                    }
                    AutoSyncCommand::SetVaultLocked(locked) => {
                        task.vault_locked = locked;
                        if locked {
                            task.set_phase(AutoSyncPhase::PausedVaultLocked);
                        } else if task.enabled {
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
            _ = event_rx.recv(), if task.enabled => {
                if !task.vault_locked {
                    debounce_deadline = Some(Instant::now() + AUTO_SYNC_DEBOUNCE);
                    task.set_phase(AutoSyncPhase::Debouncing);
                }
            }
            _ = &mut debounce, if debounce_pending && task.enabled && !task.vault_locked => {
                debounce_deadline = None;
                task.push_if_dirty().await;
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
            _ = &mut tick, if task.enabled && !task.vault_locked => {
                tick = Box::pin(tokio::time::sleep(AUTO_SYNC_POLL_INTERVAL));
                task.on_tick().await;
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
        push_calls: Mutex<usize>,
        pull_calls: Mutex<usize>,
        remote_calls: Mutex<usize>,
        remote_state_result: Mutex<RemoteSyncState>,
        local_revision_result: Mutex<String>,
    }
    impl MockSyncOps {
        fn new() -> Self {
            Self {
                push_calls: Mutex::new(0),
                pull_calls: Mutex::new(0),
                remote_calls: Mutex::new(0),
                remote_state_result: Mutex::new(RemoteSyncState::UpToDate),
                local_revision_result: Mutex::new("local-revision".into()),
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
    }

    impl SyncOps for Arc<MockSyncOps> {
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

        async fn pull(
            &self,
            _engine: SyncEngine,
            _settings_store: SettingsStore,
        ) -> anyhow::Result<SyncTaskResult> {
            *self.pull_calls.lock().expect("pull counter should lock") += 1;
            Ok(MockSyncOps::pulled_result(
                self.local_revision_result.lock().unwrap().clone(),
            ))
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
            Ok(self.local_revision_result.lock().unwrap().clone())
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
        let (state_tx, _state_rx) = watch::channel(snapshot(true, AutoSyncPhase::Watching));
        AutoSyncTask {
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
    async fn poll_remote_pauses_when_remote_updated_and_local_dirty() {
        let dir = temp_config_dir("pull-dirty");
        let mock = Arc::new(MockSyncOps::new());
        *mock.remote_state_result.lock().unwrap() = RemoteSyncState::Updated {
            synced_at: 2,
            etag: None,
            payload_id: Some("payload-2".into()),
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
}
