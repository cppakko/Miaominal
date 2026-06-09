use crate::error::{AgentError, AgentResult};
use miaominal_core::profile::SessionProfile;
use miaominal_secrets::SecretStore;
use miaominal_ssh as ssh;
use miaominal_storage::known_hosts_store::KnownHostsStore;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendRoute {
    SshExec,
    Sftp,
    Pty,
    Local,
}

impl BackendRoute {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SshExec => "ssh_exec",
            Self::Sftp => "sftp",
            Self::Pty => "pty",
            Self::Local => "local",
        }
    }
}

#[derive(Clone)]
pub struct SshExecRequest {
    pub profile: SessionProfile,
    pub all_profiles: Vec<SessionProfile>,
    pub secrets: SecretStore,
    pub known_hosts: KnownHostsStore,
    pub command: String,
}

#[derive(Debug, Clone, Default)]
pub struct SshExecBackend;

impl SshExecBackend {
    pub async fn execute(&self, request: SshExecRequest) -> AgentResult<String> {
        ssh::execute_profile_command(
            request.profile,
            request.all_profiles,
            request.secrets,
            request.known_hosts,
            request.command,
        )
        .await
        .map_err(AgentError::from)
    }
}

#[derive(Debug, Clone, Default)]
pub struct BackendRouter {
    ssh_exec: SshExecBackend,
}

impl BackendRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ensure_supported(&self, route: BackendRoute) -> AgentResult<()> {
        match route {
            BackendRoute::SshExec => Ok(()),
            other => Err(AgentError::UnsupportedRoute(other.as_str().into())),
        }
    }

    pub async fn exec(&self, route: BackendRoute, request: SshExecRequest) -> AgentResult<String> {
        self.ensure_supported(route)?;
        match route {
            BackendRoute::SshExec => self.ssh_exec.execute(request).await,
            other => Err(AgentError::UnsupportedRoute(other.as_str().into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_accepts_only_ssh_exec_in_v1() {
        let router = BackendRouter::new();

        assert!(router.ensure_supported(BackendRoute::SshExec).is_ok());
        for route in [BackendRoute::Sftp, BackendRoute::Pty, BackendRoute::Local] {
            assert!(matches!(
                router.ensure_supported(route),
                Err(AgentError::UnsupportedRoute(_))
            ));
        }
    }
}
