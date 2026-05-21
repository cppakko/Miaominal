use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{Context, Result, anyhow};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine as _;
use keyring::Entry;
use miaominal_paths as paths;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const APP_CREDENTIAL_SERVICE: &str = "dev.akko.miaominal";

const VAULT_FILE_NAME: &str = "secret_vault.json";
const VAULT_VERSION: u32 = 1;
const VAULT_OUTPUT_LEN: usize = 32;
const VAULT_MEMORY_COST: u32 = 65536;
const VAULT_TIME_COST: u32 = 3;
const VAULT_PARALLELISM: u32 = 4;
const VAULT_AAD: &[u8] = b"miaominal.secret-vault.v1";
const VAULT_METADATA_SERVICE: &str = "__vault__";
const VAULT_METADATA_ACCOUNT: &str = "status";
const VAULT_METADATA_VALUE: &str = "ready";

pub trait CredentialBackend: Send + Sync {
    fn name(&self) -> &'static str;

    fn initialize(&self, _service: &str) -> Result<()> {
        Ok(())
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<String>>;

    fn get_many(&self, service: &str, accounts: &[&str]) -> Result<Vec<Option<String>>> {
        accounts
            .iter()
            .map(|account| self.get(service, account))
            .collect()
    }

    fn set(&self, service: &str, account: &str, value: &str) -> Result<()>;

    fn delete(&self, service: &str, account: &str) -> Result<()>;
}

#[derive(Clone)]
pub struct CredentialStore {
    service: Arc<str>,
    backend: Arc<dyn CredentialBackend>,
}

impl CredentialStore {
    pub fn new_keyring(service: impl Into<Arc<str>>) -> Self {
        Self {
            service: service.into(),
            backend: Arc::new(KeyringCredentialBackend),
        }
    }

    pub fn with_backend(
        service: impl Into<Arc<str>>,
        backend: impl CredentialBackend + 'static,
    ) -> Self {
        Self {
            service: service.into(),
            backend: Arc::new(backend),
        }
    }

    pub fn get(&self, account: &str) -> Result<Option<String>> {
        self.backend.get(&self.service, account)
    }

    pub fn get_many(&self, accounts: &[&str]) -> Result<Vec<Option<String>>> {
        self.backend.get_many(&self.service, accounts)
    }

    pub fn initialize(&self) -> Result<()> {
        self.backend.initialize(&self.service)
    }

    pub fn set(&self, account: &str, value: &str) -> Result<()> {
        self.backend.set(&self.service, account, value)
    }

    pub fn delete(&self, account: &str) -> Result<()> {
        self.backend.delete(&self.service, account)
    }
}

impl std::fmt::Debug for CredentialStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialStore")
            .field("service", &self.service)
            .field("backend", &self.backend.name())
            .finish()
    }
}

#[derive(Debug, Default)]
pub struct KeyringCredentialBackend;

#[derive(Debug, Default)]
pub struct LockedCredentialBackend;

impl KeyringCredentialBackend {
    fn entry(service: &str, account: &str) -> Result<Entry> {
        Entry::new(service, account)
            .with_context(|| format!("failed to access keyring entry for {service}/{account}"))
    }
}

impl CredentialBackend for KeyringCredentialBackend {
    fn name(&self) -> &'static str {
        "keyring"
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<String>> {
        let entry = Self::entry(service, account)?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error)
                .with_context(|| format!("failed to read keyring secret for {service}/{account}")),
        }
    }

    fn set(&self, service: &str, account: &str, value: &str) -> Result<()> {
        Self::entry(service, account)?
            .set_password(value)
            .with_context(|| format!("failed to store keyring secret for {service}/{account}"))
    }

    fn delete(&self, service: &str, account: &str) -> Result<()> {
        match Self::entry(service, account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error).with_context(|| {
                format!("failed to delete keyring secret for {service}/{account}")
            }),
        }
    }
}

impl CredentialBackend for LockedCredentialBackend {
    fn name(&self) -> &'static str {
        "vault-locked"
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<String>> {
        Err(locked_vault_error(service, account))
    }

    fn set(&self, service: &str, account: &str, _value: &str) -> Result<()> {
        Err(locked_vault_error(service, account))
    }

    fn delete(&self, service: &str, account: &str) -> Result<()> {
        Err(locked_vault_error(service, account))
    }
}

#[derive(Debug)]
pub struct VaultCredentialBackend {
    file_path: PathBuf,
    passphrase: String,
    io_lock: Mutex<()>,
}

impl VaultCredentialBackend {
    pub fn new(passphrase: impl Into<String>) -> Result<Self> {
        Ok(Self::new_with_path(Self::default_file_path()?, passphrase))
    }

    pub fn new_with_path(file_path: PathBuf, passphrase: impl Into<String>) -> Self {
        Self {
            file_path,
            passphrase: passphrase.into(),
            io_lock: Mutex::new(()),
        }
    }

    pub fn default_store_exists() -> Result<bool> {
        let path = Self::default_file_path()?;
        Ok(path.exists())
    }

    pub fn erase_default_store_file() -> Result<()> {
        let file_path = Self::default_file_path()?;
        if !file_path.exists() {
            return Ok(());
        }

        fs::remove_file(&file_path)
            .with_context(|| format!("failed to remove {}", file_path.display()))
    }

    pub fn rotate_default_store_passphrase(
        old_passphrase: &str,
        new_passphrase: &str,
    ) -> Result<()> {
        Self::rotate_store_passphrase(Self::default_file_path()?, old_passphrase, new_passphrase)
    }

    fn default_file_path() -> Result<PathBuf> {
        paths::config_file(VAULT_FILE_NAME)
    }

    fn rotate_store_passphrase(
        file_path: PathBuf,
        old_passphrase: &str,
        new_passphrase: &str,
    ) -> Result<()> {
        let current = Self::new_with_path(file_path.clone(), old_passphrase.to_string());
        let document = current.load_document()?;
        let next = Self::new_with_path(file_path, new_passphrase.to_string());
        next.save_document(&document)
    }

    fn load_document(&self) -> Result<VaultDocument> {
        if !self.file_path.exists() {
            return Ok(VaultDocument::default());
        }

        let serialized = fs::read_to_string(&self.file_path)
            .with_context(|| format!("failed to read {}", self.file_path.display()))?;
        if serialized.trim().is_empty() {
            return Ok(VaultDocument::default());
        }

        let stored: StoredVaultDocument = serde_json::from_str(&serialized)
            .with_context(|| format!("failed to parse {}", self.file_path.display()))?;
        self.decrypt_document(stored)
    }

    fn initialize_store(&self) -> Result<()> {
        let _guard = self
            .io_lock
            .lock()
            .map_err(|_| anyhow!("vault lock poisoned"))?;

        if !self.file_path.exists() {
            return self.save_document(&VaultDocument::default());
        }

        let serialized = fs::read_to_string(&self.file_path)
            .with_context(|| format!("failed to read {}", self.file_path.display()))?;
        if serialized.trim().is_empty() {
            return self.save_document(&VaultDocument::default());
        }

        let stored: StoredVaultDocument = serde_json::from_str(&serialized)
            .with_context(|| format!("failed to parse {}", self.file_path.display()))?;
        self.decrypt_document(stored).map(|_| ())
    }

    fn save_document(&self, document: &VaultDocument) -> Result<()> {
        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let mut document = document.clone();
        Self::account_map_mut(&mut document, VAULT_METADATA_SERVICE)
            .entry(VAULT_METADATA_ACCOUNT.to_string())
            .or_insert_with(|| VAULT_METADATA_VALUE.to_string());

        let stored = self.encrypt_document(&document)?;
        let serialized =
            serde_json::to_string_pretty(&stored).context("failed to serialize vault document")?;
        fs::write(&self.file_path, serialized)
            .with_context(|| format!("failed to write {}", self.file_path.display()))
    }

    fn encrypt_document(&self, document: &VaultDocument) -> Result<StoredVaultDocument> {
        let salt_bytes: [u8; 32] = rand::random();
        let key = derive_key(&self.passphrase, &salt_bytes)?;
        let plaintext = serde_json::to_vec(document).context("failed to serialize vault")?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(
                &nonce,
                aes_gcm::aead::Payload {
                    msg: &plaintext,
                    aad: VAULT_AAD,
                },
            )
            .map_err(|error| anyhow!("vault encryption failed: {error}"))?;

        let mut combined = nonce.to_vec();
        combined.extend_from_slice(&ciphertext);

        Ok(StoredVaultDocument {
            version: VAULT_VERSION,
            salt: base64::engine::general_purpose::STANDARD.encode(salt_bytes),
            encrypted_payload: base64::engine::general_purpose::STANDARD.encode(combined),
        })
    }

    fn decrypt_document(&self, stored: StoredVaultDocument) -> Result<VaultDocument> {
        if stored.version != VAULT_VERSION {
            anyhow::bail!("unsupported vault version: {}", stored.version);
        }

        let salt = base64::engine::general_purpose::STANDARD
            .decode(stored.salt)
            .context("failed to decode vault salt")?;
        if salt.len() != 32 {
            anyhow::bail!("vault salt must be 32 bytes");
        }

        let key = derive_key(&self.passphrase, &salt)?;
        let combined = base64::engine::general_purpose::STANDARD
            .decode(stored.encrypted_payload)
            .context("failed to decode vault payload")?;
        if combined.len() < 12 {
            anyhow::bail!("vault payload missing nonce");
        }
        let (nonce_bytes, ciphertext) = combined.split_at(12);

        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(nonce_bytes),
                aes_gcm::aead::Payload {
                    msg: ciphertext,
                    aad: VAULT_AAD,
                },
            )
            .map_err(|error| anyhow!("vault decryption failed: {error}"))?;

        serde_json::from_slice(&plaintext).context("failed to parse decrypted vault")
    }

    fn with_document<T>(&self, f: impl FnOnce(&mut VaultDocument) -> Result<T>) -> Result<T> {
        let _guard = self
            .io_lock
            .lock()
            .map_err(|_| anyhow!("vault lock poisoned"))?;
        let mut document = self.load_document()?;
        let output = f(&mut document)?;
        self.save_document(&document)?;
        Ok(output)
    }

    fn account_map_mut<'a>(
        document: &'a mut VaultDocument,
        service: &str,
    ) -> &'a mut BTreeMap<String, String> {
        document.services.entry(service.to_string()).or_default()
    }
}

impl CredentialBackend for VaultCredentialBackend {
    fn name(&self) -> &'static str {
        "vault"
    }

    fn initialize(&self, _service: &str) -> Result<()> {
        self.initialize_store()
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<String>> {
        let _guard = self
            .io_lock
            .lock()
            .map_err(|_| anyhow!("vault lock poisoned"))?;
        let document = self.load_document()?;
        Ok(document
            .services
            .get(service)
            .and_then(|accounts| accounts.get(account).cloned()))
    }

    fn get_many(&self, service: &str, accounts: &[&str]) -> Result<Vec<Option<String>>> {
        let _guard = self
            .io_lock
            .lock()
            .map_err(|_| anyhow!("vault lock poisoned"))?;
        let document = self.load_document()?;
        let stored_accounts = document.services.get(service);

        Ok(accounts
            .iter()
            .map(|account| stored_accounts.and_then(|values| values.get(*account).cloned()))
            .collect())
    }

    fn set(&self, service: &str, account: &str, value: &str) -> Result<()> {
        self.with_document(|document| {
            Self::account_map_mut(document, service).insert(account.to_string(), value.to_string());
            Ok(())
        })
    }

    fn delete(&self, service: &str, account: &str) -> Result<()> {
        self.with_document(|document| {
            if let Some(accounts) = document.services.get_mut(service) {
                accounts.remove(account);
                if accounts.is_empty() {
                    document.services.remove(service);
                }
            }
            Ok(())
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredVaultDocument {
    version: u32,
    salt: String,
    encrypted_payload: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct VaultDocument {
    #[serde(default)]
    services: BTreeMap<String, BTreeMap<String, String>>,
}

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; 32]> {
    let params = Params::new(
        VAULT_MEMORY_COST,
        VAULT_TIME_COST,
        VAULT_PARALLELISM,
        Some(VAULT_OUTPUT_LEN),
    )
    .map_err(|error| anyhow!("failed to create vault Argon2 params: {error}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|error| anyhow!("vault key derivation failed: {error}"))?;
    Ok(key)
}

fn locked_vault_error(service: &str, account: &str) -> anyhow::Error {
    anyhow!("local vault is locked; unlock it before accessing {service}/{account}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_backend_round_trips_credentials() {
        let path = test_vault_path("round-trip");
        let backend = VaultCredentialBackend::new_with_path(path.clone(), "correct horse");

        backend
            .set(APP_CREDENTIAL_SERVICE, "sync:github-token", "secret-token")
            .expect("set should succeed");

        let value = backend
            .get(APP_CREDENTIAL_SERVICE, "sync:github-token")
            .expect("get should succeed");

        assert_eq!(value.as_deref(), Some("secret-token"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn vault_backend_rejects_wrong_passphrase() {
        let path = test_vault_path("wrong-passphrase");
        VaultCredentialBackend::new_with_path(path.clone(), "correct horse")
            .set(APP_CREDENTIAL_SERVICE, "sync:webdav-password", "secret")
            .expect("initial set should succeed");

        let error = VaultCredentialBackend::new_with_path(path.clone(), "wrong horse")
            .get(APP_CREDENTIAL_SERVICE, "sync:webdav-password")
            .expect_err("wrong passphrase should fail");

        assert!(error.to_string().contains("vault decryption failed"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn vault_backend_rotates_passphrase() {
        let path = test_vault_path("rotate-passphrase");
        VaultCredentialBackend::new_with_path(path.clone(), "correct horse")
            .set(APP_CREDENTIAL_SERVICE, "sync:github-token", "secret-token")
            .expect("initial set should succeed");

        VaultCredentialBackend::rotate_store_passphrase(path.clone(), "correct horse", "new horse")
            .expect("rotation should succeed");

        let value = VaultCredentialBackend::new_with_path(path.clone(), "new horse")
            .get(APP_CREDENTIAL_SERVICE, "sync:github-token")
            .expect("new passphrase should decrypt vault");
        assert_eq!(value.as_deref(), Some("secret-token"));

        let error = VaultCredentialBackend::new_with_path(path.clone(), "correct horse")
            .get(APP_CREDENTIAL_SERVICE, "sync:github-token")
            .expect_err("old passphrase should no longer decrypt vault");
        assert!(error.to_string().contains("vault decryption failed"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn vault_backend_initialize_does_not_rewrite_existing_store() {
        let path = test_vault_path("initialize-no-rewrite");
        let backend = VaultCredentialBackend::new_with_path(path.clone(), "correct horse");

        backend
            .set(APP_CREDENTIAL_SERVICE, "sync:github-token", "secret-token")
            .expect("initial set should succeed");

        let before = fs::read_to_string(&path).expect("vault file should exist");

        backend
            .initialize(APP_CREDENTIAL_SERVICE)
            .expect("initialize should validate vault");

        let after = fs::read_to_string(&path).expect("vault file should still exist");
        assert_eq!(before, after);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn vault_backend_initialize_rejects_wrong_passphrase() {
        let path = test_vault_path("initialize-wrong-passphrase");
        VaultCredentialBackend::new_with_path(path.clone(), "correct horse")
            .set(APP_CREDENTIAL_SERVICE, "sync:webdav-password", "secret")
            .expect("initial set should succeed");

        let error = VaultCredentialBackend::new_with_path(path.clone(), "wrong horse")
            .initialize(APP_CREDENTIAL_SERVICE)
            .expect_err("wrong passphrase should fail validation");

        assert!(error.to_string().contains("vault decryption failed"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn vault_backend_get_many_returns_requested_accounts_in_order() {
        let path = test_vault_path("get-many");
        let backend = VaultCredentialBackend::new_with_path(path.clone(), "correct horse");

        backend
            .set(APP_CREDENTIAL_SERVICE, "session-1:password", "hunter2")
            .expect("password should save");
        backend
            .set(APP_CREDENTIAL_SERVICE, "sync:github-token", "secret-token")
            .expect("token should save");

        let values = backend
            .get_many(
                APP_CREDENTIAL_SERVICE,
                &[
                    "session-1:password",
                    "session-1:passphrase",
                    "sync:github-token",
                ],
            )
            .expect("grouped get should succeed");

        assert_eq!(
            values,
            vec![
                Some("hunter2".to_string()),
                None,
                Some("secret-token".to_string()),
            ]
        );

        let _ = fs::remove_file(path);
    }

    fn test_vault_path(suffix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "miaominal-secret-vault-{suffix}-{}.json",
            uuid::Uuid::new_v4()
        ))
    }
}
