use anyhow::{Result, anyhow};
use miaominal_core::keychain::ManagedKeyRecord;
use miaominal_core::profile::SessionProfile;
use miaominal_core::snippet::SnippetRecord;
use miaominal_secrets::SecretStore;
use miaominal_storage::SettingsStore;
use miaominal_storage::config_store::store::{SessionStore, SnippetStore};
use miaominal_storage::keychain_store::ManagedKeyStore;
use miaominal_sync::engine::SyncEngine;
use miaominal_sync::{SyncConfig, SyncStatus};
use tokio::runtime::Handle as TokioHandle;

#[derive(Debug, Clone)]
pub struct SyncTaskResult {
    pub status: SyncStatus,
    pub updated_config: SyncConfig,
    pub reload: Option<SyncReloadResult>,
}

#[derive(Debug, Clone)]
pub struct SyncReloadResult {
    pub settings: Result<SettingsStore, String>,
    pub sessions: Result<Vec<SessionProfile>, String>,
    pub snippets: Result<Vec<SnippetRecord>, String>,
    pub managed_keys: Result<Vec<ManagedKeyRecord>, String>,
}

impl SyncReloadResult {
    pub fn any_failed(&self) -> bool {
        self.settings.is_err()
            || self.sessions.is_err()
            || self.snippets.is_err()
            || self.managed_keys.is_err()
    }
}

#[derive(Clone, Debug)]
pub struct SyncService {
    runtime: TokioHandle,
    session_store: SessionStore,
    snippet_store: SnippetStore,
    keychain_store: ManagedKeyStore,
    secrets: SecretStore,
}

impl SyncService {
    pub fn new(
        runtime: TokioHandle,
        session_store: Option<SessionStore>,
        snippet_store: Option<SnippetStore>,
        keychain_store: Option<ManagedKeyStore>,
        secrets: SecretStore,
    ) -> Result<Self> {
        Ok(Self {
            runtime,
            session_store: session_store.ok_or_else(|| anyhow!("session store unavailable"))?,
            snippet_store: snippet_store.ok_or_else(|| anyhow!("snippet store unavailable"))?,
            keychain_store: keychain_store
                .ok_or_else(|| anyhow!("managed key store unavailable"))?,
            secrets,
        })
    }

    pub fn runtime(&self) -> &TokioHandle {
        &self.runtime
    }

    pub async fn push(
        &self,
        mut engine: SyncEngine,
        settings_store: SettingsStore,
    ) -> Result<SyncTaskResult> {
        self.push_inner(&mut engine, settings_store, false).await
    }

    pub async fn push_force(
        &self,
        mut engine: SyncEngine,
        settings_store: SettingsStore,
    ) -> Result<SyncTaskResult> {
        self.push_inner(&mut engine, settings_store, true).await
    }

    async fn push_inner(
        &self,
        engine: &mut SyncEngine,
        settings_store: SettingsStore,
        force: bool,
    ) -> Result<SyncTaskResult> {
        let status = if force {
            engine
                .push_force(
                    &self.session_store,
                    &self.snippet_store,
                    &self.keychain_store,
                    &self.secrets,
                    &settings_store,
                )
                .await?
        } else {
            engine
                .push(
                    &self.session_store,
                    &self.snippet_store,
                    &self.keychain_store,
                    &self.secrets,
                    &settings_store,
                )
                .await?
        };
        Ok(SyncTaskResult {
            status,
            updated_config: engine.config_store.config.clone(),
            reload: None,
        })
    }

    pub async fn pull(
        &self,
        mut engine: SyncEngine,
        mut settings_store: SettingsStore,
    ) -> Result<SyncTaskResult> {
        let status = engine
            .pull(
                &self.session_store,
                &self.snippet_store,
                &self.keychain_store,
                &self.secrets,
                &mut settings_store,
            )
            .await?;
        let reload = matches!(status, SyncStatus::Pulled { .. }).then(|| self.reload_all());
        Ok(SyncTaskResult {
            status,
            updated_config: engine.config_store.config.clone(),
            reload,
        })
    }

    pub fn reload_all(&self) -> SyncReloadResult {
        SyncReloadResult {
            settings: SettingsStore::load().map_err(|error| error.to_string()),
            sessions: self.reload_sessions().map_err(|error| error.to_string()),
            snippets: self.reload_snippets().map_err(|error| error.to_string()),
            managed_keys: self
                .reload_managed_keys()
                .map_err(|error| error.to_string()),
        }
    }

    pub fn reload_sessions(&self) -> Result<Vec<SessionProfile>> {
        self.session_store.load(&self.secrets)
    }

    pub fn reload_snippets(&self) -> Result<Vec<SnippetRecord>> {
        self.snippet_store.load()
    }

    pub fn reload_managed_keys(&self) -> Result<Vec<ManagedKeyRecord>> {
        self.keychain_store.load()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_service_requires_all_stores() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime should build");
        let error = SyncService::new(
            runtime.handle().clone(),
            None,
            None,
            None,
            SecretStore::new_locked_vault(),
        )
        .expect_err("missing stores should fail");

        assert!(error.to_string().contains("session store unavailable"));
    }
}
