use anyhow::Result;
use miaominal_core::keychain::{ManagedKeyGenerationAlgorithm, ManagedKeyRecord, ManagedKeySource};
use miaominal_core::profile::{AuthMethod, SessionProfile, ShellType};
use miaominal_core::proxy::ProxyProfile;
use miaominal_secrets::{SecretKind, SecretStore};
use miaominal_ssh::{self as ssh, AgentIdentitySummary};
use miaominal_storage::keychain_store::ManagedKeyStore;
use miaominal_storage::known_hosts_store::KnownHostsStore;
use tokio::runtime::Handle as TokioHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedManagedKey {
    pub record: ManagedKeyRecord,
    pub normalized_private_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteManagedKeyOutcome {
    pub removed: ManagedKeyRecord,
    pub cleared_profile_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct KeychainRefreshData {
    pub managed_keys: Vec<ManagedKeyRecord>,
    pub agent_identities: Vec<AgentIdentitySummary>,
    pub agent_scan_error: Option<String>,
}

#[derive(Clone)]
pub struct KeychainService {
    runtime: TokioHandle,
    store: ManagedKeyStore,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
}

impl KeychainService {
    pub fn new(
        runtime: TokioHandle,
        store: ManagedKeyStore,
        secrets: SecretStore,
        known_hosts: KnownHostsStore,
    ) -> Self {
        Self {
            runtime,
            store,
            secrets,
            known_hosts,
        }
    }

    pub fn refresh_data(&self) -> Result<KeychainRefreshData> {
        let agent_scan_error = match self.runtime.block_on(ssh::list_local_agent_identities()) {
            Ok(agent_identities) => {
                return Ok(KeychainRefreshData {
                    managed_keys: self.store.load()?,
                    agent_identities,
                    agent_scan_error: None,
                });
            }
            Err(error) => Some(error.to_string()),
        };

        Ok(KeychainRefreshData {
            managed_keys: self.store.load()?,
            agent_identities: Vec::new(),
            agent_scan_error,
        })
    }

    pub fn generate_material(algorithm: ManagedKeyGenerationAlgorithm) -> Result<(String, String)> {
        ManagedKeyStore::generate_material(algorithm)
    }

    pub fn import_key(
        &self,
        existing_keys: &[ManagedKeyRecord],
        name: String,
        source: ManagedKeySource,
        private_key_material: &str,
        public_key_material: Option<&str>,
        passphrase: Option<&str>,
    ) -> Result<ImportedManagedKey> {
        let _sync_guard = miaominal_secrets::lock_sync_data();
        let (record, normalized_private_key) = self.store.import_private_key(
            existing_keys,
            name,
            source,
            private_key_material,
            public_key_material,
            passphrase,
        )?;

        self.secrets.set(
            &record.id,
            SecretKind::ManagedPrivateKey,
            &normalized_private_key,
        )?;

        Ok(ImportedManagedKey {
            record,
            normalized_private_key,
        })
    }

    pub fn persist_keys(&self, keys: &[ManagedKeyRecord]) -> Result<()> {
        self.store.save(keys)
    }

    /// Import the private key secret and publish its metadata under one outer
    /// sync-data gate so payload readers can never observe only one half.
    pub fn import_and_persist_key(
        &self,
        existing_keys: &[ManagedKeyRecord],
        name: String,
        source: ManagedKeySource,
        private_key_material: &str,
        public_key_material: Option<&str>,
        passphrase: Option<&str>,
    ) -> Result<ImportedManagedKey> {
        let _sync_guard = miaominal_secrets::lock_sync_data();
        let imported = self.import_key(
            existing_keys,
            name,
            source,
            private_key_material,
            public_key_material,
            passphrase,
        )?;
        let mut updated_keys = existing_keys.to_vec();
        updated_keys.push(imported.record.clone());

        if let Err(error) = self.persist_keys(&updated_keys) {
            return match self
                .secrets
                .delete(&imported.record.id, SecretKind::ManagedPrivateKey)
            {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(anyhow::anyhow!(
                    "failed to persist managed key metadata: {error:#}; removing the imported private key also failed: {rollback_error:#}"
                )),
            };
        }

        Ok(imported)
    }

    /// Remove a managed-key secret and metadata record atomically with respect
    /// to sync payload reads. If persisting the remaining records fails, the
    /// previous private key is restored before the gate is released.
    pub fn delete_and_persist_key(
        &self,
        managed_keys: &mut Vec<ManagedKeyRecord>,
        key_id: &str,
    ) -> Result<Option<ManagedKeyRecord>> {
        let _sync_guard = miaominal_secrets::lock_sync_data();
        let Some(index) = managed_keys.iter().position(|key| key.id == key_id) else {
            return Ok(None);
        };
        let previous_secret = self.secrets.get(key_id, SecretKind::ManagedPrivateKey)?;
        let removed = managed_keys.remove(index);

        if previous_secret.is_some()
            && let Err(error) = self.secrets.delete(key_id, SecretKind::ManagedPrivateKey)
        {
            managed_keys.insert(index, removed);
            return Err(error);
        }
        if let Err(error) = self.persist_keys(managed_keys) {
            managed_keys.insert(index, removed.clone());
            return match previous_secret.as_deref() {
                Some(previous) => {
                    match self
                        .secrets
                        .set(key_id, SecretKind::ManagedPrivateKey, previous)
                    {
                        Ok(()) => Err(error),
                        Err(rollback_error) => Err(anyhow::anyhow!(
                            "failed to persist managed key removal: {error:#}; restoring the private key also failed: {rollback_error:#}"
                        )),
                    }
                }
                None => Err(error),
            };
        }

        Ok(Some(removed))
    }

    pub fn delete_key(
        &self,
        managed_keys: &mut Vec<ManagedKeyRecord>,
        sessions: &mut [SessionProfile],
        key_id: &str,
    ) -> Option<DeleteManagedKeyOutcome> {
        let removed = self.delete_key_record(managed_keys, key_id)?;

        let mut cleared_profile_ids = Vec::new();
        for profile in sessions {
            if profile.managed_key_id == removed.id {
                profile.managed_key_id.clear();
                if profile.auth_method == Some(AuthMethod::ManagedKey) {
                    profile.auth_method = Some(AuthMethod::Password);
                }
                cleared_profile_ids.push(profile.id.clone());
            }
        }

        Some(DeleteManagedKeyOutcome {
            removed,
            cleared_profile_ids,
        })
    }

    pub fn delete_key_record(
        &self,
        managed_keys: &mut Vec<ManagedKeyRecord>,
        key_id: &str,
    ) -> Option<ManagedKeyRecord> {
        let index = managed_keys.iter().position(|key| key.id == key_id)?;
        let removed = managed_keys.remove(index);
        self.secrets.delete_managed_key(&removed.id);
        Some(removed)
    }

    pub fn profile_supports_deploy(profile: &SessionProfile) -> bool {
        !profile.host.trim().is_empty()
            && !profile.username.trim().is_empty()
            && !matches!(
                profile.effective_auth_method(),
                AuthMethod::KeyboardInteractive
            )
            && !matches!(profile.shell_type, ShellType::PowerShell | ShellType::Cmd)
    }

    pub fn deploy_command(
        template: &str,
        location: &str,
        filename: &str,
        public_key: &str,
    ) -> String {
        format!(
            "sh -lc {} gpui-keychain-deploy {} {} {}",
            shell_quote(template),
            shell_quote(location),
            shell_quote(filename),
            shell_quote(public_key),
        )
    }

    pub async fn execute_deploy(
        &self,
        profile: SessionProfile,
        all_profiles: Vec<SessionProfile>,
        all_proxies: Vec<ProxyProfile>,
        command: String,
    ) -> Result<String> {
        ssh::execute_profile_command(
            profile,
            all_profiles,
            all_proxies,
            self.secrets.clone(),
            self.known_hosts.clone(),
            command,
        )
        .await
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deploy_exec_command_uses_positional_arguments() {
        let command = KeychainService::deploy_command(
            "echo $1/$2/$3",
            ".ssh",
            "authorized_keys",
            "ssh-ed25519 AAAA",
        );

        assert_eq!(
            command,
            "sh -lc 'echo $1/$2/$3' gpui-keychain-deploy '.ssh' 'authorized_keys' 'ssh-ed25519 AAAA'"
        );
    }

    #[test]
    fn deploy_exec_command_escapes_single_quotes() {
        let command =
            KeychainService::deploy_command("echo '$3'", "/tmp/o'clock", "keys", "ssh 'key'");

        assert!(command.contains("'echo '\"'\"'$3'\"'\"''"));
        assert!(command.contains("'/tmp/o'\"'\"'clock'"));
        assert!(command.contains("'ssh '\"'\"'key'\"'\"''"));
    }

    #[test]
    fn deploy_support_rejects_keyboard_interactive_profiles() {
        let mut profile = SessionProfile::blank("session-1", 1);
        profile.host = "example.com".into();
        profile.username = "akko".into();
        profile.auth_method = Some(AuthMethod::KeyboardInteractive);

        assert!(!KeychainService::profile_supports_deploy(&profile));
    }
}
