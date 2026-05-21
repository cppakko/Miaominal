use anyhow::{Context, Result};
use miaominal_core::keychain::{ManagedKeyRecord, ManagedKeySource};
use miaominal_paths as paths;
use russh::keys::{self, Algorithm, PrivateKey, PublicKey};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ManagedKeysDocument {
    #[serde(default)]
    keys: Vec<ManagedKeyRecord>,
}

#[derive(Debug, Clone)]
pub struct ManagedKeyStore {
    keys_file: PathBuf,
}

impl ManagedKeyStore {
    pub fn new() -> Result<Self> {
        Ok(Self {
            keys_file: paths::config_file("managed_keys.toml")?,
        })
    }

    pub fn load(&self) -> Result<Vec<ManagedKeyRecord>> {
        if !self.keys_file.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&self.keys_file)
            .with_context(|| format!("failed to read {}", self.keys_file.display()))?;

        if content.trim().is_empty() {
            return Ok(Vec::new());
        }

        let document: ManagedKeysDocument = toml::from_str(&content)
            .with_context(|| format!("failed to parse {}", self.keys_file.display()))?;
        Ok(document.keys)
    }

    pub fn save(&self, keys: &[ManagedKeyRecord]) -> Result<()> {
        if let Some(parent) = self.keys_file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let content = toml::to_string_pretty(&ManagedKeysDocument {
            keys: keys.to_vec(),
        })
        .context("failed to serialize managed keys")?;

        fs::write(&self.keys_file, content)
            .with_context(|| format!("failed to write {}", self.keys_file.display()))?;

        Ok(())
    }

    pub fn next_key_id(&self, keys: &[ManagedKeyRecord]) -> String {
        let mut next = keys.len() + 1;
        loop {
            let candidate = format!("managed-key-{next}");
            if keys.iter().all(|key| key.id != candidate) {
                return candidate;
            }
            next += 1;
        }
    }

    pub fn generate_ed25519_material() -> Result<(String, String)> {
        let mut random = rand::rng();
        let key = PrivateKey::random(&mut random, Algorithm::Ed25519)
            .context("failed to generate Ed25519 key")?;
        let private_key_material = key
            .to_openssh(keys::ssh_key::LineEnding::LF)
            .context("failed to serialize private key")?
            .to_string();
        let public_key = key
            .public_key()
            .to_openssh()
            .context("failed to serialize public key")?;

        Ok((private_key_material, public_key))
    }

    pub fn import_private_key(
        &self,
        keys: &[ManagedKeyRecord],
        name: impl Into<String>,
        source: ManagedKeySource,
        private_key_material: &str,
        public_key_material: Option<&str>,
        passphrase: Option<&str>,
    ) -> Result<(ManagedKeyRecord, String)> {
        let key = keys::decode_secret_key(private_key_material, passphrase)
            .context("failed to parse imported private key")?;
        self.build_record(keys, name.into(), source, &key, public_key_material)
    }

    fn normalize_public_key(public_key_material: &str) -> Result<String> {
        PublicKey::from_openssh(public_key_material)
            .context("failed to parse imported public key")?
            .to_openssh()
            .context("failed to serialize public key")
    }

    fn build_record(
        &self,
        keys: &[ManagedKeyRecord],
        name: String,
        source: ManagedKeySource,
        key: &PrivateKey,
        public_key_material: Option<&str>,
    ) -> Result<(ManagedKeyRecord, String)> {
        let private_key_material = key
            .to_openssh(keys::ssh_key::LineEnding::LF)
            .context("failed to serialize private key")?
            .to_string();
        let public_key = match public_key_material.filter(|material| !material.trim().is_empty()) {
            Some(public_key_material) => Self::normalize_public_key(public_key_material)?,
            None => key
                .public_key()
                .to_openssh()
                .context("failed to serialize public key")?,
        };
        let algorithm = key.algorithm().to_string();
        let name = if name.trim().is_empty() {
            format!("{} key", algorithm)
        } else {
            name.trim().to_string()
        };

        Ok((
            ManagedKeyRecord {
                id: self.next_key_id(keys),
                name,
                algorithm,
                public_key,
                source,
            },
            private_key_material,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_key_id_starts_after_current_key_count() {
        let store = ManagedKeyStore {
            keys_file: PathBuf::new(),
        };
        let keys = vec![
            key("managed-key-1"),
            key("managed-key-2"),
            key("managed-key-4"),
        ];

        assert_eq!(store.next_key_id(&keys), "managed-key-5");
    }

    #[test]
    fn managed_keys_round_trip() {
        let key = key("managed-key-1");
        let content = toml::to_string_pretty(&ManagedKeysDocument {
            keys: vec![key.clone()],
        })
        .expect("managed key should serialize");
        let parsed: ManagedKeysDocument =
            toml::from_str(&content).expect("managed key should deserialize");

        assert_eq!(parsed.keys, vec![key]);
    }

    fn key(id: &str) -> ManagedKeyRecord {
        ManagedKeyRecord {
            id: id.to_string(),
            name: "Deploy".into(),
            algorithm: "ssh-ed25519".into(),
            public_key: "ssh-ed25519 AAAA".into(),
            source: ManagedKeySource::Generated,
        }
    }
}
