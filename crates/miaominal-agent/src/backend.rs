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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecMode {
    Raw,
    Pty { columns: u32, lines: u32 },
}

impl Default for ExecMode {
    fn default() -> Self {
        Self::Raw
    }
}

impl ExecMode {
    pub fn pty_default() -> Self {
        Self::Pty {
            columns: 120,
            lines: 40,
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
    pub mode: ExecMode,
}

#[derive(Debug, Clone, Default)]
pub struct SshBackend;

impl SshBackend {
    pub async fn execute(&self, request: SshExecRequest) -> AgentResult<String> {
        match request.mode {
            ExecMode::Raw => ssh::execute_profile_command(
                request.profile,
                request.all_profiles,
                request.secrets,
                request.known_hosts,
                request.command,
            )
            .await
            .map_err(AgentError::from),
            ExecMode::Pty { columns, lines } => ssh::execute_profile_pty_command(
                request.profile,
                request.all_profiles,
                request.secrets,
                request.known_hosts,
                request.command,
                columns,
                lines,
            )
            .await
            .map_err(AgentError::from),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct BackendRouter {
    ssh: SshBackend,
}

impl BackendRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ensure_supported(&self, route: BackendRoute) -> AgentResult<()> {
        match route {
            BackendRoute::SshExec | BackendRoute::Pty => Ok(()),
            other => Err(AgentError::UnsupportedRoute(other.as_str().into())),
        }
    }

    pub async fn exec(&self, route: BackendRoute, request: SshExecRequest) -> AgentResult<String> {
        self.ensure_supported(route)?;
        match route {
            BackendRoute::SshExec | BackendRoute::Pty => self.ssh.execute(request).await,
            other => Err(AgentError::UnsupportedRoute(other.as_str().into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_accepts_ssh_exec_and_pty_routes() {
        let router = BackendRouter::new();

        assert!(router.ensure_supported(BackendRoute::SshExec).is_ok());
        assert!(router.ensure_supported(BackendRoute::Pty).is_ok());
        for route in [BackendRoute::Sftp, BackendRoute::Local] {
            assert!(matches!(
                router.ensure_supported(route),
                Err(AgentError::UnsupportedRoute(_))
            ));
        }
    }
}
