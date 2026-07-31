use anyhow::{Context, Result};
use miaominal_core::proxy::ProxyProfile;
use miaominal_paths::{self as paths, atomic_write};
use miaominal_secrets::{SecretKind, SecretStore};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProxiesDocument {
    #[serde(default)]
    pub proxies: Vec<ProxyProfile>,
}

#[derive(Debug, Clone)]
pub struct ProxyStore {
    proxies_file: PathBuf,
}

impl ProxyStore {
    pub fn new() -> Result<Self> {
        Ok(Self {
            proxies_file: paths::config_file("proxies.toml")?,
        })
    }

    #[doc(hidden)]
    pub fn with_path(proxies_file: PathBuf) -> Self {
        Self { proxies_file }
    }

    pub fn read_content(&self) -> Result<Option<String>> {
        if !self.proxies_file.exists() {
            return Ok(None);
        }
        fs::read_to_string(&self.proxies_file)
            .map(Some)
            .with_context(|| format!("failed to read {}", self.proxies_file.display()))
    }

    pub fn parse(&self, content: &str) -> Result<Vec<ProxyProfile>> {
        let document: ProxiesDocument = toml::from_str(content)
            .with_context(|| format!("failed to parse {}", self.proxies_file.display()))?;
        Ok(document.proxies)
    }

    pub fn load(&self, secrets: &SecretStore) -> Result<Vec<ProxyProfile>> {
        let mut proxies = self
            .read_content()?
            .map(|content| self.parse(&content))
            .transpose()?
            .unwrap_or_default();
        for proxy in &mut proxies {
            match secrets.get(&proxy.id, SecretKind::ProxyPassword) {
                Ok(password) => proxy.has_stored_password = password.is_some(),
                Err(error) if SecretStore::is_locked_error(&error) => {
                    // Keep the serialized last-known state while the vault is unavailable.
                    // Proxy metadata must remain usable and must never disappear merely because
                    // its separately stored credential cannot currently be inspected.
                }
                Err(error) => return Err(error),
            }
        }
        Ok(proxies)
    }

    pub fn save(&self, proxies: &[ProxyProfile]) -> Result<()> {
        if let Some(parent) = self.proxies_file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let content = toml::to_string_pretty(&ProxiesDocument {
            proxies: proxies.to_vec(),
        })
        .context("failed to serialize proxies")?;
        atomic_write(&self.proxies_file, content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::proxy::{ProxyAuthMode, ProxyProtocol};

    #[test]
    fn proxy_document_round_trips_without_password_material() {
        let proxy = ProxyProfile {
            id: "proxy-1".into(),
            name: "Local".into(),
            protocol: ProxyProtocol::HttpConnect,
            host: "127.0.0.1".into(),
            port: 8080,
            auth_mode: ProxyAuthMode::UsernamePassword,
            username: "akko".into(),
            resolve_dns_through_proxy: true,
            has_stored_password: true,
        };
        let content = toml::to_string_pretty(&ProxiesDocument {
            proxies: vec![proxy.clone()],
        })
        .expect("proxy should serialize");
        let parsed: ProxiesDocument = toml::from_str(&content).expect("proxy should parse");

        assert_eq!(parsed.proxies, vec![proxy]);
        assert!(
            content
                .lines()
                .all(|line| !line.trim_start().starts_with("password ="))
        );
    }

    #[test]
    fn locked_vault_keeps_proxy_metadata_and_last_known_password_state() {
        let path = std::env::temp_dir().join(format!(
            "miaominal-locked-proxy-store-{}.toml",
            uuid::Uuid::new_v4()
        ));
        let store = ProxyStore::with_path(path.clone());
        let mut proxy = ProxyProfile::blank("proxy-1", 1);
        proxy.name = "Locked vault proxy".into();
        proxy.host = "127.0.0.1".into();
        proxy.auth_mode = ProxyAuthMode::UsernamePassword;
        proxy.username = "akko".into();
        proxy.has_stored_password = true;
        store
            .save(std::slice::from_ref(&proxy))
            .expect("proxy metadata should save");

        let loaded = store
            .load(&SecretStore::new_locked_vault())
            .expect("locked vault must not hide proxy metadata");

        assert_eq!(loaded, vec![proxy]);
        let _ = std::fs::remove_file(path);
    }
}
