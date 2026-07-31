use crate::credential_backend::{
    APP_CREDENTIAL_SERVICE, CredentialStore, LockedCredentialBackend, VaultCredentialBackend,
};
use crate::{ProtectedPassphrase, SecretKind};
use anyhow::{Context, Result, anyhow};

const MANAGED_KEY_CHUNK_UTF16_UNITS: usize = 1024;
const MANAGED_KEY_CHUNK_MAX_COUNT: usize = 64;
const MANAGED_KEY_CHUNK_MANIFEST_PREFIX: &str = "\u{1f}miaominal.managed-key-chunks.v1:";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManagedKeyChunkManifest {
    generation: String,
    count: usize,
}

impl ManagedKeyChunkManifest {
    fn new(count: usize) -> Self {
        Self {
            generation: uuid::Uuid::new_v4().to_string(),
            count,
        }
    }

    fn parse(value: &str) -> Option<Self> {
        let payload = value.strip_prefix(MANAGED_KEY_CHUNK_MANIFEST_PREFIX)?;
        let (generation, count) = payload.rsplit_once(':')?;
        uuid::Uuid::parse_str(generation).ok()?;
        let count = count.parse::<usize>().ok()?;
        if !(1..=MANAGED_KEY_CHUNK_MAX_COUNT).contains(&count) {
            return None;
        }

        Some(Self {
            generation: generation.to_string(),
            count,
        })
    }

    fn encode(&self) -> String {
        format!(
            "{MANAGED_KEY_CHUNK_MANIFEST_PREFIX}{}:{}",
            self.generation, self.count
        )
    }

    fn chunk_account(&self, account: &str, index: usize) -> String {
        format!("{account}:chunk:{}:{index}", self.generation)
    }
}

fn managed_key_fits_single_credential(value: &str) -> bool {
    value.encode_utf16().count() <= MANAGED_KEY_CHUNK_UTF16_UNITS
}

fn split_managed_key_chunks(value: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut chunk = String::new();
    let mut chunk_units = 0;

    for character in value.chars() {
        let character_units = character.len_utf16();
        if chunk_units + character_units > MANAGED_KEY_CHUNK_UTF16_UNITS && !chunk.is_empty() {
            chunks.push(std::mem::take(&mut chunk));
            chunk_units = 0;
        }
        chunk.push(character);
        chunk_units += character_units;
    }

    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    chunks
}

#[derive(Clone, Default, PartialEq, Eq)]
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
    const REVOKED_VAULT_MESSAGE: &'static str = "local vault session has been revoked";

    pub fn new() -> Self {
        Self {
            credentials: CredentialStore::new_keyring(APP_CREDENTIAL_SERVICE),
        }
    }

    pub fn with_credentials(credentials: CredentialStore) -> Self {
        Self { credentials }
    }

    pub fn credentials(&self) -> CredentialStore {
        self.credentials.clone()
    }

    pub fn new_vault(passphrase: ProtectedPassphrase) -> Result<Self> {
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

    fn read_managed_key_chunks(
        &self,
        account: &str,
        manifest: &ManagedKeyChunkManifest,
    ) -> Result<String> {
        let accounts = (0..manifest.count)
            .map(|index| manifest.chunk_account(account, index))
            .collect::<Vec<_>>();
        let account_refs = accounts.iter().map(String::as_str).collect::<Vec<_>>();
        let chunks = self.credentials.get_many(&account_refs)?;
        let mut value = String::new();

        for (index, chunk) in chunks.into_iter().enumerate() {
            let chunk = chunk.ok_or_else(|| {
                anyhow!(
                    "managed private key chunk {index} of {} is missing",
                    manifest.count
                )
            })?;
            value.push_str(&chunk);
        }
        Ok(value)
    }

    fn delete_managed_key_chunks(
        &self,
        account: &str,
        manifest: &ManagedKeyChunkManifest,
    ) -> Result<()> {
        for index in 0..manifest.count {
            self.credentials
                .delete(&manifest.chunk_account(account, index))?;
        }
        Ok(())
    }

    fn cleanup_managed_key_chunks(&self, account: &str, manifest: &ManagedKeyChunkManifest) {
        if let Err(error) = self.delete_managed_key_chunks(account, manifest) {
            log::warn!("failed to clean up managed private key chunks for {account}: {error:?}");
        }
    }

    pub fn get(&self, profile_id: &str, kind: SecretKind) -> Result<Option<String>> {
        let account = self.account(profile_id, kind);
        let value = self
            .credentials
            .get(&account)
            .with_context(|| format!("failed to read secret for {profile_id}:{}", kind.suffix()))?;

        if kind != SecretKind::ManagedPrivateKey {
            return Ok(value);
        }

        let Some(value) = value else {
            return Ok(None);
        };
        let Some(manifest) = ManagedKeyChunkManifest::parse(&value) else {
            return Ok(Some(value));
        };

        self.read_managed_key_chunks(&account, &manifest)
            .map(Some)
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
        let account = self.account(profile_id, kind);
        if kind != SecretKind::ManagedPrivateKey {
            return self.credentials.set(&account, value).with_context(|| {
                format!("failed to store secret for {profile_id}:{}", kind.suffix())
            });
        }

        let previous_manifest = self
            .credentials
            .get(&account)
            .with_context(|| {
                format!(
                    "failed to inspect secret for {profile_id}:{}",
                    kind.suffix()
                )
            })?
            .as_deref()
            .and_then(ManagedKeyChunkManifest::parse);

        if managed_key_fits_single_credential(value) {
            self.credentials.set(&account, value).with_context(|| {
                format!("failed to store secret for {profile_id}:{}", kind.suffix())
            })?;
            if let Some(manifest) = previous_manifest.as_ref() {
                self.cleanup_managed_key_chunks(&account, manifest);
            }
            return Ok(());
        }

        let chunks = split_managed_key_chunks(value);
        if chunks.len() > MANAGED_KEY_CHUNK_MAX_COUNT {
            return Err(anyhow!(
                "managed private key requires {} credential chunks, exceeding the limit of {}",
                chunks.len(),
                MANAGED_KEY_CHUNK_MAX_COUNT
            ))
            .with_context(|| format!("failed to store secret for {profile_id}:{}", kind.suffix()));
        }
        let manifest = ManagedKeyChunkManifest::new(chunks.len());

        for (index, chunk) in chunks.iter().enumerate() {
            if let Err(error) = self
                .credentials
                .set(&manifest.chunk_account(&account, index), chunk)
            {
                let partial_manifest = ManagedKeyChunkManifest {
                    generation: manifest.generation.clone(),
                    count: index,
                };
                self.cleanup_managed_key_chunks(&account, &partial_manifest);
                return Err(error).with_context(|| {
                    format!(
                        "failed to store secret chunk {index} for {profile_id}:{}",
                        kind.suffix()
                    )
                });
            }
        }

        if let Err(error) = self.credentials.set(&account, &manifest.encode()) {
            self.cleanup_managed_key_chunks(&account, &manifest);
            return Err(error).with_context(|| {
                format!("failed to store secret for {profile_id}:{}", kind.suffix())
            });
        }
        if let Some(previous_manifest) = previous_manifest.as_ref() {
            self.cleanup_managed_key_chunks(&account, previous_manifest);
        }
        Ok(())
    }

    pub fn delete(&self, profile_id: &str, kind: SecretKind) -> Result<()> {
        let account = self.account(profile_id, kind);
        let manifest = if kind == SecretKind::ManagedPrivateKey {
            self.credentials
                .get(&account)
                .with_context(|| {
                    format!(
                        "failed to inspect secret for {profile_id}:{}",
                        kind.suffix()
                    )
                })?
                .as_deref()
                .and_then(ManagedKeyChunkManifest::parse)
        } else {
            None
        };

        self.credentials.delete(&account).with_context(|| {
            format!("failed to delete secret for {profile_id}:{}", kind.suffix())
        })?;
        if let Some(manifest) = manifest.as_ref() {
            self.delete_managed_key_chunks(&account, manifest)
                .with_context(|| {
                    format!("failed to delete secret for {profile_id}:{}", kind.suffix())
                })?;
        }
        Ok(())
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

    pub fn delete_ai_provider_api_key(&self, provider_id: &str) {
        if let Err(error) = self.delete(provider_id, SecretKind::AiProviderApiKey) {
            log::warn!("{error:?}");
        }
    }

    pub fn delete_web_search_api_key(&self) {
        if let Err(error) = self.delete("web_search", SecretKind::WebSearchApiKey) {
            log::warn!("{error:?}");
        }
    }

    pub fn delete_proxy_password(&self, proxy_id: &str) {
        if let Err(error) = self.delete(proxy_id, SecretKind::ProxyPassword) {
            log::warn!("{error:?}");
        }
    }

    pub fn is_locked_error(error: &anyhow::Error) -> bool {
        error.chain().any(|cause| {
            let message = cause.to_string();
            message.contains(Self::LOCKED_VAULT_MESSAGE)
                || message.contains(Self::REVOKED_VAULT_MESSAGE)
        })
    }
}

impl Default for SecretStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential_backend::CredentialBackend;
    use crate::{APP_CREDENTIAL_SERVICE, CredentialStore};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct LimitedMemoryBackend {
        values: Arc<Mutex<BTreeMap<String, String>>>,
    }

    impl CredentialBackend for LimitedMemoryBackend {
        fn name(&self) -> &'static str {
            "limited-memory"
        }

        fn get(&self, _service: &str, account: &str) -> Result<Option<String>> {
            Ok(self
                .values
                .lock()
                .expect("memory credential lock")
                .get(account)
                .cloned())
        }

        fn set(&self, _service: &str, account: &str, value: &str) -> Result<()> {
            if value.encode_utf16().count() > MANAGED_KEY_CHUNK_UTF16_UNITS {
                return Err(anyhow!("credential value is too long"));
            }
            self.values
                .lock()
                .expect("memory credential lock")
                .insert(account.to_string(), value.to_string());
            Ok(())
        }

        fn delete(&self, _service: &str, account: &str) -> Result<()> {
            self.values
                .lock()
                .expect("memory credential lock")
                .remove(account);
            Ok(())
        }
    }

    fn limited_secret_store() -> (SecretStore, LimitedMemoryBackend) {
        let backend = LimitedMemoryBackend::default();
        let credentials = CredentialStore::with_backend("test", backend.clone());
        (SecretStore::with_credentials(credentials), backend)
    }

    #[test]
    fn v0_1_keyring_identifiers_remain_stable() {
        assert_eq!(APP_CREDENTIAL_SERVICE, "dev.akko.miaominal");
        assert_eq!(SecretKind::Password.suffix(), "password");
        assert_eq!(SecretKind::Passphrase.suffix(), "passphrase");
        assert_eq!(
            SecretKind::ManagedPrivateKey.suffix(),
            "managed-private-key"
        );
    }

    #[test]
    fn revoked_protected_memory_is_reported_as_a_locked_vault() {
        let error = anyhow::anyhow!("local vault session has been revoked");

        assert!(SecretStore::is_locked_error(&error));
    }

    #[test]
    fn large_managed_private_key_is_chunked_and_round_trips() {
        let (store, backend) = limited_secret_store();
        let private_key = format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\n",
            "A".repeat(3600)
        );

        store
            .set("managed-key-1", SecretKind::ManagedPrivateKey, &private_key)
            .expect("large private key should be chunked");

        assert_eq!(
            store
                .get("managed-key-1", SecretKind::ManagedPrivateKey)
                .expect("chunked key should load")
                .as_deref(),
            Some(private_key.as_str())
        );
        let values = backend.values.lock().expect("memory credential lock");
        let manifest = values
            .get("managed-key-1:managed-private-key")
            .and_then(|value| ManagedKeyChunkManifest::parse(value))
            .expect("primary credential should contain a chunk manifest");
        assert!(manifest.count > 1);
        for index in 0..manifest.count {
            let chunk = values
                .get(&manifest.chunk_account("managed-key-1:managed-private-key", index))
                .expect("chunk should exist");
            assert!(chunk.encode_utf16().count() <= MANAGED_KEY_CHUNK_UTF16_UNITS);
        }
    }

    #[test]
    fn replacing_chunked_managed_key_with_small_value_cleans_old_chunks() {
        let (store, backend) = limited_secret_store();
        let large_private_key = "A".repeat(3600);
        store
            .set(
                "managed-key-1",
                SecretKind::ManagedPrivateKey,
                &large_private_key,
            )
            .expect("large private key should save");

        store
            .set(
                "managed-key-1",
                SecretKind::ManagedPrivateKey,
                "small-private-key",
            )
            .expect("small replacement should save");

        assert_eq!(
            store
                .get("managed-key-1", SecretKind::ManagedPrivateKey)
                .expect("replacement should load")
                .as_deref(),
            Some("small-private-key")
        );
        let values = backend.values.lock().expect("memory credential lock");
        assert_eq!(
            values
                .get("managed-key-1:managed-private-key")
                .map(String::as_str),
            Some("small-private-key")
        );
        assert!(values.keys().all(|account| !account.contains(":chunk:")));
    }

    #[test]
    fn deleting_chunked_managed_key_removes_manifest_and_chunks() {
        let (store, backend) = limited_secret_store();
        store
            .set(
                "managed-key-1",
                SecretKind::ManagedPrivateKey,
                &"A".repeat(3600),
            )
            .expect("large private key should save");

        store
            .delete("managed-key-1", SecretKind::ManagedPrivateKey)
            .expect("chunked private key should delete");

        assert!(
            backend
                .values
                .lock()
                .expect("memory credential lock")
                .is_empty()
        );
    }
}
