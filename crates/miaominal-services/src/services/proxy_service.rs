use anyhow::{Context, Result, anyhow, bail};
use miaominal_core::{
    profile::SessionProfile,
    proxy::{ProxyAuthMode, ProxyProfile, ProxyProtocol},
};
use miaominal_secrets::{SecretKind, SecretStore};
use miaominal_storage::ProxyStore;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyPasswordUpdate {
    Keep,
    Set(String),
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpsertProxyOutcome {
    Inserted { index: usize },
    Updated { index: usize },
}

#[derive(Clone, Debug)]
pub struct ProxyService {
    store: Option<ProxyStore>,
    secrets: SecretStore,
}

impl ProxyService {
    pub fn new(store: Option<ProxyStore>, secrets: SecretStore) -> Self {
        Self { store, secrets }
    }

    pub fn next_proxy_id() -> String {
        format!("proxy-{}", Uuid::new_v4())
    }

    pub fn validate(proxies: &[ProxyProfile]) -> Result<()> {
        for (index, proxy) in proxies.iter().enumerate() {
            if proxy.id.trim().is_empty() {
                bail!("proxy id is required");
            }
            if proxy.name.trim().is_empty() {
                bail!("proxy name is required");
            }
            if proxy.host.trim().is_empty() {
                bail!("proxy host is required");
            }
            if proxy.host.chars().any(char::is_control)
                || proxy.host.chars().any(char::is_whitespace)
            {
                bail!("proxy host contains invalid whitespace or control characters");
            }
            if proxy.port == 0 {
                bail!("proxy port must be greater than zero");
            }
            if proxy.auth_mode == ProxyAuthMode::UsernamePassword {
                if proxy.username.trim().is_empty() {
                    bail!("proxy username is required when authentication is enabled");
                }
                if proxy.protocol == ProxyProtocol::HttpConnect && proxy.username.contains(':') {
                    bail!("HTTP CONNECT proxy username cannot contain a colon");
                }
            }

            if proxies[..index]
                .iter()
                .any(|candidate| candidate.id == proxy.id)
            {
                bail!("proxy id {} is duplicated", proxy.id);
            }
            if proxies[..index].iter().any(|candidate| {
                candidate
                    .name
                    .trim()
                    .eq_ignore_ascii_case(proxy.name.trim())
            }) {
                bail!("proxy name {} is duplicated", proxy.name.trim());
            }
        }
        Ok(())
    }

    pub fn validate_session_references(
        sessions: &[SessionProfile],
        proxies: &[ProxyProfile],
    ) -> Result<()> {
        for session in sessions {
            let Some(proxy_id) = session.entry_proxy_id.as_deref() else {
                continue;
            };
            if !proxies.iter().any(|proxy| proxy.id == proxy_id) {
                bail!(
                    "host {} references missing proxy {}",
                    session.connection_label(),
                    proxy_id
                );
            }
        }
        Ok(())
    }

    pub fn upsert(
        &self,
        proxies: &mut Vec<ProxyProfile>,
        mut proxy: ProxyProfile,
        password_update: ProxyPasswordUpdate,
    ) -> Result<UpsertProxyOutcome> {
        let _sync_guard = miaominal_secrets::lock_sync_data();
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| anyhow!("proxy store unavailable"))?;
        proxy.name = proxy.name.trim().to_string();
        proxy.host = proxy.host.trim().to_string();
        proxy.username = proxy.username.trim().to_string();

        if proxy.auth_mode == ProxyAuthMode::None {
            proxy.username.clear();
            proxy.has_stored_password = false;
        }

        let existing_index = proxies
            .iter()
            .position(|candidate| candidate.id == proxy.id);
        let previous_secret = self
            .secrets
            .get(&proxy.id, SecretKind::ProxyPassword)
            .context("failed to snapshot proxy password")?;
        let effective_update = if proxy.auth_mode == ProxyAuthMode::None {
            ProxyPasswordUpdate::Clear
        } else {
            password_update
        };

        match &effective_update {
            ProxyPasswordUpdate::Keep => {
                proxy.has_stored_password = previous_secret.is_some();
            }
            ProxyPasswordUpdate::Set(password) => {
                if password.is_empty() {
                    bail!("proxy password cannot be empty");
                }
                proxy.has_stored_password = true;
            }
            ProxyPasswordUpdate::Clear => proxy.has_stored_password = false,
        }
        let mut next = proxies.clone();
        let outcome = if let Some(index) = existing_index {
            next[index] = proxy.clone();
            UpsertProxyOutcome::Updated { index }
        } else {
            next.push(proxy.clone());
            UpsertProxyOutcome::Inserted {
                index: next.len() - 1,
            }
        };
        Self::validate(&next)?;

        if let Err(error) = self.apply_password_update(&proxy.id, &effective_update) {
            return Err(error.context("failed to update proxy password"));
        }
        if let Err(error) = store.save(&next) {
            let rollback = self.restore_password(&proxy.id, previous_secret.as_deref());
            return match rollback {
                Ok(()) => Err(error.context("failed to save proxy; password change rolled back")),
                Err(rollback_error) => Err(anyhow!(
                    "failed to save proxy: {error:#}; password rollback also failed: {rollback_error:#}"
                )),
            };
        }

        *proxies = next;
        Ok(outcome)
    }

    pub fn delete(
        &self,
        proxies: &mut Vec<ProxyProfile>,
        proxy_id: &str,
        sessions: &[SessionProfile],
    ) -> Result<ProxyProfile> {
        let _sync_guard = miaominal_secrets::lock_sync_data();
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| anyhow!("proxy store unavailable"))?;
        let referenced_by = sessions
            .iter()
            .filter(|session| session.entry_proxy_id.as_deref() == Some(proxy_id))
            .map(SessionProfile::connection_label)
            .collect::<Vec<_>>();
        if !referenced_by.is_empty() {
            bail!("proxy is used by: {}", referenced_by.join(", "));
        }
        let index = proxies
            .iter()
            .position(|proxy| proxy.id == proxy_id)
            .ok_or_else(|| anyhow!("proxy {proxy_id} is no longer available"))?;
        let removed = proxies[index].clone();
        let previous_secret = self
            .secrets
            .get(proxy_id, SecretKind::ProxyPassword)
            .context("failed to snapshot proxy password")?;
        self.secrets
            .delete(proxy_id, SecretKind::ProxyPassword)
            .context("failed to delete proxy password")?;

        let mut next = proxies.clone();
        next.remove(index);
        if let Err(error) = store.save(&next) {
            let rollback = self.restore_password(proxy_id, previous_secret.as_deref());
            return match rollback {
                Ok(()) => Err(error.context("failed to delete proxy; password restored")),
                Err(rollback_error) => Err(anyhow!(
                    "failed to delete proxy: {error:#}; password rollback also failed: {rollback_error:#}"
                )),
            };
        }
        *proxies = next;
        Ok(removed)
    }

    fn apply_password_update(&self, proxy_id: &str, update: &ProxyPasswordUpdate) -> Result<()> {
        match update {
            ProxyPasswordUpdate::Keep => Ok(()),
            ProxyPasswordUpdate::Set(password) => {
                self.secrets
                    .set(proxy_id, SecretKind::ProxyPassword, password)
            }
            ProxyPasswordUpdate::Clear => self.secrets.delete(proxy_id, SecretKind::ProxyPassword),
        }
    }

    fn restore_password(&self, proxy_id: &str, value: Option<&str>) -> Result<()> {
        match value {
            Some(value) => self.secrets.set(proxy_id, SecretKind::ProxyPassword, value),
            None => self.secrets.delete(proxy_id, SecretKind::ProxyPassword),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_secrets::{
        APP_CREDENTIAL_SERVICE, CredentialStore, ProtectedPassphrase, VaultCredentialBackend,
        set_vault_test_parameters,
    };
    use tempfile::tempdir;

    fn proxy(id: &str, name: &str) -> ProxyProfile {
        let mut proxy = ProxyProfile::blank(id, 1);
        proxy.name = name.into();
        proxy.host = "127.0.0.1".into();
        proxy
    }

    fn vault_secret_store(path: std::path::PathBuf) -> SecretStore {
        set_vault_test_parameters();
        let credentials = CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new_with_path(
                path,
                ProtectedPassphrase::try_from_string("proxy-service-test".to_string())
                    .expect("test passphrase should use protected memory"),
            ),
        );
        credentials
            .initialize()
            .expect("test credential store should initialize");
        SecretStore::with_credentials(credentials)
    }

    #[test]
    fn duplicate_names_are_rejected_case_insensitively() {
        let error = ProxyService::validate(&[proxy("proxy-1", "Local"), proxy("proxy-2", "local")])
            .expect_err("duplicate names should fail");

        assert!(error.to_string().contains("duplicated"));
    }

    #[test]
    fn referenced_proxy_cannot_be_deleted() {
        let root = tempdir().expect("tempdir should exist");
        let service = ProxyService::new(
            Some(ProxyStore::with_path(root.path().join("proxies.toml"))),
            SecretStore::new_locked_vault(),
        );
        let mut proxies = vec![proxy("proxy-1", "Local")];
        let mut session = SessionProfile::blank("session-1", 1);
        session.name = "Production".into();
        session.entry_proxy_id = Some("proxy-1".into());

        let error = service
            .delete(&mut proxies, "proxy-1", &[session])
            .expect_err("referenced proxy should not delete");

        assert!(error.to_string().contains("Production"));
        assert_eq!(proxies.len(), 1);
    }

    #[test]
    fn password_set_keep_and_clear_are_persisted_transactionally() {
        let root = tempdir().expect("tempdir should exist");
        let secrets = vault_secret_store(root.path().join("secrets.json"));
        let service = ProxyService::new(
            Some(ProxyStore::with_path(root.path().join("proxies.toml"))),
            secrets.clone(),
        );
        let mut proxies = Vec::new();
        let mut configured = proxy("proxy-1", "Local");
        configured.auth_mode = ProxyAuthMode::UsernamePassword;
        configured.username = "akko".into();

        service
            .upsert(
                &mut proxies,
                configured.clone(),
                ProxyPasswordUpdate::Set("secret".into()),
            )
            .expect("password should save");
        assert!(proxies[0].has_stored_password);
        assert_eq!(
            secrets
                .get("proxy-1", SecretKind::ProxyPassword)
                .expect("password should read")
                .as_deref(),
            Some("secret")
        );

        configured.name = "Renamed".into();
        service
            .upsert(&mut proxies, configured.clone(), ProxyPasswordUpdate::Keep)
            .expect("password should be kept");
        assert!(proxies[0].has_stored_password);

        service
            .upsert(&mut proxies, configured, ProxyPasswordUpdate::Clear)
            .expect("password should clear");
        assert!(!proxies[0].has_stored_password);
        assert_eq!(
            secrets
                .get("proxy-1", SecretKind::ProxyPassword)
                .expect("password state should read"),
            None
        );
    }

    #[test]
    fn failed_proxy_file_save_rolls_back_password_change() {
        let root = tempdir().expect("tempdir should exist");
        let secrets = vault_secret_store(root.path().join("secrets.json"));
        let blocking_parent = root.path().join("not-a-directory");
        std::fs::write(&blocking_parent, "block").expect("blocking file should save");
        let service = ProxyService::new(
            Some(ProxyStore::with_path(blocking_parent.join("proxies.toml"))),
            secrets.clone(),
        );
        let mut proxies = Vec::new();
        let mut configured = proxy("proxy-1", "Local");
        configured.auth_mode = ProxyAuthMode::UsernamePassword;
        configured.username = "akko".into();

        let error = service
            .upsert(
                &mut proxies,
                configured,
                ProxyPasswordUpdate::Set("secret".into()),
            )
            .expect_err("file save should fail");
        assert!(error.to_string().contains("rolled back"));
        assert!(proxies.is_empty());
        assert_eq!(
            secrets
                .get("proxy-1", SecretKind::ProxyPassword)
                .expect("rolled back password state should read"),
            None
        );
    }

    #[test]
    fn invalid_proxy_fields_are_rejected() {
        let mut invalid_host = proxy("proxy-1", "Invalid host");
        invalid_host.host = "proxy example".into();
        assert!(ProxyService::validate(&[invalid_host]).is_err());

        let mut invalid_username = proxy("proxy-2", "Invalid username");
        invalid_username.protocol = ProxyProtocol::HttpConnect;
        invalid_username.auth_mode = ProxyAuthMode::UsernamePassword;
        invalid_username.username = "user:name".into();
        assert!(ProxyService::validate(&[invalid_username]).is_err());
    }
}
