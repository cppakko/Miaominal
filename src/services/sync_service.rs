use crate::domain::keychain::ManagedKeyRecord;
use crate::domain::profile::SessionProfile;
use crate::domain::snippet::SnippetRecord;
use crate::domain::sync::{SyncConfig, SyncStatus};
use crate::infra::config_store::store::{SessionStore, SnippetStore};
use crate::infra::keychain_store::ManagedKeyStore;
use crate::infra::sync::engine::SyncEngine;
use crate::secrets::SecretStore;
use crate::settings::SettingsStore;
use anyhow::{Result, anyhow};
use tokio::runtime::Handle as TokioHandle;

#[derive(Debug, Clone)]
pub(crate) struct SyncTaskResult {
    pub status: SyncStatus,
    pub updated_config: SyncConfig,
}

#[derive(Clone, Debug)]
pub(crate) struct SyncService {
    runtime: TokioHandle,
    session_store: SessionStore,
    snippet_store: SnippetStore,
    keychain_store: ManagedKeyStore,
    secrets: SecretStore,
}

impl SyncService {
    pub(crate) fn new(
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

    pub(crate) fn runtime(&self) -> &TokioHandle {
        &self.runtime
    }

    pub(crate) async fn push(
        &self,
        mut engine: SyncEngine,
        settings_store: SettingsStore,
    ) -> Result<SyncTaskResult> {
        self.push_inner(&mut engine, settings_store, false).await
    }

    pub(crate) async fn push_force(
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
        })
    }

    pub(crate) async fn pull(
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
        Ok(SyncTaskResult {
            status,
            updated_config: engine.config_store.config.clone(),
        })
    }

    pub(crate) fn reload_sessions(&self) -> Result<Vec<SessionProfile>> {
        self.session_store.load(&self.secrets)
    }

    pub(crate) fn reload_snippets(&self) -> Result<Vec<SnippetRecord>> {
        self.snippet_store.load()
    }

    pub(crate) fn reload_managed_keys(&self) -> Result<Vec<ManagedKeyRecord>> {
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
