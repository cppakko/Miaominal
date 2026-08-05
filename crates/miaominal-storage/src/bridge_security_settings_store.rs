use crate::settings_store::{
    SettingsFileLock, load_settings_document_unlocked, persist_settings_document_unlocked,
};
use anyhow::{Context, Result, anyhow};
use miaominal_core::ssh_bridge_security::{BridgeSecurityLevel, BridgeSecurityPolicy};
use std::path::{Path, PathBuf};

const SETTINGS_FILE_NAME: &str = "settings.toml";

#[derive(Clone)]
pub struct BridgeSecuritySettingsStore {
    path: PathBuf,
}

impl BridgeSecuritySettingsStore {
    pub fn open_default() -> Result<Self> {
        Self::open(&miaominal_paths::config_file(SETTINGS_FILE_NAME)?)
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn policy(&self) -> Result<BridgeSecurityPolicy> {
        let _lock = SettingsFileLock::acquire(&self.path)?;
        Ok(load_settings_document_unlocked(&self.path)?
            .ssh_bridge
            .security_policy)
    }

    pub fn set_policy(
        &self,
        level: BridgeSecurityLevel,
        updated_at: i64,
    ) -> Result<BridgeSecurityPolicy> {
        let level = level.validate().map_err(anyhow::Error::msg)?;
        let _lock = SettingsFileLock::acquire(&self.path)?;
        let mut settings = load_settings_document_unlocked(&self.path)?;
        let generation = settings
            .ssh_bridge
            .security_policy
            .generation
            .checked_add(1)
            .ok_or_else(|| anyhow!("SSH Bridge policy generation is exhausted"))?;
        let policy = BridgeSecurityPolicy {
            level,
            updated_at,
            generation,
        };
        settings.ssh_bridge.security_policy = policy.clone();
        persist_settings_document_unlocked(&self.path, &settings)?;
        Ok(policy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_policy_defaults_and_round_trips_in_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.toml");
        let store = BridgeSecuritySettingsStore::open(&path).unwrap();
        assert_eq!(store.policy().unwrap(), BridgeSecurityPolicy::default());

        let policy = store
            .set_policy(
                BridgeSecurityLevel::RequireApproval { timeout_secs: 30 },
                42,
            )
            .unwrap();
        assert_eq!(policy.generation, 1);
        assert_eq!(store.policy().unwrap(), policy);

        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("security_policy"));
        assert!(content.contains("require_approval"));
    }

    #[test]
    fn updates_from_multiple_store_instances_advance_generation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.toml");
        let first = BridgeSecuritySettingsStore::open(&path).unwrap();
        let second = BridgeSecuritySettingsStore::open(&path).unwrap();

        assert_eq!(
            first
                .set_policy(BridgeSecurityLevel::Standard, 1)
                .unwrap()
                .generation,
            1
        );
        assert_eq!(
            second
                .set_policy(BridgeSecurityLevel::RequireSystemAuth, 2)
                .unwrap()
                .generation,
            2
        );
    }

    #[test]
    fn invalid_approval_timeout_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let store =
            BridgeSecuritySettingsStore::open(&directory.path().join("settings.toml")).unwrap();
        let error = store
            .set_policy(BridgeSecurityLevel::RequireApproval { timeout_secs: 4 }, 42)
            .expect_err("out-of-range approval timeouts must not be accepted");
        assert!(error.to_string().contains("between 5 and 120"));
        assert_eq!(store.policy().unwrap(), BridgeSecurityPolicy::default());
    }
}
