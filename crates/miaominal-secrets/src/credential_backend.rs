use crate::protected_memory::{ProtectedDerivedKey, ProtectedPassphrase};
use aes_gcm::Aes256Gcm;
use aes_gcm::aead::{Aead, AeadInOut, KeyInit, Nonce, array::Array};
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
use zeroize::{Zeroize, Zeroizing};

pub const APP_CREDENTIAL_SERVICE: &str = "dev.akko.miaominal";

const VAULT_FILE_NAME: &str = "secret_vault.json";
const VAULT_VERSION: u32 = 1;
const VAULT_OUTPUT_LEN: usize = 32;

static VAULT_MEMORY_COST: AtomicU32 = AtomicU32::new(65536);
static VAULT_TIME_COST: AtomicU32 = AtomicU32::new(3);
static VAULT_PARALLELISM: AtomicU32 = AtomicU32::new(4);
const VAULT_AAD: &[u8] = b"miaominal.secret-vault.v1";
const VAULT_MEMORY_CACHE_AAD: &[u8] = b"miaominal.secret-vault.memory-cache.v1";
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

pub struct VaultCredentialBackend {
    file_path: PathBuf,
    passphrase: ProtectedPassphrase,
    state: Mutex<Option<CachedVaultDocument>>,
}

struct CachedVaultDocument {
    serialized: Option<Vec<u8>>,
    encrypted_document: EncryptedVaultCache,
}

struct EncryptedVaultCache {
    nonce: [u8; 12],
    ciphertext: Vec<u8>,
}

// Serializing within the process avoids platform-specific behavior when the
// same process tries to lock the sidecar through multiple file handles. The
// sidecar lock then extends the same critical section across processes.
fn vault_io_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn vault_rotation_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
thread_local! {
    static VAULT_LOCK_TRACE: std::cell::RefCell<Vec<&'static str>> = const {
        std::cell::RefCell::new(Vec::new())
    };
    static VAULT_DERIVE_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
fn record_vault_lock_trace(lock: &'static str) {
    VAULT_LOCK_TRACE.with(|trace| trace.borrow_mut().push(lock));
}

#[cfg(not(test))]
fn record_vault_lock_trace(_lock: &'static str) {}

#[cfg(test)]
fn record_vault_derivation() {
    VAULT_DERIVE_COUNT.with(|count| count.set(count.get() + 1));
}

#[cfg(not(test))]
fn record_vault_derivation() {}

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
    record_vault_lock_trace("file");
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
    pub fn new(passphrase: ProtectedPassphrase) -> Result<Self> {
        Ok(Self::new_with_path(Self::default_file_path()?, passphrase))
    }

    pub fn new_with_path(file_path: PathBuf, passphrase: ProtectedPassphrase) -> Self {
        Self {
            file_path,
            passphrase,
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
        old_passphrase: &ProtectedPassphrase,
        new_passphrase: &ProtectedPassphrase,
    ) -> Result<()> {
        Self::rotate_store_passphrase(
            Self::default_file_path()?,
            old_passphrase.clone(),
            new_passphrase.clone(),
        )
    }

    fn default_file_path() -> Result<PathBuf> {
        paths::config_file(VAULT_FILE_NAME)
    }

    fn rotate_store_passphrase(
        file_path: PathBuf,
        old_passphrase: ProtectedPassphrase,
        new_passphrase: ProtectedPassphrase,
    ) -> Result<()> {
        let _rotation_lock = vault_rotation_lock()
            .lock()
            .map_err(|_| anyhow!("vault rotation lock poisoned"))?;
        record_vault_lock_trace("rotation");
        let current = Self::new_with_path(file_path.clone(), old_passphrase.clone());
        let next = Self::new_with_path(file_path.clone(), new_passphrase.clone());

        old_passphrase.with_bytes(|old_bytes| {
            record_vault_lock_trace("session");
            if old_passphrase.shares_allocation_with(&new_passphrase) {
                return with_vault_file_lock(&file_path, || {
                    let serialized = current.read_serialized_document()?;
                    let mut document =
                        current.decrypt_serialized_document(serialized.as_deref(), old_bytes)?;
                    next.write_document_locked(&mut document, old_bytes)
                });
            }

            new_passphrase.with_bytes(|new_bytes| {
                record_vault_lock_trace("session");
                with_vault_file_lock(&file_path, || {
                    let serialized = current.read_serialized_document()?;
                    let mut document =
                        current.decrypt_serialized_document(serialized.as_deref(), old_bytes)?;
                    next.write_document_locked(&mut document, new_bytes)
                })
            })
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

    fn decrypt_serialized_document(
        &self,
        serialized: Option<&[u8]>,
        passphrase: &[u8],
    ) -> Result<VaultDocument> {
        let Some(serialized) = serialized else {
            return Ok(VaultDocument::default());
        };
        if serialized.iter().all(|byte| byte.is_ascii_whitespace()) {
            return Ok(VaultDocument::default());
        }

        let stored: StoredVaultDocument = serde_json::from_slice(serialized)
            .with_context(|| format!("failed to parse {}", self.file_path.display()))?;
        self.decrypt_document(stored, passphrase)
    }

    fn initialize_store(&self) -> Result<()> {
        self.passphrase
            .with_session_material(|passphrase, cache_key| {
                self.with_store_lock(|| {
                    let mut state = self
                        .state
                        .lock()
                        .map_err(|_| anyhow!("vault lock poisoned"))?;
                    let serialized = self.read_serialized_document()?;

                    if serialized.as_deref().is_none_or(|contents| {
                        contents.iter().all(|byte| byte.is_ascii_whitespace())
                    }) {
                        let mut document = VaultDocument::default();
                        *state = Some(self.save_document_locked(
                            &mut document,
                            passphrase,
                            cache_key,
                        )?);
                        return Ok(());
                    }

                    if state
                        .as_ref()
                        .is_some_and(|cached| cached.serialized == serialized)
                    {
                        return Ok(());
                    }

                    let document =
                        self.decrypt_serialized_document(serialized.as_deref(), passphrase)?;
                    let encrypted_document =
                        self.encrypt_document_for_cache(&document, cache_key)?;
                    *state = Some(CachedVaultDocument {
                        serialized,
                        encrypted_document,
                    });
                    Ok(())
                })
            })
    }

    fn save_document_locked(
        &self,
        document: &mut VaultDocument,
        passphrase: &[u8],
        cache_key: &[u8; 32],
    ) -> Result<CachedVaultDocument> {
        Self::account_map_mut(document, VAULT_METADATA_SERVICE)
            .entry(VAULT_METADATA_ACCOUNT.to_string())
            .or_insert_with(|| VAULT_METADATA_VALUE.to_string());

        let plaintext = Zeroizing::new(
            serde_json::to_vec(&document).context("failed to serialize vault document")?,
        );
        let stored = self.encrypt_plaintext(&plaintext, passphrase)?;
        let serialized =
            serde_json::to_vec_pretty(&stored).context("failed to serialize vault document")?;
        let encrypted_document = self.encrypt_memory_cache(&plaintext, cache_key)?;

        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        atomic_write(&self.file_path, &serialized)?;

        Ok(CachedVaultDocument {
            serialized: Some(serialized),
            encrypted_document,
        })
    }

    fn write_document_locked(&self, document: &mut VaultDocument, passphrase: &[u8]) -> Result<()> {
        Self::account_map_mut(document, VAULT_METADATA_SERVICE)
            .entry(VAULT_METADATA_ACCOUNT.to_string())
            .or_insert_with(|| VAULT_METADATA_VALUE.to_string());
        let plaintext = Zeroizing::new(
            serde_json::to_vec(&document).context("failed to serialize vault document")?,
        );
        let stored = self.encrypt_plaintext(&plaintext, passphrase)?;
        let serialized =
            serde_json::to_vec_pretty(&stored).context("failed to serialize vault document")?;

        if let Some(parent) = self.file_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        atomic_write(&self.file_path, &serialized)
    }

    fn encrypt_plaintext(
        &self,
        plaintext: &[u8],
        passphrase: &[u8],
    ) -> Result<StoredVaultDocument> {
        let salt_bytes: [u8; 32] = rand::random();
        let mut key = derive_key(passphrase, &salt_bytes)?;
        let nonce = Nonce::<Aes256Gcm>::from(rand::random::<[u8; 12]>());
        let ciphertext = key.with_bytes(|key| {
            let cipher = Aes256Gcm::new_from_slice(key)
                .map_err(|error| anyhow!("failed to initialize vault cipher: {error}"))?;
            cipher
                .encrypt(
                    &nonce,
                    aes_gcm::aead::Payload {
                        msg: plaintext,
                        aad: VAULT_AAD,
                    },
                )
                .map_err(|error| anyhow!("vault encryption failed: {error}"))
        })?;

        let mut combined = nonce.to_vec();
        combined.extend_from_slice(&ciphertext);

        Ok(StoredVaultDocument {
            version: VAULT_VERSION,
            salt: base64::engine::general_purpose::STANDARD.encode(salt_bytes),
            encrypted_payload: base64::engine::general_purpose::STANDARD.encode(combined),
        })
    }

    fn encrypt_document_for_cache(
        &self,
        document: &VaultDocument,
        cache_key: &[u8; 32],
    ) -> Result<EncryptedVaultCache> {
        let plaintext = Zeroizing::new(
            serde_json::to_vec(document).context("failed to serialize in-memory vault cache")?,
        );
        self.encrypt_memory_cache(&plaintext, cache_key)
    }

    fn encrypt_memory_cache(
        &self,
        plaintext: &[u8],
        cache_key: &[u8; 32],
    ) -> Result<EncryptedVaultCache> {
        let nonce_bytes = rand::random::<[u8; 12]>();
        let nonce = Nonce::<Aes256Gcm>::from(nonce_bytes);
        let cipher = Aes256Gcm::new_from_slice(cache_key).map_err(|error| {
            anyhow!("failed to initialize in-memory vault cache cipher: {error}")
        })?;
        let ciphertext = cipher
            .encrypt(
                &nonce,
                aes_gcm::aead::Payload {
                    msg: plaintext,
                    aad: VAULT_MEMORY_CACHE_AAD,
                },
            )
            .map_err(|error| anyhow!("failed to encrypt in-memory vault cache: {error}"))?;

        Ok(EncryptedVaultCache {
            nonce: nonce_bytes,
            ciphertext,
        })
    }

    fn decrypt_memory_cache(
        &self,
        encrypted: &EncryptedVaultCache,
        cache_key: &[u8; 32],
    ) -> Result<VaultDocument> {
        let cipher = Aes256Gcm::new_from_slice(cache_key).map_err(|error| {
            anyhow!("failed to initialize in-memory vault cache cipher: {error}")
        })?;
        let nonce = Nonce::<Aes256Gcm>::from(encrypted.nonce);
        let mut plaintext = Zeroizing::new(encrypted.ciphertext.clone());
        cipher
            .decrypt_in_place(&nonce, VAULT_MEMORY_CACHE_AAD, &mut *plaintext)
            .map_err(|error| anyhow!("failed to decrypt in-memory vault cache: {error}"))?;

        serde_json::from_slice(&plaintext).context("failed to parse in-memory vault cache")
    }

    fn decrypt_document(
        &self,
        stored: StoredVaultDocument,
        passphrase: &[u8],
    ) -> Result<VaultDocument> {
        if stored.version != VAULT_VERSION {
            anyhow::bail!("unsupported vault version: {}", stored.version);
        }

        let salt = base64::engine::general_purpose::STANDARD
            .decode(stored.salt)
            .context("failed to decode vault salt")?;
        if salt.len() != 32 {
            anyhow::bail!("vault salt must be 32 bytes");
        }

        let mut key = derive_key(passphrase, &salt)?;
        let mut combined = base64::engine::general_purpose::STANDARD
            .decode(stored.encrypted_payload)
            .context("failed to decode vault payload")?;
        if combined.len() < 12 {
            anyhow::bail!("vault payload missing nonce");
        }
        let ciphertext = combined.split_off(12);
        let nonce = Nonce::<Aes256Gcm>::try_from(combined.as_slice())
            .map_err(|_| anyhow!("vault nonce must be 12 bytes"))?;
        let mut plaintext = Zeroizing::new(ciphertext);
        key.with_bytes(|key| {
            let cipher = Aes256Gcm::new_from_slice(key)
                .map_err(|error| anyhow!("failed to initialize vault cipher: {error}"))?;
            cipher
                .decrypt_in_place(&nonce, VAULT_AAD, &mut *plaintext)
                .map_err(|error| anyhow!("vault decryption failed: {error}"))
        })?;

        serde_json::from_slice(&plaintext).context("failed to parse decrypted vault")
    }

    fn load_document_locked(
        &self,
        state: &mut Option<CachedVaultDocument>,
        passphrase: &[u8],
        cache_key: &[u8; 32],
    ) -> Result<VaultDocument> {
        let serialized = self.read_serialized_document()?;
        if let Some(cached) = state.as_ref()
            && cached.serialized == serialized
        {
            match self.decrypt_memory_cache(&cached.encrypted_document, cache_key) {
                Ok(document) => return Ok(document),
                Err(error) => {
                    log::warn!(
                        "in-memory vault cache validation failed; rebuilding from disk: {error:#}"
                    );
                    state.take();
                }
            }
        }

        let document = self.decrypt_serialized_document(serialized.as_deref(), passphrase)?;
        let encrypted_document = self.encrypt_document_for_cache(&document, cache_key)?;
        *state = Some(CachedVaultDocument {
            serialized,
            encrypted_document,
        });
        Ok(document)
    }

    fn with_document<T>(&self, f: impl FnOnce(&mut VaultDocument) -> Result<T>) -> Result<T> {
        self.passphrase
            .with_session_material(|passphrase, cache_key| {
                self.with_store_lock(|| {
                    let mut state = self
                        .state
                        .lock()
                        .map_err(|_| anyhow!("vault lock poisoned"))?;
                    let mut document =
                        self.load_document_locked(&mut state, passphrase, cache_key)?;
                    let output = f(&mut document)?;
                    let cached = self.save_document_locked(&mut document, passphrase, cache_key)?;
                    *state = Some(cached);
                    Ok(output)
                })
            })
    }

    fn account_map_mut<'a>(
        document: &'a mut VaultDocument,
        service: &str,
    ) -> &'a mut BTreeMap<String, String> {
        document.services.entry(service.to_string()).or_default()
    }

    fn zeroize_string(value: &mut String) {
        value.zeroize();
    }

    fn zeroize_accounts(accounts: BTreeMap<String, String>) {
        for (mut account, mut value) in accounts {
            Self::zeroize_string(&mut account);
            Self::zeroize_string(&mut value);
        }
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
        self.passphrase
            .with_session_material(|passphrase, cache_key| {
                self.with_store_lock(|| {
                    let mut state = self
                        .state
                        .lock()
                        .map_err(|_| anyhow!("vault lock poisoned"))?;
                    let document = self.load_document_locked(&mut state, passphrase, cache_key)?;
                    Ok(document
                        .services
                        .get(service)
                        .and_then(|accounts| accounts.get(account).cloned()))
                })
            })
    }

    fn get_many(&self, service: &str, accounts: &[&str]) -> Result<Vec<Option<String>>> {
        self.passphrase
            .with_session_material(|passphrase, cache_key| {
                self.with_store_lock(|| {
                    let mut state = self
                        .state
                        .lock()
                        .map_err(|_| anyhow!("vault lock poisoned"))?;
                    let document = self.load_document_locked(&mut state, passphrase, cache_key)?;
                    let stored_accounts = document.services.get(service);

                    Ok(accounts
                        .iter()
                        .map(|account| {
                            stored_accounts.and_then(|values| values.get(*account).cloned())
                        })
                        .collect())
                })
            })
    }

    fn set(&self, service: &str, account: &str, value: &str) -> Result<()> {
        self.with_document(|document| {
            if let Some(mut previous) = Self::account_map_mut(document, service)
                .insert(account.to_string(), value.to_string())
            {
                Self::zeroize_string(&mut previous);
            }
            Ok(())
        })
    }

    fn delete(&self, service: &str, account: &str) -> Result<()> {
        self.with_document(|document| {
            let remove_service = if let Some(accounts) = document.services.get_mut(service) {
                if let Some((mut stored_account, mut value)) = accounts.remove_entry(account) {
                    Self::zeroize_string(&mut stored_account);
                    Self::zeroize_string(&mut value);
                }
                accounts.is_empty()
            } else {
                false
            };

            if remove_service
                && let Some((mut stored_service, accounts)) =
                    document.services.remove_entry(service)
            {
                Self::zeroize_string(&mut stored_service);
                Self::zeroize_accounts(accounts);
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

#[derive(Default, Serialize, Deserialize, PartialEq, Eq)]
struct VaultDocument {
    #[serde(default)]
    services: BTreeMap<String, BTreeMap<String, String>>,
}

impl Zeroize for VaultDocument {
    fn zeroize(&mut self) {
        for (mut service, accounts) in std::mem::take(&mut self.services) {
            service.zeroize();
            for (mut account, mut value) in accounts {
                account.zeroize();
                value.zeroize();
            }
        }
    }
}

impl Drop for VaultDocument {
    fn drop(&mut self) {
        self.zeroize();
    }
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

fn derive_key(passphrase: &[u8], salt: &[u8]) -> Result<ProtectedDerivedKey> {
    record_vault_derivation();
    let params = Params::new(
        VAULT_MEMORY_COST.load(Ordering::Relaxed),
        VAULT_TIME_COST.load(Ordering::Relaxed),
        VAULT_PARALLELISM.load(Ordering::Relaxed),
        Some(VAULT_OUTPUT_LEN),
    )
    .map_err(|error| anyhow!("failed to create vault Argon2 params: {error}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    ProtectedDerivedKey::try_new(|key| {
        argon2
            .hash_password_into(passphrase, salt, key)
            .map_err(|error| anyhow!("vault key derivation failed: {error}"))
    })
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
    use crate::ProtectedPassphrase;
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

    fn protected(value: &str) -> ProtectedPassphrase {
        ProtectedPassphrase::try_from_string(value.to_string())
            .expect("test passphrase should use protected memory")
    }

    fn legacy_derive_key(passphrase: &str, salt: &[u8]) -> [u8; 32] {
        let params = Params::new(
            VAULT_MEMORY_COST.load(Ordering::Relaxed),
            VAULT_TIME_COST.load(Ordering::Relaxed),
            VAULT_PARALLELISM.load(Ordering::Relaxed),
            Some(VAULT_OUTPUT_LEN),
        )
        .expect("test Argon2 parameters should be valid");
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let mut key = [0u8; 32];
        argon2
            .hash_password_into(passphrase.as_bytes(), salt, &mut key)
            .expect("legacy test key derivation should succeed");
        key
    }

    fn test_backend(path: PathBuf) -> VaultCredentialBackend {
        set_vault_test_parameters();
        VaultCredentialBackend::new_with_path(path, protected("correct horse"))
    }

    fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
        !needle.is_empty()
            && haystack
                .windows(needle.len())
                .any(|window| window == needle)
    }

    #[test]
    fn vault_cache_does_not_retain_plaintext_secret_bytes() {
        let path = test_vault_path("encrypted-memory-cache");
        let backend = test_backend(path.clone());
        let secret = "memory-cache-secret-marker";
        backend
            .set(APP_CREDENTIAL_SERVICE, "sync:github-token", secret)
            .expect("secret should be stored");

        let state = backend.state.lock().expect("vault state should lock");
        let cached = state.as_ref().expect("vault state should be cached");
        let cached_bytes = &cached.encrypted_document.ciphertext;

        assert!(!contains_bytes(cached_bytes, secret.as_bytes()));
        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_document_zeroize_removes_all_plaintext_fields() {
        let mut document = VaultDocument::default();
        VaultCredentialBackend::account_map_mut(&mut document, APP_CREDENTIAL_SERVICE)
            .insert("sync:github-token".to_string(), "secret-token".to_string());

        document.zeroize();

        assert!(document.services.is_empty());
    }

    #[test]
    fn vault_string_zeroize_helper_clears_removed_plaintext() {
        let mut secret = "removed-secret".to_string();

        VaultCredentialBackend::zeroize_string(&mut secret);

        assert!(secret.is_empty());
    }

    #[test]
    fn corrupted_memory_cache_is_rebuilt_from_disk() {
        let path = test_vault_path("corrupted-memory-cache");
        let backend = test_backend(path.clone());
        backend
            .set(
                APP_CREDENTIAL_SERVICE,
                "sync:github-token",
                "disk-backed-token",
            )
            .expect("secret should be stored");

        {
            let mut state = backend.state.lock().expect("vault state should lock");
            let cached = state.as_mut().expect("vault state should be cached");
            cached.encrypted_document.ciphertext[0] ^= 0xff;
        }

        let value = backend
            .get(APP_CREDENTIAL_SERVICE, "sync:github-token")
            .expect("corrupted memory cache should be rebuilt from disk");

        assert_eq!(value.as_deref(), Some("disk-backed-token"));
        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_rotation_acquires_session_material_before_the_file_lock() {
        set_vault_test_parameters();
        let path = test_vault_path("rotation-lock-order");
        VaultCredentialBackend::new_with_path(path.clone(), protected("old passphrase"))
            .set(APP_CREDENTIAL_SERVICE, "sync:github-token", "token")
            .expect("initial vault should be written");
        VAULT_LOCK_TRACE.with(|trace| trace.borrow_mut().clear());

        VaultCredentialBackend::rotate_store_passphrase(
            path.clone(),
            protected("old passphrase"),
            protected("new passphrase"),
        )
        .expect("rotation should succeed");

        let trace = VAULT_LOCK_TRACE.with(|trace| trace.borrow().clone());
        assert_eq!(trace, vec!["rotation", "session", "session", "file"]);
        cleanup_test_vault(&path);
    }

    #[test]
    fn unchanged_memory_cache_reads_do_not_run_argon2() {
        let path = test_vault_path("cache-hit-no-argon2");
        let backend = test_backend(path.clone());
        backend
            .set(APP_CREDENTIAL_SERVICE, "sync:github-token", "token")
            .expect("secret should be stored");
        VAULT_DERIVE_COUNT.with(|count| count.set(0));

        assert_eq!(
            backend
                .get(APP_CREDENTIAL_SERVICE, "sync:github-token")
                .expect("cache read should succeed")
                .as_deref(),
            Some("token")
        );
        assert_eq!(
            backend
                .get_many(APP_CREDENTIAL_SERVICE, &["sync:github-token"])
                .expect("batched cache read should succeed"),
            vec![Some("token".to_string())]
        );

        assert_eq!(VAULT_DERIVE_COUNT.with(std::cell::Cell::get), 0);
        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_rotation_with_one_session_does_not_relock_the_same_secret() {
        set_vault_test_parameters();
        let path = test_vault_path("rotation-shared-session-lock-order");
        let passphrase = protected("same passphrase");
        VaultCredentialBackend::new_with_path(path.clone(), passphrase.clone())
            .set(APP_CREDENTIAL_SERVICE, "sync:github-token", "token")
            .expect("initial vault should be written");
        VAULT_LOCK_TRACE.with(|trace| trace.borrow_mut().clear());

        VaultCredentialBackend::rotate_store_passphrase(
            path.clone(),
            passphrase.clone(),
            passphrase,
        )
        .expect("rotation with one shared session should succeed");

        let trace = VAULT_LOCK_TRACE.with(|trace| trace.borrow().clone());
        assert_eq!(trace, vec!["rotation", "session", "file"]);
        cleanup_test_vault(&path);
    }

    #[test]
    fn revoked_session_write_preserves_file_and_encrypted_cache() {
        set_vault_test_parameters();
        let path = test_vault_path("revoked-write-preserves-state");
        let passphrase = protected("correct horse");
        let backend = VaultCredentialBackend::new_with_path(path.clone(), passphrase.clone());
        backend
            .set(
                APP_CREDENTIAL_SERVICE,
                "sync:github-token",
                "original-token",
            )
            .expect("initial secret should be stored");
        let file_before = fs::read(&path).expect("vault file should be readable");
        let cache_before = {
            let state = backend.state.lock().expect("vault state should lock");
            state
                .as_ref()
                .expect("vault state should be cached")
                .encrypted_document
                .ciphertext
                .clone()
        };
        passphrase.revoke();

        backend
            .set(
                APP_CREDENTIAL_SERVICE,
                "sync:github-token",
                "replacement-token",
            )
            .expect_err("revoked session must reject writes");

        assert_eq!(
            fs::read(&path).expect("vault file should remain readable"),
            file_before
        );
        let state = backend.state.lock().expect("vault state should lock");
        assert_eq!(
            state
                .as_ref()
                .expect("encrypted cache should remain installed")
                .encrypted_document
                .ciphertext,
            cache_before
        );
        cleanup_test_vault(&path);
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
    fn protected_backend_reads_a_legacy_v1_fixture() {
        set_vault_test_parameters();
        let path = test_vault_path("legacy-v1-fixture");
        let salt = [3u8; 32];
        let nonce = Nonce::<Aes256Gcm>::from([9u8; 12]);
        let key = legacy_derive_key("correct horse", &salt);
        let cipher = Aes256Gcm::new(&Array(key));
        let mut document = VaultDocument::default();
        VaultCredentialBackend::account_map_mut(&mut document, APP_CREDENTIAL_SERVICE)
            .insert("sync:github-token".to_string(), "legacy-token".to_string());
        let plaintext = serde_json::to_vec(&document).expect("legacy document should serialize");
        let ciphertext = cipher
            .encrypt(
                &nonce,
                aes_gcm::aead::Payload {
                    msg: &plaintext,
                    aad: VAULT_AAD,
                },
            )
            .expect("legacy fixture encryption should succeed");
        let mut combined = nonce.to_vec();
        combined.extend_from_slice(&ciphertext);
        let stored = StoredVaultDocument {
            version: VAULT_VERSION,
            salt: base64::engine::general_purpose::STANDARD.encode(salt),
            encrypted_payload: base64::engine::general_purpose::STANDARD.encode(combined),
        };
        fs::write(
            &path,
            serde_json::to_vec_pretty(&stored).expect("fixture should serialize"),
        )
        .expect("fixture should be written");

        let value = VaultCredentialBackend::new_with_path(path.clone(), protected("correct horse"))
            .get(APP_CREDENTIAL_SERVICE, "sync:github-token")
            .expect("protected backend should read legacy fixture");

        assert_eq!(value.as_deref(), Some("legacy-token"));
        cleanup_test_vault(&path);
    }

    #[test]
    fn protected_backend_writes_legacy_v1_compatible_documents() {
        let path = test_vault_path("legacy-v1-output");
        let backend = test_backend(path.clone());
        backend
            .set(
                APP_CREDENTIAL_SERVICE,
                "sync:github-token",
                "protected-token",
            )
            .expect("protected backend should write vault");

        let stored: StoredVaultDocument =
            serde_json::from_slice(&fs::read(&path).expect("vault should be readable"))
                .expect("vault envelope should parse");
        assert_eq!(stored.version, VAULT_VERSION);
        let salt = base64::engine::general_purpose::STANDARD
            .decode(stored.salt)
            .expect("salt should decode");
        let combined = base64::engine::general_purpose::STANDARD
            .decode(stored.encrypted_payload)
            .expect("payload should decode");
        let (nonce, ciphertext) = combined.split_at(12);
        let key = legacy_derive_key("correct horse", &salt);
        let plaintext = Aes256Gcm::new(&Array(key))
            .decrypt(
                &Nonce::<Aes256Gcm>::try_from(nonce).expect("nonce should be valid"),
                aes_gcm::aead::Payload {
                    msg: ciphertext,
                    aad: VAULT_AAD,
                },
            )
            .expect("legacy crypto path should decrypt protected output");
        let document: VaultDocument =
            serde_json::from_slice(&plaintext).expect("legacy plaintext should parse");

        assert_eq!(
            document
                .services
                .get(APP_CREDENTIAL_SERVICE)
                .and_then(|accounts| accounts.get("sync:github-token"))
                .map(String::as_str),
            Some("protected-token")
        );
        cleanup_test_vault(&path);
    }

    #[test]
    fn revoking_the_passphrase_invalidates_existing_backend_clones() {
        set_vault_test_parameters();
        let path = test_vault_path("revoked-passphrase");
        let passphrase = ProtectedPassphrase::try_from_string("correct horse".to_string())
            .expect("passphrase should be protected");
        let backend = VaultCredentialBackend::new_with_path(path.clone(), passphrase.clone());

        backend
            .set(APP_CREDENTIAL_SERVICE, "sync:github-token", "secret-token")
            .expect("set should succeed");
        passphrase.revoke();

        let error = backend
            .get(APP_CREDENTIAL_SERVICE, "sync:github-token")
            .expect_err("revoked backend must not decrypt the vault");
        assert!(error.to_string().contains("revoked"));

        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_backend_rejects_wrong_passphrase() {
        set_vault_test_parameters();
        let path = test_vault_path("wrong-passphrase");
        VaultCredentialBackend::new_with_path(path.clone(), protected("correct horse"))
            .set(APP_CREDENTIAL_SERVICE, "sync:webdav-password", "secret")
            .expect("initial set should succeed");

        let error = VaultCredentialBackend::new_with_path(path.clone(), protected("wrong horse"))
            .get(APP_CREDENTIAL_SERVICE, "sync:webdav-password")
            .expect_err("wrong passphrase should fail");

        assert!(error.to_string().contains("vault decryption failed"));

        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_backend_rotates_passphrase() {
        set_vault_test_parameters();
        let path = test_vault_path("rotate-passphrase");
        VaultCredentialBackend::new_with_path(path.clone(), protected("correct horse"))
            .set(APP_CREDENTIAL_SERVICE, "sync:github-token", "secret-token")
            .expect("initial set should succeed");

        VaultCredentialBackend::rotate_store_passphrase(
            path.clone(),
            protected("correct horse"),
            protected("new horse"),
        )
        .expect("rotation should succeed");

        let value = VaultCredentialBackend::new_with_path(path.clone(), protected("new horse"))
            .get(APP_CREDENTIAL_SERVICE, "sync:github-token")
            .expect("new passphrase should decrypt vault");
        assert_eq!(value.as_deref(), Some("secret-token"));

        let error = VaultCredentialBackend::new_with_path(path.clone(), protected("correct horse"))
            .get(APP_CREDENTIAL_SERVICE, "sync:github-token")
            .expect_err("old passphrase should no longer decrypt vault");
        assert!(error.to_string().contains("vault decryption failed"));

        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_backend_rejects_rotation_with_wrong_passphrase_without_rewriting() {
        set_vault_test_parameters();
        let path = test_vault_path("rotate-wrong-passphrase");
        VaultCredentialBackend::new_with_path(path.clone(), protected("correct horse"))
            .set(APP_CREDENTIAL_SERVICE, "sync:github-token", "secret-token")
            .expect("initial set should succeed");
        let before = fs::read(&path).expect("vault file should exist");

        let error = VaultCredentialBackend::rotate_store_passphrase(
            path.clone(),
            protected("wrong horse"),
            protected("new horse"),
        )
        .expect_err("rotation with the wrong current passphrase should fail");

        assert!(error.to_string().contains("vault decryption failed"));
        assert_eq!(
            fs::read(&path).expect("vault file should remain readable"),
            before
        );
        let value = VaultCredentialBackend::new_with_path(path.clone(), protected("correct horse"))
            .get(APP_CREDENTIAL_SERVICE, "sync:github-token")
            .expect("the original passphrase should still decrypt the vault");
        assert_eq!(value.as_deref(), Some("secret-token"));

        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_backend_initialize_does_not_rewrite_existing_store() {
        set_vault_test_parameters();
        let path = test_vault_path("initialize-no-rewrite");
        let backend =
            VaultCredentialBackend::new_with_path(path.clone(), protected("correct horse"));

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
        VaultCredentialBackend::new_with_path(path.clone(), protected("correct horse"))
            .set(APP_CREDENTIAL_SERVICE, "sync:webdav-password", "secret")
            .expect("initial set should succeed");

        let error = VaultCredentialBackend::new_with_path(path.clone(), protected("wrong horse"))
            .initialize(APP_CREDENTIAL_SERVICE)
            .expect_err("wrong passphrase should fail validation");

        assert!(error.to_string().contains("vault decryption failed"));

        cleanup_test_vault(&path);
    }

    #[test]
    fn vault_backend_get_many_returns_requested_accounts_in_order() {
        set_vault_test_parameters();
        let path = test_vault_path("get-many");
        let backend =
            VaultCredentialBackend::new_with_path(path.clone(), protected("correct horse"));

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

        VaultCredentialBackend::rotate_store_passphrase(
            path.clone(),
            protected("correct horse"),
            protected("new horse"),
        )
        .expect("rotation should succeed");

        let error = old_backend
            .set(
                APP_CREDENTIAL_SERVICE,
                "sync:webdav-password",
                "must-not-save",
            )
            .expect_err("old cached backend must refresh and reject the new ciphertext");
        assert!(error.to_string().contains("vault decryption failed"));

        let new_backend =
            VaultCredentialBackend::new_with_path(path.clone(), protected("new horse"));
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
