use anyhow::{Context, Result, anyhow};
use miaominal_core::ssh_bridge_security::{BridgeSecurityLevel, BridgeSecurityPolicy};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DATABASE_FILE_NAME: &str = "ssh_bridge_security.db";

#[derive(Clone)]
pub struct BridgeSecurityStore {
    path: PathBuf,
    connection: Arc<Mutex<Connection>>,
}

impl BridgeSecurityStore {
    pub fn open_default() -> Result<Self> {
        Self::open(&miaominal_paths::config_file(DATABASE_FILE_NAME)?)
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let connection =
            Connection::open(path).with_context(|| format!("failed to open {}", path.display()))?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .context("failed to configure SSH Bridge security database busy timeout")?;
        let store = Self {
            path: path.to_path_buf(),
            connection: Arc::new(Mutex::new(connection)),
        };
        store.run_schema()?;
        store.secure_database_file()?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn policy(&self) -> Result<BridgeSecurityPolicy> {
        let connection = self.lock()?;
        let row = connection
            .query_row(
                "SELECT level, updated_at, generation
                 FROM bridge_security_policy
                 WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .context("failed to query SSH Bridge policy")?;
        row.map(|(level, updated_at, generation)| {
            Ok(BridgeSecurityPolicy {
                level: decode_security_level(&level)?,
                updated_at,
                generation: decode_generation(generation, "SSH Bridge policy generation")?,
            })
        })
        .transpose()
        .map(|policy| policy.unwrap_or_default())
    }

    pub fn set_policy(
        &self,
        level: BridgeSecurityLevel,
        updated_at: i64,
    ) -> Result<BridgeSecurityPolicy> {
        let level = level
            .validate()
            .map_err(anyhow::Error::msg)
            .and_then(|level| encode_json(&level, "SSH Bridge security level"))?;
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to start SSH Bridge policy update")?;
        let current_generation = transaction
            .query_row(
                "SELECT generation FROM bridge_security_policy WHERE id = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .context("failed to read SSH Bridge policy generation")?
            .unwrap_or(0);
        let generation = decode_generation(current_generation, "SSH Bridge policy generation")?
            .checked_add(1)
            .ok_or_else(|| anyhow!("SSH Bridge policy generation is exhausted"))?;
        let sqlite_generation = i64::try_from(generation)
            .context("SSH Bridge policy generation exceeds SQLite range")?;
        transaction
            .execute(
                "INSERT INTO bridge_security_policy (id, level, updated_at, generation)
                 VALUES (1, ?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET
                    level = excluded.level,
                    updated_at = excluded.updated_at,
                    generation = excluded.generation",
                params![level, updated_at, sqlite_generation],
            )
            .context("failed to persist SSH Bridge policy")?;
        transaction
            .commit()
            .context("failed to commit SSH Bridge policy update")?;
        drop(connection);
        self.secure_database_file()?;
        Ok(BridgeSecurityPolicy {
            level: decode_security_level(&level)?,
            updated_at,
            generation,
        })
    }

    fn run_schema(&self) -> Result<()> {
        let connection = self.lock()?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;
                 CREATE TABLE IF NOT EXISTS bridge_security_policy (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    level TEXT NOT NULL,
                    updated_at INTEGER NOT NULL,
                    generation INTEGER NOT NULL
                 );",
            )
            .context("failed to create SSH Bridge security schema")?;
        Ok(())
    }

    #[cfg(unix)]
    fn secure_database_file(&self) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        for path in database_files(&self.path) {
            match std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("failed to secure {}", path.display()));
                }
            }
        }
        Ok(())
    }

    #[cfg(not(unix))]
    fn secure_database_file(&self) -> Result<()> {
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow!("SSH Bridge security database lock is poisoned"))
    }
}

fn encode_json<T: serde::Serialize>(value: &T, label: &str) -> Result<String> {
    serde_json::to_string(value).with_context(|| format!("failed to encode {label}"))
}

fn decode_security_level(value: &str) -> Result<BridgeSecurityLevel> {
    serde_json::from_str::<BridgeSecurityLevel>(value)
        .context("failed to decode SSH Bridge security level")?
        .validate()
        .map_err(anyhow::Error::msg)
}

fn decode_generation(value: i64, label: &str) -> Result<u64> {
    u64::try_from(value).with_context(|| format!("{label} is negative"))
}

#[cfg(unix)]
fn database_files(path: &Path) -> [PathBuf; 3] {
    fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
        let mut value = path.as_os_str().to_os_string();
        value.push(suffix);
        value.into()
    }

    [
        path.to_path_buf(),
        with_suffix(path, "-wal"),
        with_suffix(path, "-shm"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::ssh_bridge_security::{BridgeSecurityLevel, BridgeSecurityPolicy};

    #[test]
    fn global_policy_defaults_and_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let store = BridgeSecurityStore::open(&directory.path().join("security.db")).unwrap();
        assert_eq!(store.policy().unwrap(), BridgeSecurityPolicy::default());
        let policy = store
            .set_policy(
                BridgeSecurityLevel::RequireApproval { timeout_secs: 30 },
                42,
            )
            .unwrap();
        assert_eq!(policy.generation, 1);
        assert_eq!(store.policy().unwrap(), policy);
    }

    #[test]
    fn every_policy_update_advances_the_generation() {
        let directory = tempfile::tempdir().unwrap();
        let store = BridgeSecurityStore::open(&directory.path().join("security.db")).unwrap();
        let first = store
            .set_policy(BridgeSecurityLevel::RequireApproval { timeout_secs: 30 }, 1)
            .unwrap();
        assert_eq!(first.generation, 1);
        let second = store
            .set_policy(BridgeSecurityLevel::RequireSystemAuth, 2)
            .unwrap();
        assert_eq!(second.generation, 2);
        assert_eq!(store.policy().unwrap(), second);
    }

    #[test]
    fn invalid_approval_timeout_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let store = BridgeSecurityStore::open(&directory.path().join("security.db")).unwrap();
        let error = store
            .set_policy(BridgeSecurityLevel::RequireApproval { timeout_secs: 4 }, 42)
            .expect_err("out-of-range approval timeouts must not be silently clamped");
        assert!(error.to_string().contains("between 5 and 120"));
        assert_eq!(store.policy().unwrap(), BridgeSecurityPolicy::default());
    }

    #[test]
    fn corrupt_stored_level_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("security.db");
        let store = BridgeSecurityStore::open(&path).unwrap();
        store
            .set_policy(BridgeSecurityLevel::RequireApproval { timeout_secs: 30 }, 1)
            .unwrap();
        {
            let connection = store.lock().unwrap();
            connection
                .execute(
                    "UPDATE bridge_security_policy SET level = 'corrupt' WHERE id = 1",
                    [],
                )
                .unwrap();
        }
        assert!(store.policy().is_err());
    }
}
