use crate::backend::{BackendRoute, BackendRouter, ExecMode, SshExecRequest};
use crate::capabilities::RemoteCapabilities;
use crate::error::{AgentError, AgentResult};
use crate::jobs::{AgentJobId, AgentJobRegistry, AgentJobSummary, JobPollResult};
use crate::path_guard::resolve_workspace_path;
use crate::policy::{AgentPathAccess, AgentPolicy};
use crate::tools::{self, ListEntry};
use crate::web::{
    ConfiguredWebSearchProvider, DisabledWebSearchProvider, WebFetchConfig, WebSearchProvider,
};
use anyhow::anyhow;
use miaominal_core::profile::{AuthMethod, SessionProfile, ShellType};
use miaominal_secrets::SecretStore;
use miaominal_storage::known_hosts_store::KnownHostsStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};

pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Handle for executing commands inside a user-visible terminal tab's PTY.
#[derive(Clone)]
pub struct TerminalExecHandle {
    pub command_sender: miaominal_ssh::SessionCommandSender,
    pub output_tap: Arc<Mutex<Option<mpsc::UnboundedReceiver<Vec<u8>>>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolCallRequest {
    pub tool_name: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub route: Option<BackendRoute>,
    #[serde(default)]
    pub skip_policy: bool,
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
    web_search: Arc<dyn WebSearchProvider>,
    web_search_configured: bool,
    web_fetch: WebFetchConfig,
    use_pty: bool,
    terminal_exec: Option<TerminalExecHandle>,
    aux_channels: HashMap<String, AgentExecChannel>,
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
            web_search_configured: false,
            web_fetch: WebFetchConfig::default(),
            use_pty: false,
            terminal_exec: None,
            aux_channels: HashMap::new(),
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

    pub fn shell_type(&self) -> ShellType {
        self.profile.shell_type
    }

    pub fn is_fish_shell(&self) -> bool {
        matches!(self.profile.shell_type, ShellType::Fish)
    }

    pub fn web_search(&self) -> &dyn WebSearchProvider {
        self.web_search.as_ref()
    }

    pub fn web_search_enabled(&self) -> bool {
        self.web_search_configured
    }

    pub fn web_fetch_config(&self) -> &WebFetchConfig {
        &self.web_fetch
    }

    pub fn with_web_search_config(
        mut self,
        config: miaominal_settings::WebSearchConfig,
        api_key: Option<String>,
    ) -> Self {
        self.web_search = Arc::new(ConfiguredWebSearchProvider::new(config, api_key));
        self.web_search_configured = true;
        self
    }

    pub fn with_pty(mut self) -> Self {
        self.use_pty = true;
        self
    }

    pub fn with_terminal_exec(mut self, handle: TerminalExecHandle) -> Self {
        self.terminal_exec = Some(handle);
        self.use_pty = true;
        self
    }

    pub fn with_aux_channels(mut self, channels: HashMap<String, AgentExecChannel>) -> Self {
        self.aux_channels = channels;
        self
    }

    pub fn uses_pty(&self) -> bool {
        self.use_pty
    }

    pub fn terminal_exec(&self) -> Option<&TerminalExecHandle> {
        self.terminal_exec.as_ref()
    }

    pub async fn call_tool(
        &self,
        request: AgentToolCallRequest,
    ) -> AgentResult<AgentToolCallResponse> {
        if let Some(target) = string_arg(&request.arguments, "target").map(str::to_string)
            && let Some(channel) = self.aux_channels.get(&target)
        {
            let mut routed_request = request;
            remove_arg(&mut routed_request.arguments, "target");
            return Box::pin(channel.call_tool(routed_request)).await;
        }

        log::info!(
            "agent tool call requested: tool={} approved={} arguments={}",
            request.tool_name,
            request.approved,
            request.arguments
        );
        if !request.skip_policy {
            self.policy.enforce(&request.tool_name, request.approved)?;
            self.enforce_context_policy(&request)?;
        }
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
        // All ShellType variants are now supported (Posix, Fish, PowerShell, Cmd).
        Ok(())
    }

    pub async fn exec(&self, command: impl Into<String>) -> AgentResult<String> {
        self.exec_with_mode(BackendRoute::SshExec, command, ExecMode::Raw)
            .await
    }

    pub async fn exec_via(
        &self,
        route: BackendRoute,
        command: impl Into<String>,
    ) -> AgentResult<String> {
        self.exec_with_mode(route, command, ExecMode::Raw).await
    }

    pub async fn exec_with_mode(
        &self,
        route: BackendRoute,
        command: impl Into<String>,
        mode: ExecMode,
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
                    mode,
                },
            )
            .await
    }

    /// Execute a command inside the user-visible terminal tab's PTY.
    ///
    /// WinkTerm-inspired approach: no stty-echo, no bracketed paste.
    /// The wrapper is sent as raw bytes + \r. The caller provides a unique
    /// sentinel string that the wrapper prints after the command finishes,
    /// along with the exit code and $PWD. Output is collected from the
    /// output tap until the sentinel is confirmed (followed by a digit).
    ///
    /// The sentinel appears twice in the tap: once as a literal inside the
    /// echoed wrapper text, and once as the actual evaluated output.
    /// `rfind` + digit-check discriminates the real occurrence.
    pub async fn exec_via_terminal(
        &self,
        command: impl Into<String>,
        sentinel: &str,
        timeout_secs: u64,
    ) -> AgentResult<String> {
        let handle = self
            .terminal_exec
            .as_ref()
            .ok_or_else(|| AgentError::Backend(anyhow!("terminal exec is not available")))?;

        // Send wrapper + Enter as raw bytes (no stty, no bracketed paste).
        let wrapper = format!("{}\r", command.into());
        handle
            .command_sender
            .send_bytes(wrapper.into_bytes())
            .map_err(AgentError::from)?;

        let mut collected = String::new();
        let sentinel_owned = sentinel.to_string();
        let done = {
            let mut guard = handle.output_tap.lock().await;
            let receiver = guard
                .as_mut()
                .ok_or_else(|| AgentError::Backend(anyhow!("terminal output tap is closed")))?;

            tokio::time::timeout(Duration::from_secs(timeout_secs.max(1)), async {
                while let Some(chunk) = receiver.recv().await {
                    collected.push_str(&String::from_utf8_lossy(&chunk));
                    // The sentinel string appears twice:
                    //   (a) Literal in echoed wrapper → followed by '%' (not a digit)
                    //   (b) Real output                     → followed by '0'..'9'
                    // rfind + digit-check picks the real one.
                    if let Some(pos) = collected.rfind(&sentinel_owned) {
                        let after = &collected[pos + sentinel_owned.len()..];
                        if after.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                            return;
                        }
                    }
                    if collected.len() > DEFAULT_MAX_OUTPUT_BYTES * 4 {
                        let drain_to = collected.len() - DEFAULT_MAX_OUTPUT_BYTES * 2;
                        collected.drain(..drain_to);
                    }
                }
            })
            .await
            .is_ok()
        };

        if !done {
            return Err(AgentError::Backend(anyhow!(
                "timed out waiting for terminal command completion"
            )));
        }

        Ok(collected)
    }
}

fn string_arg<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(Value::as_str)
}

fn remove_arg(arguments: &mut Value, key: &str) {
    if let Some(object) = arguments.as_object_mut() {
        object.remove(key);
    }
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

        for route in [BackendRoute::Sftp, BackendRoute::Local] {
            assert!(matches!(
                channel.backend_router.ensure_supported(route),
                Err(AgentError::UnsupportedRoute(_))
            ));
        }
    }

    #[test]
    fn non_posix_profiles_are_now_supported() {
        for shell_type in [ShellType::PowerShell, ShellType::Cmd] {
            let channel = AgentExecChannel::for_profile(
                profile(shell_type),
                Vec::new(),
                SecretStore::new_locked_vault(),
                KnownHostsStore::with_path(
                    std::env::temp_dir().join("agent-known-hosts-non-posix"),
                ),
            );

            assert!(
                channel.ensure_posix_supported().is_ok(),
                "{shell_type:?} should be supported by ensure_posix_supported",
            );
        }
    }

    #[test]
    fn powershell_profile_passes_ensure_supported() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::PowerShell),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(
                std::env::temp_dir().join("agent-known-hosts-powershell"),
            ),
        );

        assert!(
            channel.ensure_posix_supported().is_ok(),
            "PowerShell profile should be accepted by ensure_posix_supported",
        );
    }

    #[test]
    fn cmd_profile_passes_ensure_supported() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Cmd),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-cmd")),
        );

        assert!(
            channel.ensure_posix_supported().is_ok(),
            "Cmd profile should be accepted by ensure_posix_supported",
        );
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
            skip_policy: false,
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
            skip_policy: false,
        };

        assert!(matches!(
            channel.enforce_context_policy(&request),
            Err(AgentError::ApprovalRequired { .. })
        ));
        request.approved = true;
        assert!(channel.enforce_context_policy(&request).is_ok());
    }
}
