#[cfg(test)]
use anyhow::Context as _;
use anyhow::Result;
use miaominal_paths as paths;
use miaominal_secrets::{
    APP_CREDENTIAL_SERVICE, CredentialStore, ProtectedPassphrase, SecretKind, SecretStore,
    VaultCredentialBackend,
};
use miaominal_settings::{
    AiProviderConfig, FONT_SIZE_MAX, FONT_SIZE_MIN, LINE_HEIGHT_MAX, LINE_HEIGHT_MIN,
    WebSearchConfig,
};
use miaominal_storage::SettingsStore;
use miaominal_sync::SyncProvider;
use miaominal_sync::{credential_migration, engine::SyncEngine};
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::io::ErrorKind;
#[cfg(test)]
use std::path::Path;

use crate::ChatService;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalVaultMode {
    Disabled,
    Locked,
    Unlocked,
}

#[derive(Clone)]
pub struct LocalVaultTransition {
    pub secrets: SecretStore,
    pub sync_engine: SyncEngine,
    pub mode: LocalVaultMode,
    pub session_passphrase: Option<ProtectedPassphrase>,
}

pub enum LocalVaultPassphraseChangeOutcome {
    Reopened(LocalVaultTransition),
    Locked {
        transition: LocalVaultTransition,
        error: anyhow::Error,
    },
}

pub struct SettingsService;

impl SettingsService {
    pub fn persist_ai_provider(
        settings_store: &mut SettingsStore,
        secrets: &SecretStore,
        provider: &mut AiProviderConfig,
        api_key: Option<&str>,
    ) -> Result<bool> {
        let _sync_guard = miaominal_secrets::lock_sync_data();
        settings_store.reload_from_disk()?;
        let previous_api_key = match api_key {
            Some(_) => secrets.get(&provider.id, SecretKind::AiProviderApiKey)?,
            None => None,
        };

        if let Some(api_key) = api_key {
            secrets.set(&provider.id, SecretKind::AiProviderApiKey, api_key)?;
            provider.has_api_key = true;
        }

        let mut settings = settings_store.settings().clone();
        if let Some(existing) = settings
            .ai_providers
            .iter_mut()
            .find(|existing| existing.id == provider.id)
        {
            *existing = provider.clone();
        } else {
            settings.ai_providers.push(provider.clone());
        }

        match settings_store.replace(settings) {
            Ok(changed) => Ok(changed),
            Err(error) => {
                if api_key.is_none() {
                    return Err(error);
                }
                match restore_secret(
                    secrets,
                    &provider.id,
                    SecretKind::AiProviderApiKey,
                    previous_api_key.as_deref(),
                ) {
                    Ok(()) => Err(error),
                    Err(rollback_error) => Err(anyhow::anyhow!(
                        "failed to persist AI provider settings: {error:#}; restoring the previous API key also failed: {rollback_error:#}"
                    )),
                }
            }
        }
    }

    pub fn persist_web_search(
        settings_store: &mut SettingsStore,
        secrets: &SecretStore,
        config: &mut WebSearchConfig,
        api_key: Option<&str>,
    ) -> Result<bool> {
        const SECRET_ID: &str = "web_search";

        let _sync_guard = miaominal_secrets::lock_sync_data();
        settings_store.reload_from_disk()?;
        let previous_api_key = match api_key {
            Some(_) => secrets.get(SECRET_ID, SecretKind::WebSearchApiKey)?,
            None => None,
        };

        if let Some(api_key) = api_key {
            secrets.set(SECRET_ID, SecretKind::WebSearchApiKey, api_key)?;
            config.has_api_key = true;
        }

        let mut settings = settings_store.settings().clone();
        settings.web_search = config.clone();
        match settings_store.replace(settings) {
            Ok(changed) => Ok(changed),
            Err(error) => {
                if api_key.is_none() {
                    return Err(error);
                }
                match restore_secret(
                    secrets,
                    SECRET_ID,
                    SecretKind::WebSearchApiKey,
                    previous_api_key.as_deref(),
                ) {
                    Ok(()) => Err(error),
                    Err(rollback_error) => Err(anyhow::anyhow!(
                        "failed to persist web search settings: {error:#}; restoring the previous API key also failed: {rollback_error:#}"
                    )),
                }
            }
        }
    }

    pub fn adjust_font_size(settings_store: &mut SettingsStore, delta: f32) -> Option<f32> {
        let target =
            (settings_store.settings().font_size + delta).clamp(FONT_SIZE_MIN, FONT_SIZE_MAX);
        settings_store
            .update(|settings| settings.font_size = target)
            .then_some(target)
    }

    pub fn adjust_line_height(settings_store: &mut SettingsStore, delta: f32) -> Option<f32> {
        let target =
            (settings_store.settings().line_height + delta).clamp(LINE_HEIGHT_MIN, LINE_HEIGHT_MAX);
        settings_store
            .update(|settings| settings.line_height = target)
            .then_some(target)
    }

    pub fn set_sync_provider(sync_engine: &mut SyncEngine, provider: SyncProvider) -> Result<()> {
        sync_engine.config_store.update(|config| {
            if config.provider != provider {
                Self::clear_sync_baseline(config);
            }
            config.provider = provider;
            config.normalize_legacy_provider_flags();
        })
    }

    pub fn set_auto_sync_enabled(sync_engine: &mut SyncEngine, enabled: bool) -> Result<()> {
        sync_engine
            .config_store
            .update(|config| config.auto_sync_enabled = enabled)
    }

    pub fn persist_sync_github_token(sync_engine: &mut SyncEngine, token: &str) -> Result<()> {
        sync_engine.config_store.set_github_token(token)?;
        sync_engine
            .config_store
            .update(|config| config.has_github_token = !token.trim().is_empty())
    }

    pub fn persist_sync_gist_config(
        sync_engine: &mut SyncEngine,
        token: &str,
        gist_id: Option<String>,
    ) -> Result<()> {
        sync_engine.config_store.set_github_token(token)?;
        sync_engine.config_store.update(|config| {
            if config.gist_id != gist_id {
                Self::clear_sync_baseline(config);
            }
            config.gist_id = gist_id;
            config.has_github_token = !token.trim().is_empty();
        })
    }

    pub fn persist_sync_webdav_password(
        sync_engine: &mut SyncEngine,
        password: &str,
    ) -> Result<()> {
        sync_engine.config_store.set_webdav_password(password)?;
        sync_engine
            .config_store
            .update(|config| config.has_webdav_password = !password.trim().is_empty())
    }

    pub fn persist_sync_webdav_config(
        sync_engine: &mut SyncEngine,
        url: String,
        username: String,
        password: &str,
    ) -> Result<()> {
        sync_engine.config_store.set_webdav_password(password)?;
        sync_engine.config_store.update(|config| {
            if config.webdav_url != url || config.webdav_username != username {
                Self::clear_sync_baseline(config);
            }
            config.webdav_url = url;
            config.webdav_username = username;
            config.has_webdav_password = !password.trim().is_empty();
        })
    }

    pub fn persist_sync_passphrase(
        sync_engine: &mut SyncEngine,
        passphrase: Option<&str>,
    ) -> Result<bool> {
        let previous = sync_engine.config_store.get_passphrase()?;
        let configured = match passphrase {
            Some(passphrase) => {
                sync_engine.config_store.set_passphrase(passphrase)?;
                !passphrase.trim().is_empty()
            }
            None => {
                sync_engine.config_store.delete_passphrase()?;
                false
            }
        };
        let changed = previous.as_deref() != passphrase;

        if let Err(error) = sync_engine.config_store.update(|config| {
            config.has_passphrase = configured;
            if changed {
                // The plaintext payload may be identical, but changing the
                // encryption passphrase requires a fresh conditional push so
                // the remote can be decrypted with the new passphrase.
                config.last_synced_local_revision = None;
            }
        }) {
            let rollback = match previous.as_deref() {
                Some(previous) => sync_engine.config_store.set_passphrase(previous),
                None => sync_engine.config_store.delete_passphrase(),
            };
            return match rollback {
                Ok(()) => Err(error.context("failed to update sync passphrase configuration")),
                Err(rollback_error) => Err(anyhow::anyhow!(
                    "failed to update sync passphrase configuration: {error:#}; restoring the previous passphrase also failed: {rollback_error:#}"
                )),
            };
        }

        Ok(configured)
    }

    fn clear_sync_baseline(config: &mut miaominal_sync::SyncConfig) {
        config.last_sync_at = 0;
        config.remote_etag = None;
        config.remote_payload_id = None;
        config.last_synced_local_revision = None;
    }

    pub fn local_vault_lock_transition(settings_store: &SettingsStore) -> LocalVaultTransition {
        if settings_store.settings().local_vault_enabled {
            Self::locked_transition()
        } else {
            Self::disabled_transition()
        }
    }

    pub fn prepare_vault_enable(
        passphrase: &ProtectedPassphrase,
        session_ids: Vec<String>,
        managed_key_ids: Vec<String>,
        ai_provider_ids: Vec<String>,
        proxy_ids: Vec<String>,
        source_secrets: SecretStore,
        source_sync_engine: SyncEngine,
    ) -> Result<(SecretStore, SyncEngine)> {
        let (vault_secrets, vault_sync_engine) = Self::open_vault(passphrase.clone())?;

        if paths::credential_policy()? == paths::CredentialPolicy::LocalVaultRequired {
            return Ok((vault_secrets, vault_sync_engine));
        }

        let copy_result = Self::copy_secrets_between_backends(
            &session_ids,
            &managed_key_ids,
            &ai_provider_ids,
            &proxy_ids,
            &source_secrets,
            &source_sync_engine,
            &vault_secrets,
            &vault_sync_engine,
        );

        if copy_result.is_err()
            && let Err(erase_err) = VaultCredentialBackend::erase_default_store_file()
        {
            log::warn!("failed to erase partial vault file after enable failure: {erase_err:?}");
        }

        copy_result?;
        Ok((vault_secrets, vault_sync_engine))
    }

    pub fn apply_vault_enable(
        passphrase: ProtectedPassphrase,
        vault_secrets: SecretStore,
        vault_sync_engine: SyncEngine,
        settings_store: &mut SettingsStore,
    ) -> Result<LocalVaultTransition> {
        let mut settings = settings_store.settings().clone();
        settings.local_vault_enabled = true;
        settings_store.replace(settings)?;

        Ok(LocalVaultTransition {
            secrets: vault_secrets,
            sync_engine: vault_sync_engine,
            mode: LocalVaultMode::Unlocked,
            session_passphrase: Some(passphrase),
        })
    }

    pub fn unlock_local_vault(passphrase: ProtectedPassphrase) -> Result<LocalVaultTransition> {
        if !VaultCredentialBackend::default_store_exists()? {
            anyhow::bail!(
                "local vault file is missing; the vault may have been corrupted or deleted"
            );
        }

        let (vault_secrets, vault_sync_engine) = Self::open_vault(passphrase.clone())?;

        Ok(LocalVaultTransition {
            secrets: vault_secrets,
            sync_engine: vault_sync_engine,
            mode: LocalVaultMode::Unlocked,
            session_passphrase: Some(passphrase),
        })
    }

    pub fn change_local_vault_passphrase(
        current_passphrase: &ProtectedPassphrase,
        new_passphrase: ProtectedPassphrase,
    ) -> Result<LocalVaultPassphraseChangeOutcome> {
        VaultCredentialBackend::rotate_default_store_passphrase(
            current_passphrase,
            &new_passphrase,
        )?;

        match Self::open_vault(new_passphrase.clone()) {
            Ok((vault_secrets, vault_sync_engine)) => Ok(
                LocalVaultPassphraseChangeOutcome::Reopened(LocalVaultTransition {
                    secrets: vault_secrets,
                    sync_engine: vault_sync_engine,
                    mode: LocalVaultMode::Unlocked,
                    session_passphrase: Some(new_passphrase),
                }),
            ),
            Err(error) => Ok(LocalVaultPassphraseChangeOutcome::Locked {
                transition: Self::locked_transition(),
                error: error.context(
                    "vault passphrase changed, but the vault could not be reopened automatically",
                ),
            }),
        }
    }

    fn open_vault(passphrase: ProtectedPassphrase) -> Result<(SecretStore, SyncEngine)> {
        Self::open_vault_with_backend(VaultCredentialBackend::new(passphrase)?)
    }

    fn open_vault_with_backend(
        backend: VaultCredentialBackend,
    ) -> Result<(SecretStore, SyncEngine)> {
        let credentials = CredentialStore::with_backend(APP_CREDENTIAL_SERVICE, backend);
        credentials.initialize()?;

        Ok((
            SecretStore::with_credentials(credentials.clone()),
            SyncEngine::new_with_credentials(credentials),
        ))
    }

    pub fn prepare_vault_disable(
        previous_secrets: &SecretStore,
        previous_sync_engine: &SyncEngine,
        session_ids: &[String],
        managed_key_ids: &[String],
        ai_provider_ids: &[String],
        proxy_ids: &[String],
    ) -> Result<LocalVaultTransition> {
        if paths::credential_policy()? == paths::CredentialPolicy::LocalVaultRequired {
            anyhow::bail!("the local vault cannot be disabled in portable mode");
        }
        let keyring_secrets = SecretStore::new();
        let keyring_sync_engine = SyncEngine::new();

        Self::copy_secrets_between_backends(
            session_ids,
            managed_key_ids,
            ai_provider_ids,
            proxy_ids,
            previous_secrets,
            previous_sync_engine,
            &keyring_secrets,
            &keyring_sync_engine,
        )?;

        Ok(LocalVaultTransition {
            secrets: keyring_secrets,
            sync_engine: keyring_sync_engine,
            mode: LocalVaultMode::Disabled,
            session_passphrase: None,
        })
    }

    pub fn apply_vault_disable(settings_store: &mut SettingsStore) -> Result<()> {
        if paths::credential_policy()? == paths::CredentialPolicy::LocalVaultRequired {
            anyhow::bail!("the local vault cannot be disabled in portable mode");
        }
        let mut settings = settings_store.settings().clone();
        settings.local_vault_enabled = false;
        settings_store.replace(settings)?;

        Ok(())
    }

    pub fn delete_migrated_keyring_secrets(
        session_ids: &[String],
        managed_key_ids: &[String],
        ai_provider_ids: &[String],
        proxy_ids: &[String],
        source_secrets: &SecretStore,
        source_sync_engine: &SyncEngine,
    ) {
        credential_migration::delete_keyring_secrets(
            session_ids.iter().map(String::as_str),
            managed_key_ids.iter().map(String::as_str),
            ai_provider_ids,
            proxy_ids,
            source_secrets,
            &source_sync_engine.config_store,
        );
    }

    pub fn erase_vault_file() -> Result<()> {
        VaultCredentialBackend::erase_default_store_file()
    }

    pub fn reset_local_data(
        session_ids: &[String],
        managed_key_ids: &[String],
        ai_provider_ids: &[String],
        proxy_ids: &[String],
    ) -> Result<()> {
        if paths::credential_policy()? == paths::CredentialPolicy::LocalVaultRequired {
            return paths::clear_active_data_dir();
        }
        let keyring_secrets = SecretStore::new();
        let keyring_sync_engine = SyncEngine::new();
        let chat_credentials = CredentialStore::new_keyring(APP_CREDENTIAL_SERVICE);
        paths::clear_active_data_dir()?;
        Self::delete_migrated_keyring_secrets(
            session_ids,
            managed_key_ids,
            ai_provider_ids,
            proxy_ids,
            &keyring_secrets,
            &keyring_sync_engine,
        );
        if let Err(error) = ChatService::delete_key(&chat_credentials) {
            log::warn!("failed to delete chat database key from keyring: {error:?}");
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn copy_secrets_between_backends(
        session_ids: &[String],
        managed_key_ids: &[String],
        ai_provider_ids: &[String],
        proxy_ids: &[String],
        source_secrets: &SecretStore,
        source_sync_engine: &SyncEngine,
        target_secrets: &SecretStore,
        target_sync_engine: &SyncEngine,
    ) -> Result<()> {
        credential_migration::copy_secrets_between_backends(
            session_ids.iter().map(String::as_str),
            managed_key_ids.iter().map(String::as_str),
            ai_provider_ids,
            proxy_ids,
            source_secrets,
            &source_sync_engine.config_store,
            target_secrets,
            &target_sync_engine.config_store,
        )
    }

    fn locked_transition() -> LocalVaultTransition {
        LocalVaultTransition {
            secrets: SecretStore::new_locked_vault(),
            sync_engine: SyncEngine::new_locked_vault(),
            mode: LocalVaultMode::Locked,
            session_passphrase: None,
        }
    }

    fn disabled_transition() -> LocalVaultTransition {
        LocalVaultTransition {
            secrets: SecretStore::new(),
            sync_engine: SyncEngine::new(),
            mode: LocalVaultMode::Disabled,
            session_passphrase: None,
        }
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn reset_local_data_with(
        config_dir: &Path,
        keyring_secrets: &SecretStore,
        keyring_sync_engine: &SyncEngine,
        chat_credentials: &CredentialStore,
        session_ids: &[String],
        managed_key_ids: &[String],
        ai_provider_ids: &[String],
        proxy_ids: &[String],
    ) -> Result<()> {
        match fs::remove_dir_all(config_dir) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to remove {}", config_dir.display()));
            }
        }

        Self::delete_migrated_keyring_secrets(
            session_ids,
            managed_key_ids,
            ai_provider_ids,
            proxy_ids,
            keyring_secrets,
            keyring_sync_engine,
        );
        if let Err(error) = ChatService::delete_key(chat_credentials) {
            log::warn!("failed to delete chat database key from keyring: {error:?}");
        }

        Ok(())
    }
}

fn restore_secret(
    secrets: &SecretStore,
    id: &str,
    kind: SecretKind,
    previous: Option<&str>,
) -> Result<()> {
    match previous {
        Some(value) => secrets.set(id, kind, value),
        None => secrets.delete(id, kind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_secrets::set_vault_test_parameters;
    use miaominal_secrets::{APP_CREDENTIAL_SERVICE, CredentialStore};
    use miaominal_secrets::{ProtectedPassphrase, SecretKind, SecretStore, VaultCredentialBackend};
    use miaominal_sync::SyncConfig;
    use miaominal_sync::store::SyncConfigStore;
    use std::fs;
    use std::path::PathBuf;

    fn temp_sync_config_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "miaominal-settings-service-{label}-{}.toml",
            uuid::Uuid::new_v4()
        ))
    }

    fn temp_vault_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "miaominal-settings-service-{label}-{}.json",
            uuid::Uuid::new_v4()
        ))
    }

    fn temp_config_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "miaominal-settings-service-{label}-{}",
            uuid::Uuid::new_v4()
        ))
    }

    fn test_settings_store(label: &str) -> (SettingsStore, PathBuf) {
        let path = temp_config_dir(label).join("settings.toml");
        let store = SettingsStore::load_with_path(path.clone())
            .expect("test settings store should initialize");
        (store, path)
    }

    fn protected(value: &str) -> ProtectedPassphrase {
        ProtectedPassphrase::try_from_string(value.to_string())
            .expect("test passphrase should use protected memory")
    }

    fn cleanup_test_vault(path: &Path) {
        let _ = fs::remove_file(path);
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let _ = fs::remove_file(PathBuf::from(lock_path));
    }

    fn test_vault_sync_engine(
        label: &str,
        vault_passphrase: &str,
    ) -> (SyncEngine, PathBuf, PathBuf) {
        set_vault_test_parameters();
        let vault_path = temp_vault_path(label);
        let config_path = temp_sync_config_path(label);
        let credentials = CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new_with_path(vault_path.clone(), protected(vault_passphrase)),
        );

        (
            SyncEngine {
                config_store: SyncConfigStore::with_credentials(
                    config_path.clone(),
                    SyncConfig::default(),
                    credentials,
                ),
            },
            vault_path,
            config_path,
        )
    }

    fn test_keyring_like_backend(
        label: &str,
    ) -> (SecretStore, SyncEngine, CredentialStore, PathBuf, PathBuf) {
        set_vault_test_parameters();
        let secrets_path = temp_vault_path(&format!("{label}-secrets"));
        let config_path = temp_sync_config_path(&format!("{label}-config"));
        let credentials = CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new_with_path(
                secrets_path.clone(),
                protected("keyring-passphrase"),
            ),
        );
        credentials
            .initialize()
            .expect("test credential backend should initialize");

        (
            SecretStore::with_credentials(credentials.clone()),
            SyncEngine {
                config_store: SyncConfigStore::with_credentials(
                    config_path.clone(),
                    SyncConfig::default(),
                    credentials.clone(),
                ),
            },
            credentials,
            secrets_path,
            config_path,
        )
    }

    #[test]
    fn adjust_font_size_clamps_to_supported_range() {
        let (mut store, settings_path) = test_settings_store("adjust-font-size");

        let value =
            SettingsService::adjust_font_size(&mut store, 99.0).expect("font size should update");

        assert_eq!(value, FONT_SIZE_MAX);

        let _ = fs::remove_dir_all(settings_path.parent().unwrap());
    }

    #[test]
    fn persist_ai_provider_commits_key_and_settings_together() {
        let (mut store, settings_path) = test_settings_store("persist-ai-provider");
        let (secrets, _, _, secrets_path, sync_path) =
            test_keyring_like_backend("persist-ai-provider");
        let mut provider = AiProviderConfig::new(miaominal_settings::AiProviderKind::OpenAi);
        let provider_id = provider.id.clone();

        SettingsService::persist_ai_provider(
            &mut store,
            &secrets,
            &mut provider,
            Some("new-api-key"),
        )
        .expect("AI provider transaction should succeed");

        assert!(provider.has_api_key);
        assert!(
            store
                .settings()
                .ai_providers
                .iter()
                .any(|saved| saved.id == provider_id && saved.has_api_key)
        );
        assert_eq!(
            secrets
                .get(&provider_id, SecretKind::AiProviderApiKey)
                .unwrap()
                .as_deref(),
            Some("new-api-key")
        );

        let _ = fs::remove_dir_all(settings_path.parent().unwrap());
        cleanup_test_vault(&secrets_path);
        let _ = fs::remove_file(sync_path);
    }

    #[test]
    fn persist_web_search_preserves_unrelated_concurrent_settings() {
        let (mut worker_store, settings_path) = test_settings_store("persist-web-search");
        let mut ui_store = worker_store.clone();
        let (secrets, _, _, secrets_path, sync_path) =
            test_keyring_like_backend("persist-web-search");
        ui_store.update(|settings| settings.font_size = 19.0);
        let mut config = WebSearchConfig::default();
        config.enabled = true;

        SettingsService::persist_web_search(
            &mut worker_store,
            &secrets,
            &mut config,
            Some("web-api-key"),
        )
        .expect("web search transaction should succeed");

        assert_eq!(worker_store.settings().font_size, 19.0);
        assert!(worker_store.settings().web_search.enabled);
        assert!(worker_store.settings().web_search.has_api_key);
        assert_eq!(
            secrets
                .get("web_search", SecretKind::WebSearchApiKey)
                .unwrap()
                .as_deref(),
            Some("web-api-key")
        );

        let _ = fs::remove_dir_all(settings_path.parent().unwrap());
        cleanup_test_vault(&secrets_path);
        let _ = fs::remove_file(sync_path);
    }

    #[test]
    fn lock_local_vault_respects_disabled_settings() {
        let store = SettingsStore::fallback();
        let transition = SettingsService::local_vault_lock_transition(&store);

        assert_eq!(transition.mode, LocalVaultMode::Disabled);
        assert!(transition.session_passphrase.is_none());
    }

    #[test]
    fn shared_vault_backend_is_revoked_for_secret_and_sync_stores() {
        set_vault_test_parameters();
        let vault_path = temp_vault_path("shared-revocation");
        let passphrase = protected("vault-passphrase");
        let backend = VaultCredentialBackend::new_with_path(vault_path.clone(), passphrase.clone());
        let (secrets, sync_engine) = SettingsService::open_vault_with_backend(backend)
            .expect("shared vault backend should open");

        secrets
            .set("profile-1", SecretKind::Password, "password")
            .expect("profile password should save");
        sync_engine
            .config_store
            .set_github_token("github-token")
            .expect("sync token should save through shared backend");
        passphrase.revoke();

        assert!(secrets.get("profile-1", SecretKind::Password).is_err());
        assert!(sync_engine.config_store.get_github_token().is_err());

        cleanup_test_vault(&vault_path);
    }

    #[test]
    fn set_sync_provider_updates_provider() {
        let mut engine = SyncEngine {
            config_store: SyncConfigStore::fallback(),
        };

        SettingsService::set_sync_provider(&mut engine, SyncProvider::WebDav)
            .expect("sync provider should update");

        assert_eq!(engine.config_store.config.provider, SyncProvider::WebDav);
    }

    #[test]
    fn changing_sync_provider_clears_remote_and_local_baselines() {
        let (mut engine, vault_path, config_path) =
            test_vault_sync_engine("provider-baseline", "vault-passphrase");
        engine
            .config_store
            .update(|config| {
                config.provider = SyncProvider::GithubGist;
                config.last_sync_at = 42;
                config.remote_etag = Some("\"etag-v1\"".into());
                config.remote_payload_id = Some("payload-v1".into());
                config.last_synced_local_revision = Some("local-v1".into());
            })
            .unwrap();

        SettingsService::set_sync_provider(&mut engine, SyncProvider::WebDav).unwrap();

        assert_eq!(engine.config_store.config.last_sync_at, 0);
        assert_eq!(engine.config_store.config.remote_etag, None);
        assert_eq!(engine.config_store.config.remote_payload_id, None);
        assert_eq!(engine.config_store.config.last_synced_local_revision, None);
        cleanup_test_vault(&vault_path);
        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn persist_sync_passphrase_writes_and_clears_vault_secret() {
        let (mut engine, vault_path, config_path) =
            test_vault_sync_engine("persist-sync-passphrase", "vault-passphrase");

        let configured =
            SettingsService::persist_sync_passphrase(&mut engine, Some("sync-passphrase"))
                .expect("sync passphrase should save into vault");

        assert!(configured);
        assert!(engine.config_store.config.has_passphrase);
        assert_eq!(
            engine
                .config_store
                .get_passphrase()
                .expect("stored passphrase should read back")
                .as_deref(),
            Some("sync-passphrase")
        );

        let persisted = fs::read_to_string(&config_path).expect("sync config should persist");
        let persisted: SyncConfig = toml::from_str(&persisted).expect("sync config should parse");
        assert!(persisted.has_passphrase);

        let configured = SettingsService::persist_sync_passphrase(&mut engine, None)
            .expect("sync passphrase should clear from vault");

        assert!(!configured);
        assert!(!engine.config_store.config.has_passphrase);
        assert_eq!(
            engine
                .config_store
                .get_passphrase()
                .expect("cleared passphrase should read back as missing"),
            None
        );

        let persisted = fs::read_to_string(&config_path).expect("sync config should persist");
        let persisted: SyncConfig = toml::from_str(&persisted).expect("sync config should parse");
        assert!(!persisted.has_passphrase);

        cleanup_test_vault(&vault_path);
        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn changing_existing_sync_passphrase_invalidates_local_sync_baseline() {
        let (mut engine, vault_path, config_path) =
            test_vault_sync_engine("change-sync-passphrase", "vault-passphrase");
        SettingsService::persist_sync_passphrase(&mut engine, Some("old-sync-passphrase")).unwrap();
        engine
            .config_store
            .update(|config| {
                config.last_synced_local_revision = Some("local-v1".into());
                config.remote_etag = Some("\"etag-v1\"".into());
            })
            .unwrap();

        SettingsService::persist_sync_passphrase(&mut engine, Some("new-sync-passphrase")).unwrap();

        assert_eq!(engine.config_store.config.last_synced_local_revision, None);
        assert_eq!(
            engine.config_store.config.remote_etag.as_deref(),
            Some("\"etag-v1\"")
        );
        assert_eq!(
            engine.config_store.get_passphrase().unwrap().as_deref(),
            Some("new-sync-passphrase")
        );

        cleanup_test_vault(&vault_path);
        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn saving_identical_sync_passphrase_keeps_local_sync_baseline() {
        let (mut engine, vault_path, config_path) =
            test_vault_sync_engine("same-sync-passphrase", "vault-passphrase");
        SettingsService::persist_sync_passphrase(&mut engine, Some("sync-passphrase")).unwrap();
        engine
            .config_store
            .update(|config| config.last_synced_local_revision = Some("local-v1".into()))
            .unwrap();

        SettingsService::persist_sync_passphrase(&mut engine, Some("sync-passphrase")).unwrap();

        assert_eq!(
            engine
                .config_store
                .config
                .last_synced_local_revision
                .as_deref(),
            Some("local-v1")
        );

        cleanup_test_vault(&vault_path);
        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn persist_sync_github_token_writes_vault_secret() {
        let (mut engine, vault_path, config_path) =
            test_vault_sync_engine("persist-github-token", "vault-passphrase");

        SettingsService::persist_sync_github_token(&mut engine, "github-token")
            .expect("github token should save into vault");

        assert_eq!(
            engine
                .config_store
                .get_github_token()
                .expect("stored github token should read back")
                .as_deref(),
            Some("github-token")
        );
        assert!(engine.config_store.config.has_github_token);

        let persisted = fs::read_to_string(&config_path).expect("sync config should persist");
        let persisted: SyncConfig = toml::from_str(&persisted).expect("sync config should parse");
        assert!(persisted.has_github_token);

        cleanup_test_vault(&vault_path);
        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn persist_sync_webdav_password_writes_vault_secret() {
        let (mut engine, vault_path, config_path) =
            test_vault_sync_engine("persist-webdav-password", "vault-passphrase");

        SettingsService::persist_sync_webdav_password(&mut engine, "webdav-password")
            .expect("webdav password should save into vault");

        assert_eq!(
            engine
                .config_store
                .get_webdav_password()
                .expect("stored webdav password should read back")
                .as_deref(),
            Some("webdav-password")
        );
        assert!(engine.config_store.config.has_webdav_password);

        let persisted = fs::read_to_string(&config_path).expect("sync config should persist");
        let persisted: SyncConfig = toml::from_str(&persisted).expect("sync config should parse");
        assert!(persisted.has_webdav_password);

        cleanup_test_vault(&vault_path);
        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn reset_local_data_with_removes_config_dir_and_known_secrets() {
        let config_dir = temp_config_dir("reset-local-data");
        let (
            keyring_secrets,
            keyring_sync_engine,
            chat_credentials,
            secrets_path,
            sync_config_path,
        ) = test_keyring_like_backend("reset-local-data");

        fs::create_dir_all(&config_dir).expect("config dir should exist");
        fs::write(config_dir.join("settings.toml"), "font_size = 14.0\n")
            .expect("settings file should be created");
        fs::write(config_dir.join("sessions.toml"), "sessions = []\n")
            .expect("sessions file should be created");
        fs::write(config_dir.join("secret_vault.json"), "{}")
            .expect("vault file should be created");

        keyring_secrets
            .set("session-1", SecretKind::Password, "pw")
            .expect("session password should save");
        keyring_secrets
            .set("session-1", SecretKind::Passphrase, "pp")
            .expect("session passphrase should save");
        keyring_secrets
            .set(
                "managed-key-1",
                SecretKind::ManagedPrivateKey,
                "private-key",
            )
            .expect("managed key should save");
        keyring_secrets
            .set("provider-1", SecretKind::AiProviderApiKey, "sk-test")
            .expect("AI provider key should save");
        keyring_secrets
            .set("proxy-1", SecretKind::ProxyPassword, "proxy-password")
            .expect("proxy password should save");
        keyring_secrets
            .set("web_search", SecretKind::WebSearchApiKey, "web-search-key")
            .expect("web search API key should save");
        keyring_sync_engine
            .config_store
            .set_github_token("gh-token")
            .expect("github token should save");
        keyring_sync_engine
            .config_store
            .set_webdav_password("dav-password")
            .expect("webdav password should save");
        keyring_sync_engine
            .config_store
            .set_passphrase("sync-passphrase")
            .expect("sync passphrase should save");
        chat_credentials
            .set("chat-db-key", "chat-key")
            .expect("chat database key should save");

        SettingsService::reset_local_data_with(
            &config_dir,
            &keyring_secrets,
            &keyring_sync_engine,
            &chat_credentials,
            &["session-1".to_string()],
            &["managed-key-1".to_string()],
            &["provider-1".to_string()],
            &["proxy-1".to_string()],
        )
        .expect("local data reset should succeed");

        assert!(!config_dir.exists());
        assert_eq!(
            keyring_secrets
                .get("session-1", SecretKind::Password)
                .expect("session password should be readable after reset"),
            None
        );
        assert_eq!(
            keyring_secrets
                .get("session-1", SecretKind::Passphrase)
                .expect("session passphrase should be readable after reset"),
            None
        );
        assert_eq!(
            keyring_secrets
                .get("managed-key-1", SecretKind::ManagedPrivateKey)
                .expect("managed key should be readable after reset"),
            None
        );
        assert_eq!(
            keyring_secrets
                .get("provider-1", SecretKind::AiProviderApiKey)
                .expect("AI provider key should be readable after reset"),
            None
        );
        assert_eq!(
            keyring_secrets
                .get("proxy-1", SecretKind::ProxyPassword)
                .expect("proxy password should be readable after reset"),
            None
        );
        assert_eq!(
            keyring_secrets
                .get("web_search", SecretKind::WebSearchApiKey)
                .expect("web search API key should be readable after reset"),
            None
        );
        assert_eq!(
            keyring_sync_engine
                .config_store
                .get_github_token()
                .expect("github token should be readable after reset"),
            None
        );
        assert_eq!(
            keyring_sync_engine
                .config_store
                .get_webdav_password()
                .expect("webdav password should be readable after reset"),
            None
        );
        assert_eq!(
            keyring_sync_engine
                .config_store
                .get_passphrase()
                .expect("sync passphrase should be readable after reset"),
            None
        );
        assert_eq!(
            chat_credentials
                .get("chat-db-key")
                .expect("chat database key should be readable after reset"),
            None
        );

        cleanup_test_vault(&secrets_path);
        let _ = fs::remove_file(sync_config_path);
    }

    #[test]
    fn reset_local_data_with_preserves_credentials_when_config_removal_fails() {
        let config_path = temp_config_dir("reset-local-data-removal-failure");
        let (
            keyring_secrets,
            keyring_sync_engine,
            chat_credentials,
            secrets_path,
            sync_config_path,
        ) = test_keyring_like_backend("reset-local-data-removal-failure");

        fs::write(&config_path, "not a directory").expect("blocking file should be created");
        keyring_secrets
            .set("session-1", SecretKind::Password, "pw")
            .expect("session password should save");
        chat_credentials
            .set("chat-db-key", "chat-key")
            .expect("chat database key should save");

        let result = SettingsService::reset_local_data_with(
            &config_path,
            &keyring_secrets,
            &keyring_sync_engine,
            &chat_credentials,
            &["session-1".to_string()],
            &[],
            &[],
            &[],
        );

        assert!(result.is_err());
        assert_eq!(
            keyring_secrets
                .get("session-1", SecretKind::Password)
                .expect("session password should remain readable"),
            Some("pw".to_string())
        );
        assert_eq!(
            chat_credentials
                .get("chat-db-key")
                .expect("chat database key should remain readable"),
            Some("chat-key".to_string())
        );

        let _ = fs::remove_file(config_path);
        cleanup_test_vault(&secrets_path);
        let _ = fs::remove_file(sync_config_path);
    }
}
