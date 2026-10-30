use anyhow::{Context, Result, anyhow};
use miaominal_core::known_host::{HostKeyCheck, KnownHostEntry};
use miaominal_paths as paths;
use russh::keys::known_hosts::{
    check_known_hosts_path, known_host_keys_path, learn_known_hosts_path,
};
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

    /// Trust `key` for `host:port`.
    ///
    /// Any recorded key of the same algorithm that differs from `key` is
    /// replaced. `check` reports a mismatch as soon as such a stale line
    /// exists, even when the new key is also present, so appending alone would
    /// make every later connection prompt again after a host was reinstalled.
    pub fn learn(&self, host: &str, port: u16, key: &PublicKey) -> Result<()> {
        let _guard = self.inner.write_lock.lock().ok();
        if let Some(parent) = self.inner.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let recorded = known_host_keys_path(host, port, &self.inner.path)
            .map_err(|error| anyhow!(error))
            .with_context(|| {
                format!(
                    "failed to read known_hosts at {}",
                    self.inner.path.display()
                )
            })?;
        let stale_lines: Vec<usize> = recorded
            .iter()
            .filter(|(_, recorded)| recorded.algorithm() == key.algorithm() && recorded != key)
            .map(|(line, _)| *line)
            .collect();
        let already_trusted = recorded.iter().any(|(_, recorded)| recorded == key);

        if !stale_lines.is_empty() {
            self.remove_known_hosts_lines(&stale_lines)?;
        }
        if already_trusted {
            return Ok(());
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

    /// Drop the given lines, numbered the way `russh::keys::known_hosts`
    /// numbers them: lines starting with `#` are not counted.
    fn remove_known_hosts_lines(&self, lines: &[usize]) -> Result<()> {
        let content = fs::read_to_string(&self.inner.path)
            .with_context(|| format!("failed to read {}", self.inner.path.display()))?;
        let kept = without_known_hosts_lines(&content, lines);
        let temporary = self.inner.path.with_extension("tmp");
        fs::write(&temporary, kept)
            .with_context(|| format!("failed to write {}", temporary.display()))?;
        fs::rename(&temporary, &self.inner.path)
            .with_context(|| format!("failed to replace {}", self.inner.path.display()))
    }
}

fn without_known_hosts_lines(content: &str, lines: &[usize]) -> String {
    let mut kept = String::with_capacity(content.len());
    let mut line = 1;
    for raw in content.split_inclusive('\n') {
        if raw.starts_with('#') {
            kept.push_str(raw);
            continue;
        }
        if !lines.contains(&line) {
            kept.push_str(raw);
        }
        line += 1;
    }
    kept
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

    fn random_key() -> PublicKey {
        russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)
            .expect("generate key")
            .public_key()
            .clone()
    }

    #[test]
    fn learning_a_changed_key_replaces_the_stale_one() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("known_hosts");
        let old = random_key();
        let new = random_key();
        let other = random_key();
        let store = KnownHostsStore::with_path(path.clone());
        fs::write(&path, "# comment\n").expect("seed known_hosts");
        store
            .learn("other.example", 22, &other)
            .expect("learn other");
        store.learn("vps.example", 2222, &old).expect("learn old");

        assert!(matches!(
            store.check("vps.example", 2222, &new).expect("check"),
            HostKeyCheck::Mismatch { .. }
        ));
        store.learn("vps.example", 2222, &new).expect("learn new");
        store
            .learn("vps.example", 2222, &new)
            .expect("learn new again");

        assert!(matches!(
            store.check("vps.example", 2222, &new).expect("check"),
            HostKeyCheck::Match
        ));
        assert!(matches!(
            store.check("other.example", 22, &other).expect("check"),
            HostKeyCheck::Match
        ));
        let content = fs::read_to_string(&path).expect("read");
        assert!(content.starts_with("# comment\n"));
        assert_eq!(store.list().expect("list").len(), 2);
    }

    #[test]
    fn line_removal_skips_comment_lines_like_russh() {
        let content = "# a\nhost1 k1\n#b\nhost2 k2\nhost3 k3";
        assert_eq!(
            without_known_hosts_lines(content, &[2, 3]),
            "# a\nhost1 k1\n#b\n"
        );
    }
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

    #[test]
    fn comma_separated_host_field_uses_primary_host() {
        assert_eq!(
            parse_host_field("example.com,192.0.2.1"),
            ("example.com".into(), 22)
        );
    }

    #[test]
    fn list_skips_invalid_public_keys() {
        let path = std::env::temp_dir().join(format!(
            "miaominal-known-hosts-invalid-{}",
            std::process::id()
        ));
        fs::write(
            &path,
            "example.com ssh-ed25519 not-a-public-key\n[valid.example.com]:2222 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n",
        )
        .expect("write test known_hosts");

        let store = KnownHostsStore::with_path(path.clone());
        let entries = store.list().expect("list known_hosts");

        let _ = fs::remove_file(path);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].host, "valid.example.com");
        assert_eq!(entries[0].port, 2222);
    }
}
