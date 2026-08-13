use anyhow::{Context, Result};
use miaominal_core::keychain::{ManagedKeyGenerationAlgorithm, ManagedKeyRecord, ManagedKeySource};
use miaominal_paths::{self as paths, atomic_write};
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

    #[doc(hidden)]
    pub fn with_path(keys_file: PathBuf) -> Self {
        Self { keys_file }
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
        let _sync_guard = miaominal_secrets::lock_sync_data();
        if let Some(parent) = self.keys_file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let content = toml::to_string_pretty(&ManagedKeysDocument {
            keys: keys.to_vec(),
        })
        .context("failed to serialize managed keys")?;

        atomic_write(&self.keys_file, content)?;

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

    pub fn generate_material(algorithm: ManagedKeyGenerationAlgorithm) -> Result<(String, String)> {
        let mut random = rand::rng();
        let key = match algorithm {
            ManagedKeyGenerationAlgorithm::Ed25519 => {
                PrivateKey::random(&mut random, Algorithm::Ed25519)
                    .context("failed to generate Ed25519 key")?
            }
            ManagedKeyGenerationAlgorithm::Rsa4096 => PrivateKey::from(
                keys::ssh_key::private::RsaKeypair::random(&mut random, 4096)
                    .context("failed to generate RSA-4096 key")?,
            ),
        };
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

    fn generated_rsa_4096_material() -> (String, String) {
        static MATERIAL: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
        MATERIAL
            .get_or_init(|| {
                ManagedKeyStore::generate_material(ManagedKeyGenerationAlgorithm::Rsa4096)
                    .expect("RSA-4096 key should generate")
            })
            .clone()
    }

    #[test]
    fn generated_ed25519_material_round_trips() {
        let (private_key, public_key) =
            ManagedKeyStore::generate_material(ManagedKeyGenerationAlgorithm::Ed25519)
                .expect("Ed25519 key should generate");
        let decoded = keys::decode_secret_key(&private_key, None)
            .expect("generated Ed25519 key should decode");

        assert_eq!(decoded.algorithm(), Algorithm::Ed25519);
        assert_eq!(
            decoded
                .public_key()
                .to_openssh()
                .expect("public key should serialize"),
            public_key
        );
    }

    #[test]
    fn generated_rsa_4096_material_round_trips() {
        let (private_key, public_key) = generated_rsa_4096_material();
        let decoded =
            keys::decode_secret_key(&private_key, None).expect("generated RSA key should decode");
        let rsa = decoded
            .key_data()
            .rsa()
            .expect("generated key should contain RSA material");

        assert_eq!(decoded.algorithm(), Algorithm::Rsa { hash: None });
        assert_eq!(rsa.key_size(), 4096);
        assert_eq!(
            decoded
                .public_key()
                .to_openssh()
                .expect("public key should serialize"),
            public_key
        );
    }

    #[test]
    fn encrypted_rsa_material_imports_and_normalizes() {
        let store = ManagedKeyStore::with_path(PathBuf::new());
        let (private_key, _) = generated_rsa_4096_material();
        let decoded =
            keys::decode_secret_key(&private_key, None).expect("generated RSA key should decode");
        let encrypted = decoded
            .encrypt(&mut rand::rng(), "test-passphrase")
            .expect("RSA key should encrypt")
            .to_openssh(keys::ssh_key::LineEnding::LF)
            .expect("encrypted RSA key should serialize")
            .to_string();

        let (record, normalized) = store
            .import_private_key(
                &[],
                "RSA deploy key",
                ManagedKeySource::Generated,
                &encrypted,
                None,
                Some("test-passphrase"),
            )
            .expect("encrypted RSA key should import");
        let normalized_key = keys::decode_secret_key(&normalized, None)
            .expect("normalized RSA key should decode without a passphrase");

        assert_eq!(record.algorithm, "ssh-rsa");
        assert_eq!(
            record.public_key,
            normalized_key.public_key().to_openssh().unwrap()
        );
        assert_eq!(
            normalized_key
                .key_data()
                .rsa()
                .expect("normalized key should be RSA")
                .key_size(),
            4096
        );
    }

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
