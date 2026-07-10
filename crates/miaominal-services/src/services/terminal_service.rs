use miaominal_core::profile::SessionProfile;
use miaominal_secrets::SecretStore;
use miaominal_ssh::{self as ssh, SessionConnection};
use miaominal_storage::known_hosts_store::KnownHostsStore;
use tokio::runtime::Handle as TokioHandle;

#[derive(Clone)]
pub struct TerminalService {
    runtime: TokioHandle,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
}

impl TerminalService {
    pub fn new(runtime: TokioHandle, secrets: SecretStore, known_hosts: KnownHostsStore) -> Self {
        Self {
            runtime,
            secrets,
            known_hosts,
        }
    }

    pub fn start_session(
        &self,
        profile: SessionProfile,
        all_profiles: Vec<SessionProfile>,
        columns: usize,
        lines: usize,
        monitoring_enabled: bool,
    ) -> SessionConnection {
        ssh::start_session(
            &self.runtime,
            profile,
            all_profiles,
            self.secrets.clone(),
            self.known_hosts.clone(),
            columns,
            lines,
            monitoring_enabled,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_secrets::{
        APP_CREDENTIAL_SERVICE, CredentialStore, VaultCredentialBackend, set_vault_test_parameters,
    };
    use miaominal_storage::known_hosts_store::KnownHostsStore;
    use std::fs;

    #[test]
    fn hydrate_profile_loads_stored_password_and_passphrase() {
        set_vault_test_parameters();
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime should start");
        let _runtime_guard = runtime.enter();
        let path = std::env::temp_dir().join(format!(
            "miaominal-terminal-service-{}.json",
            uuid::Uuid::new_v4()
        ));
        let credentials = CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new_with_path(path.clone(), "passphrase"),
        );
        let secrets = SecretStore::with_credentials(credentials);
        secrets
            .set(
                "session-1",
                miaominal_secrets::SecretKind::Password,
                "hunter2",
            )
            .expect("password should save");
        secrets
            .set(
                "session-1",
                miaominal_secrets::SecretKind::Passphrase,
                "secret",
            )
            .expect("passphrase should save");

        let _known_hosts =
            KnownHostsStore::with_path(std::env::temp_dir().join("terminal-service-known-hosts"));
        let mut profile = SessionProfile::blank("session-1", 1);
        profile.has_stored_password = true;
        profile.has_stored_passphrase = true;

        let hydrated = ssh::hydrate_profile_from_secrets(profile, &secrets);

        assert_eq!(hydrated.password, "hunter2");
        assert_eq!(hydrated.passphrase, "secret");

        cleanup_test_vault(&path);
    }

    fn cleanup_test_vault(path: &std::path::Path) {
        let _ = fs::remove_file(path);
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let _ = fs::remove_file(std::path::PathBuf::from(lock_path));
    }
}
