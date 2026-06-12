use crate::backend::{BackendRoute, BackendRouter, SshExecRequest};
use crate::capabilities::RemoteCapabilities;
use crate::error::{AgentError, AgentResult};
use crate::jobs::{AgentJobId, AgentJobRegistry, AgentJobSummary, JobPollResult};
use crate::path_guard::resolve_workspace_path;
use crate::policy::{AgentPathAccess, AgentPolicy};
use crate::tools::{self, ListEntry};
use crate::web::{DisabledWebSearchProvider, WebFetchConfig};
use anyhow::anyhow;
use miaominal_core::profile::{AuthMethod, SessionProfile, ShellType};
use miaominal_secrets::SecretStore;
use miaominal_storage::known_hosts_store::KnownHostsStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolCallRequest {
    pub tool_name: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub route: Option<BackendRoute>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolCallResponse {
    pub tool_name: String,
    pub route: BackendRoute,
    pub output: ToolOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolOutput {
    WorkspaceInfo {
        host: String,
        user: String,
        platform: String,
        arch: String,
        shell: String,
        cwd: String,
        workspace_roots: Vec<String>,
        trusted_read_roots: Vec<String>,
        sensitive_paths: Vec<String>,
        capabilities: RemoteCapabilities,
        route: BackendRoute,
        supported_tools: Vec<String>,
    },
    Text {
        content: String,
        truncated: bool,
    },
    List {
        entries: Vec<String>,
        truncated: bool,
    },
    DirectoryList {
        path: String,
        entries: Vec<ListEntry>,
        truncated: bool,
    },
    Shell {
        result: ShellCommandResult,
    },
    JobStarted {
        job_id: AgentJobId,
        poll_after_ms: u64,
        next_action: String,
    },
    JobList {
        jobs: Vec<AgentJobSummary>,
    },
    JobPoll {
        result: JobPollResult,
    },
    WebSearch {
        results: Value,
    },
    WebFetch {
        url: String,
        content: String,
        truncated: bool,
    },
    Patch {
        summary: String,
        validation: Option<Box<ToolOutput>>,
    },
    Approval {
        message: String,
        operation_hash: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShellCommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_status: i32,
    pub timed_out: bool,
    pub truncated: bool,
}

#[derive(Clone)]
pub struct AgentExecChannel {
    profile: SessionProfile,
    all_profiles: Vec<SessionProfile>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    policy: AgentPolicy,
    backend_router: BackendRouter,
    jobs: AgentJobRegistry,
    web_search: Arc<DisabledWebSearchProvider>,
    web_fetch: WebFetchConfig,
}

impl AgentExecChannel {
    pub fn for_profile(
        profile: SessionProfile,
        all_profiles: Vec<SessionProfile>,
        secrets: SecretStore,
        known_hosts: KnownHostsStore,
    ) -> Self {
        Self::for_profile_with_jobs(
            profile,
            all_profiles,
            secrets,
            known_hosts,
            AgentJobRegistry::new(),
        )
    }

    pub fn for_profile_with_jobs(
        profile: SessionProfile,
        all_profiles: Vec<SessionProfile>,
        secrets: SecretStore,
        known_hosts: KnownHostsStore,
        jobs: AgentJobRegistry,
    ) -> Self {
        Self {
            profile,
            all_profiles,
            secrets,
            known_hosts,
            policy: AgentPolicy,
            backend_router: BackendRouter::new(),
            jobs,
            web_search: Arc::new(DisabledWebSearchProvider),
            web_fetch: WebFetchConfig::default(),
        }
    }

    pub fn profile_id(&self) -> &str {
        &self.profile.id
    }

    pub fn policy(&self) -> &AgentPolicy {
        &self.policy
    }

    pub fn jobs(&self) -> AgentJobRegistry {
        self.jobs.clone()
    }

    pub fn profile_name(&self) -> &str {
        &self.profile.name
    }

    pub fn shell_label(&self) -> &'static str {
        match self.profile.shell_type {
            ShellType::Posix => "posix-sh",
            ShellType::Fish => "fish",
            ShellType::PowerShell => "powershell",
            ShellType::Cmd => "cmd",
        }
    }

    pub fn web_search(&self) -> &DisabledWebSearchProvider {
        &self.web_search
    }

    pub fn web_fetch_config(&self) -> &WebFetchConfig {
        &self.web_fetch
    }

    pub async fn call_tool(
        &self,
        request: AgentToolCallRequest,
    ) -> AgentResult<AgentToolCallResponse> {
        log::info!(
            "agent tool call requested: tool={} approved={} arguments={}",
            request.tool_name,
            request.approved,
            request.arguments
        );
        self.policy.enforce(&request.tool_name, request.approved)?;
        self.enforce_context_policy(&request)?;
        let route = request.route.unwrap_or(BackendRoute::SshExec);
        self.backend_router.ensure_supported(route)?;
        self.ensure_posix_supported()?;

        let output = match request.tool_name.as_str() {
            "workspace_info" => tools::workspace_info(self).await?,
            "read" => tools::read(self, parse_args(request.arguments)?).await?,
            "list" => tools::list(self, parse_args(request.arguments)?).await?,
            "glob" => tools::glob(self, parse_args(request.arguments)?).await?,
            "grep" => tools::grep(self, parse_args(request.arguments)?).await?,
            "apply_patch" => tools::apply_patch(self, parse_args(request.arguments)?).await?,
            "run_shell" => tools::run_shell(self, parse_args(request.arguments)?).await?,
            "start_job" => tools::start_job(self, parse_args(request.arguments)?).await?,
            "list_jobs" => tools::list_jobs(self).await?,
            "poll_job" => tools::poll_job(self, parse_args(request.arguments)?).await?,
            "stop_job" => tools::stop_job(self, parse_args(request.arguments)?).await?,
            "web_search" => tools::web_search(self, parse_args(request.arguments)?).await?,
            "web_fetch" => tools::web_fetch(self, parse_args(request.arguments)?).await?,
            "ask_user" | "approval" => tools::approval(parse_args(request.arguments)?)?,
            other => return Err(AgentError::UnknownTool(other.to_string())),
        };

        Ok(AgentToolCallResponse {
            tool_name: request.tool_name,
            route,
            output,
        })
    }

    fn enforce_context_policy(&self, request: &AgentToolCallRequest) -> AgentResult<()> {
        match request.tool_name.as_str() {
            "read" | "list" => {
                if let Some(path) = string_arg(&request.arguments, "path") {
                    let path = resolve_workspace_path(path)?;
                    self.policy
                        .enforce_path(AgentPathAccess::Read, &path, request.approved)?;
                }
            }
            "glob" => {
                if let Some(root) = string_arg(&request.arguments, "root") {
                    let root = resolve_workspace_path(root)?;
                    self.policy
                        .enforce_path(AgentPathAccess::Read, &root, request.approved)?;
                }
            }
            "grep" => {
                if let Some(root) = string_arg(&request.arguments, "root") {
                    let root = resolve_workspace_path(root)?;
                    self.policy
                        .enforce_path(AgentPathAccess::Read, &root, request.approved)?;
                }
            }
            "apply_patch" => {
                let base_dir = string_arg(&request.arguments, "base_dir").unwrap_or(".");
                let base_dir = resolve_workspace_path(base_dir)?;
                self.policy
                    .enforce_path(AgentPathAccess::Edit, &base_dir, request.approved)?;
            }
            "run_shell" | "start_job" => {
                if let Some(cwd) = string_arg(&request.arguments, "cwd") {
                    let cwd = resolve_workspace_path(cwd)?;
                    self.policy
                        .enforce_path(AgentPathAccess::Read, &cwd, request.approved)?;
                }
                if let Some(command) = string_arg(&request.arguments, "command") {
                    self.policy.enforce_command(command, request.approved)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn ensure_posix_supported(&self) -> AgentResult<()> {
        if matches!(
            self.profile.effective_auth_method(),
            AuthMethod::KeyboardInteractive
        ) {
            return Err(AgentError::Backend(anyhow!(
                "keyboard-interactive authentication is not supported for agent exec channel"
            )));
        }
        if !matches!(self.profile.shell_type, ShellType::Posix | ShellType::Fish) {
            return Err(AgentError::PosixOnly(
                "agent exec channel v1 only supports POSIX-like remote shells".into(),
            ));
        }
        Ok(())
    }

    pub async fn exec(&self, command: impl Into<String>) -> AgentResult<String> {
        self.exec_via(BackendRoute::SshExec, command).await
    }

    pub async fn exec_via(
        &self,
        route: BackendRoute,
        command: impl Into<String>,
    ) -> AgentResult<String> {
        self.backend_router
            .exec(
                route,
                SshExecRequest {
                    profile: self.profile.clone(),
                    all_profiles: self.all_profiles.clone(),
                    secrets: self.secrets.clone(),
                    known_hosts: self.known_hosts.clone(),
                    command: command.into(),
                },
            )
            .await
    }
}

fn string_arg<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(Value::as_str)
}

fn parse_args<T>(arguments: Value) -> AgentResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(arguments)
        .map_err(|error| AgentError::InvalidArguments(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(shell_type: ShellType) -> SessionProfile {
        let mut profile = SessionProfile::blank("session-1", 1);
        profile.host = "example.com".into();
        profile.username = "akko".into();
        profile.shell_type = shell_type;
        profile
    }

    #[test]
    fn default_route_is_ssh_exec() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Posix),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-route")),
        );

        assert!(
            channel
                .backend_router
                .ensure_supported(BackendRoute::SshExec)
                .is_ok()
        );
    }

    #[test]
    fn unsupported_routes_return_typed_errors() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Posix),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-unsupported")),
        );

        for route in [BackendRoute::Sftp, BackendRoute::Pty, BackendRoute::Local] {
            assert!(matches!(
                channel.backend_router.ensure_supported(route),
                Err(AgentError::UnsupportedRoute(_))
            ));
        }
    }

    #[test]
    fn non_posix_profiles_are_rejected() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::PowerShell),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-posix")),
        );

        assert!(matches!(
            channel.ensure_posix_supported(),
            Err(AgentError::PosixOnly(_))
        ));
    }

    #[test]
    fn context_policy_denies_sensitive_read_paths() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Posix),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-policy-read")),
        );
        let request = AgentToolCallRequest {
            tool_name: "read".into(),
            arguments: serde_json::json!({ "path": ".env" }),
            approved: true,
            route: None,
        };

        assert!(matches!(
            channel.enforce_context_policy(&request),
            Err(AgentError::Denied { .. })
        ));
    }

    #[test]
    fn context_policy_requires_approval_for_service_restart() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Posix),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-policy-shell")),
        );
        let mut request = AgentToolCallRequest {
            tool_name: "run_shell".into(),
            arguments: serde_json::json!({ "command": "systemctl restart nginx", "cwd": "." }),
            approved: false,
            route: None,
        };

        assert!(matches!(
            channel.enforce_context_policy(&request),
            Err(AgentError::ApprovalRequired { .. })
        ));
        request.approved = true;
        assert!(channel.enforce_context_policy(&request).is_ok());
    }
}
