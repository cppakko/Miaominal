use anyhow::{Context, Result, anyhow};
use miaominal_secrets::{decrypt_with_aad, encrypt_with_aad};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

const CHAT_AAD: &[u8] = b"miaominal.chat.v1";
const CHAT_ITEM_AAD: &[u8] = b"miaominal.chat.v2";
const DECRYPT_FAILED_TEXT: &str = "[无法解密]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatMessageRole {
    User,
    Assistant,
    ToolCall,
    Thinking,
    Error,
}

impl ChatMessageRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::ToolCall => "tool_call",
            Self::Thinking => "thinking",
            Self::Error => "error",
        }
    }

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "tool_call" => Ok(Self::ToolCall),
            "thinking" => Ok(Self::Thinking),
            "error" => Ok(Self::Error),
            _ => Err(anyhow!("unknown chat message role: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSessionRecord {
    pub id: String,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessageRecord {
    pub id: String,
    pub session_id: String,
    pub role: ChatMessageRole,
    pub content: String,
    pub tool_name: Option<String>,
    pub tool_summary: Option<String>,
    pub tool_status: Option<String>,
    pub sort_order: i64,
    pub created_at: i64,
    /// JSON-serialized `Vec<ChatAttachment>` (encrypted at rest), or `None`
    /// for messages without attachments or pre-migration rows.
    pub attachments: Option<String>,
}

pub struct ChatStore(Connection);

impl ChatStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let connection =
            Connection::open(path).with_context(|| format!("failed to open {}", path.display()))?;
        let store = Self(connection);
        store.run_migrations()?;
        Ok(store)
    }

    pub fn create_session(&self, id: &str, now: i64) -> Result<()> {
        self.0
            .execute(
                "INSERT OR IGNORE INTO chat_sessions (id, title, created_at, updated_at)
                 VALUES (?1, '', ?2, ?2)",
                params![id, now],
            )
            .context("failed to create chat session")?;
        Ok(())
    }

    pub fn update_session_title(&self, id: &str, title: &str) -> Result<()> {
        self.0
            .execute(
                "UPDATE chat_sessions
                 SET title = ?2, updated_at = MAX(updated_at, strftime('%s','now'))
                 WHERE id = ?1",
                params![id, title],
            )
            .context("failed to update chat session title")?;
        Ok(())
    }

    pub fn list_sessions(&self) -> Result<Vec<ChatSessionRecord>> {
        let mut statement = self
            .0
            .prepare(
                "SELECT id, title, created_at, updated_at
                 FROM chat_sessions
                 ORDER BY updated_at DESC, created_at DESC",
            )
            .context("failed to prepare chat session list query")?;

        let records = statement
            .query_map([], |row| {
                Ok(ChatSessionRecord {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to list chat sessions")?;
        Ok(records)
    }

    pub fn insert_message(&self, record: &ChatMessageRecord, key: &[u8; 32]) -> Result<()> {
        let encrypted_content = encrypt_text(key, &record.content)?;
        let encrypted_tool_summary = record
            .tool_summary
            .as_deref()
            .map(|summary| encrypt_text(key, summary))
            .transpose()?;
        let encrypted_attachments = record
            .attachments
            .as_deref()
            .map(|attachments| encrypt_text(key, attachments))
            .transpose()?;

        self.0
            .execute(
                "INSERT OR REPLACE INTO chat_messages
                 (id, session_id, role, content, tool_name, tool_summary, tool_status, sort_order, created_at, attachments)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    record.id,
                    record.session_id,
                    record.role.as_str(),
                    encrypted_content,
                    record.tool_name,
                    encrypted_tool_summary,
                    record.tool_status,
                    record.sort_order,
                    record.created_at,
                    encrypted_attachments,
                ],
            )
            .context("failed to insert chat message")?;
        self.0
            .execute(
                "UPDATE chat_sessions
                 SET updated_at = MAX(updated_at, ?2)
                 WHERE id = ?1",
                params![record.session_id, record.created_at],
            )
            .context("failed to update chat session timestamp")?;
        Ok(())
    }

    pub fn replace_session_messages(
        &self,
        session_id: &str,
        records: &[ChatMessageRecord],
        key: &[u8; 32],
    ) -> Result<()> {
        if records.iter().any(|record| record.session_id != session_id) {
            return Err(anyhow!(
                "cannot replace chat messages with records from another session"
            ));
        }

        let transaction = self
            .0
            .unchecked_transaction()
            .context("failed to start chat message replacement transaction")?;
        transaction
            .execute(
                "DELETE FROM chat_messages WHERE session_id = ?1",
                [session_id],
            )
            .context("failed to delete existing chat messages")?;

        for record in records {
            let encrypted_content = encrypt_text(key, &record.content)?;
            let encrypted_tool_summary = record
                .tool_summary
                .as_deref()
                .map(|summary| encrypt_text(key, summary))
                .transpose()?;
            let encrypted_attachments = record
                .attachments
                .as_deref()
                .map(|attachments| encrypt_text(key, attachments))
                .transpose()?;

            transaction
                .execute(
                    "INSERT INTO chat_messages
                     (id, session_id, role, content, tool_name, tool_summary, tool_status, sort_order, created_at, attachments)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        record.id,
                        record.session_id,
                        record.role.as_str(),
                        encrypted_content,
                        record.tool_name,
                        encrypted_tool_summary,
                        record.tool_status,
                        record.sort_order,
                        record.created_at,
                        encrypted_attachments,
                    ],
                )
                .context("failed to insert replacement chat message")?;
        }

        if let Some(updated_at) = records.iter().map(|record| record.created_at).max() {
            transaction
                .execute(
                    "UPDATE chat_sessions
                     SET updated_at = MAX(updated_at, ?2)
                     WHERE id = ?1",
                    params![session_id, updated_at],
                )
                .context("failed to update chat session timestamp")?;
        }

        transaction
            .commit()
            .context("failed to commit chat message replacement")?;
        Ok(())
    }

    pub fn load_session_messages(
        &self,
        session_id: &str,
        key: &[u8; 32],
    ) -> Result<Vec<ChatMessageRecord>> {
        let mut statement = self
            .0
            .prepare(
                "SELECT id, session_id, role, content, tool_name, tool_summary, tool_status, sort_order, created_at, attachments
                 FROM chat_messages
                 WHERE session_id = ?1
                 ORDER BY sort_order ASC",
            )
            .context("failed to prepare chat message load query")?;

        let records = statement
            .query_map([session_id], |row| {
                let role_text: String = row.get(2)?;
                let content: Vec<u8> = row.get(3)?;
                let tool_summary: Option<Vec<u8>> = row.get(5)?;
                let attachments_cipher: Option<Vec<u8>> = row.get(9)?;
                Ok(ChatMessageRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    role: ChatMessageRole::from_str(&role_text).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Text,
                            Box::new(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                error.to_string(),
                            )),
                        )
                    })?,
                    content: decrypt_text(key, &content)
                        .unwrap_or_else(|| DECRYPT_FAILED_TEXT.to_string()),
                    tool_name: row.get(4)?,
                    tool_summary: tool_summary.as_deref().map(|cipher| {
                        decrypt_text(key, cipher).unwrap_or_else(|| DECRYPT_FAILED_TEXT.to_string())
                    }),
                    tool_status: row.get(6)?,
                    sort_order: row.get(7)?,
                    created_at: row.get(8)?,
                    attachments: attachments_cipher
                        .as_deref()
                        .and_then(|cipher| decrypt_text(key, cipher)),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load chat messages")?;
        Ok(records)
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        self.0
            .execute("DELETE FROM chat_sessions WHERE id = ?1", [id])
            .context("failed to delete chat session")?;
        Ok(())
    }

    pub fn session_title(&self, id: &str) -> Result<Option<String>> {
        self.0
            .query_row(
                "SELECT title FROM chat_sessions WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .context("failed to load chat session title")
    }

    fn run_migrations(&self) -> Result<()> {
        self.0
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS chat_sessions (
                     id TEXT PRIMARY KEY,
                     title TEXT NOT NULL DEFAULT '',
                     created_at INTEGER NOT NULL,
                     updated_at INTEGER NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS chat_messages (
                     id TEXT PRIMARY KEY,
                     session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
                     role TEXT NOT NULL,
                     content BLOB NOT NULL,
                     tool_name TEXT,
                     tool_summary BLOB,
                     sort_order INTEGER NOT NULL,
                     created_at INTEGER NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_messages_session
                     ON chat_messages(session_id, sort_order);",
            )
            .context("failed to migrate chat store")?;
        if !self.chat_messages_has_column("tool_status")? {
            self.0
                .execute("ALTER TABLE chat_messages ADD COLUMN tool_status TEXT", [])
                .context("failed to add chat_messages.tool_status")?;
        }
        if !self.chat_messages_has_column("attachments")? {
            self.0
                .execute("ALTER TABLE chat_messages ADD COLUMN attachments BLOB", [])
                .context("failed to add chat_messages.attachments")?;
        }
        self.run_item_migrations()?;
        Ok(())
    }

    fn chat_messages_has_column(&self, column: &str) -> Result<bool> {
        let mut statement = self
            .0
            .prepare("PRAGMA table_info(chat_messages)")
            .context("failed to inspect chat_messages columns")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to read chat_messages columns")?;
        Ok(columns.iter().any(|name| name == column))
    }

    fn chat_sessions_has_column(&self, column: &str) -> Result<bool> {
        let mut statement = self
            .0
            .prepare("PRAGMA table_info(chat_sessions)")
            .context("failed to inspect chat_sessions columns")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to read chat_sessions columns")?;
        Ok(columns.iter().any(|name| name == column))
    }

    fn run_item_migrations(&self) -> Result<()> {
        self.0
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS chat_items (
                     id TEXT PRIMARY KEY,
                     session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
                     seq INTEGER NOT NULL,
                     turn_id TEXT NOT NULL,
                     kind TEXT NOT NULL,
                     payload BLOB NOT NULL,
                     created_at INTEGER NOT NULL,
                     UNIQUE(session_id, seq)
                 );
                 CREATE INDEX IF NOT EXISTS idx_items_session ON chat_items(session_id, seq);",
            )
            .context("failed to create chat_items")?;
        if !self.chat_sessions_has_column("schema_version")? {
            self.0
                .execute(
                    "ALTER TABLE chat_sessions ADD COLUMN schema_version INTEGER NOT NULL DEFAULT 1",
                    [],
                )
                .context("failed to add chat_sessions.schema_version")?;
        }
        if !self.chat_sessions_has_column("config")? {
            self.0
                .execute("ALTER TABLE chat_sessions ADD COLUMN config BLOB", [])
                .context("failed to add chat_sessions.config")?;
        }
        Ok(())
    }

    /// Session schema: 1 = legacy `chat_messages`, 2 = `chat_items`.
    pub fn session_schema_version(&self, session_id: &str) -> Result<Option<u32>> {
        self.0
            .query_row(
                "SELECT schema_version FROM chat_sessions WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()
            .context("failed to load chat session schema version")
    }

    /// Create a session already on schema v2.
    pub fn create_session_v2(&self, id: &str, now: i64) -> Result<()> {
        self.0
            .execute(
                "INSERT OR IGNORE INTO chat_sessions (id, title, created_at, updated_at, schema_version)
                 VALUES (?1, '', ?2, ?2, 2)",
                params![id, now],
            )
            .context("failed to create chat session")?;
        Ok(())
    }

    /// Append transcript items. Items are immutable once written: `seq`
    /// values must be greater than existing ones and ids must be new.
    pub fn append_items(
        &self,
        session_id: &str,
        items: &[ChatItemRecord],
        key: &[u8; 32],
    ) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let transaction = self
            .0
            .unchecked_transaction()
            .context("failed to start chat item transaction")?;
        for item in items {
            let payload = encrypt_item(key, &item.payload)?;
            transaction
                .execute(
                    "INSERT INTO chat_items (id, session_id, seq, turn_id, kind, payload, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        item.id,
                        session_id,
                        item.seq,
                        item.turn_id,
                        item.kind,
                        payload,
                        item.created_at
                    ],
                )
                .context("failed to append chat item")?;
        }
        let updated_at = items.iter().map(|item| item.created_at).max().unwrap_or(0);
        transaction
            .execute(
                "UPDATE chat_sessions SET updated_at = MAX(updated_at, ?2), schema_version = 2
                 WHERE id = ?1",
                params![session_id, updated_at],
            )
            .context("failed to update chat session timestamp")?;
        transaction
            .commit()
            .context("failed to commit chat items")?;
        Ok(())
    }

    /// Replace every item of a session. Used only by the one-time legacy
    /// migration. Leaves the legacy `chat_messages` rows in place.
    pub fn replace_items(
        &self,
        session_id: &str,
        items: &[ChatItemRecord],
        key: &[u8; 32],
    ) -> Result<()> {
        let transaction = self
            .0
            .unchecked_transaction()
            .context("failed to start chat item replacement")?;
        transaction
            .execute("DELETE FROM chat_items WHERE session_id = ?1", [session_id])
            .context("failed to clear chat items")?;
        for item in items {
            let payload = encrypt_item(key, &item.payload)?;
            transaction
                .execute(
                    "INSERT INTO chat_items (id, session_id, seq, turn_id, kind, payload, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        item.id,
                        session_id,
                        item.seq,
                        item.turn_id,
                        item.kind,
                        payload,
                        item.created_at
                    ],
                )
                .context("failed to insert chat item")?;
        }
        transaction
            .execute(
                "UPDATE chat_sessions SET schema_version = 2 WHERE id = ?1",
                [session_id],
            )
            .context("failed to mark chat session as v2")?;
        transaction
            .commit()
            .context("failed to commit chat item replacement")?;
        Ok(())
    }

    /// Load items in sequence order. Items whose payload cannot be decrypted
    /// are returned with `payload = None`.
    pub fn load_items(&self, session_id: &str, key: &[u8; 32]) -> Result<Vec<LoadedChatItem>> {
        let mut statement = self
            .0
            .prepare(
                "SELECT id, seq, turn_id, kind, payload, created_at FROM chat_items
                 WHERE session_id = ?1 ORDER BY seq ASC",
            )
            .context("failed to prepare chat item query")?;
        let rows = statement
            .query_map([session_id], |row| {
                let cipher: Vec<u8> = row.get(4)?;
                Ok(LoadedChatItem {
                    id: row.get(0)?,
                    seq: row.get(1)?,
                    turn_id: row.get(2)?,
                    kind: row.get(3)?,
                    payload: decrypt_item(key, &cipher),
                    created_at: row.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load chat items")?;
        Ok(rows)
    }

    pub fn max_item_seq(&self, session_id: &str) -> Result<i64> {
        self.0
            .query_row(
                "SELECT COALESCE(MAX(seq), -1) FROM chat_items WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .context("failed to read max chat item seq")
    }

    pub fn save_session_config(
        &self,
        session_id: &str,
        config: &str,
        key: &[u8; 32],
    ) -> Result<()> {
        let cipher = encrypt_item(key, config)?;
        self.0
            .execute(
                "UPDATE chat_sessions SET config = ?2 WHERE id = ?1",
                params![session_id, cipher],
            )
            .context("failed to save chat session config")?;
        Ok(())
    }

    pub fn load_session_config(&self, session_id: &str, key: &[u8; 32]) -> Result<Option<String>> {
        let cipher: Option<Option<Vec<u8>>> = self
            .0
            .query_row(
                "SELECT config FROM chat_sessions WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()
            .context("failed to load chat session config")?;
        Ok(cipher
            .flatten()
            .and_then(|cipher| decrypt_item(key, &cipher)))
    }
}

/// One transcript item row. `payload` is the item JSON (plaintext here; it is
/// encrypted at rest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatItemRecord {
    pub id: String,
    pub seq: i64,
    pub turn_id: String,
    pub kind: String,
    pub payload: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedChatItem {
    pub id: String,
    pub seq: i64,
    pub turn_id: String,
    pub kind: String,
    pub payload: Option<String>,
    pub created_at: i64,
}

fn encrypt_item(key: &[u8; 32], plain: &str) -> Result<Vec<u8>> {
    encrypt_with_aad(key, plain.as_bytes(), CHAT_ITEM_AAD)
}

fn decrypt_item(key: &[u8; 32], cipher: &[u8]) -> Option<String> {
    let plaintext = decrypt_with_aad(key, cipher, CHAT_ITEM_AAD).ok()?;
    String::from_utf8(plaintext).ok()
}

pub fn encrypt_text(key: &[u8; 32], plain: &str) -> Result<Vec<u8>> {
    encrypt_with_aad(key, plain.as_bytes(), CHAT_AAD)
}

pub fn decrypt_text(key: &[u8; 32], cipher: &[u8]) -> Option<String> {
    let plaintext = decrypt_with_aad(key, cipher, CHAT_AAD).ok()?;
    String::from_utf8(plaintext).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_store_round_trips_encrypted_messages() {
        let path = test_db_path("round-trip");
        let store = ChatStore::open(&path).expect("store should open");
        let key = [7u8; 32];

        store
            .create_session("session-1", 10)
            .expect("session should be created");
        store
            .insert_message(
                &ChatMessageRecord {
                    id: "message-1".to_string(),
                    session_id: "session-1".to_string(),
                    role: ChatMessageRole::Assistant,
                    content: "hello".to_string(),
                    tool_name: Some("read".to_string()),
                    tool_summary: Some("read file".to_string()),
                    tool_status: Some("completed".to_string()),
                    sort_order: 0,
                    created_at: 11,
                    attachments: None,
                },
                &key,
            )
            .expect("message should be inserted");

        let messages = store
            .load_session_messages("session-1", &key)
            .expect("messages should load");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[0].tool_summary.as_deref(), Some("read file"));
        assert_eq!(messages[0].tool_status.as_deref(), Some("completed"));
        assert_eq!(messages[0].attachments, None);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn chat_store_uses_placeholder_for_tampered_content() {
        let path = test_db_path("tampered");
        let store = ChatStore::open(&path).expect("store should open");
        let key = [9u8; 32];

        store
            .create_session("session-1", 10)
            .expect("session should be created");
        store
            .insert_message(
                &ChatMessageRecord {
                    id: "message-1".to_string(),
                    session_id: "session-1".to_string(),
                    role: ChatMessageRole::User,
                    content: "secret".to_string(),
                    tool_name: None,
                    tool_summary: None,
                    tool_status: None,
                    sort_order: 0,
                    created_at: 11,
                    attachments: None,
                },
                &key,
            )
            .expect("message should be inserted");
        store
            .0
            .execute(
                "UPDATE chat_messages SET content = ?1 WHERE id = ?2",
                params![vec![1u8, 2, 3], "message-1"],
            )
            .expect("message should be tampered");

        let messages = store
            .load_session_messages("session-1", &key)
            .expect("tampered messages should still load");
        assert_eq!(messages[0].content, DECRYPT_FAILED_TEXT);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn chat_store_migrates_old_chat_messages_without_tool_status() {
        let path = test_db_path("migration");
        {
            let connection = Connection::open(&path).expect("old database should open");
            connection
                .execute_batch(
                    "CREATE TABLE chat_sessions (
                        id TEXT PRIMARY KEY,
                        title TEXT NOT NULL DEFAULT '',
                        created_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    );
                    CREATE TABLE chat_messages (
                        id TEXT PRIMARY KEY,
                        session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
                        role TEXT NOT NULL,
                        content BLOB NOT NULL,
                        tool_name TEXT,
                        tool_summary BLOB,
                        sort_order INTEGER NOT NULL,
                        created_at INTEGER NOT NULL
                    );",
                )
                .expect("old schema should be created");
        }

        let store = ChatStore::open(&path).expect("store should migrate");
        assert!(
            store
                .chat_messages_has_column("tool_status")
                .expect("columns should be readable")
        );

        let key = [3u8; 32];
        store
            .create_session("session-1", 10)
            .expect("session should be created");
        store
            .insert_message(
                &ChatMessageRecord {
                    id: "message-1".to_string(),
                    session_id: "session-1".to_string(),
                    role: ChatMessageRole::ToolCall,
                    content: "{\"path\":\"Cargo.toml\"}".to_string(),
                    tool_name: Some("read".to_string()),
                    tool_summary: Some("read Cargo.toml".to_string()),
                    tool_status: Some("completed".to_string()),
                    sort_order: 0,
                    created_at: 11,
                    attachments: None,
                },
                &key,
            )
            .expect("message should be inserted after migration");
        let messages = store
            .load_session_messages("session-1", &key)
            .expect("messages should load");
        assert_eq!(messages[0].tool_status.as_deref(), Some("completed"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn chat_store_round_trips_attachments() {
        let path = test_db_path("attachments-round-trip");
        let store = ChatStore::open(&path).expect("store should open");
        let key = [5u8; 32];

        store
            .create_session("session-1", 10)
            .expect("session should be created");
        let attachments_json = serde_json::json!([
            {
                "id": "att-1",
                "filename": "screenshot.png",
                "mime_type": "image/png",
                "size_bytes": 4096,
                "content": {
                    "Image": {
                        "data_base64": "iVBORw0KGgo=",
                        "thumbnail_base64": "iVBOR=",
                        "width": 800,
                        "height": 600
                    }
                }
            }
        ])
        .to_string();
        store
            .insert_message(
                &ChatMessageRecord {
                    id: "message-1".to_string(),
                    session_id: "session-1".to_string(),
                    role: ChatMessageRole::User,
                    content: "look at this".to_string(),
                    tool_name: None,
                    tool_summary: None,
                    tool_status: None,
                    sort_order: 0,
                    created_at: 11,
                    attachments: Some(attachments_json.clone()),
                },
                &key,
            )
            .expect("message with attachments should be inserted");

        let messages = store
            .load_session_messages("session-1", &key)
            .expect("messages should load");
        assert_eq!(messages.len(), 1);
        let restored = messages[0]
            .attachments
            .as_deref()
            .expect("attachments present");
        let restored_value: serde_json::Value =
            serde_json::from_str(restored).expect("attachments JSON parses");
        assert_eq!(
            restored_value[0]["filename"].as_str(),
            Some("screenshot.png")
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn chat_store_replaces_session_messages_transactionally() {
        let path = test_db_path("replace-session-messages");
        let store = ChatStore::open(&path).expect("store should open");
        let key = [6u8; 32];

        store
            .create_session("session-1", 10)
            .expect("session should be created");
        store
            .insert_message(
                &ChatMessageRecord {
                    id: "old-message".to_string(),
                    session_id: "session-1".to_string(),
                    role: ChatMessageRole::User,
                    content: "old".to_string(),
                    tool_name: None,
                    tool_summary: None,
                    tool_status: None,
                    sort_order: 0,
                    created_at: 11,
                    attachments: None,
                },
                &key,
            )
            .expect("old message should be inserted");

        let replacement = vec![ChatMessageRecord {
            id: "new-message".to_string(),
            session_id: "session-1".to_string(),
            role: ChatMessageRole::Assistant,
            content: "new".to_string(),
            tool_name: None,
            tool_summary: None,
            tool_status: None,
            sort_order: 0,
            created_at: 12,
            attachments: None,
        }];
        store
            .replace_session_messages("session-1", &replacement, &key)
            .expect("messages should be replaced");

        let messages = store
            .load_session_messages("session-1", &key)
            .expect("messages should load");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, "new-message");
        assert_eq!(messages[0].content, "new");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn chat_store_migrates_old_chat_messages_without_attachments() {
        let path = test_db_path("attachments-migration");
        {
            let connection = Connection::open(&path).expect("old database should open");
            connection
                .execute_batch(
                    "CREATE TABLE chat_sessions (
                        id TEXT PRIMARY KEY,
                        title TEXT NOT NULL DEFAULT '',
                        created_at INTEGER NOT NULL,
                        updated_at INTEGER NOT NULL
                    );
                    CREATE TABLE chat_messages (
                        id TEXT PRIMARY KEY,
                        session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
                        role TEXT NOT NULL,
                        content BLOB NOT NULL,
                        tool_name TEXT,
                        tool_summary BLOB,
                        sort_order INTEGER NOT NULL,
                        created_at INTEGER NOT NULL,
                        tool_status TEXT
                    );",
                )
                .expect("old schema with tool_status should be created");
        }

        let store = ChatStore::open(&path).expect("store should migrate");
        assert!(
            store
                .chat_messages_has_column("attachments")
                .expect("columns should be readable")
        );

        let key = [4u8; 32];
        store
            .create_session("session-1", 10)
            .expect("session should be created");
        store
            .insert_message(
                &ChatMessageRecord {
                    id: "message-1".to_string(),
                    session_id: "session-1".to_string(),
                    role: ChatMessageRole::User,
                    content: "plain text".to_string(),
                    tool_name: None,
                    tool_summary: None,
                    tool_status: None,
                    sort_order: 0,
                    created_at: 11,
                    attachments: None,
                },
                &key,
            )
            .expect("message should be inserted after migration");
        let messages = store
            .load_session_messages("session-1", &key)
            .expect("messages should load");
        assert_eq!(messages[0].attachments, None);

        let _ = std::fs::remove_file(path);
    }

    fn item(id: &str, seq: i64, payload: &str) -> ChatItemRecord {
        ChatItemRecord {
            id: id.into(),
            seq,
            turn_id: "turn".into(),
            kind: "user".into(),
            payload: payload.into(),
            created_at: 100 + seq,
        }
    }

    #[test]
    fn chat_items_append_replace_and_reload() {
        let path = test_db_path("items");
        let store = ChatStore::open(&path).expect("store should open");
        let key = [8u8; 32];
        store.create_session_v2("s", 1).expect("session");
        assert_eq!(store.session_schema_version("s").unwrap(), Some(2));
        assert_eq!(store.max_item_seq("s").unwrap(), -1);
        store
            .append_items("s", &[item("a", 0, "{\"x\":1}"), item("b", 1, "two")], &key)
            .expect("append");
        // Items are immutable: re-appending an existing id is rejected.
        assert!(
            store
                .append_items("s", &[item("b", 2, "two-again")], &key)
                .is_err()
        );
        let loaded = store.load_items("s", &key).expect("load");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[1].payload.as_deref(), Some("two"));
        assert_eq!(store.max_item_seq("s").unwrap(), 1);
        let wrong = store.load_items("s", &[1u8; 32]).expect("load");
        assert_eq!(wrong[0].payload, None);

        store
            .replace_items("s", &[item("c", 0, "only")], &key)
            .expect("replace");
        let loaded = store.load_items("s", &key).expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "c");

        store
            .save_session_config("s", "{\"mode\":\"standard\"}", &key)
            .unwrap();
        assert_eq!(
            store.load_session_config("s", &key).unwrap().as_deref(),
            Some("{\"mode\":\"standard\"}")
        );
        store.delete_session("s").unwrap();
        assert!(store.load_items("s", &key).unwrap().is_empty());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn legacy_sessions_report_schema_v1() {
        let path = test_db_path("schema-v1");
        let store = ChatStore::open(&path).expect("store should open");
        store.create_session("old", 1).unwrap();
        assert_eq!(store.session_schema_version("old").unwrap(), Some(1));
        let _ = std::fs::remove_file(path);
    }

    fn test_db_path(suffix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "miaominal-chat-store-{suffix}-{}.db",
            uuid::Uuid::new_v4()
        ))
    }
}
