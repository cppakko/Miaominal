use aes_gcm::Aes256Gcm;
use aes_gcm::aead::{Aead, KeyInit, Nonce, array::Array};
use anyhow::{Context, Result, anyhow};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine as _;
use keyring::Entry;
use miaominal_paths::{self as paths, atomic_write};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

pub const APP_CREDENTIAL_SERVICE: &str = "dev.akko.miaominal";

const VAULT_FILE_NAME: &str = "secret_vault.json";
const VAULT_VERSION: u32 = 1;
const VAULT_OUTPUT_LEN: usize = 32;

static VAULT_MEMORY_COST: AtomicU32 = AtomicU32::new(65536);
static VAULT_TIME_COST: AtomicU32 = AtomicU32::new(3);
static VAULT_PARALLELISM: AtomicU32 = AtomicU32::new(4);
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
        #[cfg(target_os = "macos")]
        let backend: Arc<dyn CredentialBackend> = Arc::new(BlobKeyringCredentialBackend);
        #[cfg(not(target_os = "macos"))]
        let backend: Arc<dyn CredentialBackend> = Arc::new(KeyringCredentialBackend);

        Self {
            service: service.into(),
            backend,
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
pub struct LockedCredentialBackend;

#[derive(Debug, Default)]
pub struct KeyringCredentialBackend;

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

// Process-wide lock to serialise read-modify-write operations on any blob entry.
// All CredentialStore instances share this lock regardless of which service they use.
fn blob_io_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

const BLOB_ACCOUNT: &str = "__blob__";

#[derive(Debug, Default)]
pub struct BlobKeyringCredentialBackend;

impl BlobKeyringCredentialBackend {
    fn load_blob(service: &str) -> Result<BTreeMap<String, String>> {
        let entry = Entry::new(service, BLOB_ACCOUNT)
            .with_context(|| format!("failed to access keyring blob for {service}"))?;
        match entry.get_password() {
            Ok(json) => serde_json::from_str(&json).context("failed to parse keyring blob"),
            Err(keyring::Error::NoEntry) => Ok(BTreeMap::new()),
            Err(error) => {
                Err(error).with_context(|| format!("failed to read keyring blob for {service}"))
            }
        }
    }

    fn save_blob(service: &str, blob: &BTreeMap<String, String>) -> Result<()> {
        let entry = Entry::new(service, BLOB_ACCOUNT)
            .with_context(|| format!("failed to access keyring blob for {service}"))?;
        let json = serde_json::to_string(blob).context("failed to serialize keyring blob")?;
        entry
            .set_password(&json)
            .with_context(|| format!("failed to write keyring blob for {service}"))
    }

    fn load_legacy(service: &str, account: &str) -> Result<Option<String>> {
        match Entry::new(service, account).and_then(|e| e.get_password()) {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error).with_context(|| {
                format!("failed to read legacy keyring entry {service}/{account}")
            }),
        }
    }

    fn delete_legacy(service: &str, account: &str) -> Result<()> {
        let entry = Entry::new(service, account).with_context(|| {
            format!("failed to access legacy keyring entry {service}/{account}")
        })?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error).with_context(|| {
                format!("failed to delete legacy keyring entry {service}/{account}")
            }),
        }
    }

    fn cleanup_legacy(service: &str, account: &str) {
        if let Err(error) = Self::delete_legacy(service, account) {
            log::warn!("{error:?}");
        }
    }
}

impl CredentialBackend for BlobKeyringCredentialBackend {
    fn name(&self) -> &'static str {
        "keyring-blob"
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<String>> {
        let _lock = blob_io_lock()
            .lock()
            .map_err(|_| anyhow!("keyring blob lock poisoned"))?;
        let mut blob = Self::load_blob(service)?;
        if let Some(value) = blob.get(account) {
            return Ok(Some(value.clone()));
        }
        // Lazy migration from legacy individual keychain entry.
        if let Some(secret) = Self::load_legacy(service, account)? {
            blob.insert(account.to_string(), secret.clone());
            Self::save_blob(service, &blob)?;
            Self::cleanup_legacy(service, account);
            return Ok(Some(secret));
        }
        Ok(None)
    }

    fn get_many(&self, service: &str, accounts: &[&str]) -> Result<Vec<Option<String>>> {
        let _lock = blob_io_lock()
            .lock()
            .map_err(|_| anyhow!("keyring blob lock poisoned"))?;
        let mut blob = Self::load_blob(service)?;
        let mut dirty = false;
        let mut results = Vec::with_capacity(accounts.len());
        for account in accounts {
            if let Some(value) = blob.get(*account) {
                results.push(Some(value.clone()));
                continue;
            }
            if let Some(secret) = Self::load_legacy(service, account)? {
                dirty = true;
                blob.insert((*account).to_string(), secret.clone());
                results.push(Some(secret));
            } else {
                results.push(None);
            }
        }
        if dirty {
            Self::save_blob(service, &blob)?;
            for account in accounts {
                if blob.contains_key(*account) {
                    Self::cleanup_legacy(service, account);
                }
            }
        }
        Ok(results)
    }

    fn set(&self, service: &str, account: &str, value: &str) -> Result<()> {
        let _lock = blob_io_lock()
            .lock()
            .map_err(|_| anyhow!("keyring blob lock poisoned"))?;
        let mut blob = Self::load_blob(service)?;
        blob.insert(account.to_string(), value.to_string());
        Self::save_blob(service, &blob)
    }

    fn delete(&self, service: &str, account: &str) -> Result<()> {
        let _lock = blob_io_lock()
            .lock()
            .map_err(|_| anyhow!("keyring blob lock poisoned"))?;
        let mut blob = Self::load_blob(service)?;
        if blob.remove(account).is_some() {
            Self::save_blob(service, &blob)?;
        }
        // Also clean up any legacy individual entry.
        Self::delete_legacy(service, account)
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
    state: Mutex<Option<CachedVaultDocument>>,
}

#[derive(Debug)]
struct CachedVaultDocument {
    serialized: Option<Vec<u8>>,
    document: VaultDocument,
}

// Serializing within the process avoids platform-specific behavior when the
// same process tries to lock the sidecar through multiple file handles. The
// sidecar lock then extends the same critical section across processes.
fn vault_io_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn vault_lock_file_path(file_path: &Path) -> PathBuf {
    let mut lock_path = file_path.as_os_str().to_os_string();
    lock_path.push(".lock");
    PathBuf::from(lock_path)
}

fn open_vault_lock_file(file_path: &Path) -> Result<File> {
    let lock_path = vault_lock_file_path(file_path);
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("failed to open vault lock file {}", lock_path.display()))
}

fn with_vault_file_lock<T>(file_path: &Path, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let _process_lock = vault_io_lock()
        .lock()
        .map_err(|_| anyhow!("vault I/O lock poisoned"))?;

    let lock_path = vault_lock_file_path(file_path);
    let lock_file = open_vault_lock_file(file_path)?;
    lock_file
        .lock()
        .with_context(|| format!("failed to lock vault file {}", lock_path.display()))?;

    operation()
}

impl VaultCredentialBackend {
    pub fn new(passphrase: impl Into<String>) -> Result<Self> {
        Ok(Self::new_with_path(Self::default_file_path()?, passphrase))
    }

    pub fn new_with_path(file_path: PathBuf, passphrase: impl Into<String>) -> Self {
        Self {
            file_path,
            passphrase: passphrase.into(),
            state: Mutex::new(None),
        }
    }

    pub fn default_store_exists() -> Result<bool> {
        let path = Self::default_file_path()?;
        Ok(path.exists())
    }

    pub fn erase_default_store_file() -> Result<()> {
        Self::erase_store_file(Self::default_file_path()?)
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
        with_vault_file_lock(&file_path, || {
            let current = Self::new_with_path(file_path.clone(), old_passphrase.to_string());
            let serialized = current.read_serialized_document()?;
            let document = current.decrypt_serialized_document(serialized.as_deref())?;
            let next = Self::new_with_path(file_path.clone(), new_passphrase.to_string());
            next.save_document_locked(&document)?;
            Ok(())
        })
    }

    fn erase_store_file(file_path: PathBuf) -> Result<()> {
        with_vault_file_lock(&file_path, || match fs::remove_file(&file_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("failed to remove {}", file_path.display()))
            }
        })
    }

    fn with_store_lock<T>(&self, operation: impl FnOnce() -> Result<T>) -> Result<T> {
        with_vault_file_lock(&self.file_path, operation)
    }

    fn read_serialized_document(&self) -> Result<Option<Vec<u8>>> {
        match fs::read(&self.file_path) {
            Ok(serialized) => Ok(Some(serialized)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => {
                Err(error).with_context(|| format!("failed to read {}", self.file_path.display()))
            }
        }
    }

    fn decrypt_serialized_document(&self, serialized: Option<&[u8]>) -> Result<VaultDocument> {
        let Some(serialized) = serialized else {
            return Ok(VaultDocument::default());
        };
        if serialized.iter().all(|byte| byte.is_ascii_whitespace()) {
            return Ok(VaultDocument::default());
        }

        let stored: StoredVaultDocument = serde_json::from_slice(serialized)
            .with_context(|| format!("failed to parse {}", self.file_path.display()))?;
        self.decrypt_document(stored)
    }

    fn initialize_store(&self) -> Result<()> {
        self.with_store_lock(|| {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow!("vault lock poisoned"))?;
            let serialized = self.read_serialized_document()?;

            if serialized
                .as_deref()
                .is_none_or(|contents| contents.iter().all(|byte| byte.is_ascii_whitespace()))
            {
                *state = Some(self.save_document_locked(&VaultDocument::default())?);
                return Ok(());
            }

            if state
                .as_ref()
                .is_some_and(|cached| cached.serialized == serialized)
            {
                return Ok(());
            }

            let document = self.decrypt_serialized_document(serialized.as_deref())?;
            *state = Some(CachedVaultDocument {
                serialized,
                document,
            });
            Ok(())
        })
    }

    fn save_document_locked(&self, document: &VaultDocument) -> Result<CachedVaultDocument> {
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
            serde_json::to_vec_pretty(&stored).context("failed to serialize vault document")?;
        atomic_write(&self.file_path, &serialized)?;
        Ok(CachedVaultDocument {
            serialized: Some(serialized),
            document,
        })
    }

    fn encrypt_document(&self, document: &VaultDocument) -> Result<StoredVaultDocument> {
        let salt_bytes: [u8; 32] = rand::random();
        let key = derive_key(&self.passphrase, &salt_bytes)?;
        let plaintext = serde_json::to_vec(document).context("failed to serialize vault")?;
        let cipher = Aes256Gcm::new(&Array(key));
        let nonce = Nonce::<Aes256Gcm>::from(rand::random::<[u8; 12]>());
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

        let cipher = Aes256Gcm::new(&Array(key));
        let plaintext = cipher
            .decrypt(
                &Nonce::<Aes256Gcm>::try_from(nonce_bytes)
                    .map_err(|_| anyhow!("vault nonce must be 12 bytes"))?,
                aes_gcm::aead::Payload {
                    msg: ciphertext,
                    aad: VAULT_AAD,
                },
            )
            .map_err(|error| anyhow!("vault decryption failed: {error}"))?;

        serde_json::from_slice(&plaintext).context("failed to parse decrypted vault")
    }

    fn refresh_document_locked<'a>(
        &self,
        state: &'a mut Option<CachedVaultDocument>,
    ) -> Result<&'a VaultDocument> {
        let serialized = self.read_serialized_document()?;
        if state
            .as_ref()
            .is_none_or(|cached| cached.serialized != serialized)
        {
            let document = self.decrypt_serialized_document(serialized.as_deref())?;
            *state = Some(CachedVaultDocument {
                serialized,
                document,
            });
        }

        Ok(&state
            .as_ref()
            .expect("vault state is populated after refresh")
            .document)
    }

    fn with_document<T>(&self, f: impl FnOnce(&mut VaultDocument) -> Result<T>) -> Result<T> {
        self.with_store_lock(|| {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow!("vault lock poisoned"))?;
            let mut document = self.refresh_document_locked(&mut state)?.clone();
            let output = f(&mut document)?;
            let cached = self.save_document_locked(&document)?;
            *state = Some(cached);
            Ok(output)
        })
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
        self.with_store_lock(|| {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow!("vault lock poisoned"))?;
            let document = self.refresh_document_locked(&mut state)?;
            Ok(document
                .services
                .get(service)
                .and_then(|accounts| accounts.get(account).cloned()))
        })
    }

    fn get_many(&self, service: &str, accounts: &[&str]) -> Result<Vec<Option<String>>> {
        self.with_store_lock(|| {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow!("vault lock poisoned"))?;
            let document = self.refresh_document_locked(&mut state)?;
            let stored_accounts = document.services.get(service);

            Ok(accounts
                .iter()
                .map(|account| stored_accounts.and_then(|values| values.get(*account).cloned()))
                .collect())
        })
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

pub fn encrypt_with_aad(key: &[u8; 32], plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(&Array(*key));
    let nonce = Nonce::<Aes256Gcm>::from(rand::random::<[u8; 12]>());
    let ciphertext = cipher
        .encrypt(
            &nonce,
            aes_gcm::aead::Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|error| anyhow!("AES-GCM encryption failed: {error}"))?;

    let mut combined = nonce.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(combined)
}

pub fn decrypt_with_aad(key: &[u8; 32], ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    if ciphertext.len() < 12 {
        anyhow::bail!("ciphertext too short to contain nonce");
    }
    let (nonce_bytes, payload) = ciphertext.split_at(12);
    let cipher = Aes256Gcm::new(&Array(*key));

    cipher
        .decrypt(
            &Nonce::<Aes256Gcm>::try_from(nonce_bytes)
                .map_err(|_| anyhow!("ciphertext nonce must be 12 bytes"))?,
            aes_gcm::aead::Payload { msg: payload, aad },
        )
        .map_err(|error| anyhow!("AES-GCM decryption failed: {error}"))
}

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; 32]> {
    let params = Params::new(
        VAULT_MEMORY_COST.load(Ordering::Relaxed),
        VAULT_TIME_COST.load(Ordering::Relaxed),
        VAULT_PARALLELISM.load(Ordering::Relaxed),
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

/// Override Argon2id parameters for tests. Call in test setup to avoid
/// the expensive default parameters (64 MB / 3 iterations).
#[doc(hidden)]
pub fn set_vault_test_parameters() {
    VAULT_MEMORY_COST.store(2048, Ordering::Relaxed);
    VAULT_TIME_COST.store(1, Ordering::Relaxed);
    VAULT_PARALLELISM.store(1, Ordering::Relaxed);
}

fn locked_vault_error(service: &str, account: &str) -> anyhow::Error {
    anyhow!("local vault is locked; unlock it before accessing {service}/{account}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::TryLockError;
    use std::process::{Child, Command, Output, Stdio};
    use std::sync::Barrier;
    use std::thread;
    use std::time::{Duration, Instant};

    const VAULT_LOCK_PROBE_TEST: &str = "credential_backend::tests::vault_sidecar_lock_child_probe";
    const VAULT_LOCK_PROBE_PATH_ENV: &str = "MIAOMINAL_TEST_VAULT_LOCK_PROBE_PATH";
    const VAULT_LOCK_PROBE_EXPECT_ENV: &str = "MIAOMINAL_TEST_VAULT_LOCK_PROBE_EXPECT";
    const VAULT_LOCK_PROBE_MARKER_ENV: &str = "MIAOMINAL_TEST_VAULT_LOCK_PROBE_MARKER";
    const VAULT_LOCK_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

    fn test_backend(path: PathBuf) -> VaultCredentialBackend {
        set_vault_test_parameters();
        VaultCredentialBackend::new_with_path(path, "correct horse")
    }

    #[test]
    fn vault_backend_round_trips_credentials() {
        let path = test_vault_path("round-trip");
        let backend = test_backend(path.clone());

        backend
            .set(APP_CREDENTIAL_SERVICE, "sync:github-token", "secret-token")
            .expect("set should succeed");

        let value = backend
            .get(APP_CREDENTIAL_SERVICE, "sync:github-token")
            .expect("get should succeed");

        assert_eq!(value.as_deref(), Some("secret-token"));

        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_backend_rejects_wrong_passphrase() {
        set_vault_test_parameters();
        let path = test_vault_path("wrong-passphrase");
        VaultCredentialBackend::new_with_path(path.clone(), "correct horse")
            .set(APP_CREDENTIAL_SERVICE, "sync:webdav-password", "secret")
            .expect("initial set should succeed");

        let error = VaultCredentialBackend::new_with_path(path.clone(), "wrong horse")
            .get(APP_CREDENTIAL_SERVICE, "sync:webdav-password")
            .expect_err("wrong passphrase should fail");

        assert!(error.to_string().contains("vault decryption failed"));

        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_backend_rotates_passphrase() {
        set_vault_test_parameters();
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

        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_backend_initialize_does_not_rewrite_existing_store() {
        set_vault_test_parameters();
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

        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_backend_initialize_rejects_wrong_passphrase() {
        set_vault_test_parameters();
        let path = test_vault_path("initialize-wrong-passphrase");
        VaultCredentialBackend::new_with_path(path.clone(), "correct horse")
            .set(APP_CREDENTIAL_SERVICE, "sync:webdav-password", "secret")
            .expect("initial set should succeed");

        let error = VaultCredentialBackend::new_with_path(path.clone(), "wrong horse")
            .initialize(APP_CREDENTIAL_SERVICE)
            .expect_err("wrong passphrase should fail validation");

        assert!(error.to_string().contains("vault decryption failed"));

        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_backend_get_many_returns_requested_accounts_in_order() {
        set_vault_test_parameters();
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

        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_backend_preserves_interleaved_writes_from_independent_caches() {
        set_vault_test_parameters();
        let path = test_vault_path("interleaved-independent-caches");
        let password_backend = test_backend(path.clone());
        let token_backend = test_backend(path.clone());

        password_backend
            .initialize(APP_CREDENTIAL_SERVICE)
            .expect("password backend should initialize");
        token_backend
            .initialize(APP_CREDENTIAL_SERVICE)
            .expect("token backend should initialize from the same snapshot");

        password_backend
            .set(APP_CREDENTIAL_SERVICE, "session-1:password", "hunter2")
            .expect("password should save");
        token_backend
            .set(APP_CREDENTIAL_SERVICE, "sync:github-token", "secret-token")
            .expect("token should merge with the latest vault");

        let reader = test_backend(path.clone());
        assert_eq!(
            reader
                .get_many(
                    APP_CREDENTIAL_SERVICE,
                    &["session-1:password", "sync:github-token"],
                )
                .expect("reader should decrypt the merged vault"),
            vec![
                Some("hunter2".to_string()),
                Some("secret-token".to_string())
            ]
        );

        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_backend_refreshes_reads_and_does_not_resurrect_deleted_secrets() {
        set_vault_test_parameters();
        let path = test_vault_path("refresh-and-delete");
        let first = test_backend(path.clone());
        let second = test_backend(path.clone());

        first
            .initialize(APP_CREDENTIAL_SERVICE)
            .expect("first backend should initialize");
        second
            .initialize(APP_CREDENTIAL_SERVICE)
            .expect("second backend should initialize");
        first
            .set(APP_CREDENTIAL_SERVICE, "session-1:password", "hunter2")
            .expect("password should save");
        assert_eq!(
            second
                .get(APP_CREDENTIAL_SERVICE, "session-1:password")
                .expect("second backend should refresh its cached document")
                .as_deref(),
            Some("hunter2")
        );

        second
            .set(APP_CREDENTIAL_SERVICE, "sync:github-token", "secret-token")
            .expect("token should save");
        first
            .delete(APP_CREDENTIAL_SERVICE, "session-1:password")
            .expect("delete should preserve the token from the latest document");
        second
            .set(APP_CREDENTIAL_SERVICE, "session-1:passphrase", "key-secret")
            .expect("later write should not resurrect the deleted password");

        let reader = test_backend(path.clone());
        assert_eq!(
            reader
                .get_many(
                    APP_CREDENTIAL_SERVICE,
                    &[
                        "session-1:password",
                        "sync:github-token",
                        "session-1:passphrase",
                    ],
                )
                .expect("reader should see the final document"),
            vec![
                None,
                Some("secret-token".to_string()),
                Some("key-secret".to_string()),
            ]
        );

        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_backend_serializes_concurrent_writes_from_independent_instances() {
        set_vault_test_parameters();
        let path = test_vault_path("concurrent-independent-instances");
        const WORKER_COUNT: usize = 6;
        let barrier = Arc::new(Barrier::new(WORKER_COUNT));
        let mut workers = Vec::with_capacity(WORKER_COUNT);

        for index in 0..WORKER_COUNT {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                let backend = test_backend(path);
                backend
                    .initialize(APP_CREDENTIAL_SERVICE)
                    .expect("worker backend should initialize");
                barrier.wait();
                backend
                    .set(
                        APP_CREDENTIAL_SERVICE,
                        &format!("concurrent:{index}"),
                        &format!("secret-{index}"),
                    )
                    .expect("concurrent write should succeed");
            }));
        }

        for worker in workers {
            worker.join().expect("vault worker should not panic");
        }

        let reader = test_backend(path.clone());
        for index in 0..WORKER_COUNT {
            assert_eq!(
                reader
                    .get(APP_CREDENTIAL_SERVICE, &format!("concurrent:{index}"))
                    .expect("reader should decrypt concurrent result"),
                Some(format!("secret-{index}"))
            );
        }

        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_backend_with_old_cached_passphrase_cannot_overwrite_rotated_store() {
        set_vault_test_parameters();
        let path = test_vault_path("cached-old-passphrase-after-rotation");
        let old_backend = test_backend(path.clone());
        old_backend
            .set(APP_CREDENTIAL_SERVICE, "sync:github-token", "secret-token")
            .expect("initial secret should save");
        old_backend
            .get(APP_CREDENTIAL_SERVICE, "sync:github-token")
            .expect("old backend should cache the pre-rotation snapshot");

        VaultCredentialBackend::rotate_store_passphrase(path.clone(), "correct horse", "new horse")
            .expect("rotation should succeed");

        let error = old_backend
            .set(
                APP_CREDENTIAL_SERVICE,
                "sync:webdav-password",
                "must-not-save",
            )
            .expect_err("old cached backend must refresh and reject the new ciphertext");
        assert!(error.to_string().contains("vault decryption failed"));

        let new_backend = VaultCredentialBackend::new_with_path(path.clone(), "new horse");
        assert_eq!(
            new_backend
                .get_many(
                    APP_CREDENTIAL_SERVICE,
                    &["sync:github-token", "sync:webdav-password"],
                )
                .expect("new passphrase should read the intact vault"),
            vec![Some("secret-token".to_string()), None]
        );

        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_backend_erase_keeps_stable_sidecar_and_invalidates_cached_document() {
        set_vault_test_parameters();
        let path = test_vault_path("erase-sidecar");
        let backend = test_backend(path.clone());
        backend
            .set(APP_CREDENTIAL_SERVICE, "sync:github-token", "secret-token")
            .expect("secret should save");

        VaultCredentialBackend::erase_store_file(path.clone()).expect("vault erase should succeed");

        assert!(!path.exists());
        assert!(vault_lock_file_path(&path).exists());
        assert_eq!(
            backend
                .get(APP_CREDENTIAL_SERVICE, "sync:github-token")
                .expect("cached backend should refresh after erasure"),
            None
        );

        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_sidecar_lock_is_visible_across_processes() {
        let path = test_vault_path("cross-process-sidecar-lock");
        let lock_file = open_vault_lock_file(&path).expect("parent should open vault sidecar");
        lock_file
            .lock()
            .expect("parent should acquire vault sidecar lock");

        run_vault_lock_probe(&path, "blocked");

        lock_file
            .unlock()
            .expect("parent should release vault sidecar lock");
        run_vault_lock_probe(&path, "available");

        drop(lock_file);
        cleanup_test_vault(&path);
    }

    #[test]
    #[ignore = "subprocess helper for the cross-process vault lock test"]
    fn vault_sidecar_lock_child_probe() {
        let Some(path) = std::env::var_os(VAULT_LOCK_PROBE_PATH_ENV) else {
            return;
        };
        let expected = std::env::var(VAULT_LOCK_PROBE_EXPECT_ENV)
            .expect("parent should provide the expected lock state");
        let marker = PathBuf::from(
            std::env::var_os(VAULT_LOCK_PROBE_MARKER_ENV)
                .expect("parent should provide the probe marker path"),
        );
        let lock_file = open_vault_lock_file(Path::new(&path))
            .expect("child should open the same vault sidecar");

        match expected.as_str() {
            "blocked" => match lock_file.try_lock() {
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(error)) => {
                    panic!("child failed to probe the held sidecar lock: {error}")
                }
                Ok(()) => {
                    let _ = lock_file.unlock();
                    panic!("child unexpectedly acquired the held sidecar lock");
                }
            },
            "available" => match lock_file.try_lock() {
                Ok(()) => lock_file
                    .unlock()
                    .expect("child should release the available sidecar lock"),
                Err(TryLockError::WouldBlock) => {
                    panic!("child still observed the released sidecar lock as held")
                }
                Err(TryLockError::Error(error)) => {
                    panic!("child failed to probe the released sidecar lock: {error}")
                }
            },
            other => panic!("unsupported vault lock probe expectation: {other}"),
        }

        fs::write(&marker, &expected).expect("child should acknowledge that the probe ran");
    }

    fn run_vault_lock_probe(path: &Path, expected: &str) {
        let marker = test_vault_path(&format!("cross-process-lock-marker-{expected}"));
        let child = Command::new(std::env::current_exe().expect("test executable should exist"))
            .arg(VAULT_LOCK_PROBE_TEST)
            .arg("--exact")
            .arg("--ignored")
            .arg("--test-threads")
            .arg("1")
            .env(VAULT_LOCK_PROBE_PATH_ENV, path)
            .env(VAULT_LOCK_PROBE_EXPECT_ENV, expected)
            .env(VAULT_LOCK_PROBE_MARKER_ENV, &marker)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("vault lock probe process should start");
        let output = wait_for_vault_lock_probe(child);
        let marker_contents = fs::read_to_string(&marker);
        let _ = fs::remove_file(&marker);

        assert!(
            output.status.success(),
            "vault lock probe for {expected} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            marker_contents.expect("probe marker proves that the exact child test ran"),
            expected
        );
    }

    fn wait_for_vault_lock_probe(mut child: Child) -> Output {
        let deadline = Instant::now() + VAULT_LOCK_PROBE_TIMEOUT;
        loop {
            match child
                .try_wait()
                .expect("vault lock probe status should be readable")
            {
                Some(_) => {
                    return child
                        .wait_with_output()
                        .expect("vault lock probe output should be readable");
                }
                None if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                None => {
                    let _ = child.kill();
                    let output = child
                        .wait_with_output()
                        .expect("timed-out vault lock probe should terminate");
                    panic!(
                        "vault lock probe exceeded {:?}\nstdout:\n{}\nstderr:\n{}",
                        VAULT_LOCK_PROBE_TIMEOUT,
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }
        }
    }

    fn test_vault_path(suffix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "miaominal-secret-vault-{suffix}-{}.json",
            uuid::Uuid::new_v4()
        ))
    }

    fn cleanup_test_vault(path: &Path) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(vault_lock_file_path(path));
    }
}
