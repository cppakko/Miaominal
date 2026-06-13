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
    pub fn open(credentials: &CredentialStore) -> Result<Self> {
        let key = load_or_create_key(credentials)?;
        let db_path = miaominal_paths::config_file(CHAT_DB_FILE_NAME)?;
        let store = ChatStore::open(&db_path)?;
        Ok(Self { store, key })
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

fn load_or_create_key(credentials: &CredentialStore) -> Result<[u8; 32]> {
    if let Some(encoded) = credentials.get(CHAT_KEY_ACCOUNT)? {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .context("failed to decode chat database key")?;
        let key: [u8; 32] = decoded
            .try_into()
            .map_err(|_| anyhow::anyhow!("chat database key must be 32 bytes"))?;
        return Ok(key);
    }

    let key: [u8; 32] = rand::random();
    let encoded = base64::engine::general_purpose::STANDARD.encode(key);
    credentials
        .set(CHAT_KEY_ACCOUNT, &encoded)
        .context("failed to store chat database key")?;
    Ok(key)
}
