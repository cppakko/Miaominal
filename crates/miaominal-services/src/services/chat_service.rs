use anyhow::{Context, Result};
use base64::Engine as _;
use miaominal_agent::engine::TranscriptSink;
use miaominal_agent::transcript::{
    Item, item_to_record, items_from_legacy_records, items_from_records,
};
use miaominal_secrets::CredentialStore;
use miaominal_storage::chat_store::{
    ChatItemRecord, ChatMessageRecord, ChatSessionRecord, ChatStore, LoadedChatItem,
};

const CHAT_KEY_ACCOUNT: &str = "chat-db-key";
const CHAT_DB_FILE_NAME: &str = "chat_history.db";

/// Persists agent engine items for one conversation.
pub struct ChatTranscriptSink {
    service: std::sync::Mutex<ChatService>,
    session_id: String,
}

impl TranscriptSink for ChatTranscriptSink {
    fn append(&self, items: &[Item]) -> std::result::Result<(), String> {
        let service = self
            .service
            .lock()
            .map_err(|_| "chat store lock poisoned".to_string())?;
        service
            .append_transcript(&self.session_id, items)
            .map_err(|error| format!("{error:#}"))
    }
}

pub struct ChatService {
    store: ChatStore,
    key: [u8; 32],
    path: std::path::PathBuf,
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
        Ok(Self {
            store,
            key,
            path: db_path,
        })
    }

    /// Open a second connection to the same database (for use from the
    /// agent session task, which needs `Send + Sync` access).
    pub fn reopen(&self) -> Result<Self> {
        Ok(Self {
            store: ChatStore::open(&self.path)?,
            key: self.key,
            path: self.path.clone(),
        })
    }

    /// Load a conversation as transcript items. Pre-v2 sessions are
    /// migrated once (legacy rows stay in place for rollback); dangling
    /// tool calls left by a crash are repaired and the repairs persisted.
    pub fn load_transcript(&self, session_id: &str) -> Result<Vec<Item>> {
        match self.store.session_schema_version(session_id)? {
            None => Ok(Vec::new()),
            Some(version) if version < 2 => {
                let legacy = self.store.load_session_messages(session_id, &self.key)?;
                let items = items_from_legacy_records(legacy);
                let records: Vec<ChatItemRecord> = items
                    .iter()
                    .enumerate()
                    .map(|(seq, item)| item_to_record(item, seq as i64))
                    .collect();
                self.store.replace_items(session_id, &records, &self.key)?;
                Ok(items)
            }
            Some(_) => {
                let rows = self.store.load_items(session_id, &self.key)?;
                let (items, repairs) = items_from_records(rows);
                if !repairs.is_empty() {
                    self.append_transcript(session_id, &repairs)?;
                }
                Ok(items)
            }
        }
    }

    /// Append committed transcript items in order.
    pub fn append_transcript(&self, session_id: &str, items: &[Item]) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        if self.store.session_schema_version(session_id)?.is_none() {
            let now = items.first().map(|item| item.created_at).unwrap_or(0);
            self.store.create_session_v2(session_id, now)?;
        }
        let next = self.store.max_item_seq(session_id)? + 1;
        let records: Vec<ChatItemRecord> = items
            .iter()
            .enumerate()
            .map(|(offset, item)| item_to_record(item, next + offset as i64))
            .collect();
        self.store.append_items(session_id, &records, &self.key)
    }

    /// A thread-safe sink that persists engine items for one session.
    pub fn transcript_sink(&self, session_id: &str) -> Result<ChatTranscriptSink> {
        Ok(ChatTranscriptSink {
            service: std::sync::Mutex::new(self.reopen()?),
            session_id: session_id.to_string(),
        })
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

    pub fn session_schema_version(&self, id: &str) -> Result<Option<u32>> {
        self.store.session_schema_version(id)
    }

    pub fn create_session_v2(&self, id: &str, now: i64) -> Result<()> {
        self.store.create_session_v2(id, now)
    }

    pub fn append_items(&self, session_id: &str, items: &[ChatItemRecord]) -> Result<()> {
        self.store.append_items(session_id, items, &self.key)
    }

    pub fn replace_items(&self, session_id: &str, items: &[ChatItemRecord]) -> Result<()> {
        self.store.replace_items(session_id, items, &self.key)
    }

    pub fn load_items(&self, session_id: &str) -> Result<Vec<LoadedChatItem>> {
        self.store.load_items(session_id, &self.key)
    }

    pub fn max_item_seq(&self, session_id: &str) -> Result<i64> {
        self.store.max_item_seq(session_id)
    }

    pub fn save_session_config(&self, session_id: &str, config: &str) -> Result<()> {
        self.store
            .save_session_config(session_id, config, &self.key)
    }

    pub fn load_session_config(&self, session_id: &str) -> Result<Option<String>> {
        self.store.load_session_config(session_id, &self.key)
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

    #[test]
    fn legacy_sessions_migrate_and_engine_items_append() {
        use miaominal_agent::transcript::ItemKind;
        use miaominal_storage::chat_store::ChatMessageRole;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chat.db");
        let service = ChatService {
            store: ChatStore::open(&path).unwrap(),
            key: [3u8; 32],
            path: path.clone(),
        };
        service.create_session("old", 1).unwrap();
        for (order, (role, content)) in [
            (ChatMessageRole::User, "hi"),
            (ChatMessageRole::Assistant, "hello"),
        ]
        .into_iter()
        .enumerate()
        {
            service
                .insert_message(&ChatMessageRecord {
                    id: format!("m{order}"),
                    session_id: "old".into(),
                    role,
                    content: content.into(),
                    tool_name: None,
                    tool_summary: None,
                    tool_status: None,
                    sort_order: order as i64,
                    created_at: 1,
                    attachments: None,
                })
                .unwrap();
        }

        let items = service.load_transcript("old").unwrap();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0].kind, ItemKind::UserMessage { .. }));
        assert_eq!(service.session_schema_version("old").unwrap(), Some(2));

        // Appends through the sink continue the sequence; reload is stable.
        let sink = service.transcript_sink("old").unwrap();
        let reply = Item::new(
            "t",
            ItemKind::AssistantText {
                text: "more".into(),
                completion_id: "c".into(),
            },
        );
        sink.append(std::slice::from_ref(&reply)).unwrap();
        let reloaded = service.load_transcript("old").unwrap();
        assert_eq!(reloaded.len(), 3);
        assert_eq!(reloaded[2], reply);
        assert_eq!(reloaded[..2], items[..]);

        // New sessions are created on first append.
        let first = Item::new(
            "t2",
            ItemKind::AssistantText {
                text: "first".into(),
                completion_id: "c2".into(),
            },
        );
        service.append_transcript("new", &[first.clone()]).unwrap();
        assert_eq!(service.load_transcript("new").unwrap(), vec![first]);
        assert!(service.load_transcript("missing").unwrap().is_empty());
    }
}
