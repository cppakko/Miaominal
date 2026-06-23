use anyhow::{Context, Result, anyhow};
use miaominal_secrets::{decrypt_with_aad, encrypt_with_aad};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

const CHAT_AAD: &[u8] = b"miaominal.chat.v1";
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

        self.0
            .execute(
                "INSERT OR REPLACE INTO chat_messages
                 (id, session_id, role, content, tool_name, tool_summary, tool_status, sort_order, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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

    pub fn load_session_messages(
        &self,
        session_id: &str,
        key: &[u8; 32],
    ) -> Result<Vec<ChatMessageRecord>> {
        let mut statement = self
            .0
            .prepare(
                "SELECT id, session_id, role, content, tool_name, tool_summary, tool_status, sort_order, created_at
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

    fn test_db_path(suffix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "miaominal-chat-store-{suffix}-{}.db",
            uuid::Uuid::new_v4()
        ))
    }
}
