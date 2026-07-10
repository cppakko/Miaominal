use crate::SyncConfig;
use anyhow::{Context, Result};
use miaominal_paths::{self as paths, atomic_write};
use miaominal_secrets::{
    APP_CREDENTIAL_SERVICE, CredentialStore, LockedCredentialBackend, VaultCredentialBackend,
};
use std::fs;
use std::path::PathBuf;

const ACCOUNT_GITHUB_TOKEN: &str = "sync:github-token";
const ACCOUNT_WEBDAV_PASSWORD: &str = "sync:webdav-password";
const ACCOUNT_PASSPHRASE: &str = "sync:encryption-passphrase";

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

    pub fn load_with_vault(passphrase: impl Into<String>) -> Result<Self> {
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

        if config.device_id.is_empty() {
            config.device_id = uuid::Uuid::new_v4().to_string();
        }
        config.normalize_legacy_provider_flags();

        let store = Self::with_credentials(config_file, config, credentials);
        store.persist()?;
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

    pub fn fallback_with_vault(passphrase: impl Into<String>) -> Result<Self> {
        Ok(Self::with_credentials(
            std::env::temp_dir().join("miaominal_sync_config.toml"),
            SyncConfig::default(),
            CredentialStore::with_backend(
                APP_CREDENTIAL_SERVICE,
                VaultCredentialBackend::new(passphrase)?,
            ),
        ))
    }

    pub fn with_credentials(
        config_file: PathBuf,
        config: SyncConfig,
        credentials: CredentialStore,
    ) -> Self {
        Self {
            config_file,
            credentials,
            config,
        }
    }

    pub fn update<F: FnOnce(&mut SyncConfig)>(&mut self, f: F) -> Result<()> {
        let mut next = self.config.clone();
        f(&mut next);
        next.normalize_legacy_provider_flags();
        self.persist_config(&next)?;
        self.config = next;
        Ok(())
    }

    pub fn sync_from_disk(&mut self) {
        let Ok(content) = fs::read_to_string(&self.config_file) else {
            return;
        };
        let Ok(persisted) = toml::from_str::<SyncConfig>(&content) else {
            return;
        };
        self.config.last_sync_at = persisted.last_sync_at;
        self.config.gist_id = persisted.gist_id;
    }

    fn persist(&self) -> Result<()> {
        self.persist_config(&self.config)
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

#[cfg(test)]
mod tests {
    use super::*;
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
            VaultCredentialBackend::new_with_path(vault_path.clone(), "passphrase"),
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
    fn sync_from_disk_updates_last_sync_at() {
        let config_path = temp_sync_config_path();
        let credentials =
            CredentialStore::with_backend(APP_CREDENTIAL_SERVICE, LockedCredentialBackend);
        let mut store = SyncConfigStore::with_credentials(
            config_path.clone(),
            SyncConfig::default(),
            credentials.clone(),
        );
        store.persist().expect("initial config should persist");

        let updated_config = SyncConfig {
            last_sync_at: 42,
            ..SyncConfig::default()
        };
        SyncConfigStore::with_credentials(config_path.clone(), updated_config, credentials)
            .persist()
            .expect("updated config should persist");

        store.sync_from_disk();

        assert_eq!(store.config.last_sync_at, 42);

        let _ = fs::remove_file(config_path);
    }
}
