//! [`ToolRuntime`] over [`AgentExecChannel`]: maps tool set v2 onto the
//! existing remote execution layer and shapes results for the model.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures::future::BoxFuture;
use miaominal_core::profile::ShellType;
use serde_json::{Map, Value, json};

use super::edit::{new_file_diff, replace_exact, split_parent, unified_diff};
use super::shape::{MODEL_OUTPUT_LIMIT, clean_output, head_tail, number_lines, shell_model_text};
use super::terminal::{TerminalContextSource, TerminalReadRequest};
use super::{
    ProgressSender, ToolAssessment, ToolInvocation, ToolOutcome, ToolRuntime, ToolSetOptions,
    tool_class, tool_specs,
};
use crate::channel::{AgentExecChannel, AgentToolCallRequest, ToolOutput};
use crate::context::TargetInfo;
use crate::error::{AgentError, AgentResult};
use crate::model::ToolSpec;
use crate::path_guard::shell_quote;
use crate::policy::{
    AgentPathAccess, AgentRiskLevel, ShellDialect, assess_command, is_sensitive_path,
    is_system_path,
};
use crate::tools::{
    AgentToolCancellation, ListEntryType, ensure_directory, read_whole_file,
    run_cancellable_tool_worker,
};
use crate::transcript::{ToolDisplay, ToolResultStatus};

const READ_DEFAULT_LINES: usize = 400;
const READ_MAX_BYTES: usize = 64 * 1024;
const JOB_CHUNK_BYTES: u64 = 64 * 1024;

/// Tool runtime backed by the session's execution channel(s).
#[derive(Clone)]
pub struct ChannelToolRuntime {
    inner: Arc<Inner>,
}

struct Inner {
    channel: Option<AgentExecChannel>,
    /// Name of the primary target (without `@`).
    default_target: String,
    terminal: Option<Arc<dyn TerminalContextSource>>,
    /// job_id → (target, bytes already returned).
    jobs: Mutex<HashMap<String, (Option<String>, u64)>>,
}

impl ChannelToolRuntime {
    pub fn new(
        channel: Option<AgentExecChannel>,
        default_target: impl Into<String>,
        terminal: Option<Arc<dyn TerminalContextSource>>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                channel,
                default_target: default_target.into().trim_start_matches('@').to_string(),
                terminal,
                jobs: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Basic facts for `<environment>` from the configured channels.
    pub fn target_infos(&self) -> Vec<TargetInfo> {
        let Some(channel) = &self.inner.channel else {
            return Vec::new();
        };
        let mut targets = vec![target_info(&self.inner.default_target, channel, true)];
        for name in channel.aux_target_names() {
            if let Ok(aux) = channel.target_channel(Some(&name)) {
                targets.push(target_info(name.trim_start_matches('@'), aux, false));
            }
        }
        targets
    }
}

fn target_info(name: &str, channel: &AgentExecChannel, is_default: bool) -> TargetInfo {
    let (host, user) = channel.host_and_user();
    let shell = channel.shell_type();
    TargetInfo {
        name: name.to_string(),
        is_default,
        host,
        user,
        os: match shell {
            ShellType::PowerShell | ShellType::Cmd => "windows".into(),
            _ => String::new(),
        },
        exec_shell: shell_name(shell).into(),
        cwd: String::new(),
        pty: channel.uses_pty(),
        notes: Vec::new(),
    }
}

fn shell_name(shell: ShellType) -> &'static str {
    match shell {
        ShellType::Posix => "posix-sh",
        ShellType::Fish => "fish",
        ShellType::PowerShell => "powershell",
        ShellType::Cmd => "cmd",
    }
}

fn dialect(shell: ShellType) -> ShellDialect {
    match shell {
        ShellType::PowerShell => ShellDialect::PowerShell,
        ShellType::Cmd => ShellDialect::Cmd,
        ShellType::Posix | ShellType::Fish => ShellDialect::Posix,
    }
}

fn str_arg<'a>(arguments: &'a Value, key: &str) -> &'a str {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
}

impl Inner {
    /// `(target for routing, display name)`. Routing `None` = primary.
    fn target(&self, arguments: &Value) -> (Option<String>, String) {
        let requested = arguments
            .get("target")
            .and_then(Value::as_str)
            .map(|target| target.trim().trim_start_matches('@').to_string())
            .filter(|target| !target.is_empty());
        match requested {
            Some(name) if name != self.default_target => (Some(name.clone()), name),
            _ => (None, self.default_target.clone()),
        }
    }

    fn channel_for(&self, target: Option<&str>) -> Result<&AgentExecChannel, String> {
        let channel = self.channel.as_ref().ok_or_else(|| {
            "No execution target is connected; open an SSH session first.".to_string()
        })?;
        channel.target_channel(target).map_err(|error| {
            let mut names = vec![format!("@{}", self.default_target)];
            names.extend(channel.aux_target_names());
            format!("{error}. Available targets: {}.", names.join(", "))
        })
    }
}

fn path_risk(path: &str, write: bool) -> (AgentRiskLevel, String) {
    if is_sensitive_path(path) {
        return (
            AgentRiskLevel::L4Forbidden,
            format!("`{path}` is on the sensitive path denylist"),
        );
    }
    if !write {
        return (AgentRiskLevel::L0ReadOnly, "read-only".into());
    }
    if is_system_path(path) {
        (
            AgentRiskLevel::L2ServiceImpacting,
            format!("modifies system file `{path}`"),
        )
    } else {
        (AgentRiskLevel::L1LowMutation, format!("modifies `{path}`"))
    }
}

fn join_path(base: &str, relative: &str) -> String {
    if relative.starts_with('/') || base.is_empty() || base == "." {
        relative.to_string()
    } else {
        format!("{}/{}", base.trim_end_matches(['/', '\\']), relative)
    }
}

fn patch_paths(patch: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in patch.lines() {
        if let Some(rest) = line
            .strip_prefix("+++ ")
            .or_else(|| line.strip_prefix("--- "))
        {
            let path = rest.split('\t').next().unwrap_or("").trim();
            let path = path
                .strip_prefix("a/")
                .or_else(|| path.strip_prefix("b/"))
                .unwrap_or(path);
            if !path.is_empty() && path != "/dev/null" && !paths.iter().any(|p| p == path) {
                paths.push(path.to_string());
            }
        }
    }
    paths
}

impl ToolRuntime for ChannelToolRuntime {
    fn specs(&self, options: &ToolSetOptions) -> Vec<ToolSpec> {
        let mut options = options.clone();
        if self.inner.channel.is_none() {
            options.has_targets = false;
        }
        if self.inner.terminal.is_none() {
            options.terminal = false;
        }
        tool_specs(&options)
    }

    fn assess<'a>(
        &'a self,
        call: &'a ToolInvocation,
    ) -> BoxFuture<'a, Result<ToolAssessment, String>> {
        Box::pin(async move { self.assess_sync(call) })
    }

    fn apply_edit(&self, call: &ToolInvocation, edited: &str) -> Value {
        let mut arguments = call.arguments.clone();
        let key = match call.name.as_str() {
            "apply_patch" => "patch",
            "write_file" => "content",
            _ => "command",
        };
        if let Some(object) = arguments.as_object_mut() {
            object.insert(key.into(), Value::String(edited.to_string()));
        }
        arguments
    }

    fn execute(
        &self,
        call: ToolInvocation,
        cancel: AgentToolCancellation,
        _progress: ProgressSender,
    ) -> BoxFuture<'static, ToolOutcome> {
        let inner = self.inner.clone();
        Box::pin(async move {
            if call.name == "terminal_read" {
                return terminal_read(&inner, &call.arguments);
            }
            let worker_cancel = cancel.clone();
            let result = run_cancellable_tool_worker(
                worker_cancel,
                move || async move { execute_on_channel(&inner, call).await },
                None,
            )
            .await;
            match result {
                Ok(outcome) => outcome,
                Err(error) => outcome_from_error(error),
            }
        })
    }

    fn environment(&self) -> BoxFuture<'_, Option<Vec<TargetInfo>>> {
        Box::pin(async move {
            let channel = self.inner.channel.as_ref()?;
            let mut targets = Vec::new();
            let mut entries = vec![(self.inner.default_target.clone(), channel.clone(), true)];
            for name in channel.aux_target_names() {
                if let Ok(aux) = channel.target_channel(Some(&name)) {
                    entries.push((name.trim_start_matches('@').to_string(), aux.clone(), false));
                }
            }
            for (name, target_channel, is_default) in entries {
                let base = target_info(&name, &target_channel, is_default);
                targets.push(probe_target(base, target_channel).await);
            }
            Some(targets)
        })
    }
}

/// Probe results are reused for a while so every turn does not pay an SSH
/// round trip. Keyed by profile id and target name.
const ENVIRONMENT_TTL: std::time::Duration = std::time::Duration::from_secs(600);
const ENVIRONMENT_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

fn environment_cache() -> &'static Mutex<HashMap<String, (Instant, TargetInfo)>> {
    static CACHE: std::sync::OnceLock<Mutex<HashMap<String, (Instant, TargetInfo)>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn probe_target(base: TargetInfo, channel: AgentExecChannel) -> TargetInfo {
    let key = format!("{}|{}", channel.profile_id(), base.name);
    if let Some((at, cached)) = environment_cache()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&key)
        .cloned()
        && at.elapsed() < ENVIRONMENT_TTL
    {
        return TargetInfo {
            is_default: base.is_default,
            pty: base.pty,
            ..cached
        };
    }
    let probe = run_cancellable_tool_worker(
        AgentToolCancellation::new(),
        move || async move { call_channel(&channel, "workspace_info", json!({})).await },
        None,
    );
    let output = match tokio::time::timeout(ENVIRONMENT_PROBE_TIMEOUT, probe).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            log::info!("environment probe for {} failed: {error}", base.name);
            return base;
        }
        Err(_) => return base,
    };
    let info = target_info_from_probe(base, output);
    environment_cache()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(key, (Instant::now(), info.clone()));
    info
}

fn target_info_from_probe(mut base: TargetInfo, output: ToolOutput) -> TargetInfo {
    let ToolOutput::WorkspaceInfo {
        host,
        user,
        platform,
        arch,
        shell,
        cwd,
        capabilities,
        ..
    } = output
    else {
        return base;
    };
    if !host.trim().is_empty() {
        base.host = host;
    }
    if !user.trim().is_empty() {
        base.user = user;
    }
    let os = format!("{} {}", platform.trim(), arch.trim());
    if !os.trim().is_empty() {
        base.os = os.trim().to_string();
    }
    if !shell.trim().is_empty() {
        base.exec_shell = shell;
    }
    base.cwd = cwd;
    let yes_no = |value: bool| if value { "yes" } else { "no" };
    base.notes = vec![format!(
        "tools: rg={} git={} python3={} patch={} sed={}",
        yes_no(capabilities.rg),
        yes_no(capabilities.git),
        yes_no(capabilities.python3),
        yes_no(capabilities.patch),
        yes_no(capabilities.sed),
    )];
    base
}

impl ChannelToolRuntime {
    fn assess_sync(&self, call: &ToolInvocation) -> Result<ToolAssessment, String> {
        let class =
            tool_class(&call.name).ok_or_else(|| format!("unknown tool `{}`", call.name))?;
        let arguments = &call.arguments;
        let (route, target) = self.inner.target(arguments);
        let mut assessment = ToolAssessment {
            class,
            target: target.clone(),
            risk: AgentRiskLevel::L0ReadOnly,
            reason: "read-only".into(),
            grant_prefix: None,
            summary: format!("{} @{target}", call.name),
            editable: None,
        };
        let needs_channel = !matches!(
            call.name.as_str(),
            "web_search" | "web_fetch" | "terminal_read" | "ask_user" | "update_plan"
        );
        let channel = if needs_channel {
            Some(self.inner.channel_for(route.as_deref())?)
        } else {
            None
        };

        match call.name.as_str() {
            "shell" | "job_start" => {
                let command = str_arg(arguments, "command");
                let shell = channel.map(|c| c.shell_type()).unwrap_or(ShellType::Posix);
                let result = assess_command(command, dialect(shell));
                assessment.risk = result.risk;
                assessment.reason = if result.uses_sudo && result.risk < AgentRiskLevel::L4Forbidden
                {
                    format!("{} (sudo cannot prompt for a password here)", result.reason)
                } else {
                    result.reason
                };
                assessment.grant_prefix = result.grant_prefix;
                assessment.summary = format!("{} @{target}: {command}", call.name);
                assessment.editable = Some(command.to_string());
                if call.name == "job_start"
                    && matches!(shell, ShellType::PowerShell | ShellType::Cmd)
                {
                    return Err("job_start is only available on POSIX targets; use shell with a timeout instead.".into());
                }
            }
            "job_stop" => {
                assessment.risk = AgentRiskLevel::L1LowMutation;
                assessment.reason = "stops a background job".into();
                assessment.grant_prefix = Some("job_stop".into());
            }
            "edit_file" | "write_file" => {
                let path = str_arg(arguments, "path");
                let (risk, reason) = path_risk(path, true);
                assessment.risk = risk;
                assessment.reason = reason;
                assessment.summary = format!("{} @{target}: {path}", call.name);
                assessment.grant_prefix = Some(format!("{} {path}", call.name));
                if call.name == "write_file" {
                    assessment.editable = Some(str_arg(arguments, "content").to_string());
                }
            }
            "apply_patch" => {
                let base = str_arg(arguments, "base_dir");
                let patch = str_arg(arguments, "patch");
                let mut risk = AgentRiskLevel::L1LowMutation;
                let mut reason = format!("applies a patch in `{base}`");
                for path in patch_paths(patch) {
                    let (path_risk, path_reason) = path_risk(&join_path(base, &path), true);
                    if path_risk > risk {
                        risk = path_risk;
                        reason = path_reason;
                    }
                }
                assessment.risk = risk;
                assessment.reason = reason;
                assessment.summary = format!("apply_patch @{target}: {base}");
                assessment.editable = Some(patch.to_string());
            }
            "read_file" | "list_dir" => {
                let (risk, reason) = path_risk(str_arg(arguments, "path"), false);
                assessment.risk = risk;
                assessment.reason = reason;
                assessment.summary =
                    format!("{} @{target}: {}", call.name, str_arg(arguments, "path"));
            }
            "glob" | "grep" => {
                let (risk, reason) = path_risk(str_arg(arguments, "root"), false);
                assessment.risk = risk;
                assessment.reason = reason;
            }
            _ => {}
        }
        Ok(assessment)
    }
}

fn outcome_from_error(error: AgentError) -> ToolOutcome {
    match error {
        AgentError::Cancelled => ToolOutcome::cancelled(),
        AgentError::Denied { reason, .. } => ToolOutcome {
            status: ToolResultStatus::Denied {
                by_user: false,
                reason: Some(reason.clone()),
            },
            model_text: format!("Denied by policy: {reason}. Choose another approach."),
            display: ToolDisplay::None,
        },
        AgentError::InvalidArguments(message) => {
            ToolOutcome::failed(format!("Invalid arguments: {message}"))
        }
        AgentError::InvalidPath(message) => ToolOutcome::failed(format!(
            "Invalid path: {message}. Check the path with list_dir or glob."
        )),
        AgentError::PosixOnly(message) => ToolOutcome::failed(message),
        other => {
            let message = other.to_string();
            let hint = if message.contains("timed out") || message.contains("timeout") {
                " For long-running work use job_start."
            } else if message.contains("connect") || message.contains("session") {
                " The connection may have dropped; retry once, then tell the user."
            } else {
                ""
            };
            ToolOutcome::failed(format!("Failed: {message}.{hint}"))
        }
    }
}

async fn call_channel(
    channel: &AgentExecChannel,
    tool: &str,
    arguments: Value,
) -> AgentResult<ToolOutput> {
    channel
        .call_tool(AgentToolCallRequest {
            tool_name: tool.into(),
            arguments,
            approved: true,
            route: None,
            skip_policy: false,
        })
        .await
        .map(|response| response.output)
}

fn object(pairs: Vec<(&str, Value)>) -> Value {
    let mut map = Map::new();
    for (key, value) in pairs {
        if !value.is_null() {
            map.insert(key.into(), value);
        }
    }
    Value::Object(map)
}

async fn execute_on_channel(inner: &Inner, call: ToolInvocation) -> AgentResult<ToolOutcome> {
    let arguments = &call.arguments;
    let (route, target) = inner.target(arguments);
    if matches!(call.name.as_str(), "web_search" | "web_fetch") {
        let Some(channel) = inner.channel.as_ref() else {
            return Ok(ToolOutcome::failed(
                "Web tools need an open session in this version.",
            ));
        };
        return web_tool(channel, &call).await;
    }
    let channel = match inner.channel_for(route.as_deref()) {
        Ok(channel) => channel,
        Err(message) => return Ok(ToolOutcome::failed(message)),
    };

    match call.name.as_str() {
        "shell" => {
            let command = str_arg(arguments, "command").to_string();
            if channel.uses_pty()
                && inner
                    .terminal
                    .as_ref()
                    .and_then(|terminal| terminal.snapshot(Some(&target), 0))
                    .is_some_and(|snapshot| snapshot.busy)
            {
                return Ok(ToolOutcome::failed(
                    "Failed: terminal busy — a program is running in the user's terminal. Wait, or ask the user to switch this target to the exec channel.",
                ));
            }
            let started = Instant::now();
            let output = call_channel(
                channel,
                "run_shell",
                object(vec![
                    ("command", json!(command)),
                    ("cwd", arguments.get("cwd").cloned().unwrap_or(Value::Null)),
                    (
                        "timeout_seconds",
                        json!(
                            arguments
                                .get("timeout_s")
                                .and_then(Value::as_u64)
                                .unwrap_or(60)
                        ),
                    ),
                    ("max_bytes", json!(512 * 1024)),
                ]),
            )
            .await?;
            let ToolOutput::Shell { result } = output else {
                return Ok(ToolOutcome::failed("Unexpected shell output."));
            };
            let model_text = shell_model_text(
                Some(result.exit_status),
                &result.stdout,
                &result.stderr,
                result.timed_out,
                result.truncated,
            );
            Ok(ToolOutcome {
                status: if result.timed_out {
                    ToolResultStatus::TimedOut
                } else {
                    ToolResultStatus::Ok
                },
                model_text,
                display: ToolDisplay::Shell {
                    target: Some(target),
                    command,
                    exit_code: (!result.timed_out).then_some(result.exit_status),
                    stdout: clean_output(&result.stdout),
                    stderr: clean_output(&result.stderr),
                    timed_out: result.timed_out,
                    truncated: result.truncated,
                    duration_ms: started.elapsed().as_millis() as u64,
                },
            })
        }
        "read_file" => {
            let path = str_arg(arguments, "path").to_string();
            let offset = arguments
                .get("offset")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .max(1) as usize;
            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .map(|limit| limit as usize)
                .unwrap_or(READ_DEFAULT_LINES)
                .clamp(1, 2000);
            let output = call_channel(
                channel,
                "read",
                json!({
                    "path": path,
                    "start_line": offset,
                    "end_line": offset + limit - 1,
                    "max_bytes": READ_MAX_BYTES,
                }),
            )
            .await?;
            let ToolOutput::Text { content, truncated } = output else {
                return Ok(ToolOutcome::failed("Unexpected read output."));
            };
            let line_count = content.lines().count();
            let mut model_text = if content.is_empty() {
                if offset > 1 {
                    format!("(no lines at offset {offset}; the file is shorter)")
                } else {
                    "(empty file)".to_string()
                }
            } else {
                number_lines(&content, offset)
            };
            if truncated {
                model_text.push_str(&format!(
                    "[truncated at {} KB; read a smaller range with offset/limit]\n",
                    READ_MAX_BYTES / 1024
                ));
            } else if line_count >= limit {
                model_text.push_str(&format!(
                    "[more lines may follow; continue with offset={}]\n",
                    offset + line_count
                ));
            }
            Ok(ToolOutcome::ok(
                model_text.trim_end().to_string(),
                ToolDisplay::File {
                    path,
                    content,
                    start_line: offset,
                    truncated,
                },
            ))
        }
        "list_dir" => {
            let path = str_arg(arguments, "path").to_string();
            let output = call_channel(
                channel,
                "list",
                json!({
                    "path": path,
                    "include_hidden": arguments.get("include_hidden").and_then(Value::as_bool).unwrap_or(false),
                }),
            )
            .await?;
            let ToolOutput::DirectoryList {
                path: listed,
                entries,
                truncated,
            } = output
            else {
                return Ok(ToolOutcome::failed("Unexpected list output."));
            };
            let lines: Vec<String> = entries
                .iter()
                .map(|entry| match entry.entry_type {
                    ListEntryType::Directory => format!("{}/", entry.name),
                    ListEntryType::Symlink => format!("{}@", entry.name),
                    _ => match entry.size {
                        Some(size) => format!("{}  {size}", entry.name),
                        None => entry.name.clone(),
                    },
                })
                .collect();
            let mut text = if lines.is_empty() {
                format!("{listed}: (empty)")
            } else {
                format!("{listed}:\n{}", lines.join("\n"))
            };
            if truncated {
                text.push_str("\n[list truncated; use glob to narrow]");
            }
            Ok(ToolOutcome::ok(
                text,
                ToolDisplay::Entries {
                    title: listed,
                    entries: lines,
                    truncated,
                },
            ))
        }
        "glob" => {
            let output = call_channel(
                channel,
                "glob",
                json!({
                    "root": str_arg(arguments, "root"),
                    "pattern": str_arg(arguments, "pattern"),
                    "include_hidden": arguments.get("include_hidden").and_then(Value::as_bool).unwrap_or(false),
                }),
            )
            .await?;
            let ToolOutput::List { entries, truncated } = output else {
                return Ok(ToolOutcome::failed("Unexpected glob output."));
            };
            let mut text = if entries.is_empty() {
                "No matches.".to_string()
            } else {
                entries.join("\n")
            };
            if truncated {
                text.push_str("\n[results truncated; use a narrower pattern]");
            }
            Ok(ToolOutcome::ok(
                text,
                ToolDisplay::Entries {
                    title: format!(
                        "{} {}",
                        str_arg(arguments, "root"),
                        str_arg(arguments, "pattern")
                    ),
                    entries,
                    truncated,
                },
            ))
        }
        "grep" => {
            let output = call_channel(
                channel,
                "grep",
                object(vec![
                    ("pattern", json!(str_arg(arguments, "pattern"))),
                    ("root", json!(str_arg(arguments, "root"))),
                    (
                        "include",
                        arguments.get("include").cloned().unwrap_or(Value::Null),
                    ),
                    (
                        "case_insensitive",
                        json!(
                            arguments
                                .get("case_insensitive")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                        ),
                    ),
                ]),
            )
            .await?;
            let (content, truncated) = match output {
                ToolOutput::Text { content, truncated } => (content, truncated),
                ToolOutput::List { entries, truncated } => (entries.join("\n"), truncated),
                _ => return Ok(ToolOutcome::failed("Unexpected grep output.")),
            };
            let (mut text, shaped) = head_tail(content.trim_end(), MODEL_OUTPUT_LIMIT);
            if text.is_empty() {
                text = "No matches.".into();
            }
            if truncated || shaped {
                text.push_str("\n[matches truncated; narrow the pattern or root]");
            }
            Ok(ToolOutcome::ok(
                text,
                ToolDisplay::Text {
                    title: format!("grep {}", str_arg(arguments, "pattern")),
                    body: content,
                },
            ))
        }
        "host_info" => {
            let output = call_channel(channel, "workspace_info", json!({})).await?;
            let ToolOutput::WorkspaceInfo {
                host,
                user,
                platform,
                arch,
                shell,
                cwd,
                capabilities,
                ..
            } = output
            else {
                return Ok(ToolOutcome::failed("Unexpected host_info output."));
            };
            let text = format!(
                "target=@{target} host={host} user={user} os={platform} arch={arch}\nexec_shell={shell} cwd={cwd}\ncapabilities: {}",
                serde_json::to_string(&capabilities).unwrap_or_default()
            );
            Ok(ToolOutcome::ok(
                text.clone(),
                ToolDisplay::Text {
                    title: format!("@{target}"),
                    body: text,
                },
            ))
        }
        "apply_patch" => {
            let patch = str_arg(arguments, "patch").to_string();
            let base_dir = str_arg(arguments, "base_dir").to_string();
            let output = call_channel(
                channel,
                "apply_patch",
                json!({"patch": patch, "base_dir": base_dir}),
            )
            .await?;
            let summary = match output {
                ToolOutput::Patch { summary, .. } => summary,
                _ => String::new(),
            };
            let (summary, _) = head_tail(clean_output(&summary).trim(), 4096);
            Ok(ToolOutcome::ok(
                format!("Patch applied in {base_dir}.\n{summary}")
                    .trim_end()
                    .to_string(),
                ToolDisplay::Diff {
                    path: base_dir,
                    diff: patch,
                },
            ))
        }
        "edit_file" | "write_file" => edit_or_write(channel, &call).await,
        "job_start" | "job_poll" | "job_stop" => job_tool(inner, channel, route, &call).await,
        other => Ok(ToolOutcome::failed(format!(
            "Tool `{other}` is not available on this target."
        ))),
    }
}

async fn edit_or_write(
    channel: &AgentExecChannel,
    call: &ToolInvocation,
) -> AgentResult<ToolOutcome> {
    let arguments = &call.arguments;
    let path = str_arg(arguments, "path").to_string();
    channel
        .policy()
        .enforce_path(AgentPathAccess::Edit, &path, true)?;
    let (parent, name) = split_parent(&path);
    let existing = read_whole_file(channel, &path).await?;

    let (patch, model_text) = if call.name == "edit_file" {
        let Some(old) = existing else {
            return Ok(ToolOutcome::failed(format!(
                "`{path}` does not exist. Use write_file to create it."
            )));
        };
        let replaced = replace_exact(
            &old,
            str_arg(arguments, "old_string"),
            str_arg(arguments, "new_string"),
            arguments
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        );
        let (new, count) = match replaced {
            Ok(result) => result,
            Err(message) => return Ok(ToolOutcome::failed(format!("{message} (path: {path})"))),
        };
        let diff = unified_diff(&name, &old, &new, 3);
        let text = format!(
            "Edited {path}: replaced {count} occurrence(s).\n{}",
            head_tail(&diff, 8 * 1024).0
        );
        (diff, text)
    } else {
        let content = str_arg(arguments, "content");
        match existing {
            Some(old) if old == content => {
                return Ok(ToolOutcome::ok(
                    format!("{path} already has this content; nothing changed."),
                    ToolDisplay::None,
                ));
            }
            Some(old) => {
                let diff = unified_diff(&name, &old, content, 3);
                let text = format!(
                    "Replaced {path} ({} lines).\n{}",
                    content.lines().count(),
                    head_tail(&diff, 8 * 1024).0
                );
                (diff, text)
            }
            None => {
                if parent != "." {
                    ensure_directory(channel, &parent).await?;
                }
                (
                    new_file_diff(&name, content),
                    format!("Created {path} ({} lines).", content.lines().count()),
                )
            }
        }
    };

    call_channel(
        channel,
        "apply_patch",
        json!({"patch": patch, "base_dir": parent}),
    )
    .await?;
    Ok(ToolOutcome::ok(
        model_text.trim_end().to_string(),
        ToolDisplay::Diff { path, diff: patch },
    ))
}

async fn web_tool(channel: &AgentExecChannel, call: &ToolInvocation) -> AgentResult<ToolOutcome> {
    if call.name == "web_search" {
        let query = str_arg(&call.arguments, "query").to_string();
        let output = call_channel(channel, "web_search", json!({"query": query})).await?;
        let ToolOutput::WebSearch { results } = output else {
            return Ok(ToolOutcome::failed("Unexpected web_search output."));
        };
        let pretty = serde_json::to_string_pretty(&results).unwrap_or_default();
        let (text, _) = head_tail(&pretty, MODEL_OUTPUT_LIMIT);
        return Ok(ToolOutcome::ok(
            text,
            ToolDisplay::Text {
                title: format!("web_search: {query}"),
                body: pretty,
            },
        ));
    }
    let url = str_arg(&call.arguments, "url").to_string();
    let output = call_channel(channel, "web_fetch", json!({"url": url})).await?;
    let ToolOutput::WebFetch {
        url,
        content,
        truncated,
    } = output
    else {
        return Ok(ToolOutcome::failed("Unexpected web_fetch output."));
    };
    let (mut text, shaped) = head_tail(content.trim(), MODEL_OUTPUT_LIMIT);
    if truncated || shaped {
        text.push_str("\n[page truncated]");
    }
    Ok(ToolOutcome::ok(
        text,
        ToolDisplay::Text {
            title: url,
            body: content,
        },
    ))
}

fn terminal_read(inner: &Inner, arguments: &Value) -> ToolOutcome {
    let Some(terminal) = &inner.terminal else {
        return ToolOutcome::failed("The terminal is not available in this session.");
    };
    let lines = arguments
        .get("lines")
        .and_then(Value::as_u64)
        .unwrap_or(200)
        .clamp(1, 2000) as usize;
    let Some(result) = terminal.read(TerminalReadRequest {
        target: None,
        lines,
    }) else {
        return ToolOutcome::failed("No terminal is open for this conversation.");
    };
    let text = clean_output(&result.text);
    let body = match arguments.get("search").and_then(Value::as_str) {
        Some(search) if !search.is_empty() => search_lines(&text, search, 2),
        _ => text.clone(),
    };
    let (shaped, _) = head_tail(body.trim_end(), MODEL_OUTPUT_LIMIT);
    let shaped = if shaped.trim().is_empty() {
        "(no matching terminal output)".to_string()
    } else {
        shaped
    };
    ToolOutcome::ok(
        shaped,
        ToolDisplay::Text {
            title: format!("terminal @{}", result.target),
            body: text,
        },
    )
}

/// Lines containing `needle` (case-insensitive) with `context` lines around,
/// groups separated by `--`.
pub(crate) fn search_lines(text: &str, needle: &str, context: usize) -> String {
    let needle = needle.to_lowercase();
    let lines: Vec<&str> = text.lines().collect();
    let mut keep = vec![false; lines.len()];
    for (index, line) in lines.iter().enumerate() {
        if line.to_lowercase().contains(&needle) {
            let start = index.saturating_sub(context);
            let end = (index + context + 1).min(lines.len());
            keep[start..end].iter_mut().for_each(|flag| *flag = true);
        }
    }
    let mut out = String::new();
    let mut previous: Option<usize> = None;
    for (index, line) in lines.iter().enumerate() {
        if !keep[index] {
            continue;
        }
        if previous.is_some_and(|previous| previous + 1 != index) {
            out.push_str("--\n");
        }
        out.push_str(line);
        out.push('\n');
        previous = Some(index);
    }
    out
}

fn job_dir(job_id: &str) -> Option<String> {
    uuid::Uuid::parse_str(job_id)
        .ok()
        .map(|uuid| format!("/tmp/miaominal-agent-{uuid}"))
}

fn posix_command(shell: ShellType, script: &str) -> String {
    format!("sh -c {}", shell_quote(script, shell))
}

async fn job_tool(
    inner: &Inner,
    channel: &AgentExecChannel,
    route: Option<String>,
    call: &ToolInvocation,
) -> AgentResult<ToolOutcome> {
    let shell = channel.shell_type();
    if matches!(shell, ShellType::PowerShell | ShellType::Cmd) {
        return Ok(ToolOutcome::failed(
            "Background jobs are only available on POSIX targets; use shell with timeout_s instead.",
        ));
    }
    let arguments = &call.arguments;
    match call.name.as_str() {
        "job_start" => {
            let command = str_arg(arguments, "command").to_string();
            channel.policy().enforce_command(&command, true)?;
            let job_id = uuid::Uuid::new_v4().to_string();
            let dir = job_dir(&job_id).unwrap_or_default();
            let cwd = arguments
                .get("cwd")
                .and_then(Value::as_str)
                .map(|cwd| format!(" && cd {}", shell_quote(cwd, ShellType::Posix)))
                .unwrap_or_default();
            let inner_script = format!("{command}\necho $? > {dir}/exit",);
            let script = format!(
                "mkdir -p {dir} && cd \"$HOME\"{cwd} && \
                 if command -v setsid >/dev/null 2>&1; then S=setsid; else S=; fi; \
                 $S nohup sh -c {quoted} > {dir}/out 2>&1 < /dev/null & echo $! > {dir}/pid; echo started",
                quoted = shell_quote(&inner_script, ShellType::Posix),
            );
            channel.exec(posix_command(shell, &script)).await?;
            inner
                .jobs
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(job_id.clone(), (route, 0));
            Ok(ToolOutcome::ok(
                format!("Started job {job_id}. Call job_poll with this job_id to read output."),
                ToolDisplay::Text {
                    title: format!("job {job_id}"),
                    body: command,
                },
            ))
        }
        "job_poll" | "job_stop" => {
            let job_id = str_arg(arguments, "job_id").to_string();
            let Some(dir) = job_dir(&job_id) else {
                return Ok(ToolOutcome::failed(format!("Unknown job_id `{job_id}`.")));
            };
            let offset = inner
                .jobs
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&job_id)
                .map(|(_, offset)| *offset)
                .unwrap_or(0);
            if call.name == "job_stop" {
                let script = format!(
                    "P=$(cat {dir}/pid 2>/dev/null) || exit 3; \
                     kill -TERM -- -$P 2>/dev/null || {{ pkill -TERM -P $P 2>/dev/null; kill -TERM $P 2>/dev/null; }}; \
                     [ -f {dir}/exit ] || echo 143 > {dir}/exit; echo stopped"
                );
                return match channel.exec(posix_command(shell, &script)).await {
                    Ok(_) => Ok(ToolOutcome::ok(
                        format!("Job {job_id} stopped."),
                        ToolDisplay::None,
                    )),
                    Err(_) => Ok(ToolOutcome::failed(format!("Unknown job_id `{job_id}`."))),
                };
            }
            let wait = arguments
                .get("wait_s")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(60);
            let script = format!(
                "[ -d {dir} ] || {{ echo 'STATUS missing'; exit 0; }}; \
                 i=0; while [ ! -f {dir}/exit ] && [ $i -lt {wait} ]; do sleep 1; i=$((i+1)); done; \
                 if [ -f {dir}/exit ]; then echo \"STATUS exited $(cat {dir}/exit)\"; \
                 elif kill -0 $(cat {dir}/pid) 2>/dev/null; then echo 'STATUS running'; \
                 else echo 'STATUS stopped'; fi; \
                 echo \"SIZE $(wc -c < {dir}/out)\"; \
                 tail -c +{start} {dir}/out | head -c {chunk}",
                start = offset + 1,
                chunk = JOB_CHUNK_BYTES,
            );
            let output = channel.exec(posix_command(shell, &script)).await?;
            let mut lines = output.splitn(3, '\n');
            let status = lines.next().unwrap_or_default().trim().to_string();
            let size: u64 = lines
                .next()
                .and_then(|line| line.trim().strip_prefix("SIZE "))
                .and_then(|size| size.trim().parse().ok())
                .unwrap_or(offset);
            let chunk = lines.next().unwrap_or_default().to_string();
            if status == "STATUS missing" {
                return Ok(ToolOutcome::failed(format!("Unknown job_id `{job_id}`.")));
            }
            let new_offset = (offset + chunk.len() as u64).min(size);
            inner
                .jobs
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .entry(job_id.clone())
                .and_modify(|entry| entry.1 = new_offset)
                .or_insert((None, new_offset));
            let status_text = status.trim_start_matches("STATUS ").to_string();
            let remaining = size.saturating_sub(new_offset);
            let cleaned = clean_output(&chunk);
            let (shaped, _) = head_tail(cleaned.trim_end(), MODEL_OUTPUT_LIMIT);
            let mut text = format!("status: {status_text}\n");
            if shaped.trim().is_empty() {
                text.push_str("(no new output)\n");
            } else {
                text.push_str("output:\n");
                text.push_str(&shaped);
                text.push('\n');
            }
            if remaining > 0 {
                text.push_str(&format!("[{remaining} more bytes available; poll again]\n"));
            }
            Ok(ToolOutcome::ok(
                text.trim_end().to_string(),
                ToolDisplay::Text {
                    title: format!("job {job_id}: {status_text}"),
                    body: cleaned,
                },
            ))
        }
        _ => Ok(ToolOutcome::failed("Unknown job tool.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invocation(name: &str, arguments: Value) -> ToolInvocation {
        ToolInvocation {
            call_id: "c".into(),
            name: name.into(),
            arguments,
        }
    }

    #[test]
    fn assess_without_channel_handles_engine_free_tools() {
        let runtime = ChannelToolRuntime::new(None, "web", None);
        let web = runtime
            .assess_sync(&invocation("web_fetch", json!({"url": "https://x"})))
            .unwrap();
        assert_eq!(web.risk, AgentRiskLevel::L0ReadOnly);
        let shell = runtime.assess_sync(&invocation("shell", json!({"command": "ls"})));
        assert!(shell.unwrap_err().contains("No execution target"));
    }

    #[test]
    fn path_and_patch_risks() {
        assert_eq!(
            path_risk("/etc/nginx/nginx.conf", true).0,
            AgentRiskLevel::L2ServiceImpacting
        );
        assert_eq!(
            path_risk("app/config.yml", true).0,
            AgentRiskLevel::L1LowMutation
        );
        assert_eq!(
            path_risk("/home/a/.ssh/id_rsa", false).0,
            AgentRiskLevel::L4Forbidden
        );
        assert_eq!(
            path_risk("/var/log/syslog", false).0,
            AgentRiskLevel::L0ReadOnly
        );
        assert_eq!(
            patch_paths("--- a/x.conf\n+++ b/x.conf\n@@\n--- /dev/null\n+++ y\n"),
            vec!["x.conf".to_string(), "y".to_string()]
        );
        assert_eq!(join_path("/etc", "nginx.conf"), "/etc/nginx.conf");
        assert_eq!(join_path(".", "a"), "a");
    }

    #[test]
    fn apply_edit_replaces_payload() {
        let runtime = ChannelToolRuntime::new(None, "web", None);
        let call = invocation("shell", json!({"command": "rm -rf x", "target": "web"}));
        assert_eq!(
            runtime.apply_edit(&call, "rm -r x"),
            json!({"command": "rm -r x", "target": "web"})
        );
        let patch = invocation("apply_patch", json!({"patch": "a", "base_dir": "."}));
        assert_eq!(runtime.apply_edit(&patch, "b")["patch"], "b");
    }

    #[test]
    fn target_resolution_strips_default() {
        let runtime = ChannelToolRuntime::new(None, "@web", None);
        assert_eq!(
            runtime.inner.target(&json!({"target": "@web"})),
            (None, "web".to_string())
        );
        assert_eq!(
            runtime.inner.target(&json!({"target": "db"})),
            (Some("db".to_string()), "db".to_string())
        );
        assert_eq!(runtime.inner.target(&json!({})), (None, "web".to_string()));
    }

    #[test]
    fn search_lines_adds_context() {
        let text = "a\nb\nERROR x\nc\nd\ne\nf\nerror y\n";
        assert_eq!(
            search_lines(text, "error", 1),
            "b\nERROR x\nc\n--\nf\nerror y\n"
        );
    }

    #[test]
    fn probe_output_fills_environment_facts() {
        let base = TargetInfo {
            name: "web".into(),
            is_default: true,
            exec_shell: "posix-sh".into(),
            ..Default::default()
        };
        let output = ToolOutput::WorkspaceInfo {
            host: "web-01".into(),
            user: "deploy".into(),
            platform: "Linux".into(),
            arch: "x86_64".into(),
            shell: "bash".into(),
            cwd: "/home/deploy".into(),
            workspace_roots: vec![],
            trusted_read_roots: vec![],
            sensitive_paths: vec![],
            capabilities: crate::RemoteCapabilities {
                sftp: true,
                exec: true,
                pty: true,
                rg: false,
                git: true,
                patch: true,
                python3: true,
                grep: true,
                find: true,
                sed: true,
                powershell: false,
            },
            route: crate::BackendRoute::SshExec,
            supported_tools: vec![],
        };
        let info = target_info_from_probe(base.clone(), output);
        assert_eq!(info.os, "Linux x86_64");
        assert_eq!(info.exec_shell, "bash");
        assert_eq!(info.cwd, "/home/deploy");
        assert_eq!(info.user, "deploy");
        assert!(info.is_default);
        assert_eq!(
            info.notes,
            vec!["tools: rg=no git=yes python3=yes patch=yes sed=yes".to_string()]
        );
        // Unexpected output keeps the configured facts.
        assert_eq!(
            target_info_from_probe(
                base.clone(),
                ToolOutput::Text {
                    content: String::new(),
                    truncated: false
                }
            ),
            base
        );
    }

    #[test]
    fn terminal_read_without_source_fails_cleanly() {
        let runtime = ChannelToolRuntime::new(None, "web", None);
        let outcome = terminal_read(&runtime.inner, &json!({}));
        assert_eq!(outcome.status, ToolResultStatus::Failed);
    }
}
