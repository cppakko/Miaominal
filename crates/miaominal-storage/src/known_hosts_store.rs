use anyhow::{Context, Result, anyhow};
use miaominal_core::known_host::{HostKeyCheck, KnownHostEntry};
use miaominal_paths as paths;
use russh::keys::known_hosts::{check_known_hosts_path, learn_known_hosts_path};
use russh::keys::{HashAlg, PublicKey};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct KnownHostsStore {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl KnownHostsStore {
    pub fn new() -> Result<Self> {
        Ok(Self::with_path(paths::config_file("known_hosts")?))
    }

    pub fn with_path(path: PathBuf) -> Self {
        Self {
            inner: Arc::new(Inner {
                path,
                write_lock: Mutex::new(()),
            }),
        }
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    pub fn check(&self, host: &str, port: u16, key: &PublicKey) -> Result<HostKeyCheck> {
        if !self.inner.path.exists() {
            return Ok(HostKeyCheck::Unknown);
        }
        match check_known_hosts_path(host, port, key, &self.inner.path) {
            Ok(true) => Ok(HostKeyCheck::Match),
            Ok(false) => Ok(HostKeyCheck::Unknown),
            Err(russh::keys::Error::KeyChanged { line }) => Ok(HostKeyCheck::Mismatch { line }),
            Err(error) => Err(anyhow!(error)).with_context(|| {
                format!(
                    "failed to read known_hosts at {}",
                    self.inner.path.display()
                )
            }),
        }
    }

    pub fn learn(&self, host: &str, port: u16, key: &PublicKey) -> Result<()> {
        let _guard = self.inner.write_lock.lock().ok();
        if let Some(parent) = self.inner.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        learn_known_hosts_path(host, port, key, &self.inner.path)
            .map_err(|error| anyhow!(error))
            .with_context(|| {
                format!(
                    "failed to write known_hosts at {}",
                    self.inner.path.display()
                )
            })
    }

    pub fn list(&self) -> Result<Vec<KnownHostEntry>> {
        if !self.inner.path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&self.inner.path)
            .with_context(|| format!("failed to read {}", self.inner.path.display()))?;

        let mut entries = Vec::new();
        for raw in content.lines() {
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            let Some(hosts) = parts.next() else { continue };
            let Some(_keytype) = parts.next() else {
                continue;
            };
            let Some(key_base64) = parts.next() else {
                continue;
            };

            let (host, port) = parse_host_field(hosts);
            let key = match russh::keys::parse_public_key_base64(key_base64) {
                Ok(key) => key,
                Err(_) => continue,
            };
            let algorithm = key.algorithm().to_string();
            let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();

            entries.push(KnownHostEntry {
                host,
                port,
                algorithm,
                fingerprint,
            });
        }
        Ok(entries)
    }

    pub fn remove(&self, target_host: &str, target_port: u16) -> Result<bool> {
        let _guard = self.inner.write_lock.lock().ok();
        if !self.inner.path.exists() {
            return Ok(false);
        }
        let content = fs::read_to_string(&self.inner.path)
            .with_context(|| format!("failed to read {}", self.inner.path.display()))?;

        let mut removed = false;
        let mut kept = String::with_capacity(content.len());
        for raw in content.lines() {
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                kept.push_str(raw);
                kept.push('\n');
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            let Some(hosts) = parts.next() else {
                kept.push_str(raw);
                kept.push('\n');
                continue;
            };
            let (host, port) = parse_host_field(hosts);
            if host == target_host && port == target_port {
                removed = true;
                continue;
            }
            kept.push_str(raw);
            kept.push('\n');
        }

        if removed {
            let mut file = fs::File::create(&self.inner.path)
                .with_context(|| format!("failed to rewrite {}", self.inner.path.display()))?;
            file.write_all(kept.as_bytes())?;
        }
        Ok(removed)
    }
}

fn parse_host_field(field: &str) -> (String, u16) {
    let primary = field.split(',').next().unwrap_or(field);
    if let Some(rest) = primary.strip_prefix('[')
        && let Some((host, port)) = rest.split_once("]:")
    {
        let port = port.parse().unwrap_or(22);
        return (host.to_string(), port);
    }
    (primary.to_string(), 22)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bracketed_host_field_parses_port() {
        assert_eq!(
            parse_host_field("[example.com]:2222"),
            ("example.com".into(), 2222)
        );
    }

    #[test]
    fn host_field_defaults_to_ssh_port() {
        assert_eq!(parse_host_field("example.com"), ("example.com".into(), 22));
    }
}
