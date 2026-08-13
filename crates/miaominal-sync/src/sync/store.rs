use crate::SyncConfig;
use anyhow::{Context, Result};
use miaominal_paths::{self as paths, atomic_write};
use miaominal_secrets::{
    APP_CREDENTIAL_SERVICE, CredentialStore, LockedCredentialBackend, ProtectedPassphrase,
    VaultCredentialBackend,
};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

const ACCOUNT_GITHUB_TOKEN: &str = "sync:github-token";
const ACCOUNT_WEBDAV_PASSWORD: &str = "sync:webdav-password";
const ACCOUNT_PASSPHRASE: &str = "sync:encryption-passphrase";

// Every SyncConfigStore instance in the process targets the same logical
// configuration. Serialize read-modify-write cycles so a stale engine clone
// cannot replace settings that were just saved by the UI.
static SYNC_CONFIG_WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncSecrets {
    pub github_token: Option<String>,
    pub webdav_password: Option<String>,
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SyncConfigStore {
    config_file: PathBuf,
    credentials: CredentialStore,
    pub config: SyncConfig,
    loaded_config: SyncConfig,
}

impl SyncConfigStore {
    pub fn load() -> Result<Self> {
        Self::load_with_credentials(CredentialStore::new_keyring(APP_CREDENTIAL_SERVICE))
    }

    pub fn load_with_locked_vault() -> Result<Self> {
        Self::load_with_credentials(CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            LockedCredentialBackend,
        ))
    }

    pub fn load_with_vault(passphrase: ProtectedPassphrase) -> Result<Self> {
        Self::load_with_credentials(CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new(passphrase)?,
        ))
    }

    pub fn load_with_credentials(credentials: CredentialStore) -> Result<Self> {
        let config_file = paths::config_file("sync_config.toml")?;

        let mut config = if config_file.exists() {
            let content = fs::read_to_string(&config_file)
                .with_context(|| format!("failed to read {}", config_file.display()))?;
            if content.trim().is_empty() {
                SyncConfig::default()
            } else {
                toml::from_str(&content)
                    .with_context(|| format!("failed to parse {}", config_file.display()))?
            }
        } else {
            SyncConfig::default()
        };

        let loaded_config = config.clone();
        if config.device_id.is_empty() {
            config.device_id = uuid::Uuid::new_v4().to_string();
        }
        config.normalize_legacy_provider_flags();

        let mut store = Self::with_credentials(config_file, config, credentials);
        store.loaded_config = loaded_config;
        // Re-read under the process-wide write lock before normalizing and
        // persisting. Another store may have updated the file between the
        // initial read above and this point.
        store.update(|_| {})?;
        Ok(store)
    }

    pub fn fallback() -> Self {
        Self::with_credentials(
            std::env::temp_dir().join("miaominal_sync_config.toml"),
            SyncConfig::default(),
            CredentialStore::new_keyring(APP_CREDENTIAL_SERVICE),
        )
    }

    pub fn fallback_with_locked_vault() -> Self {
        Self::with_credentials(
            std::env::temp_dir().join("miaominal_sync_config.toml"),
            SyncConfig::default(),
            CredentialStore::with_backend(APP_CREDENTIAL_SERVICE, LockedCredentialBackend),
        )
    }

    pub fn fallback_with_vault(passphrase: ProtectedPassphrase) -> Result<Self> {
        Ok(Self::with_credentials(
            std::env::temp_dir().join("miaominal_sync_config.toml"),
            SyncConfig::default(),
            CredentialStore::with_backend(
                APP_CREDENTIAL_SERVICE,
                VaultCredentialBackend::new(passphrase)?,
            ),
        ))
    }

    pub fn fallback_with_credentials(credentials: CredentialStore) -> Self {
        Self::with_credentials(
            std::env::temp_dir().join("miaominal_sync_config.toml"),
            SyncConfig::default(),
            credentials,
        )
    }

    pub fn with_credentials(
        config_file: PathBuf,
        config: SyncConfig,
        credentials: CredentialStore,
    ) -> Self {
        Self {
            config_file,
            credentials,
            loaded_config: config.clone(),
            config,
        }
    }

    pub fn update<F: FnOnce(&mut SyncConfig)>(&mut self, f: F) -> Result<()> {
        self.update_inner(None, f).map(|_| ())
    }

    /// Apply an update only if no other SyncConfig writer has committed since
    /// `expected_revision` was observed. The check and write share the same
    /// process-wide critical section.
    pub fn update_if_revision<F: FnOnce(&mut SyncConfig)>(
        &mut self,
        expected_revision: u64,
        f: F,
    ) -> Result<bool> {
        self.update_inner(Some(expected_revision), f)
    }

    fn update_inner<F: FnOnce(&mut SyncConfig)>(
        &mut self,
        expected_revision: Option<u64>,
        f: F,
    ) -> Result<bool> {
        let _guard = SYNC_CONFIG_WRITE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let persisted_config = self.read_config()?;
        let config_exists = persisted_config.is_some();
        let mut next = persisted_config.unwrap_or_else(|| self.config.clone());
        if expected_revision.is_some_and(|expected| next.config_revision != expected) {
            self.config = next.clone();
            self.loaded_config = next;
            return Ok(false);
        }
        let persisted = next.clone();
        merge_external_config_changes(&mut next, &self.loaded_config, &self.config);
        let persisted_revision = persisted.config_revision;
        let local_revision = self.config.config_revision;
        f(&mut next);
        next.normalize_legacy_provider_flags();
        if config_exists && next == persisted {
            self.config = next.clone();
            self.loaded_config = next;
            return Ok(true);
        }
        next.config_revision = persisted_revision.max(local_revision).saturating_add(1);
        self.persist_config(&next)?;
        self.config = next.clone();
        self.loaded_config = next;
        Ok(true)
    }

    pub fn sync_from_disk(&mut self) {
        if let Ok(Some(persisted)) = self.read_config() {
            self.config = persisted.clone();
            self.loaded_config = persisted;
        }
    }

    fn read_config(&self) -> Result<Option<SyncConfig>> {
        match fs::read_to_string(&self.config_file) {
            Ok(content) if content.trim().is_empty() => Ok(Some(SyncConfig::default())),
            Ok(content) => toml::from_str(&content)
                .with_context(|| format!("failed to parse {}", self.config_file.display()))
                .map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => {
                Err(error).with_context(|| format!("failed to read {}", self.config_file.display()))
            }
        }
    }

    fn persist_config(&self, config: &SyncConfig) -> Result<()> {
        if let Some(parent) = self.config_file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let serialized =
            toml::to_string_pretty(config).context("failed to serialize sync config")?;
        atomic_write(&self.config_file, serialized)?;
        Ok(())
    }

    fn get_secret(&self, account: &str) -> Result<Option<String>> {
        self.credentials
            .get(account)
            .with_context(|| format!("failed to read secret for {account}"))
    }

    pub fn get_secrets(&self) -> Result<SyncSecrets> {
        let values = self
            .credentials
            .get_many(&[
                ACCOUNT_GITHUB_TOKEN,
                ACCOUNT_WEBDAV_PASSWORD,
                ACCOUNT_PASSPHRASE,
            ])
            .context("failed to read sync secrets")?;
        let mut values = values.into_iter();

        Ok(SyncSecrets {
            github_token: values.next().flatten(),
            webdav_password: values.next().flatten(),
            passphrase: values.next().flatten(),
        })
    }

    fn set_secret(&self, account: &str, value: &str) -> Result<()> {
        self.credentials
            .set(account, value)
            .with_context(|| format!("failed to store secret for {account}"))
    }

    fn delete_secret(&self, account: &str) -> Result<()> {
        self.credentials
            .delete(account)
            .with_context(|| format!("failed to delete secret for {account}"))
    }

    pub fn get_github_token(&self) -> Result<Option<String>> {
        self.get_secret(ACCOUNT_GITHUB_TOKEN)
    }

    pub fn set_github_token(&self, token: &str) -> Result<()> {
        self.set_secret(ACCOUNT_GITHUB_TOKEN, token)
    }

    pub fn delete_github_token(&self) -> Result<()> {
        self.delete_secret(ACCOUNT_GITHUB_TOKEN)
    }

    pub fn get_webdav_password(&self) -> Result<Option<String>> {
        self.get_secret(ACCOUNT_WEBDAV_PASSWORD)
    }

    pub fn set_webdav_password(&self, password: &str) -> Result<()> {
        self.set_secret(ACCOUNT_WEBDAV_PASSWORD, password)
    }

    pub fn delete_webdav_password(&self) -> Result<()> {
        self.delete_secret(ACCOUNT_WEBDAV_PASSWORD)
    }

    pub fn get_passphrase(&self) -> Result<Option<String>> {
        self.get_secret(ACCOUNT_PASSPHRASE)
    }

    pub fn set_passphrase(&self, passphrase: &str) -> Result<()> {
        self.set_secret(ACCOUNT_PASSPHRASE, passphrase)
    }

    pub fn delete_passphrase(&self) -> Result<()> {
        self.delete_secret(ACCOUNT_PASSPHRASE)
    }
}

/// Preserve direct in-memory edits made by existing callers while still
/// starting from the latest persisted config. Fields unchanged since this
/// store was loaded are taken from disk; locally changed fields win and are
/// then combined with the update closure under the same write lock.
fn merge_external_config_changes(
    target: &mut SyncConfig,
    loaded: &SyncConfig,
    current: &SyncConfig,
) {
    if current.config_revision > loaded.config_revision {
        if current.config_revision >= target.config_revision {
            *target = current.clone();
        }
        return;
    }

    macro_rules! merge_field {
        ($field:ident) => {
            if current.$field != loaded.$field {
                target.$field = current.$field.clone();
            }
        };
    }

    merge_field!(provider);
    merge_field!(gist_enabled);
    merge_field!(webdav_enabled);
    merge_field!(gist_id);
    merge_field!(webdav_url);
    merge_field!(webdav_username);
    merge_field!(has_github_token);
    merge_field!(has_webdav_password);
    merge_field!(has_passphrase);
    merge_field!(last_sync_at);
    merge_field!(device_id);
    merge_field!(auto_sync_enabled);
    merge_field!(remote_etag);
    merge_field!(remote_payload_id);
    merge_field!(last_synced_local_revision);
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_secrets::ProtectedPassphrase;
    use miaominal_secrets::credential_backend::{
        CredentialStore, LockedCredentialBackend, VaultCredentialBackend, set_vault_test_parameters,
    };

    fn temp_sync_config_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "miaominal-sync-config-{}.toml",
            uuid::Uuid::new_v4()
        ))
    }

    fn cleanup_test_vault(path: &std::path::Path) {
        let _ = fs::remove_file(path);
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let _ = fs::remove_file(PathBuf::from(lock_path));
    }

    #[test]
    fn v0_1_sync_keyring_accounts_remain_stable() {
        assert_eq!(ACCOUNT_GITHUB_TOKEN, "sync:github-token");
        assert_eq!(ACCOUNT_WEBDAV_PASSWORD, "sync:webdav-password");
        assert_eq!(ACCOUNT_PASSPHRASE, "sync:encryption-passphrase");
    }

    #[test]
    fn get_secrets_reads_all_sync_secrets_in_one_call_shape() {
        set_vault_test_parameters();
        let vault_path = std::env::temp_dir().join(format!(
            "miaominal-sync-secrets-{}.json",
            uuid::Uuid::new_v4()
        ));
        let config_path = std::env::temp_dir().join(format!(
            "miaominal-sync-config-{}.toml",
            uuid::Uuid::new_v4()
        ));
        let credentials = CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new_with_path(
                vault_path.clone(),
                ProtectedPassphrase::try_from_string("passphrase".to_string())
                    .expect("test passphrase should use protected memory"),
            ),
        );
        let store = SyncConfigStore::with_credentials(
            config_path.clone(),
            SyncConfig::default(),
            credentials,
        );

        store
            .set_github_token("github-token")
            .expect("github token should save");
        store
            .set_webdav_password("webdav-password")
            .expect("webdav password should save");
        store
            .set_passphrase("sync-passphrase")
            .expect("passphrase should save");

        let secrets = store
            .get_secrets()
            .expect("grouped sync secret read should succeed");

        assert_eq!(secrets.github_token.as_deref(), Some("github-token"));
        assert_eq!(secrets.webdav_password.as_deref(), Some("webdav-password"));
        assert_eq!(secrets.passphrase.as_deref(), Some("sync-passphrase"));

        cleanup_test_vault(&vault_path);
        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn sync_from_disk_updates_last_sync_at_and_remote_etag() {
        let config_path = temp_sync_config_path();
        let credentials =
            CredentialStore::with_backend(APP_CREDENTIAL_SERVICE, LockedCredentialBackend);
        let mut store = SyncConfigStore::with_credentials(
            config_path.clone(),
            SyncConfig::default(),
            credentials.clone(),
        );
        store.update(|_| {}).expect("initial config should persist");

        let updated_config = SyncConfig {
            last_sync_at: 42,
            remote_etag: Some("\"etag-v2\"".into()),
            ..SyncConfig::default()
        };
        let serialized = toml::to_string_pretty(&updated_config).unwrap();
        atomic_write(&config_path, serialized).expect("updated config should persist");

        store.sync_from_disk();

        assert_eq!(store.config.last_sync_at, 42);
        assert_eq!(store.config.remote_etag.as_deref(), Some("\"etag-v2\""));

        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn sync_config_new_fields_default_off_and_roundtrip() {
        let config = SyncConfig::default();
        assert!(!config.auto_sync_enabled);
        assert_eq!(config.remote_etag, None);

        let config_path = temp_sync_config_path();
        let credentials =
            CredentialStore::with_backend(APP_CREDENTIAL_SERVICE, LockedCredentialBackend);
        let mut store = SyncConfigStore::with_credentials(
            config_path.clone(),
            SyncConfig::default(),
            credentials,
        );
        store
            .update(|config| {
                config.auto_sync_enabled = true;
                config.remote_etag = Some("\"etag-v1\"".into());
            })
            .expect("config update should persist");

        let content = std::fs::read_to_string(&config_path).expect("config should be readable");
        let loaded = toml::from_str::<SyncConfig>(&content).expect("config should parse");

        assert!(loaded.auto_sync_enabled);
        assert_eq!(loaded.remote_etag.as_deref(), Some("\"etag-v1\""));

        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn concurrent_store_updates_merge_without_overwriting_unrelated_fields() {
        let config_path = temp_sync_config_path();
        let credentials =
            CredentialStore::with_backend(APP_CREDENTIAL_SERVICE, LockedCredentialBackend);
        let initial = SyncConfig {
            provider: crate::SyncProvider::WebDav,
            webdav_url: "https://old.example/sync.json".into(),
            remote_etag: Some("\"etag-v1\"".into()),
            ..SyncConfig::default()
        };
        let mut settings_store = SyncConfigStore::with_credentials(
            config_path.clone(),
            initial.clone(),
            credentials.clone(),
        );
        settings_store.update(|_| {}).unwrap();
        let mut sync_store = SyncConfigStore::with_credentials(
            config_path.clone(),
            settings_store.config.clone(),
            credentials,
        );

        settings_store
            .update(|config| {
                config.provider = crate::SyncProvider::GithubGist;
                config.gist_id = Some("new-gist".into());
                config.webdav_url.clear();
                config.remote_etag = None;
            })
            .unwrap();
        sync_store
            .update(|config| {
                config.last_sync_at = 42;
                config.remote_etag = Some("\"stale-etag\"".into());
                config.remote_payload_id = Some("payload-v2".into());
            })
            .unwrap();

        let persisted: SyncConfig =
            toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(persisted.provider, crate::SyncProvider::GithubGist);
        assert_eq!(persisted.gist_id.as_deref(), Some("new-gist"));
        assert!(persisted.webdav_url.is_empty());
        assert_eq!(persisted.last_sync_at, 42);
        assert_eq!(persisted.remote_etag.as_deref(), Some("\"stale-etag\""));
        assert_eq!(persisted.remote_payload_id.as_deref(), Some("payload-v2"));

        let _ = fs::remove_file(config_path);
    }

    #[test]
    fn revision_guard_rejects_sync_completion_after_settings_change() {
        let config_path = temp_sync_config_path();
        let credentials =
            CredentialStore::with_backend(APP_CREDENTIAL_SERVICE, LockedCredentialBackend);
        let mut settings_store = SyncConfigStore::with_credentials(
            config_path.clone(),
            SyncConfig::default(),
            credentials.clone(),
        );
        settings_store.update(|_| {}).unwrap();
        let mut sync_store = SyncConfigStore::with_credentials(
            config_path.clone(),
            settings_store.config.clone(),
            credentials,
        );
        let observed_revision = sync_store.config.config_revision;

        settings_store
            .update(|config| config.provider = crate::SyncProvider::WebDav)
            .unwrap();
        let committed = sync_store
            .update_if_revision(observed_revision, |config| {
                config.remote_etag = Some("\"old-provider-etag\"".into());
            })
            .unwrap();

        assert!(!committed);
        assert_eq!(sync_store.config.provider, crate::SyncProvider::WebDav);
        assert_eq!(sync_store.config.remote_etag, None);

        let _ = fs::remove_file(config_path);
    }
}
