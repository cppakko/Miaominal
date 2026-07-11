use crate::backend::{BackendRoute, BackendRouter, ExecMode, SshExecRequest};
use crate::capabilities::RemoteCapabilities;
use crate::error::{AgentError, AgentResult};
use crate::jobs::{AgentJobId, AgentJobRegistry, AgentJobSummary, JobPollResult};
use crate::path_guard::{
    AuthorizedRemotePath, RemotePathKind, canonical_path_for_shell, resolve_workspace_path,
};
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
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};

pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024;
pub const TERMINAL_OUTPUT_TAP_CAPACITY: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalOutputTapError {
    Closed,
    Overflow,
}

#[derive(Clone)]
pub struct TerminalOutputTap {
    sender: Arc<StdMutex<Option<mpsc::Sender<Vec<u8>>>>>,
    overflowed: Arc<AtomicBool>,
}

pub struct TerminalOutputReceiver {
    receiver: mpsc::Receiver<Vec<u8>>,
    overflowed: Arc<AtomicBool>,
}

impl TerminalOutputTap {
    pub fn channel() -> (Self, TerminalOutputReceiver) {
        let (sender, receiver) = mpsc::channel(TERMINAL_OUTPUT_TAP_CAPACITY);
        let overflowed = Arc::new(AtomicBool::new(false));
        (
            Self {
                sender: Arc::new(StdMutex::new(Some(sender))),
                overflowed: overflowed.clone(),
            },
            TerminalOutputReceiver {
                receiver,
                overflowed,
            },
        )
    }

    pub fn try_send(&self, bytes: Vec<u8>) -> Result<(), TerminalOutputTapError> {
        let mut sender = self
            .sender
            .lock()
            .map_err(|_| TerminalOutputTapError::Closed)?;
        let Some(active) = sender.as_ref() else {
            return Err(TerminalOutputTapError::Closed);
        };
        match active.try_send(bytes) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.overflowed.store(true, Ordering::Release);
                sender.take();
                Err(TerminalOutputTapError::Overflow)
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                sender.take();
                Err(TerminalOutputTapError::Closed)
            }
        }
    }

    pub fn close(&self) {
        if let Ok(mut sender) = self.sender.lock() {
            sender.take();
        }
    }

    pub fn same_channel(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.sender, &other.sender)
    }
}

impl TerminalOutputReceiver {
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        self.receiver.recv().await
    }

    pub fn overflowed(&self) -> bool {
        self.overflowed.load(Ordering::Acquire)
    }
}

/// Handle for executing commands inside a user-visible terminal tab's PTY.
#[derive(Clone)]
pub struct TerminalExecHandle {
    pub command_sender: miaominal_ssh::SessionCommandSender,
    pub output_tap: Arc<Mutex<Option<TerminalOutputReceiver>>>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserQuestionChoice {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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
        #[serde(default, skip_serializing_if = "String::is_empty")]
        exec_shell: String,
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
    UserQuestion {
        message: String,
        choices: Vec<UserQuestionChoice>,
        allow_custom: bool,
        operation_hash: Option<String>,
    },
    UserResponse {
        answer: String,
        selected_index: Option<usize>,
        custom: bool,
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

/// Shared cache for the actual shell used by SSH exec channels.
///
/// `AgentExecChannel` instances are intentionally cheap to rebuild, but the
/// detected shell must survive those rebuilds. Entries are scoped to a
/// profile connection fingerprint so editing the destination, user, proxy
/// route, or configured shell starts with a fresh detection state.
#[derive(Clone, Default)]
pub struct AgentShellRegistry {
    entries: Arc<StdMutex<HashMap<String, AgentShellRegistryEntry>>>,
}

struct AgentShellRegistryEntry {
    connection_fingerprint: String,
    detected_shell: Arc<AtomicU8>,
}

impl AgentShellRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn detected_shell_for_profile(&self, profile: &SessionProfile) -> Arc<AtomicU8> {
        let fingerprint = shell_connection_fingerprint(profile);
        let Ok(mut entries) = self.entries.lock() else {
            return Arc::new(AtomicU8::new(0));
        };
        let entry = entries
            .entry(profile.id.clone())
            .or_insert_with(|| AgentShellRegistryEntry {
                connection_fingerprint: fingerprint.clone(),
                detected_shell: Arc::new(AtomicU8::new(0)),
            });
        if entry.connection_fingerprint != fingerprint {
            *entry = AgentShellRegistryEntry {
                connection_fingerprint: fingerprint,
                detected_shell: Arc::new(AtomicU8::new(0)),
            };
        }
        entry.detected_shell.clone()
    }
}

fn shell_connection_fingerprint(profile: &SessionProfile) -> String {
    let configured_shell = match profile.shell_type {
        ShellType::Posix => 1,
        ShellType::Fish => 2,
        ShellType::PowerShell => 3,
        ShellType::Cmd => 4,
    };
    format!(
        "{}\0{}\0{}\0{}\0{}",
        profile.host.trim().to_ascii_lowercase(),
        profile.port,
        profile.username,
        configured_shell,
        profile.proxy_jump_profile_ids.join("\0"),
    )
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
    policy_bypass: bool,
    /// Override set by workspace_info probe when the actual remote shell differs
    /// from the profile's configured shell_type.  `0` = unset (use profile).
    /// Uses AtomicU8 for lock-free interior mutability across clones.
    detected_shell: Arc<AtomicU8>,
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
        Self::for_profile_with_state(
            profile,
            all_profiles,
            secrets,
            known_hosts,
            jobs,
            Arc::new(AtomicU8::new(0)),
        )
    }

    pub fn for_profile_with_registries(
        profile: SessionProfile,
        all_profiles: Vec<SessionProfile>,
        secrets: SecretStore,
        known_hosts: KnownHostsStore,
        jobs: AgentJobRegistry,
        shells: AgentShellRegistry,
    ) -> Self {
        let detected_shell = shells.detected_shell_for_profile(&profile);
        Self::for_profile_with_state(
            profile,
            all_profiles,
            secrets,
            known_hosts,
            jobs,
            detected_shell,
        )
    }

    fn for_profile_with_state(
        profile: SessionProfile,
        all_profiles: Vec<SessionProfile>,
        secrets: SecretStore,
        known_hosts: KnownHostsStore,
        jobs: AgentJobRegistry,
        detected_shell: Arc<AtomicU8>,
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
            policy_bypass: false,
            detected_shell,
        }
    }

    pub fn profile_id(&self) -> &str {
        &self.profile.id
    }

    pub fn policy(&self) -> &AgentPolicy {
        &self.policy
    }

    pub fn policy_bypass_enabled(&self) -> bool {
        self.policy_bypass
    }

    pub fn jobs(&self) -> AgentJobRegistry {
        self.jobs.clone()
    }

    pub fn profile_name(&self) -> &str {
        &self.profile.name
    }

    pub fn shell_label(&self) -> &'static str {
        match self.effective_shell_type() {
            ShellType::Posix => "posix-sh",
            ShellType::Fish => "fish",
            ShellType::PowerShell => "powershell",
            ShellType::Cmd => "cmd",
        }
    }

    pub fn shell_type(&self) -> ShellType {
        self.effective_shell_type()
    }

    pub fn detected_shell_type(&self) -> Option<ShellType> {
        match self.detected_shell.load(Ordering::Relaxed) {
            1 => Some(ShellType::Posix),
            2 => Some(ShellType::Fish),
            3 => Some(ShellType::PowerShell),
            4 => Some(ShellType::Cmd),
            _ => None,
        }
    }

    fn effective_shell_type(&self) -> ShellType {
        self.detected_shell_type()
            .unwrap_or(self.profile.shell_type)
    }

    /// Record the actual shell type detected by workspace_info probing.
    /// Once set, all tools dispatch to this shell type instead of the profile's
    /// configured value.
    pub fn set_detected_shell(&self, shell_type: ShellType) {
        let code: u8 = match shell_type {
            ShellType::Posix => 1,
            ShellType::Fish => 2,
            ShellType::PowerShell => 3,
            ShellType::Cmd => 4,
        };
        self.detected_shell.store(code, Ordering::Relaxed);
    }

    pub fn is_fish_shell(&self) -> bool {
        matches!(self.effective_shell_type(), ShellType::Fish)
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

    pub fn with_policy_bypass(mut self, bypass: bool) -> Self {
        self.policy_bypass = bypass;
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

    pub(crate) async fn authorize_existing_path(
        &self,
        requested_path: &str,
        access: AgentPathAccess,
        expected_kind: RemotePathKind,
    ) -> AgentResult<AuthorizedRemotePath> {
        let normalized = resolve_workspace_path(requested_path)?;
        if self.policy_bypass_enabled() {
            return Ok(AuthorizedRemotePath::new(normalized));
        }

        self.policy.enforce_path(access, &normalized, true)?;
        let mut resolved = miaominal_sftp::resolve_profile_paths(
            self.profile.clone(),
            self.all_profiles.clone(),
            self.secrets.clone(),
            self.known_hosts.clone(),
            vec![normalized.clone()],
        )
        .await
        .map_err(|error| AgentError::Denied {
            tool_name: format!("{access:?}:{normalized}"),
            reason: format!("remote path could not be resolved safely: {error:#}"),
        })?;
        let resolved = resolved.pop().ok_or_else(|| AgentError::Denied {
            tool_name: format!("{access:?}:{normalized}"),
            reason: "remote path resolver returned no result".into(),
        })?;

        authorize_resolved_path(
            &self.policy,
            self.shell_type(),
            access,
            expected_kind,
            &normalized,
            resolved,
        )
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

        #[cfg(debug_assertions)]
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
        let bypass_channel = request
            .skip_policy
            .then(|| self.clone().with_policy_bypass(true));
        let channel = bypass_channel.as_ref().unwrap_or(self);

        let output = match request.tool_name.as_str() {
            "workspace_info" => tools::workspace_info(channel).await?,
            "read" => tools::read(channel, parse_args(request.arguments)?).await?,
            "list" => tools::list(channel, parse_args(request.arguments)?).await?,
            "glob" => tools::glob(channel, parse_args(request.arguments)?).await?,
            "grep" => tools::grep(channel, parse_args(request.arguments)?).await?,
            "apply_patch" => tools::apply_patch(channel, parse_args(request.arguments)?).await?,
            "run_shell" => tools::run_shell(channel, parse_args(request.arguments)?).await?,
            "start_job" => tools::start_job(channel, parse_args(request.arguments)?).await?,
            "list_jobs" => tools::list_jobs(channel).await?,
            "poll_job" => tools::poll_job(channel, parse_args(request.arguments)?).await?,
            "stop_job" => tools::stop_job(channel, parse_args(request.arguments)?).await?,
            "web_search" => tools::web_search(channel, parse_args(request.arguments)?).await?,
            "web_fetch" => {
                tools::web_fetch(channel, parse_args(request.arguments)?, request.approved).await?
            }
            "ask_user" => tools::ask_user(parse_args(request.arguments)?)?,
            "approval" => tools::approval(parse_args(request.arguments)?)?,
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
        let wait_result = {
            let mut guard = handle.output_tap.lock().await;
            let receiver = guard
                .as_mut()
                .ok_or_else(|| AgentError::Backend(anyhow!("terminal output tap is closed")))?;

            tokio::time::timeout(Duration::from_secs(timeout_secs.max(1)), async {
                loop {
                    let Some(chunk) = receiver.recv().await else {
                        let message = if receiver.overflowed() {
                            "terminal output tap overflowed before command completion"
                        } else {
                            "terminal output tap closed before command completion"
                        };
                        return Err(AgentError::Backend(anyhow!(message)));
                    };
                    collected.push_str(&String::from_utf8_lossy(&chunk));
                    // The sentinel string appears twice:
                    //   (a) Literal in echoed wrapper → followed by '%' (not a digit)
                    //   (b) Real output                     → followed by '0'..'9'
                    // rfind + digit-check picks the real one.
                    if let Some(pos) = collected.rfind(&sentinel_owned) {
                        let after = &collected[pos + sentinel_owned.len()..];
                        if after.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                            return Ok(());
                        }
                    }
                    if collected.len() > DEFAULT_MAX_OUTPUT_BYTES * 4 {
                        let drain_to = collected
                            .ceil_char_boundary(collected.len() - DEFAULT_MAX_OUTPUT_BYTES * 2);
                        collected.drain(..drain_to);
                    }
                }
            })
            .await
        };

        match wait_result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                return Err(AgentError::Backend(anyhow!(
                    "timed out waiting for terminal command completion"
                )));
            }
        }

        Ok(collected)
    }
}

fn authorize_resolved_path(
    policy: &AgentPolicy,
    shell_type: ShellType,
    access: AgentPathAccess,
    expected_kind: RemotePathKind,
    normalized_request: &str,
    resolved: miaominal_sftp::ResolvedRemotePath,
) -> AgentResult<AuthorizedRemotePath> {
    policy.enforce_path(access, &resolved.canonical_path, true)?;
    let kind_matches = match expected_kind {
        RemotePathKind::File => matches!(resolved.kind, miaominal_sftp::SftpEntryKind::File),
        RemotePathKind::Directory => {
            matches!(resolved.kind, miaominal_sftp::SftpEntryKind::Directory)
        }
    };
    if !kind_matches {
        return Err(AgentError::InvalidPath(format!(
            "`{normalized_request}` does not resolve to the required {expected_kind:?} path"
        )));
    }

    let execution_path = canonical_path_for_shell(&resolved.canonical_path, shell_type)?;
    Ok(AuthorizedRemotePath::new(execution_path))
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

    #[tokio::test]
    async fn terminal_output_tap_is_bounded_and_closes_on_overflow() {
        let (tap, mut receiver) = TerminalOutputTap::channel();
        for index in 0..TERMINAL_OUTPUT_TAP_CAPACITY {
            tap.try_send(vec![index as u8])
                .expect("tap should accept configured capacity");
        }
        assert_eq!(
            tap.try_send(vec![255]),
            Err(TerminalOutputTapError::Overflow)
        );
        assert!(receiver.overflowed());

        for expected in 0..TERMINAL_OUTPUT_TAP_CAPACITY {
            assert_eq!(receiver.recv().await, Some(vec![expected as u8]));
        }
        assert_eq!(receiver.recv().await, None);
    }

    #[tokio::test]
    async fn terminal_output_tap_clones_share_close_state() {
        let (tap, mut receiver) = TerminalOutputTap::channel();
        let clone = tap.clone();
        assert!(tap.same_channel(&clone));
        clone.close();
        assert_eq!(tap.try_send(vec![1]), Err(TerminalOutputTapError::Closed));
        assert_eq!(receiver.recv().await, None);
        assert!(!receiver.overflowed());
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
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-powershell")),
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
    fn detected_shell_overrides_profile_shell() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::PowerShell),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-detected-shell")),
        );

        assert_eq!(channel.detected_shell_type(), None);
        assert_eq!(channel.shell_label(), "powershell");

        channel.set_detected_shell(ShellType::Cmd);

        assert_eq!(channel.detected_shell_type(), Some(ShellType::Cmd));
        assert_eq!(channel.shell_label(), "cmd");
    }

    #[test]
    fn legacy_job_started_output_defaults_exec_shell() {
        let value = serde_json::json!({
            "kind": "job_started",
            "job_id": AgentJobId::new(),
            "poll_after_ms": 1000,
            "next_action": "poll"
        });
        let output: ToolOutput = serde_json::from_value(value).unwrap();

        match output {
            ToolOutput::JobStarted { exec_shell, .. } => assert!(exec_shell.is_empty()),
            other => panic!("unexpected output: {other:?}"),
        }
    }

    #[test]
    fn policy_bypass_is_explicit() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Posix),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-policy-bypass")),
        );

        assert!(!channel.policy_bypass_enabled());
        assert!(channel.with_policy_bypass(true).policy_bypass_enabled());
    }

    #[test]
    fn context_policy_denies_sensitive_read_paths() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Posix),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-known-hosts-policy-read")),
        );
        for path in [".env", ".ssh/id_rsa"] {
            let request = AgentToolCallRequest {
                tool_name: "read".into(),
                arguments: serde_json::json!({ "path": path }),
                approved: true,
                route: None,
                skip_policy: false,
            };

            assert!(matches!(
                channel.enforce_context_policy(&request),
                Err(AgentError::Denied { .. })
            ));
        }
    }

    #[test]
    fn canonical_sensitive_link_targets_are_denied() {
        let resolved = miaominal_sftp::ResolvedRemotePath {
            requested_path: "safe-link".into(),
            canonical_path: "/home/user/.ssh/id_rsa".into(),
            kind: miaominal_sftp::SftpEntryKind::File,
            is_symlink: true,
        };

        assert!(matches!(
            authorize_resolved_path(
                &AgentPolicy,
                ShellType::Posix,
                AgentPathAccess::Read,
                RemotePathKind::File,
                "safe-link",
                resolved,
            ),
            Err(AgentError::Denied { .. })
        ));
    }

    #[test]
    fn safe_canonical_link_targets_are_allowed_and_used() {
        let resolved = miaominal_sftp::ResolvedRemotePath {
            requested_path: "safe-link".into(),
            canonical_path: "/home/user/project/real.txt".into(),
            kind: miaominal_sftp::SftpEntryKind::File,
            is_symlink: true,
        };
        let authorized = authorize_resolved_path(
            &AgentPolicy,
            ShellType::Posix,
            AgentPathAccess::Read,
            RemotePathKind::File,
            "safe-link",
            resolved,
        )
        .unwrap();

        assert_eq!(authorized.as_str(), "/home/user/project/real.txt");
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

    #[test]
    fn terminal_output_compaction_handles_multibyte_boundaries() {
        let mut collected = "🚀".repeat(DEFAULT_MAX_OUTPUT_BYTES);
        collected.push('你');

        assert!(collected.len() > DEFAULT_MAX_OUTPUT_BYTES * 4);

        let target_boundary = collected.len() - DEFAULT_MAX_OUTPUT_BYTES * 2;
        assert!(!collected.is_char_boundary(target_boundary));

        let compacted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let drain_to = collected.ceil_char_boundary(target_boundary);
            collected.drain(..drain_to);
        }));

        assert!(compacted.is_ok());
        assert!(collected.len() <= DEFAULT_MAX_OUTPUT_BYTES * 2);
        assert!(collected.is_char_boundary(collected.len()));
        assert!(
            collected
                .chars()
                .all(|character| character == '🚀' || character == '你')
        );
    }
}
