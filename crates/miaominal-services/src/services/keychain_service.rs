use anyhow::Result;
use miaominal_core::keychain::{ManagedKeyRecord, ManagedKeySource};
use miaominal_core::profile::{AuthMethod, SessionProfile, ShellType};
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

    pub fn generate_material(&self) -> Result<(String, String)> {
        Self::generate_ed25519_material()
    }

    pub fn generate_ed25519_material() -> Result<(String, String)> {
        ManagedKeyStore::generate_ed25519_material()
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

    pub fn delete_key(
        &self,
        managed_keys: &mut Vec<ManagedKeyRecord>,
        sessions: &mut [SessionProfile],
        key_id: &str,
    ) -> Option<DeleteManagedKeyOutcome> {
        let index = managed_keys.iter().position(|key| key.id == key_id)?;
        let removed = managed_keys.remove(index);
        self.secrets.delete_managed_key(&removed.id);

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
        command: String,
    ) -> Result<String> {
        ssh::execute_profile_command(
            profile,
            all_profiles,
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
