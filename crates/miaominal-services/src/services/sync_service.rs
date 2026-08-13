use anyhow::{Result, anyhow};
use miaominal_core::keychain::ManagedKeyRecord;
use miaominal_core::profile::SessionProfile;
use miaominal_core::proxy::ProxyProfile;
use miaominal_core::snippet::SnippetRecord;
use miaominal_secrets::SecretStore;
use miaominal_storage::config_store::store::{SessionStore, SnippetStore};
use miaominal_storage::keychain_store::ManagedKeyStore;
use miaominal_storage::{ProxyStore, SettingsStore};
use miaominal_sync::engine::SyncEngine;
use miaominal_sync::{RemoteSyncState, SyncConfig, SyncStatus};
use std::sync::{Arc, OnceLock, RwLock};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::Mutex;

fn process_sync_lock() -> Arc<Mutex<()>> {
    static LOCK: OnceLock<Arc<Mutex<()>>> = OnceLock::new();
    LOCK.get_or_init(|| Arc::new(Mutex::new(()))).clone()
}

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
    pub proxies: Result<Vec<ProxyProfile>, String>,
    pub snippets: Result<Vec<SnippetRecord>, String>,
    pub managed_keys: Result<Vec<ManagedKeyRecord>, String>,
}

impl SyncReloadResult {
    pub fn any_failed(&self) -> bool {
        self.settings.is_err()
            || self.sessions.is_err()
            || self.proxies.is_err()
            || self.snippets.is_err()
            || self.managed_keys.is_err()
    }
}

#[derive(Clone, Debug)]
pub struct SyncService {
    runtime: TokioHandle,
    session_store: SessionStore,
    proxy_store: ProxyStore,
    snippet_store: SnippetStore,
    keychain_store: ManagedKeyStore,
    secrets: Arc<RwLock<SecretStore>>,
    operation_lock: Arc<Mutex<()>>,
}

impl SyncService {
    pub fn new(
        runtime: TokioHandle,
        session_store: Option<SessionStore>,
        proxy_store: Option<ProxyStore>,
        snippet_store: Option<SnippetStore>,
        keychain_store: Option<ManagedKeyStore>,
        secrets: SecretStore,
    ) -> Result<Self> {
        Ok(Self {
            runtime,
            session_store: session_store.ok_or_else(|| anyhow!("session store unavailable"))?,
            proxy_store: proxy_store.ok_or_else(|| anyhow!("proxy store unavailable"))?,
            snippet_store: snippet_store.ok_or_else(|| anyhow!("snippet store unavailable"))?,
            keychain_store: keychain_store
                .ok_or_else(|| anyhow!("managed key store unavailable"))?,
            secrets: Arc::new(RwLock::new(secrets)),
            operation_lock: process_sync_lock(),
        })
    }

    pub fn runtime(&self) -> &TokioHandle {
        &self.runtime
    }

    pub fn replace_secrets(&self, secrets: SecretStore) {
        *self
            .secrets
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = secrets;
    }

    fn secrets(&self) -> SecretStore {
        self.secrets
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub async fn push(
        &self,
        mut engine: SyncEngine,
        settings_store: SettingsStore,
    ) -> Result<SyncTaskResult> {
        let _guard = self.operation_lock.lock().await;
        self.push_inner(&mut engine, settings_store, false).await
    }

    pub async fn push_force(
        &self,
        mut engine: SyncEngine,
        settings_store: SettingsStore,
    ) -> Result<SyncTaskResult> {
        let _guard = self.operation_lock.lock().await;
        self.push_inner(&mut engine, settings_store, true).await
    }

    async fn push_inner(
        &self,
        engine: &mut SyncEngine,
        settings_store: SettingsStore,
        force: bool,
    ) -> Result<SyncTaskResult> {
        let secrets = self.secrets();
        let status = if force {
            engine
                .push_force(
                    &self.session_store,
                    &self.proxy_store,
                    &self.snippet_store,
                    &self.keychain_store,
                    &secrets,
                    &settings_store,
                )
                .await?
        } else {
            engine
                .push(
                    &self.session_store,
                    &self.proxy_store,
                    &self.snippet_store,
                    &self.keychain_store,
                    &secrets,
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

    /// Lightweight remote check used by auto-sync polling: reports
    /// whether the remote payload is newer without applying it locally.
    pub async fn remote_state(&self, engine: SyncEngine) -> Result<RemoteSyncState> {
        let _guard = self.operation_lock.lock().await;
        let mut engine = engine;
        engine.remote_state().await
    }

    pub async fn local_revision(
        &self,
        engine: SyncEngine,
        settings_store: SettingsStore,
    ) -> Result<String> {
        let _guard = self.operation_lock.lock().await;
        let secrets = self.secrets();
        engine.local_revision(
            &self.session_store,
            &self.proxy_store,
            &self.snippet_store,
            &self.keychain_store,
            &secrets,
            &settings_store,
        )
    }

    pub async fn pull(
        &self,
        mut engine: SyncEngine,
        mut settings_store: SettingsStore,
    ) -> Result<SyncTaskResult> {
        let _guard = self.operation_lock.lock().await;
        let secrets = self.secrets();
        let status = engine
            .pull(
                &self.session_store,
                &self.proxy_store,
                &self.snippet_store,
                &self.keychain_store,
                &secrets,
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
            proxies: self.reload_proxies().map_err(|error| error.to_string()),
            snippets: self.reload_snippets().map_err(|error| error.to_string()),
            managed_keys: self
                .reload_managed_keys()
                .map_err(|error| error.to_string()),
        }
    }

    pub fn reload_sessions(&self) -> Result<Vec<SessionProfile>> {
        self.session_store.load(&self.secrets())
    }

    pub fn reload_proxies(&self) -> Result<Vec<ProxyProfile>> {
        self.proxy_store.load(&self.secrets())
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
    use miaominal_core::profile::SessionProfile;
    use miaominal_secrets::{
        APP_CREDENTIAL_SERVICE, CredentialStore, SecretKind, VaultCredentialBackend,
        set_vault_test_parameters,
    };
    use miaominal_sync::SyncConfigStore;
    use tempfile::tempdir;

    #[test]
    fn sync_service_requires_all_stores() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime should build");
        let error = SyncService::new(
            runtime.handle().clone(),
            None,
            None,
            None,
            None,
            SecretStore::new_locked_vault(),
        )
        .expect_err("missing stores should fail");

        assert!(error.to_string().contains("session store unavailable"));
    }

    #[tokio::test]
    async fn replacing_secrets_updates_existing_service_clones_after_vault_unlock() {
        set_vault_test_parameters();
        let temp = tempdir().expect("temporary directory should exist");
        let session_store = SessionStore::with_path(temp.path().join("sessions.toml"));
        let mut session = SessionProfile::blank("session-1", 1);
        session.has_stored_password = true;
        session_store
            .save(&[session])
            .expect("session should persist");

        let runtime = tokio::runtime::Handle::current();
        let service = SyncService::new(
            runtime,
            Some(session_store),
            Some(ProxyStore::with_path(temp.path().join("proxies.toml"))),
            Some(SnippetStore::with_path(temp.path().join("snippets.toml"))),
            Some(ManagedKeyStore::with_path(
                temp.path().join("managed_keys.toml"),
            )),
            SecretStore::new_locked_vault(),
        )
        .expect("sync service should build");
        let cloned_service = service.clone();

        let credentials = CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new_with_path(
                temp.path().join("secret_vault.json"),
                miaominal_secrets::ProtectedPassphrase::try_from_string(
                    "vault-passphrase".to_string(),
                )
                .expect("passphrase should be valid"),
            ),
        );
        credentials
            .initialize()
            .expect("vault backend should initialize");
        let unlocked_secrets = SecretStore::with_credentials(credentials.clone());
        unlocked_secrets
            .set("session-1", SecretKind::Password, "password")
            .expect("password should persist");

        let engine = SyncEngine {
            config_store: SyncConfigStore::with_credentials(
                temp.path().join("sync_config.toml"),
                SyncConfig::default(),
                credentials,
            ),
        };
        let settings_store = SettingsStore::load_with_path(temp.path().join("settings.toml"))
            .expect("settings should load");

        assert!(
            cloned_service
                .local_revision(engine.clone(), settings_store.clone())
                .await
                .is_err(),
            "the startup locked backend should reject secret reads"
        );

        service.replace_secrets(unlocked_secrets);

        cloned_service
            .local_revision(engine, settings_store)
            .await
            .expect("the existing clone should use the unlocked backend");
    }
}
