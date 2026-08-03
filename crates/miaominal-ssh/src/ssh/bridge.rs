use anyhow::{Context, Result};
use miaominal_core::profile::SessionProfile;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

pub const SSH_BRIDGE_PROTOCOL_VERSION: u16 = 1;
pub const SSH_BRIDGE_MAX_CONTROL_FRAME: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshBridgeEndpoint {
    WindowsNamedPipe(String),
    UnixSocket(PathBuf),
}

impl SshBridgeEndpoint {
    pub fn derive(config_root: &Path) -> Result<Self> {
        let instance_id = bridge_instance_id(config_root)?;
        #[cfg(windows)]
        {
            Ok(Self::WindowsNamedPipe(format!(
                r"\\.\pipe\miaominal-ssh-bridge-{instance_id}"
            )))
        }
        #[cfg(unix)]
        {
            Ok(Self::UnixSocket(
                std::env::temp_dir()
                    .join("miaominal")
                    .join(instance_id)
                    .join("bridge.sock"),
            ))
        }
        #[cfg(not(any(windows, unix)))]
        {
            anyhow::bail!("SSH Bridge is unsupported on this platform")
        }
    }

    pub fn instance_id(config_root: &Path) -> Result<String> {
        bridge_instance_id(config_root)
    }

    pub fn helper_value(&self) -> String {
        match self {
            Self::WindowsNamedPipe(pipe) => pipe.replace('\\', "/"),
            Self::UnixSocket(path) => path.to_string_lossy().into_owned(),
        }
    }

    pub fn from_helper_value(value: &str) -> Result<Self> {
        if value.trim().is_empty() {
            anyhow::bail!("SSH Bridge endpoint is empty");
        }
        #[cfg(windows)]
        {
            let pipe = value.replace('/', "\\");
            if !pipe.starts_with(r"\\.\pipe\miaominal-ssh-bridge-") {
                anyhow::bail!("invalid Miaominal SSH Bridge named-pipe endpoint");
            }
            Ok(Self::WindowsNamedPipe(pipe))
        }
        #[cfg(unix)]
        {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                anyhow::bail!("SSH Bridge Unix socket endpoint must be absolute");
            }
            Ok(Self::UnixSocket(path))
        }
        #[cfg(not(any(windows, unix)))]
        {
            let _ = value;
            anyhow::bail!("SSH Bridge is unsupported on this platform")
        }
    }
}

impl std::fmt::Display for SshBridgeEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WindowsNamedPipe(pipe) => formatter.write_str(pipe),
            Self::UnixSocket(path) => write!(formatter, "{}", path.display()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshBridgeRoute {
    pub token: String,
    pub profile_id: String,
    pub profile_name: String,
}

impl SshBridgeRoute {
    pub fn derive(instance_id: &str, profile: &SessionProfile) -> Self {
        let mut digest = Sha256::new();
        digest.update(instance_id.as_bytes());
        digest.update([0]);
        digest.update(profile.id.as_bytes());
        let token = digest.finalize()[..16]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self {
            token,
            profile_id: profile.id.clone(),
            profile_name: profile.name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SshBridgeStatus {
    #[default]
    Disabled,
    Starting,
    Running {
        endpoint: SshBridgeEndpoint,
        exported_profile_count: usize,
        skipped_profile_count: usize,
        active_connection_count: usize,
        last_error: Option<String>,
    },
    Stopping,
    Error {
        endpoint: Option<SshBridgeEndpoint>,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshBridgeSyncResult {
    pub config_path: PathBuf,
    pub known_hosts_path: PathBuf,
    pub exported_profile_count: usize,
    pub skipped_profile_count: usize,
}

fn bridge_instance_id(config_root: &Path) -> Result<String> {
    let canonical = if config_root.exists() {
        fs::canonicalize(config_root)
            .with_context(|| format!("failed to canonicalize {}", config_root.display()))?
    } else if config_root.is_absolute() {
        config_root.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to determine current directory")?
            .join(config_root)
    };
    let normalized = canonical.to_string_lossy().replace('\\', "/");
    let digest = Sha256::digest(normalized.as_bytes());
    Ok(digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_and_route_tokens_are_stable_and_config_specific() {
        let root = tempfile::tempdir().expect("root");
        let other = tempfile::tempdir().expect("other");
        let endpoint_a = SshBridgeEndpoint::derive(root.path()).expect("endpoint");
        let endpoint_b = SshBridgeEndpoint::derive(root.path()).expect("endpoint");
        let endpoint_other = SshBridgeEndpoint::derive(other.path()).expect("endpoint");
        assert_eq!(endpoint_a, endpoint_b);
        assert_ne!(endpoint_a, endpoint_other);

        let profile = SessionProfile::blank("profile-a", 1);
        let instance_id = SshBridgeEndpoint::instance_id(root.path()).expect("instance id");
        let route_a = SshBridgeRoute::derive(&instance_id, &profile);
        let route_b = SshBridgeRoute::derive(&instance_id, &profile);
        assert_eq!(route_a, route_b);
        assert_eq!(route_a.token.len(), 32);

        let mut other_profile = profile.clone();
        other_profile.id = "profile-b".into();
        assert_ne!(
            route_a.token,
            SshBridgeRoute::derive(&instance_id, &other_profile).token
        );
    }
}
