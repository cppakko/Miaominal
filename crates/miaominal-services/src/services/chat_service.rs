use anyhow::{Context, Result};
use base64::Engine as _;
use miaominal_secrets::CredentialStore;
use miaominal_storage::chat_store::{ChatMessageRecord, ChatSessionRecord, ChatStore};

const CHAT_KEY_ACCOUNT: &str = "chat-db-key";
const CHAT_DB_FILE_NAME: &str = "chat_history.db";

pub struct ChatService {
    store: ChatStore,
    key: [u8; 32],
}

impl ChatService {
    pub fn open_default() -> Result<Self> {
        Self::open(&CredentialStore::new_keyring(
            miaominal_secrets::APP_CREDENTIAL_SERVICE,
        ))
    }

    pub fn open(credentials: &CredentialStore) -> Result<Self> {
        let db_path = miaominal_paths::config_file(CHAT_DB_FILE_NAME)?;
        let key = load_or_create_key(credentials, &db_path)?;
        let store = ChatStore::open(&db_path)?;
        Ok(Self { store, key })
    }

    pub fn delete_key(credentials: &CredentialStore) -> Result<()> {
        credentials.delete(CHAT_KEY_ACCOUNT)
    }

    pub fn create_session(&self, id: &str, now: i64) -> Result<()> {
        self.store.create_session(id, now)
    }

    pub fn update_session_title(&self, id: &str, title: &str) -> Result<()> {
        self.store.update_session_title(id, title)
    }

    pub fn list_sessions(&self) -> Result<Vec<ChatSessionRecord>> {
        self.store.list_sessions()
    }

    pub fn insert_message(&self, record: &ChatMessageRecord) -> Result<()> {
        self.store.insert_message(record, &self.key)
    }

    pub fn replace_session_messages(
        &self,
        session_id: &str,
        records: &[ChatMessageRecord],
    ) -> Result<()> {
        self.store
            .replace_session_messages(session_id, records, &self.key)
    }

    pub fn load_session_messages(&self, session_id: &str) -> Result<Vec<ChatMessageRecord>> {
        self.store.load_session_messages(session_id, &self.key)
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        self.store.delete_session(id)
    }

    pub fn session_title(&self, id: &str) -> Result<Option<String>> {
        self.store.session_title(id)
    }
}

fn load_or_create_key(
    credentials: &CredentialStore,
    db_path: &std::path::Path,
) -> Result<[u8; 32]> {
    if let Some(encoded) = credentials.get(CHAT_KEY_ACCOUNT)? {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .context("failed to decode chat database key")?;
        let key: [u8; 32] = decoded
            .try_into()
            .map_err(|_| anyhow::anyhow!("chat database key must be 32 bytes"))?;
        return Ok(key);
    }

    if db_path.metadata().is_ok_and(|metadata| metadata.len() > 0) {
        anyhow::bail!(
            "chat database {} exists but its encryption key is unavailable",
            db_path.display()
        );
    }
    let key: [u8; 32] = rand::random();
    let encoded = base64::engine::general_purpose::STANDARD.encode(key);
    credentials
        .set(CHAT_KEY_ACCOUNT, &encoded)
        .context("failed to store chat database key")?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_secrets::credential_backend::CredentialBackend;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct MemoryBackend(Arc<Mutex<HashMap<String, String>>>);

    impl CredentialBackend for MemoryBackend {
        fn name(&self) -> &'static str {
            "memory"
        }

        fn get(&self, _service: &str, account: &str) -> Result<Option<String>> {
            Ok(self.0.lock().unwrap().get(account).cloned())
        }

        fn set(&self, _service: &str, account: &str, value: &str) -> Result<()> {
            self.0
                .lock()
                .unwrap()
                .insert(account.to_string(), value.to_string());
            Ok(())
        }

        fn delete(&self, _service: &str, account: &str) -> Result<()> {
            self.0.lock().unwrap().remove(account);
            Ok(())
        }
    }

    fn memory_credentials() -> CredentialStore {
        CredentialStore::with_backend("test", MemoryBackend::default())
    }

    #[test]
    fn missing_key_is_rejected_for_existing_database() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("chat.db");
        std::fs::write(&database, b"existing database").unwrap();

        let error = load_or_create_key(&memory_credentials(), &database)
            .expect_err("existing database without key should be rejected");

        assert!(error.to_string().contains("encryption key is unavailable"));
    }

    #[test]
    fn new_database_gets_a_persisted_key() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("chat.db");
        let credentials = memory_credentials();

        let first = load_or_create_key(&credentials, &database).unwrap();
        let second = load_or_create_key(&credentials, &database).unwrap();

        assert_eq!(first, second);
    }
}
