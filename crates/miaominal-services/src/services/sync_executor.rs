use super::sync_service::{SyncService, SyncTaskResult};
use miaominal_secrets::SecretStore;
use miaominal_storage::SettingsStore;
use miaominal_sync::capability::{CapabilityReport, ProbeCancellation};
use miaominal_sync::{RemoteSyncState, SyncEngine};

/// Abstraction over the sync operations used by the auto-sync scheduler.
///
/// The production implementation delegates to `SyncService`, which serializes
/// every operation through one process-wide mutex; tests can substitute an
/// in-memory mock.
pub trait SyncOps: Send + Sync + 'static {
    fn check_capability(
        &self,
        engine: SyncEngine,
        cancel: ProbeCancellation,
    ) -> impl std::future::Future<Output = CapabilityReport> + Send;
    fn push(
        &self,
        engine: SyncEngine,
        settings_store: SettingsStore,
    ) -> impl std::future::Future<Output = anyhow::Result<SyncTaskResult>> + Send;

    fn pull_if_unchanged(
        &self,
        engine: SyncEngine,
        settings_store: SettingsStore,
        expected_local_revision: String,
    ) -> impl std::future::Future<Output = anyhow::Result<SyncTaskResult>> + Send;
    fn remote_state(
        &self,
        engine: SyncEngine,
    ) -> impl std::future::Future<Output = anyhow::Result<RemoteSyncState>> + Send;

    fn local_revision(
        &self,
        engine: SyncEngine,
        settings_store: SettingsStore,
    ) -> impl std::future::Future<Output = anyhow::Result<String>> + Send;
}

/// Shared sync operation facade used by manual buttons and auto-sync.
#[derive(Clone, Debug)]
pub struct SyncExecutor {
    service: SyncService,
}

impl SyncExecutor {
    pub fn new(service: SyncService) -> Self {
        Self { service }
    }

    pub fn replace_secrets(&self, secrets: SecretStore) {
        self.service.replace_secrets(secrets);
    }

    pub async fn check_capability(
        &self,
        engine: SyncEngine,
        cancel: ProbeCancellation,
    ) -> CapabilityReport {
        self.service.check_capability(engine, cancel).await
    }

    pub async fn push(
        &self,
        engine: SyncEngine,
        settings_store: SettingsStore,
    ) -> anyhow::Result<SyncTaskResult> {
        self.service.push(engine, settings_store).await
    }

    pub async fn push_manual(
        &self,
        engine: SyncEngine,
        settings_store: SettingsStore,
    ) -> anyhow::Result<SyncTaskResult> {
        self.service.push_manual(engine, settings_store).await
    }

    pub async fn push_force(
        &self,
        engine: SyncEngine,
        settings_store: SettingsStore,
    ) -> anyhow::Result<SyncTaskResult> {
        self.service.push_force(engine, settings_store).await
    }

    pub async fn pull(
        &self,
        engine: SyncEngine,
        settings_store: SettingsStore,
    ) -> anyhow::Result<SyncTaskResult> {
        self.service.pull(engine, settings_store).await
    }

    pub async fn pull_force(
        &self,
        engine: SyncEngine,
        settings_store: SettingsStore,
    ) -> anyhow::Result<SyncTaskResult> {
        self.service.pull_force(engine, settings_store).await
    }

    pub async fn remote_state(&self, engine: SyncEngine) -> anyhow::Result<RemoteSyncState> {
        self.service.remote_state(engine).await
    }

    pub async fn local_revision(
        &self,
        engine: SyncEngine,
        settings_store: SettingsStore,
    ) -> anyhow::Result<String> {
        self.service.local_revision(engine, settings_store).await
    }
}

impl SyncOps for SyncExecutor {
    async fn check_capability(
        &self,
        engine: SyncEngine,
        cancel: ProbeCancellation,
    ) -> CapabilityReport {
        SyncExecutor::check_capability(self, engine, cancel).await
    }
    async fn push(
        &self,
        engine: SyncEngine,
        settings_store: SettingsStore,
    ) -> anyhow::Result<SyncTaskResult> {
        SyncExecutor::push(self, engine, settings_store).await
    }

    async fn pull_if_unchanged(
        &self,
        engine: SyncEngine,
        settings_store: SettingsStore,
        expected_local_revision: String,
    ) -> anyhow::Result<SyncTaskResult> {
        self.service
            .pull_if_unchanged(engine, settings_store, expected_local_revision)
            .await
    }

    async fn remote_state(&self, engine: SyncEngine) -> anyhow::Result<RemoteSyncState> {
        SyncExecutor::remote_state(self, engine).await
    }

    async fn local_revision(
        &self,
        engine: SyncEngine,
        settings_store: SettingsStore,
    ) -> anyhow::Result<String> {
        SyncExecutor::local_revision(self, engine, settings_store).await
    }
}
