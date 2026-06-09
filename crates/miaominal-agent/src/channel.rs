use crate::error::{AgentError, AgentResult};
use crate::jobs::{AgentJobId, AgentJobRegistry, JobPollResult};
use crate::path_guard::{resolve_workspace_path, shell_quote};
use crate::policy::AgentPolicy;
use crate::web::{DisabledWebSearchProvider, WebFetchConfig, WebSearchProvider};
use anyhow::anyhow;
use miaominal_core::profile::{AuthMethod, SessionProfile, ShellType};
use miaominal_secrets::SecretStore;
use miaominal_ssh as ssh;
use miaominal_storage::known_hosts_store::KnownHostsStore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendRoute {
    SshExec,
    Sftp,
    Pty,
    Local,
}

impl BackendRoute {
    fn as_str(self) -> &'static str {
        match self {
            Self::SshExec => "ssh_exec",
            Self::Sftp => "sftp",
            Self::Pty => "pty",
            Self::Local => "local",
        }
    }
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
        home: String,
        pwd: String,
        profile_summary: String,
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
    Shell {
        result: ShellCommandResult,
    },
    JobStarted {
        job_id: AgentJobId,
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
    Approval {
        message: String,
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

    pub async fn call_tool(
        &self,
        request: AgentToolCallRequest,
    ) -> AgentResult<AgentToolCallResponse> {
        self.policy.enforce(&request.tool_name, request.approved)?;
        let route = request.route.unwrap_or(BackendRoute::SshExec);
        self.ensure_route_supported(route)?;
        self.ensure_posix_supported()?;

        let output = match request.tool_name.as_str() {
            "workspace_info" => self.workspace_info().await?,
            "read" => self.read(request.arguments).await?,
            "list" => self.list(request.arguments).await?,
            "glob" => self.glob(request.arguments).await?,
            "grep" => self.grep(request.arguments).await?,
            "apply_patch" => self.apply_patch(request.arguments).await?,
            "run_shell" => self.run_shell(request.arguments).await?,
            "start_job" => self.start_job(request.arguments).await?,
            "poll_job" => self.poll_job(request.arguments).await?,
            "stop_job" => self.stop_job(request.arguments).await?,
            "web_search" => self.web_search(request.arguments).await?,
            "web_fetch" => self.web_fetch(request.arguments).await?,
            "ask_user" | "approval" => self.approval(request.arguments)?,
            other => return Err(AgentError::UnknownTool(other.to_string())),
        };

        Ok(AgentToolCallResponse {
            tool_name: request.tool_name,
            route,
            output,
        })
    }

    fn ensure_route_supported(&self, route: BackendRoute) -> AgentResult<()> {
        match route {
            BackendRoute::SshExec => Ok(()),
            other => Err(AgentError::UnsupportedRoute(other.as_str().into())),
        }
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

    async fn exec(&self, command: impl Into<String>) -> AgentResult<String> {
        ssh::execute_profile_command(
            self.profile.clone(),
            self.all_profiles.clone(),
            self.secrets.clone(),
            self.known_hosts.clone(),
            command.into(),
        )
        .await
        .map_err(AgentError::from)
    }

    async fn workspace_info(&self) -> AgentResult<ToolOutput> {
        let output = self
            .exec("printf 'home=%s\\npwd=%s\\n' \"$HOME\" \"$PWD\"")
            .await?;
        let mut home = String::new();
        let mut pwd = String::new();
        for line in output.lines() {
            if let Some(value) = line.strip_prefix("home=") {
                home = value.to_string();
            } else if let Some(value) = line.strip_prefix("pwd=") {
                pwd = value.to_string();
            }
        }
        Ok(ToolOutput::WorkspaceInfo {
            home,
            pwd,
            profile_summary: self.profile.summary(),
            route: BackendRoute::SshExec,
            supported_tools: crate::tools::TOOL_NAMES
                .iter()
                .map(|tool| (*tool).to_string())
                .collect(),
        })
    }

    async fn read(&self, arguments: Value) -> AgentResult<ToolOutput> {
        let args: PathArgs = parse_args(arguments)?;
        let path = resolve_workspace_path(&args.path)?;
        let max_bytes = args.max_bytes.unwrap_or(DEFAULT_MAX_OUTPUT_BYTES);
        let command = format!(
            "cd \"$HOME\" && if [ -f {path} ]; then bytes=$(wc -c < {path}); head -c {max} {path}; if [ \"$bytes\" -gt {max} ]; then printf '\\n[MIAOMINAL_TRUNCATED]'; fi; else printf 'not a regular file: %s' {path} >&2; exit 1; fi",
            path = shell_quote(&path),
            max = max_bytes,
        );
        let output = self.exec(command).await?;
        Ok(ToolOutput::Text {
            truncated: output.contains("[MIAOMINAL_TRUNCATED]"),
            content: output.replace("\n[MIAOMINAL_TRUNCATED]", ""),
        })
    }

    async fn list(&self, arguments: Value) -> AgentResult<ToolOutput> {
        let args: PathArgs = parse_args(arguments)?;
        let path = resolve_workspace_path(&args.path)?;
        let max_entries = args.max_entries.unwrap_or(200);
        let command = format!(
            "cd \"$HOME\" && find {path} -maxdepth 1 -mindepth 1 -printf '%f\\n' | sort | head -n {max}",
            path = shell_quote(&path),
            max = max_entries + 1,
        );
        let entries = self.exec(command).await?;
        let mut entries: Vec<String> = entries.lines().map(str::to_string).collect();
        let truncated = entries.len() > max_entries;
        entries.truncate(max_entries);
        Ok(ToolOutput::List { entries, truncated })
    }

    async fn glob(&self, arguments: Value) -> AgentResult<ToolOutput> {
        let args: GlobArgs = parse_args(arguments)?;
        let pattern = resolve_workspace_path(&args.pattern)?;
        let max_entries = args.max_entries.unwrap_or(200);
        let command = format!(
            "cd \"$HOME\" && find . -path {pattern} -print | sed 's#^./##' | sort | head -n {max}",
            pattern = shell_quote(&pattern),
            max = max_entries + 1,
        );
        let entries = self.exec(command).await?;
        let mut entries: Vec<String> = entries.lines().map(str::to_string).collect();
        let truncated = entries.len() > max_entries;
        entries.truncate(max_entries);
        Ok(ToolOutput::List { entries, truncated })
    }

    async fn grep(&self, arguments: Value) -> AgentResult<ToolOutput> {
        let args: GrepArgs = parse_args(arguments)?;
        let path = resolve_workspace_path(args.path.as_deref().unwrap_or("."))?;
        let max_bytes = args.max_bytes.unwrap_or(DEFAULT_MAX_OUTPUT_BYTES);
        let command = format!(
            "cd \"$HOME\" && grep -RIn -- {pattern} {path} 2>/dev/null | head -c {max}",
            pattern = shell_quote(&args.pattern),
            path = shell_quote(&path),
            max = max_bytes,
        );
        Ok(ToolOutput::Text {
            content: self.exec(command).await?,
            truncated: false,
        })
    }

    async fn apply_patch(&self, arguments: Value) -> AgentResult<ToolOutput> {
        let args: PatchArgs = parse_args(arguments)?;
        let command = format!(
            "cd \"$HOME\" && patch -p0 <<'MIAOMINAL_AGENT_PATCH'\n{}\nMIAOMINAL_AGENT_PATCH",
            args.patch
        );
        Ok(ToolOutput::Text {
            content: self.exec(command).await?,
            truncated: false,
        })
    }

    async fn run_shell(&self, arguments: Value) -> AgentResult<ToolOutput> {
        let args: ShellArgs = parse_args(arguments)?;
        let timeout = args.timeout_ms.unwrap_or(30_000);
        let timeout_secs = (timeout.max(1) + 999) / 1000;
        let command = format!(
            concat!(
                "cd \"$HOME\" && ",
                "out=$(mktemp) && err=$(mktemp) && ",
                "timeout {timeout_secs} sh -lc {user_command} >\"$out\" 2>\"$err\"; ",
                "status=$?; ",
                "printf 'MIAOMINAL_STATUS=%s\\n' \"$status\"; ",
                "printf 'MIAOMINAL_STDOUT_BEGIN\\n'; head -c {max} \"$out\"; ",
                "printf '\\nMIAOMINAL_STDOUT_END\\n'; ",
                "printf 'MIAOMINAL_STDERR_BEGIN\\n'; head -c {max} \"$err\"; ",
                "printf '\\nMIAOMINAL_STDERR_END\\n'; ",
                "stdout_bytes=$(wc -c <\"$out\"); stderr_bytes=$(wc -c <\"$err\"); ",
                "rm -f \"$out\" \"$err\"; ",
                "if [ \"$stdout_bytes\" -gt {max} ] || [ \"$stderr_bytes\" -gt {max} ]; then ",
                "printf 'MIAOMINAL_TRUNCATED=1\\n'; ",
                "else printf 'MIAOMINAL_TRUNCATED=0\\n'; fi"
            ),
            timeout_secs = timeout_secs,
            user_command = shell_quote(&args.command),
            max = DEFAULT_MAX_OUTPUT_BYTES,
        );
        let output = self.exec(command).await?;
        let result = parse_shell_result(&output)?;
        Ok(ToolOutput::Shell { result })
    }

    async fn start_job(&self, arguments: Value) -> AgentResult<ToolOutput> {
        let args: ShellArgs = parse_args(arguments)?;
        let marker = format!("/tmp/miaominal-agent-{}.status", uuid::Uuid::new_v4());
        let command = format!(
            "cd \"$HOME\" && nohup sh -lc {} >{}.out 2>{}.err; printf $? >{}",
            shell_quote(&args.command),
            shell_quote(&marker),
            shell_quote(&marker),
            shell_quote(&marker),
        );
        let launch = format!(
            "({command}) >/dev/null 2>&1 & printf '%s' {marker}",
            marker = shell_quote(&marker)
        );
        let marker = self.exec(launch).await?.trim_matches('\'').to_string();
        let job_id = self.jobs.insert_remote_job(args.command, marker);
        Ok(ToolOutput::JobStarted { job_id })
    }

    async fn poll_job(&self, arguments: Value) -> AgentResult<ToolOutput> {
        let args: JobArgs = parse_args(arguments)?;
        let marker = self.jobs.remote_marker(&args.job_id)?;
        let command = format!(
            "if [ -f {status} ]; then printf 'status=exited\\nexit='; cat {status}; printf '\\nstdout<<EOF\\n'; cat {out} 2>/dev/null; printf '\\nEOF\\nstderr<<EOF\\n'; cat {err} 2>/dev/null; printf '\\nEOF\\n'; else printf 'status=running\\n'; fi",
            status = shell_quote(&marker),
            out = shell_quote(&format!("{marker}.out")),
            err = shell_quote(&format!("{marker}.err")),
        );
        Ok(ToolOutput::Text {
            content: self.exec(command).await?,
            truncated: false,
        })
    }

    async fn stop_job(&self, arguments: Value) -> AgentResult<ToolOutput> {
        let args: JobArgs = parse_args(arguments)?;
        let marker = self.jobs.remote_marker(&args.job_id)?;
        let command = format!(
            "pkill -f {marker} 2>/dev/null || true; printf 'stopped\\n'",
            marker = shell_quote(&marker),
        );
        self.jobs.remove(&args.job_id)?;
        Ok(ToolOutput::Text {
            content: self.exec(command).await?,
            truncated: false,
        })
    }

    async fn web_search(&self, arguments: Value) -> AgentResult<ToolOutput> {
        let args: SearchArgs = parse_args(arguments)?;
        let results = self.web_search.search(&args.query).await?;
        Ok(ToolOutput::WebSearch {
            results: json!(results),
        })
    }

    async fn web_fetch(&self, arguments: Value) -> AgentResult<ToolOutput> {
        let args: FetchArgs = parse_args(arguments)?;
        let text = reqwest::get(&args.url)
            .await
            .map_err(anyhow::Error::from)?
            .text()
            .await
            .map_err(anyhow::Error::from)?;
        let max = args.max_bytes.unwrap_or(self.web_fetch.max_bytes);
        let truncated = text.len() > max;
        let content = if truncated {
            text.chars().take(max).collect()
        } else {
            text
        };
        Ok(ToolOutput::WebFetch {
            url: args.url,
            content,
            truncated,
        })
    }

    fn approval(&self, arguments: Value) -> AgentResult<ToolOutput> {
        let message = arguments
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("approval requested")
            .to_string();
        Ok(ToolOutput::Approval { message })
    }
}

fn parse_args<T>(arguments: Value) -> AgentResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(arguments)
        .map_err(|error| AgentError::InvalidArguments(error.to_string()))
}

#[derive(Debug, Deserialize)]
struct PathArgs {
    #[serde(default = "default_dot")]
    path: String,
    max_bytes: Option<usize>,
    max_entries: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct GlobArgs {
    pattern: String,
    max_entries: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct GrepArgs {
    pattern: String,
    path: Option<String>,
    max_bytes: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct PatchArgs {
    patch: String,
}

#[derive(Debug, Deserialize)]
struct ShellArgs {
    command: String,
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct JobArgs {
    job_id: AgentJobId,
}

#[derive(Debug, Deserialize)]
struct SearchArgs {
    query: String,
}

#[derive(Debug, Deserialize)]
struct FetchArgs {
    url: String,
    max_bytes: Option<usize>,
}

fn default_dot() -> String {
    ".".into()
}

fn parse_shell_result(output: &str) -> AgentResult<ShellCommandResult> {
    let exit_status = output
        .lines()
        .find_map(|line| line.strip_prefix("MIAOMINAL_STATUS="))
        .and_then(|status| status.parse::<i32>().ok())
        .ok_or_else(|| AgentError::Backend(anyhow!("missing shell exit status")))?;
    let stdout = extract_section(output, "MIAOMINAL_STDOUT_BEGIN\n", "\nMIAOMINAL_STDOUT_END")
        .unwrap_or_default();
    let stderr = extract_section(output, "MIAOMINAL_STDERR_BEGIN\n", "\nMIAOMINAL_STDERR_END")
        .unwrap_or_default();
    let truncated = output.contains("MIAOMINAL_TRUNCATED=1");

    Ok(ShellCommandResult {
        stdout,
        stderr,
        exit_status,
        timed_out: exit_status == 124,
        truncated,
    })
}

fn extract_section(output: &str, start: &str, end: &str) -> Option<String> {
    let (_, rest) = output.split_once(start)?;
    let (section, _) = rest.split_once(end)?;
    Some(section.to_string())
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
                .ensure_route_supported(BackendRoute::SshExec)
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
                channel.ensure_route_supported(route),
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
    fn shell_result_parser_extracts_status_streams_and_truncation() {
        let output = concat!(
            "MIAOMINAL_STATUS=7\n",
            "MIAOMINAL_STDOUT_BEGIN\n",
            "hello\n",
            "MIAOMINAL_STDOUT_END\n",
            "MIAOMINAL_STDERR_BEGIN\n",
            "oops\n",
            "MIAOMINAL_STDERR_END\n",
            "MIAOMINAL_TRUNCATED=1\n"
        );

        let result = parse_shell_result(output).unwrap();

        assert_eq!(result.exit_status, 7);
        assert_eq!(result.stdout, "hello");
        assert_eq!(result.stderr, "oops");
        assert!(result.truncated);
    }
}
