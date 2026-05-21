use crate::SecretKind;
use crate::credential_backend::{
    APP_CREDENTIAL_SERVICE, CredentialStore, LockedCredentialBackend, VaultCredentialBackend,
};
use anyhow::{Context, Result};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoredProfileSecrets {
    pub password: Option<String>,
    pub passphrase: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SecretStore {
    credentials: CredentialStore,
}

impl SecretStore {
    const LOCKED_VAULT_MESSAGE: &'static str = "local vault is locked";

    pub fn new() -> Self {
        Self {
            credentials: CredentialStore::new_keyring(APP_CREDENTIAL_SERVICE),
        }
    }

    pub fn with_credentials(credentials: CredentialStore) -> Self {
        Self { credentials }
    }

    pub fn new_vault(passphrase: impl Into<String>) -> Result<Self> {
        let credentials = CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            VaultCredentialBackend::new(passphrase)?,
        );
        credentials.initialize()?;
        Ok(Self::with_credentials(credentials))
    }

    pub fn new_locked_vault() -> Self {
        Self::with_credentials(CredentialStore::with_backend(
            APP_CREDENTIAL_SERVICE,
            LockedCredentialBackend,
        ))
    }

    fn account(&self, profile_id: &str, kind: SecretKind) -> String {
        format!("{profile_id}:{}", kind.suffix())
    }

    pub fn get(&self, profile_id: &str, kind: SecretKind) -> Result<Option<String>> {
        self.credentials
            .get(&self.account(profile_id, kind))
            .with_context(|| format!("failed to read secret for {profile_id}:{}", kind.suffix()))
    }

    pub fn get_profile_secrets(&self, profile_id: &str) -> Result<StoredProfileSecrets> {
        let password_account = self.account(profile_id, SecretKind::Password);
        let passphrase_account = self.account(profile_id, SecretKind::Passphrase);
        let values = self
            .credentials
            .get_many(&[password_account.as_str(), passphrase_account.as_str()])
            .with_context(|| format!("failed to read saved secrets for {profile_id}"))?;
        let mut values = values.into_iter();

        Ok(StoredProfileSecrets {
            password: values.next().flatten(),
            passphrase: values.next().flatten(),
        })
    }

    pub fn set(&self, profile_id: &str, kind: SecretKind, value: &str) -> Result<()> {
        self.credentials
            .set(&self.account(profile_id, kind), value)
            .with_context(|| format!("failed to store secret for {profile_id}:{}", kind.suffix()))
    }

    pub fn delete(&self, profile_id: &str, kind: SecretKind) -> Result<()> {
        self.credentials
            .delete(&self.account(profile_id, kind))
            .with_context(|| format!("failed to delete secret for {profile_id}:{}", kind.suffix()))
    }

    pub fn delete_all(&self, profile_id: &str) {
        for kind in [SecretKind::Password, SecretKind::Passphrase] {
            if let Err(error) = self.delete(profile_id, kind) {
                log::warn!("{error:?}");
            }
        }
    }

    pub fn delete_managed_key(&self, key_id: &str) {
        if let Err(error) = self.delete(key_id, SecretKind::ManagedPrivateKey) {
            log::warn!("{error:?}");
        }
    }

    pub fn is_locked_error(error: &anyhow::Error) -> bool {
        error
            .chain()
            .any(|cause| cause.to_string().contains(Self::LOCKED_VAULT_MESSAGE))
    }
}

impl Default for SecretStore {
    fn default() -> Self {
        Self::new()
    }
}
