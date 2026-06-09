use anyhow::Result;

use super::store::SyncConfigStore;
use miaominal_secrets::{SecretKind, SecretStore};

pub fn copy_secrets_between_backends<S, K>(
    session_ids: S,
    managed_key_ids: K,
    ai_provider_ids: &[String],
    source_secrets: &SecretStore,
    source_sync_config: &SyncConfigStore,
    target_secrets: &SecretStore,
    target_sync_config: &SyncConfigStore,
) -> Result<()>
where
    S: IntoIterator,
    S::Item: AsRef<str>,
    K: IntoIterator,
    K::Item: AsRef<str>,
{
    for session_id in session_ids {
        let session_id = session_id.as_ref();
        let profile_secrets = source_secrets.get_profile_secrets(session_id)?;

        if let Some(password) = profile_secrets.password {
            target_secrets.set(session_id, SecretKind::Password, &password)?;
        }
        if let Some(passphrase) = profile_secrets.passphrase {
            target_secrets.set(session_id, SecretKind::Passphrase, &passphrase)?;
        }
    }

    for key_id in managed_key_ids {
        let key_id = key_id.as_ref();
        if let Some(private_key) = source_secrets.get(key_id, SecretKind::ManagedPrivateKey)? {
            target_secrets.set(key_id, SecretKind::ManagedPrivateKey, &private_key)?;
        }
    }

    for provider_id in ai_provider_ids {
        if let Some(api_key) = source_secrets.get(provider_id, SecretKind::AiProviderApiKey)? {
            target_secrets.set(provider_id, SecretKind::AiProviderApiKey, &api_key)?;
        }
    }

    let sync_secrets = source_sync_config.get_secrets()?;

    if let Some(token) = sync_secrets.github_token {
        target_sync_config.set_github_token(&token)?;
    }
    if let Some(password) = sync_secrets.webdav_password {
        target_sync_config.set_webdav_password(&password)?;
    }
    if let Some(passphrase) = sync_secrets.passphrase {
        target_sync_config.set_passphrase(&passphrase)?;
    }

    Ok(())
}

pub fn delete_keyring_secrets<S, K>(
    session_ids: S,
    managed_key_ids: K,
    ai_provider_ids: &[String],
    source_secrets: &SecretStore,
    source_sync_config: &SyncConfigStore,
) where
    S: IntoIterator,
    S::Item: AsRef<str>,
    K: IntoIterator,
    K::Item: AsRef<str>,
{
    for session_id in session_ids {
        source_secrets.delete_all(session_id.as_ref());
    }

    for key_id in managed_key_ids {
        source_secrets.delete_managed_key(key_id.as_ref());
    }

    for provider_id in ai_provider_ids {
        source_secrets.delete_ai_provider_api_key(provider_id);
    }

    if let Err(error) = source_sync_config.delete_github_token() {
        log::warn!("failed to delete migrated GitHub token from keyring: {error:?}");
    }
    if let Err(error) = source_sync_config.delete_webdav_password() {
        log::warn!("failed to delete migrated WebDAV password from keyring: {error:?}");
    }
    if let Err(error) = source_sync_config.delete_passphrase() {
        log::warn!("failed to delete migrated sync passphrase from keyring: {error:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SyncConfig;
    use miaominal_secrets::{APP_CREDENTIAL_SERVICE, CredentialStore, VaultCredentialBackend};
    use std::path::PathBuf;

    fn temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "miaominal-credential-migration-{label}-{}.json",
            uuid::Uuid::new_v4()
        ))
    }

    fn temp_config_file(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "miaominal-credential-migration-{label}-{}.toml",
            uuid::Uuid::new_v4()
        ))
    }

    fn vault_secret_store(passphrase: &str, vault_path: PathBuf) -> SecretStore {
        let backend = VaultCredentialBackend::new_with_path(vault_path, passphrase);
        let credentials = CredentialStore::with_backend(APP_CREDENTIAL_SERVICE, backend);
        credentials
            .initialize()
            .expect("vault credential store initialize");
        SecretStore::with_credentials(credentials)
    }

    fn vault_sync_config(
        passphrase: &str,
        vault_path: PathBuf,
        config_file: PathBuf,
    ) -> SyncConfigStore {
        let backend = VaultCredentialBackend::new_with_path(vault_path, passphrase);
        let credentials = CredentialStore::with_backend(APP_CREDENTIAL_SERVICE, backend);
        credentials
            .initialize()
            .expect("vault credential store initialize");
        SyncConfigStore::with_credentials(config_file, SyncConfig::default(), credentials)
    }

    struct Fixture {
        source_secrets_path: PathBuf,
        source_sync_path: PathBuf,
        source_config_file: PathBuf,
        target_secrets_path: PathBuf,
        target_sync_path: PathBuf,
        target_config_file: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            Self {
                source_secrets_path: temp_path(&format!("{label}-src-secrets")),
                source_sync_path: temp_path(&format!("{label}-src-sync")),
                source_config_file: temp_config_file(&format!("{label}-src-cfg")),
                target_secrets_path: temp_path(&format!("{label}-tgt-secrets")),
                target_sync_path: temp_path(&format!("{label}-tgt-sync")),
                target_config_file: temp_config_file(&format!("{label}-tgt-cfg")),
            }
        }

        fn cleanup(&self) {
            for path in [
                &self.source_secrets_path,
                &self.source_sync_path,
                &self.target_secrets_path,
                &self.target_sync_path,
                &self.source_config_file,
                &self.target_config_file,
            ] {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    #[test]
    fn copies_session_password_passphrase_and_managed_key() {
        let fixture = Fixture::new("copy-basic");
        let source_secrets = vault_secret_store("source", fixture.source_secrets_path.clone());
        let target_secrets = vault_secret_store("target", fixture.target_secrets_path.clone());
        let source_sync = vault_sync_config(
            "source-sync",
            fixture.source_sync_path.clone(),
            fixture.source_config_file.clone(),
        );
        let target_sync = vault_sync_config(
            "target-sync",
            fixture.target_sync_path.clone(),
            fixture.target_config_file.clone(),
        );

        source_secrets
            .set("session-1", SecretKind::Password, "pw1")
            .unwrap();
        source_secrets
            .set("session-1", SecretKind::Passphrase, "pp1")
            .unwrap();
        source_secrets
            .set("session-2", SecretKind::Password, "pw2")
            .unwrap();
        source_secrets
            .set("key-1", SecretKind::ManagedPrivateKey, "private-key-bytes")
            .unwrap();
        source_secrets
            .set("provider-1", SecretKind::AiProviderApiKey, "sk-test")
            .unwrap();
        source_sync.set_github_token("gh-token").unwrap();
        source_sync.set_webdav_password("dav-pass").unwrap();
        source_sync.set_passphrase("sync-pp").unwrap();

        copy_secrets_between_backends(
            ["session-1", "session-2"],
            ["key-1"],
            &["provider-1".to_string()],
            &source_secrets,
            &source_sync,
            &target_secrets,
            &target_sync,
        )
        .expect("migration succeeds");

        assert_eq!(
            target_secrets
                .get("session-1", SecretKind::Password)
                .unwrap()
                .as_deref(),
            Some("pw1"),
        );
        assert_eq!(
            target_secrets
                .get("session-1", SecretKind::Passphrase)
                .unwrap()
                .as_deref(),
            Some("pp1"),
        );
        assert_eq!(
            target_secrets
                .get("session-2", SecretKind::Password)
                .unwrap()
                .as_deref(),
            Some("pw2"),
        );
        assert_eq!(
            target_secrets
                .get("session-2", SecretKind::Passphrase)
                .unwrap(),
            None,
        );
        assert_eq!(
            target_secrets
                .get("key-1", SecretKind::ManagedPrivateKey)
                .unwrap()
                .as_deref(),
            Some("private-key-bytes"),
        );
        assert_eq!(
            target_secrets
                .get("provider-1", SecretKind::AiProviderApiKey)
                .unwrap()
                .as_deref(),
            Some("sk-test"),
        );
        assert_eq!(
            target_sync.get_github_token().unwrap().as_deref(),
            Some("gh-token"),
        );
        assert_eq!(
            target_sync.get_webdav_password().unwrap().as_deref(),
            Some("dav-pass"),
        );
        assert_eq!(
            target_sync.get_passphrase().unwrap().as_deref(),
            Some("sync-pp"),
        );

        fixture.cleanup();
    }

    #[test]
    fn copy_skips_missing_optional_secrets() {
        let fixture = Fixture::new("copy-missing");
        let source_secrets = vault_secret_store("source", fixture.source_secrets_path.clone());
        let target_secrets = vault_secret_store("target", fixture.target_secrets_path.clone());
        let source_sync = vault_sync_config(
            "source-sync",
            fixture.source_sync_path.clone(),
            fixture.source_config_file.clone(),
        );
        let target_sync = vault_sync_config(
            "target-sync",
            fixture.target_sync_path.clone(),
            fixture.target_config_file.clone(),
        );

        source_sync.set_github_token("only-gh").unwrap();

        copy_secrets_between_backends(
            ["session-no-secrets"],
            std::iter::empty::<&str>(),
            &[],
            &source_secrets,
            &source_sync,
            &target_secrets,
            &target_sync,
        )
        .expect("migration succeeds with missing secrets");

        assert_eq!(
            target_secrets
                .get("session-no-secrets", SecretKind::Password)
                .unwrap(),
            None,
        );
        assert_eq!(
            target_sync.get_github_token().unwrap().as_deref(),
            Some("only-gh"),
        );
        assert_eq!(target_sync.get_webdav_password().unwrap(), None);
        assert_eq!(target_sync.get_passphrase().unwrap(), None);

        fixture.cleanup();
    }

    #[test]
    fn delete_keyring_secrets_clears_source_state() {
        let fixture = Fixture::new("delete");
        let source_secrets = vault_secret_store("source", fixture.source_secrets_path.clone());
        let source_sync = vault_sync_config(
            "source-sync",
            fixture.source_sync_path.clone(),
            fixture.source_config_file.clone(),
        );

        source_secrets
            .set("session-1", SecretKind::Password, "pw")
            .unwrap();
        source_secrets
            .set("session-1", SecretKind::Passphrase, "pp")
            .unwrap();
        source_secrets
            .set("key-1", SecretKind::ManagedPrivateKey, "key-bytes")
            .unwrap();
        source_secrets
            .set("provider-1", SecretKind::AiProviderApiKey, "sk-test")
            .unwrap();
        source_sync.set_github_token("gh").unwrap();
        source_sync.set_webdav_password("dav").unwrap();
        source_sync.set_passphrase("sp").unwrap();

        delete_keyring_secrets(
            ["session-1"],
            ["key-1"],
            &["provider-1".to_string()],
            &source_secrets,
            &source_sync,
        );

        assert_eq!(
            source_secrets
                .get("session-1", SecretKind::Password)
                .unwrap(),
            None,
        );
        assert_eq!(
            source_secrets
                .get("session-1", SecretKind::Passphrase)
                .unwrap(),
            None,
        );
        assert_eq!(
            source_secrets
                .get("key-1", SecretKind::ManagedPrivateKey)
                .unwrap(),
            None,
        );
        assert_eq!(
            source_secrets
                .get("provider-1", SecretKind::AiProviderApiKey)
                .unwrap(),
            None,
        );
        assert_eq!(source_sync.get_github_token().unwrap(), None);
        assert_eq!(source_sync.get_webdav_password().unwrap(), None);
        assert_eq!(source_sync.get_passphrase().unwrap(), None);

        fixture.cleanup();
    }
}
