use crate::channel::{AgentExecChannel, DEFAULT_MAX_OUTPUT_BYTES, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::jobs::{AgentJobId, JobPollResult, JobStatus};
use crate::path_guard::{RemotePathKind, shell_quote};
use crate::policy::AgentPathAccess;
use base64::Engine as _;
use miaominal_core::profile::ShellType;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct StartJobArgs {
    pub command: String,
    pub cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PollJobArgs {
    pub job_id: AgentJobId,
}

#[derive(Debug, Deserialize)]
pub struct StopJobArgs {
    pub job_id: AgentJobId,
}

const DEFAULT_POLL_AFTER_MS: u64 = 1_000;
const STALE_JOB_HOURS: u64 = 24;
const WINDOWS_CMD_MAX_COMMAND_BYTES: usize = 8_191;
const POSIX_READY_ATTEMPTS: usize = 100;
const POSIX_LAUNCH_CLEANUP_ATTEMPTS: usize = 70;

const POSIX_PROCESS_HELPERS: &str = include_str!("job_scripts/posix_process_helpers.sh");
const POSIX_START_CHILD: &str = include_str!("job_scripts/posix_start_child.sh");
const POSIX_START_RUNNER: &str = include_str!("job_scripts/posix_start_runner.sh");
const POSIX_START_LAUNCHER: &str = include_str!("job_scripts/posix_start_launcher.sh");
const WINDOWS_START_MONITOR: &str = include_str!("job_scripts/windows_start_monitor.ps1");
const WINDOWS_START_LAUNCHER: &str = include_str!("job_scripts/windows_start_launcher.ps1");
const WINDOWS_DETACHED_LAUNCHER: &str = include_str!("job_scripts/windows_detached_launcher.cs");
const WINDOWS_CHILD_WRAPPER: &str = include_str!("job_scripts/windows_child_wrapper.ps1");
const POSIX_POLL: &str = include_str!("job_scripts/posix_poll.sh");
const WINDOWS_POLL: &str = include_str!("job_scripts/windows_poll.ps1");
const POSIX_STOP: &str = include_str!("job_scripts/posix_stop.sh");
const WINDOWS_STOP: &str = include_str!("job_scripts/windows_stop.ps1");
const POSIX_CLEANUP: &str = include_str!("job_scripts/posix_cleanup.sh");
const WINDOWS_CLEANUP: &str = include_str!("job_scripts/windows_cleanup.ps1");
const POSIX_SCAVENGE: &str = include_str!("job_scripts/posix_scavenge.sh");
const WINDOWS_SCAVENGE: &str = include_str!("job_scripts/windows_scavenge.ps1");

fn render_job_script(template: &str, values: &[(&str, &str)]) -> String {
    let mut rendered = String::with_capacity(template.len());
    let mut remaining = template;

    while let Some(start) = remaining.find("@@") {
        rendered.push_str(&remaining[..start]);
        let placeholder = &remaining[start + 2..];
        let Some(end) = placeholder.find("@@") else {
            panic!("unterminated job script placeholder");
        };
        let name = &placeholder[..end];
        let value = values
            .iter()
            .find_map(|(candidate, value)| (*candidate == name).then_some(*value))
            .unwrap_or_else(|| panic!("unknown job script placeholder: {name}"));
        rendered.push_str(value);
        remaining = &placeholder[end + 2..];
    }

    rendered.push_str(remaining);
    rendered
}

#[derive(Debug, Clone)]
struct PosixJobPaths {
    root: String,
    status: String,
    stdout: String,
    stderr: String,
    pid: String,
    ready: String,
    runner: String,
    command: String,
    child: String,
    stop: String,
    error: String,
}

impl PosixJobPaths {
    fn from_marker(marker: &str) -> Option<Self> {
        let root = marker.strip_suffix("/status")?;
        if root.is_empty() {
            return None;
        }
        Some(Self {
            root: root.to_string(),
            status: marker.to_string(),
            stdout: format!("{root}/stdout"),
            stderr: format!("{root}/stderr"),
            pid: format!("{root}/pid"),
            ready: format!("{root}/ready"),
            runner: format!("{root}/runner"),
            command: format!("{root}/command"),
            child: format!("{root}/child"),
            stop: format!("{root}/stop"),
            error: format!("{root}/error"),
        })
    }
}

fn wrap_posix_script(script: &str, shell_type: ShellType) -> String {
    format!("sh -lc {}", shell_quote(script, shell_type))
}

fn posix_process_helpers() -> &'static str {
    POSIX_PROCESS_HELPERS.trim_end()
}

/// Build the background job command for the given shell type.
///
/// POSIX/Fish and Windows both use a short-lived launcher plus an independent
/// monitor. The monitor owns the user process, redirects its output to private
/// files, and atomically writes the final exit status.
fn make_start_job_command(
    shell_type: ShellType,
    cwd: &str,
    user_command: &str,
    marker: &str,
) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => {
            make_posix_start_command(shell_type, cwd, user_command, marker)
        }
        ShellType::PowerShell => super::windows::powershell_compressed_command(
            &make_windows_start_script(shell_type, cwd, user_command, marker),
        ),
        ShellType::Cmd => super::windows::powershell_compressed_command_for_cmd(
            &make_windows_start_script(shell_type, cwd, user_command, marker),
        ),
    }
}

fn make_start_job_launch(shell_type: ShellType, job_command: &str, _marker: &str) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => job_command.to_string(),
        ShellType::PowerShell | ShellType::Cmd => job_command.to_string(),
    }
}

fn make_posix_start_command(
    shell_type: ShellType,
    cwd: &str,
    user_command: &str,
    marker: &str,
) -> String {
    let scripts = make_posix_start_scripts(cwd, user_command, marker);
    wrap_posix_script(&scripts.launcher, shell_type)
}

#[cfg_attr(not(test), allow(dead_code))]
struct PosixStartScripts {
    launcher: String,
    runner: String,
    child: String,
}

fn make_posix_start_scripts(cwd: &str, user_command: &str, marker: &str) -> PosixStartScripts {
    let paths = PosixJobPaths::from_marker(marker)
        .expect("generated POSIX job marker must end with /status");
    let quote = |value: &str| shell_quote(value, ShellType::Posix);
    let token = paths
        .root
        .strip_prefix("/tmp/miaominal-agent-")
        .unwrap_or(&paths.root);
    let child = quote(&paths.child);
    let cwd = quote(cwd);
    let command = quote(user_command);
    let child_source = render_job_script(
        POSIX_START_CHILD,
        &[
            ("HELPERS", posix_process_helpers()),
            ("CHILD", &child),
            ("CWD", &cwd),
            ("COMMAND", &command),
        ],
    );

    let root = quote(&paths.root);
    let status = quote(&paths.status);
    let stdout = quote(&paths.stdout);
    let stderr = quote(&paths.stderr);
    let pid = quote(&paths.pid);
    let ready = quote(&paths.ready);
    let runner = quote(&paths.runner);
    let command_file = quote(&paths.command);
    let stop = quote(&paths.stop);
    let error = quote(&paths.error);
    let token = quote(token);
    let ready_attempts = POSIX_READY_ATTEMPTS.to_string();
    let runner_source = render_job_script(
        POSIX_START_RUNNER,
        &[
            ("HELPERS", posix_process_helpers()),
            ("ROOT", &root),
            ("STATUS", &status),
            ("STDOUT", &stdout),
            ("STDERR", &stderr),
            ("PID", &pid),
            ("READY", &ready),
            ("RUNNER", &runner),
            ("COMMAND_FILE", &command_file),
            ("CHILD", &child),
            ("STOP", &stop),
            ("ERROR", &error),
            ("TOKEN", &token),
            ("READY_ATTEMPTS", &ready_attempts),
        ],
    );

    let runner_source_q = quote(&runner_source);
    let child_source_q = quote(&child_source);
    let grace_attempts = POSIX_LAUNCH_CLEANUP_ATTEMPTS.to_string();
    let launcher = render_job_script(
        POSIX_START_LAUNCHER,
        &[
            ("ROOT", &root),
            ("STATUS", &status),
            ("STDOUT", &stdout),
            ("STDERR", &stderr),
            ("PID", &pid),
            ("READY", &ready),
            ("RUNNER", &runner),
            ("COMMAND_FILE", &command_file),
            ("CHILD", &child),
            ("STOP", &stop),
            ("ERROR", &error),
            ("RUNNER_SOURCE", &runner_source_q),
            ("CHILD_SOURCE", &child_source_q),
            ("READY_ATTEMPTS", &ready_attempts),
            ("GRACE_ATTEMPTS", &grace_attempts),
        ],
    );

    PosixStartScripts {
        launcher,
        runner: runner_source,
        child: child_source,
    }
}

fn make_windows_start_script(
    shell_type: ShellType,
    cwd: &str,
    user_command: &str,
    marker: &str,
) -> String {
    let marker_q = shell_quote(marker, ShellType::PowerShell);
    let requested_cwd_q = shell_quote(cwd, ShellType::PowerShell);
    let (program, child_arguments) = windows_child_command(shell_type, user_command);
    let program_q = shell_quote(program, ShellType::PowerShell);
    let child_arguments_q = shell_quote(&child_arguments, ShellType::PowerShell);

    let monitor_script = render_job_script(
        WINDOWS_START_MONITOR,
        &[
            ("MARKER", &marker_q),
            ("PROGRAM", &program_q),
            ("ARGUMENTS", &child_arguments_q),
        ],
    );
    let monitor_script_q = shell_quote(&monitor_script, ShellType::PowerShell);
    let detached_launcher_q =
        shell_quote(WINDOWS_DETACHED_LAUNCHER.trim_end(), ShellType::PowerShell);

    render_job_script(
        WINDOWS_START_LAUNCHER,
        &[
            ("MARKER", &marker_q),
            ("CWD", &requested_cwd_q),
            ("MONITOR_SCRIPT", &monitor_script_q),
            ("DETACHED_LAUNCHER", &detached_launcher_q),
        ],
    )
}

fn windows_child_command(shell_type: ShellType, user_command: &str) -> (&'static str, String) {
    match shell_type {
        ShellType::PowerShell => {
            let command_q = shell_quote(user_command, ShellType::PowerShell);
            let script =
                render_job_script(WINDOWS_CHILD_WRAPPER, &[("COMMAND", command_q.as_str())]);
            let payload = super::windows::powershell_encoded_payload(&script);
            (
                "powershell.exe",
                windows_command_line_args(&["-NoProfile", "-EncodedCommand", payload.as_str()]),
            )
        }
        ShellType::Cmd => ("cmd.exe", windows_cmd_arguments(user_command)),
        ShellType::Posix | ShellType::Fish => unreachable!("not a Windows shell"),
    }
}

/// Build the raw command line consumed by `cmd.exe` itself.
///
/// The command following `/c` is not a normal CRT argument: CMD reparses the
/// remaining command line using its own quote, metacharacter, and expansion
/// rules. Passing `user_command` through `windows_command_line_arg` would turn
/// inner quotes into `\"`, which CMD does not treat as an escaped quote. In
/// particular, an explicit nested `powershell.exe -Command "..."` would then
/// execute the quoted body as literal text instead of as PowerShell code.
fn windows_cmd_arguments(user_command: &str) -> String {
    let command = user_command.trim_start();
    if command.starts_with('"') {
        // CMD strips the first and final quote around `/s /c` commands. Add
        // the conventional extra pair when the executable path itself is
        // quoted, while leaving every user-supplied inner character intact.
        format!("/d /v:off /s /c \"{command}\"")
    } else {
        format!("/d /v:off /s /c {user_command}")
    }
}

fn windows_command_line_args(arguments: &[&str]) -> String {
    arguments
        .iter()
        .map(|argument| windows_command_line_arg(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

fn windows_command_line_arg(argument: &str) -> String {
    if argument.is_empty()
        || argument
            .chars()
            .any(|character| matches!(character, ' ' | '\t' | '"'))
    {
        let mut quoted = String::from("\"");
        let mut backslashes = 0;
        for character in argument.chars() {
            match character {
                '\\' => backslashes += 1,
                '"' => {
                    quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                    quoted.push('"');
                    backslashes = 0;
                }
                _ => {
                    quoted.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                    quoted.push(character);
                }
            }
        }
        quoted.push_str(&"\\".repeat(backslashes * 2));
        quoted.push('"');
        quoted
    } else {
        argument.to_string()
    }
}

/// Build the poll command for the given shell type. Every variant emits:
/// `status=...`, optional `exit=...`, `truncated=0|1`, and bounded base64
/// stdout/stderr fields. Base64 keeps arbitrary output from colliding with the
/// framing protocol and lets Rust handle partial UTF-8 boundaries safely.
fn make_poll_command(marker: &str, shell_type: ShellType) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => make_posix_poll_command(marker, shell_type),
        ShellType::PowerShell | ShellType::Cmd => {
            super::windows::powershell_compressed_command(&make_windows_poll_script(marker))
        }
    }
}

fn make_posix_poll_command(marker: &str, shell_type: ShellType) -> String {
    let Some(paths) = PosixJobPaths::from_marker(marker) else {
        let status = shell_quote(marker, ShellType::Posix);
        let out = shell_quote(&format!("{marker}.out"), ShellType::Posix);
        let err = shell_quote(&format!("{marker}.err"), ShellType::Posix);
        let legacy = format!(
            "if [ -f {status} ]; then printf 'status=exited\\nexit='; head -c 32 {status}; printf '\\ntruncated=0\\nstdout_b64='; [ ! -f {out} ] || tail -c {max} {out} | base64 | tr -d '\\r\\n'; printf '\\nstderr_b64='; [ ! -f {err} ] || tail -c {max} {err} | base64 | tr -d '\\r\\n'; printf '\\n'; else printf 'status=not_found\\n'; fi",
            max = DEFAULT_MAX_OUTPUT_BYTES,
        );
        return wrap_posix_script(&legacy, shell_type);
    };
    let quote = |value: &str| shell_quote(value, ShellType::Posix);
    let token = paths
        .root
        .strip_prefix("/tmp/miaominal-agent-")
        .unwrap_or(&paths.root);
    let root = quote(&paths.root);
    let status = quote(&paths.status);
    let stdout = quote(&paths.stdout);
    let stderr = quote(&paths.stderr);
    let pid = quote(&paths.pid);
    let runner = quote(&paths.runner);
    let token = quote(token);
    let max = DEFAULT_MAX_OUTPUT_BYTES.to_string();
    let script = render_job_script(
        POSIX_POLL,
        &[
            ("HELPERS", posix_process_helpers()),
            ("ROOT", &root),
            ("STATUS", &status),
            ("STDOUT", &stdout),
            ("STDERR", &stderr),
            ("PID", &pid),
            ("RUNNER", &runner),
            ("TOKEN", &token),
            ("MAX", &max),
        ],
    );
    wrap_posix_script(&script, shell_type)
}

fn make_windows_poll_script(marker: &str) -> String {
    let marker = shell_quote(marker, ShellType::PowerShell);
    let max = DEFAULT_MAX_OUTPUT_BYTES.to_string();
    render_job_script(WINDOWS_POLL, &[("MARKER", &marker), ("MAX", &max)])
}

fn make_cleanup_command(marker: &str, shell_type: ShellType) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => {
            if let Some(paths) = PosixJobPaths::from_marker(marker) {
                let quote = |value: &str| shell_quote(value, ShellType::Posix);
                let root = quote(&paths.root);
                let status = quote(&paths.status);
                let stdout = quote(&paths.stdout);
                let stderr = quote(&paths.stderr);
                let pid = quote(&paths.pid);
                let ready = quote(&paths.ready);
                let runner = quote(&paths.runner);
                let command = quote(&paths.command);
                let child = quote(&paths.child);
                let stop = quote(&paths.stop);
                let error = quote(&paths.error);
                let script = render_job_script(
                    POSIX_CLEANUP,
                    &[
                        ("ROOT", &root),
                        ("STATUS", &status),
                        ("STDOUT", &stdout),
                        ("STDERR", &stderr),
                        ("PID", &pid),
                        ("READY", &ready),
                        ("RUNNER", &runner),
                        ("COMMAND", &command),
                        ("CHILD", &child),
                        ("STOP", &stop),
                        ("ERROR", &error),
                    ],
                );
                wrap_posix_script(&script, shell_type)
            } else {
                let paths = [
                    marker.to_string(),
                    format!("{marker}.out"),
                    format!("{marker}.err"),
                    format!("{marker}.pid"),
                ]
                .into_iter()
                .map(|path| shell_quote(&path, ShellType::Posix))
                .collect::<Vec<_>>()
                .join(" ");
                wrap_posix_script(&format!("rm -f {paths}"), shell_type)
            }
        }
        ShellType::PowerShell | ShellType::Cmd => {
            let marker = shell_quote(marker, ShellType::PowerShell);
            let script = render_job_script(WINDOWS_CLEANUP, &[("MARKER", &marker)]);
            super::windows::powershell_compressed_command(&script)
        }
    }
}

fn make_scavenge_command(shell_type: ShellType) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => {
            let minutes = (STALE_JOB_HOURS * 60).to_string();
            let script = render_job_script(
                POSIX_SCAVENGE,
                &[("HELPERS", posix_process_helpers()), ("MINUTES", &minutes)],
            );
            wrap_posix_script(&script, shell_type)
        }
        ShellType::PowerShell | ShellType::Cmd => {
            let hours = STALE_JOB_HOURS.to_string();
            let script = render_job_script(WINDOWS_SCAVENGE, &[("HOURS", &hours)]);
            super::windows::powershell_compressed_command(&script)
        }
    }
}

async fn scavenge_jobs(channel: &AgentExecChannel, shell_type: ShellType) {
    let command = make_scavenge_command(shell_type);
    if ensure_windows_command_fits(&command, shell_type).is_err() {
        return;
    }
    let Ok(output) = channel.exec(command).await else {
        return;
    };
    for id in output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("cleaned="))
    {
        let job_id = AgentJobId(id.to_string());
        if job_id.remote_marker_for_shell(shell_type).is_ok() {
            let _ = channel.jobs().remove(&job_id);
        }
    }
}

async fn detected_job_shell(channel: &AgentExecChannel) -> ShellType {
    if matches!(channel.shell_type(), ShellType::PowerShell | ShellType::Cmd) {
        super::workspace_info::ensure_exec_shell_detected(channel).await;
    }
    channel.shell_type()
}

fn ensure_windows_command_fits(command: &str, shell_type: ShellType) -> AgentResult<()> {
    if shell_type == ShellType::Cmd && command.len() >= WINDOWS_CMD_MAX_COMMAND_BYTES {
        return Err(AgentError::Backend(anyhow::anyhow!(
            "generated Windows background-job command exceeds CMD's 8191-byte limit; put the long command in a script and start that script instead"
        )));
    }
    Ok(())
}

fn make_stop_command(marker: &str, shell_type: ShellType) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => make_posix_stop_command(marker, shell_type),
        ShellType::PowerShell | ShellType::Cmd => {
            super::windows::powershell_compressed_command(&make_windows_stop_script(marker))
        }
    }
}

fn make_posix_stop_command(marker: &str, shell_type: ShellType) -> String {
    let Some(paths) = PosixJobPaths::from_marker(marker) else {
        return wrap_posix_script(
            "printf '%s\\n' 'legacy POSIX jobs cannot be stopped safely' >&2; exit 1",
            shell_type,
        );
    };
    let quote = |value: &str| shell_quote(value, ShellType::Posix);
    let token = paths
        .root
        .strip_prefix("/tmp/miaominal-agent-")
        .unwrap_or(&paths.root);
    let root = quote(&paths.root);
    let status = quote(&paths.status);
    let stdout = quote(&paths.stdout);
    let stderr = quote(&paths.stderr);
    let pid = quote(&paths.pid);
    let ready = quote(&paths.ready);
    let runner = quote(&paths.runner);
    let command = quote(&paths.command);
    let child = quote(&paths.child);
    let stop = quote(&paths.stop);
    let error = quote(&paths.error);
    let token = quote(token);
    let script = render_job_script(
        POSIX_STOP,
        &[
            ("HELPERS", posix_process_helpers()),
            ("ROOT", &root),
            ("STATUS", &status),
            ("STDOUT", &stdout),
            ("STDERR", &stderr),
            ("PID", &pid),
            ("READY", &ready),
            ("RUNNER", &runner),
            ("COMMAND", &command),
            ("CHILD", &child),
            ("STOP", &stop),
            ("ERROR", &error),
            ("TOKEN", &token),
        ],
    );
    wrap_posix_script(&script, shell_type)
}

fn make_windows_stop_script(marker: &str) -> String {
    let marker = shell_quote(marker, ShellType::PowerShell);
    render_job_script(WINDOWS_STOP, &[("MARKER", &marker)])
}

pub async fn start_job(channel: &AgentExecChannel, args: StartJobArgs) -> AgentResult<ToolOutput> {
    // Approval execution and later agent turns can use a newly constructed
    // channel. Revalidate before launching so a stale profile/cache cannot
    // select CMD when this SSH exec channel is actually PowerShell.
    if matches!(channel.shell_type(), ShellType::PowerShell | ShellType::Cmd) {
        super::workspace_info::refresh_exec_shell_detected(channel).await;
    }
    let cwd = channel
        .authorize_existing_path(
            args.cwd.as_deref().unwrap_or("."),
            AgentPathAccess::Read,
            RemotePathKind::Directory,
        )
        .await?;
    let cwd = cwd.as_str();
    let shell_type = channel.shell_type();
    let job_id = AgentJobId::new();
    let marker = job_id.remote_marker_for_shell(shell_type)?;
    let command = make_start_job_command(shell_type, cwd, &args.command, &marker);
    let launch = make_start_job_launch(shell_type, &command, &marker);
    ensure_windows_command_fits(&launch, shell_type)?;
    scavenge_jobs(channel, shell_type).await;

    let launch_output = channel.exec(launch).await?;
    let marker = launch_output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .trim_matches('\'')
        .to_string();
    let marker_valid = match shell_type {
        ShellType::Posix | ShellType::Fish => {
            marker == job_id.remote_marker_for_shell(shell_type)?
        }
        ShellType::PowerShell | ShellType::Cmd => {
            let expected_name = format!("miaominal-agent-{}.status", job_id.0);
            !marker.is_empty()
                && marker
                    .to_ascii_lowercase()
                    .ends_with(&expected_name.to_ascii_lowercase())
        }
    };
    if !marker_valid {
        return Err(AgentError::Backend(anyhow::anyhow!(
            "job launcher did not return the expected marker path"
        )));
    }
    let job_id = channel
        .jobs()
        .insert_remote_job_with_id(job_id, args.command, marker);
    Ok(ToolOutput::JobStarted {
        job_id,
        exec_shell: match shell_type {
            ShellType::Posix => "posix-sh",
            ShellType::Fish => "fish",
            ShellType::PowerShell => "powershell",
            ShellType::Cmd => "cmd",
        }
        .into(),
        poll_after_ms: DEFAULT_POLL_AFTER_MS,
        next_action: "Poll this job with poll_job until status is exited, or use list_jobs if you lose the job_id. Use run_shell instead of start_job for short commands."
            .into(),
    })
}

pub async fn list_jobs(channel: &AgentExecChannel) -> AgentResult<ToolOutput> {
    let shell_type = detected_job_shell(channel).await;
    scavenge_jobs(channel, shell_type).await;
    Ok(ToolOutput::JobList {
        jobs: channel.jobs().list()?,
    })
}

pub async fn poll_job(channel: &AgentExecChannel, args: PollJobArgs) -> AgentResult<ToolOutput> {
    let shell_type = detected_job_shell(channel).await;
    let marker = channel
        .jobs()
        .remote_marker_for_shell(&args.job_id, shell_type)?;
    let poll_command = make_poll_command(&marker, shell_type);
    ensure_windows_command_fits(&poll_command, shell_type)?;
    let output = channel.exec(poll_command).await?;
    let result = parse_poll_output(args.job_id.clone(), &output)?;
    if matches!(result.status, JobStatus::Exited | JobStatus::Stopped) {
        let cleanup_command = make_cleanup_command(&marker, shell_type);
        let cleaned = if ensure_windows_command_fits(&cleanup_command, shell_type).is_ok() {
            channel.exec(cleanup_command).await.is_ok()
        } else {
            false
        };
        if cleaned {
            let _ = channel.jobs().remove(&args.job_id);
        }
    } else if result.status == JobStatus::NotFound {
        let _ = channel.jobs().remove(&args.job_id);
    }
    Ok(ToolOutput::JobPoll { result })
}

fn parse_poll_output(job_id: AgentJobId, output: &str) -> AgentResult<JobPollResult> {
    let normalized = output.replace("\r\n", "\n").replace('\r', "\n");
    let status = normalized
        .lines()
        .find_map(|line| line.strip_prefix("status="))
        .ok_or_else(|| {
            AgentError::Backend(anyhow::anyhow!("job poll response is missing status"))
        })?;
    let exit_status = normalized
        .lines()
        .find_map(|line| line.strip_prefix("exit="))
        .and_then(|value| value.trim().parse::<i32>().ok());
    let truncated = normalized
        .lines()
        .find_map(|line| line.strip_prefix("truncated="))
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "True"));

    let mut stderr = encoded_output_field(&normalized, "stderr_b64")?
        .or_else(|| heredoc_section(&normalized, "stderr"))
        .unwrap_or_default();
    if let Some(diagnostic) = encoded_output_field(&normalized, "diagnostic_b64")? {
        if !stderr.is_empty() {
            stderr.push('\n');
        }
        stderr.push_str(&diagnostic);
    }

    Ok(JobPollResult {
        job_id,
        status: match status.trim() {
            "running" => JobStatus::Running,
            "exited" => JobStatus::Exited,
            "not_found" => JobStatus::NotFound,
            "stopped" => JobStatus::Stopped,
            other => {
                return Err(AgentError::Backend(anyhow::anyhow!(
                    "unknown job status `{other}`"
                )));
            }
        },
        exit_status,
        stdout: encoded_output_field(&normalized, "stdout_b64")?
            .or_else(|| heredoc_section(&normalized, "stdout"))
            .unwrap_or_default(),
        stderr,
        truncated,
    })
}

fn encoded_output_field(output: &str, name: &str) -> AgentResult<Option<String>> {
    let prefix = format!("{name}=");
    let Some(value) = output.lines().find_map(|line| line.strip_prefix(&prefix)) else {
        return Ok(None);
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .map_err(|error| AgentError::Backend(anyhow::anyhow!("invalid {name}: {error}")))?;
    let mut start = 0;
    while start < bytes.len().min(3) && bytes[start] & 0b1100_0000 == 0b1000_0000 {
        start += 1;
    }
    let mut text = String::from_utf8_lossy(&bytes[start..]).into_owned();
    if text.len() > DEFAULT_MAX_OUTPUT_BYTES {
        text.truncate(text.floor_char_boundary(DEFAULT_MAX_OUTPUT_BYTES));
    }
    Ok(Some(text))
}

fn heredoc_section(output: &str, name: &str) -> Option<String> {
    let start = format!("{name}<<EOF\n");
    let after_start = output.split_once(&start)?.1;
    let section = after_start.split_once("\nEOF")?.0;
    Some(section.to_string())
}

pub async fn stop_job(channel: &AgentExecChannel, args: StopJobArgs) -> AgentResult<ToolOutput> {
    let shell_type = detected_job_shell(channel).await;
    let marker = channel
        .jobs()
        .remote_marker_for_shell(&args.job_id, shell_type)?;
    let command = make_stop_command(&marker, shell_type);
    ensure_windows_command_fits(&command, shell_type)?;
    let content = channel.exec(command).await?;
    let _ = channel.jobs().remove(&args.job_id);
    Ok(ToolOutput::Text {
        content,
        truncated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    static WINDOWS_SCAVENGE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn posix_test_shell() -> Option<std::path::PathBuf> {
        #[cfg(windows)]
        {
            let shell = std::path::PathBuf::from(r"C:\Program Files\Git\bin\sh.exe");
            shell.exists().then_some(shell)
        }
        #[cfg(not(windows))]
        {
            Some(std::path::PathBuf::from("sh"))
        }
    }

    #[tokio::test]
    async fn start_job_uses_detected_powershell_over_configured_cmd() {
        let mut profile = miaominal_core::profile::SessionProfile::blank("session-1", 1);
        profile.host = "example.com".into();
        profile.username = "akko".into();
        profile.shell_type = ShellType::Cmd;
        let channel = AgentExecChannel::for_profile(
            profile,
            Vec::new(),
            miaominal_secrets::SecretStore::new_locked_vault(),
            miaominal_storage::known_hosts_store::KnownHostsStore::with_path(
                std::env::temp_dir().join("agent-job-detected-shell-known-hosts"),
            ),
        );
        assert_eq!(channel.shell_type(), ShellType::Cmd);

        channel.set_detected_shell(ShellType::PowerShell);
        let job_id = AgentJobId::new();
        let effective_shell = detected_job_shell(&channel).await;
        let marker = job_id
            .remote_marker_for_shell(effective_shell)
            .expect("marker should be generated");
        let command = make_start_job_command(effective_shell, ".", "Write-Output 'hello'", &marker);
        let (program, arguments) = windows_child_command(effective_shell, "Write-Output 'hello'");

        assert_eq!(effective_shell, ShellType::PowerShell);
        assert_eq!(program, "powershell.exe");
        assert!(arguments.contains("-EncodedCommand"));
        assert!(command.starts_with("powershell.exe -NoProfile -EncodedCommand "));
    }

    #[test]
    fn posix_start_job_command_uses_private_monitor_and_process_group() {
        let cmd = make_start_job_command(
            ShellType::Posix,
            "/home/user/project",
            "echo hello",
            "/tmp/miaominal-agent-00000000-0000-0000-0000-000000000000/status",
        );
        assert!(cmd.starts_with("sh -lc "));
        assert!(cmd.contains("umask 077"));
        assert!(cmd.contains("mkdir"));
        assert!(cmd.contains("nohup sh"));
        assert!(cmd.contains("setsid sh"));
        assert!(cmd.contains("child_pgid"));
        assert!(cmd.contains("/stdout"));
        assert!(cmd.contains("/stderr"));
        assert!(!cmd.contains("pkill -f"));
    }

    #[test]
    fn posix_start_scripts_are_syntactically_valid() {
        use std::io::Write as _;

        let Some(shell) = posix_test_shell() else {
            return;
        };

        let scripts = make_posix_start_scripts(
            "/home/user/project",
            "printf '%s\\n' \"hello world\"; sleep 1",
            "/tmp/miaominal-agent-00000000-0000-0000-0000-000000000000/status",
        );
        for (name, source) in [
            ("launcher", scripts.launcher),
            ("runner", scripts.runner),
            ("child", scripts.child),
        ] {
            let mut child = std::process::Command::new(&shell)
                .arg("-n")
                .stdin(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("start shell syntax check");
            child
                .stdin
                .as_mut()
                .expect("shell syntax stdin")
                .write_all(source.as_bytes())
                .expect("write shell source");
            let output = child.wait_with_output().expect("finish shell syntax check");
            assert!(
                output.status.success(),
                "{name} script syntax failed: {}\n{source}",
                String::from_utf8_lossy(&output.stderr),
            );
        }
    }

    #[test]
    fn posix_liveness_checks_fall_back_when_signal_probes_are_denied() {
        let Some(shell) = posix_test_shell() else {
            return;
        };
        let script = format!(
            r#"{helpers}
kill() {{ return 1; }}
ps() {{
    if [ "$1" = -p ]; then printf '%s\n' "$2"; return 0; fi
    if [ "$1" = -e ]; then printf '%s\n' 4242; return 0; fi
    return 1
}}
process_alive 4242 && group_alive 4242
"#,
            helpers = posix_process_helpers(),
        );
        let output = std::process::Command::new(shell)
            .args(["-c", &script])
            .output()
            .expect("run POSIX liveness fallback check");
        assert!(
            output.status.success(),
            "liveness fallback failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn posix_poll_stop_and_cleanup_scripts_parse_and_execute() {
        let Some(shell) = posix_test_shell() else {
            return;
        };
        let marker = AgentJobId::new()
            .remote_marker_for_shell(ShellType::Posix)
            .expect("marker should be generated");

        let poll = std::process::Command::new(&shell)
            .args(["-lc", &make_poll_command(&marker, ShellType::Posix)])
            .output()
            .expect("execute generated poll command");
        assert!(poll.status.success());
        assert!(String::from_utf8_lossy(&poll.stdout).contains("status=not_found"));

        let stop = std::process::Command::new(&shell)
            .args(["-lc", &make_stop_command(&marker, ShellType::Posix)])
            .output()
            .expect("execute generated stop command");
        assert!(stop.status.success());
        assert_eq!(String::from_utf8_lossy(&stop.stdout).trim(), "not_found");

        let cleanup = std::process::Command::new(shell)
            .args(["-lc", &make_cleanup_command(&marker, ShellType::Posix)])
            .output()
            .expect("execute generated cleanup command");
        assert_eq!(cleanup.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&cleanup.stderr).trim().is_empty());
    }

    #[test]
    fn posix_start_job_launch_is_already_self_contained() {
        let launch = make_start_job_launch(
            ShellType::Posix,
            "nohup sh -lc 'echo hi'",
            "/tmp/marker.status",
        );
        assert_eq!(launch, "nohup sh -lc 'echo hi'");
    }

    #[test]
    fn posix_poll_is_bounded_and_uses_base64_framing() {
        let command = make_poll_command(
            "/tmp/miaominal-agent-00000000-0000-0000-0000-000000000000/status",
            ShellType::Posix,
        );
        assert!(command.contains("status=running"));
        assert!(command.contains("status=not_found"));
        assert!(command.contains(&format!("tail -c {DEFAULT_MAX_OUTPUT_BYTES}")));
        assert!(command.contains("truncated=%s"));
        assert!(command.contains("stdout_b64="));
        assert!(command.contains("base64"));
        assert!(!command.contains("else cat"));
    }

    #[test]
    fn posix_stop_validates_identity_and_never_uses_marker_matching() {
        let command = make_stop_command(
            "/tmp/miaominal-agent-00000000-0000-0000-0000-000000000000/status",
            ShellType::Posix,
        );
        assert!(command.contains("monitor_identity"));
        assert!(command.contains("child_pgid"));
        assert!(command.contains("terminate_group"));
        assert!(command.contains("job processes survived stop verification"));
        let identity_guard = command
            .find("job monitor and child identities are no longer verifiable")
            .expect("missing fail-closed identity guard");
        let stop_marker = command
            .find("stop_tmp=")
            .expect("missing stop marker write");
        let group_signal = command
            .find("if group_alive \"$child_pgid\" && ! terminate_group")
            .expect("missing process-group termination");
        assert!(identity_guard < stop_marker);
        assert!(identity_guard < group_signal);
        assert!(command.contains("ln"));
        assert!(!command.contains("pkill -f"));
    }

    #[test]
    fn legacy_posix_stop_fails_closed() {
        let command = make_stop_command(
            "/tmp/miaominal-agent-00000000-0000-0000-0000-000000000000.status",
            ShellType::Posix,
        );
        assert!(command.contains("legacy POSIX jobs cannot be stopped safely"));
        assert!(command.contains("exit 1"));
    }

    #[test]
    fn cleanup_and_scavenge_only_target_miaominal_artifacts() {
        let marker = "/tmp/miaominal-agent-00000000-0000-0000-0000-000000000000/status";
        let cleanup = make_cleanup_command(marker, ShellType::Posix);
        assert!(cleanup.contains("rmdir"));
        assert!(cleanup.contains(marker));
        assert!(!cleanup.contains("rm -rf"));

        let posix_scavenge = make_scavenge_command(ShellType::Posix);
        assert!(posix_scavenge.contains("-mmin +1440"));
        assert!(posix_scavenge.contains("*[!0-9a-fA-F-]*"));

        let windows_scavenge = make_scavenge_command(ShellType::PowerShell);
        assert!(windows_scavenge.len() < 8_191);
        assert!(windows_scavenge.starts_with("powershell.exe -NoProfile -EncodedCommand "));
    }

    #[test]
    fn cmd_rejects_generated_commands_at_the_platform_limit() {
        let oversized = "x".repeat(WINDOWS_CMD_MAX_COMMAND_BYTES);
        assert!(ensure_windows_command_fits(&oversized, ShellType::Cmd).is_err());
        assert!(ensure_windows_command_fits(&oversized, ShellType::PowerShell).is_ok());
    }

    #[test]
    fn windows_start_uses_independent_process_monitor() {
        for shell_type in [ShellType::PowerShell, ShellType::Cmd] {
            let script = make_windows_start_script(
                shell_type,
                r"C:\Users\user\My Project",
                "echo hello",
                r"%TEMP%\miaominal-agent-test.status",
            );
            assert!(script.contains("MiaominalDetachedProcess"));
            assert!(script.contains("runner.ps1"));
            assert!(!script.contains("Start-Job"));
            assert!(!script.contains(r"\tmp\"));
        }
    }

    #[test]
    fn windows_launcher_resolves_relative_cwd_before_starting_monitor() {
        let script = make_windows_start_script(
            ShellType::PowerShell,
            r"relative\project",
            "Write-Output (Get-Location).Path",
            r"%TEMP%\miaominal-agent-test.status",
        );

        assert!(script.contains("ExpandEnvironmentVariables('relative\\project')"));
        assert!(script.contains("$cwdPath=Join-Path $env:USERPROFILE $requestedCwd"));
        assert!(script.contains("Get-Item -LiteralPath $cwdPath"));
        assert!(script.contains("$cwdItem.PSIsContainer"));
        assert!(script.contains("$resolvedCwd=$cwdItem.FullName"));
        assert!(script.contains("SetEnvironmentVariable($cwdEnvName,$resolvedCwd,'Process')"));
        assert!(script.contains("GetEnvironmentVariable"));
        assert!(script.contains("MIAOMINAL_AGENT_JOB_CWD"));
        assert!(script.contains("Remove-Item Env:MIAOMINAL_AGENT_JOB_CWD"));
        assert!(script.contains("$psi.WorkingDirectory=$workingDirectory"));
        assert!(!script.contains("$psi.WorkingDirectory='relative\\project'"));
    }

    #[test]
    fn windows_launcher_failure_stops_monitor_and_retries_artifact_cleanup() {
        let script = make_windows_start_script(
            ShellType::PowerShell,
            ".",
            "Write-Output 'hello'",
            r"%TEMP%\miaominal-agent-test.status",
        );

        assert!(script.contains("$monitorStartTicks"));
        assert!(script.contains("taskkill.exe /T /F /PID $processId"));
        assert!(script.contains("$process.WaitForExit(5000)"));
        assert!(script.contains("$process.Kill()"));
        assert!(script.contains("$childStartTicks=[int64]$process.StartTime"));
        assert!(script.contains("taskkill.exe /T /F /PID $child.Id"));
        assert!(script.contains("$child.WaitForExit(5000)"));
        assert!(script.contains("$i -lt 1000"));
        assert!(script.contains("Remove-MiaominalLaunchArtifacts; Start-Sleep -Milliseconds 100; Remove-MiaominalLaunchArtifacts"));
        assert!(script.contains("$leaf+'.pid.tmp-*'"));
        assert!(script.contains("SetEnvironmentVariable($cwdEnvName,$previousCwdEnv,'Process')"));
        assert!(script.contains("artifacts were preserved for scavenging"));
        let add_type = script.find("Add-Type -TypeDefinition").unwrap();
        let write_runner = script.find("[IO.File]::WriteAllText($runner").unwrap();
        assert!(add_type < write_runner);
    }

    #[test]
    fn windows_monitor_publishes_its_identity_before_starting_child() {
        let script = make_windows_start_script(
            ShellType::PowerShell,
            ".",
            "Write-Output 'hello'",
            r"%TEMP%\miaominal-agent-test.status",
        );

        let initial_metadata = script
            .find("$monitorMetadata=@{pid=$self.Id")
            .expect("monitor-only metadata");
        let child_start = script.find("[void]$process.Start()").expect("child start");
        let child_metadata = script
            .find("$monitorMetadata[''child_pid'']=$process.Id")
            .expect("child metadata update");
        assert!(initial_metadata < child_start);
        assert!(child_start < child_metadata);
        assert!(script.contains("Publish-MiaominalPidMetadata $monitorMetadata"));
    }

    #[test]
    fn cmd_child_preserves_nested_powershell_command_quotes() {
        let user_command = r#"powershell.exe -NoProfile -Command "Write-Output 'nested value'""#;
        let (program, arguments) = windows_child_command(ShellType::Cmd, user_command);

        assert_eq!(program, "cmd.exe");
        assert_eq!(arguments, format!("/d /v:off /s /c {user_command}"));
        assert!(!arguments.contains(r#"\""#));
    }

    #[test]
    fn cmd_child_adds_outer_quotes_only_for_a_quoted_executable() {
        let user_command = r#""C:\Program Files\PowerShell\powershell.exe" -NoProfile -Command "Write-Output 'quoted path'""#;
        let (_, arguments) = windows_child_command(ShellType::Cmd, user_command);

        assert_eq!(arguments, format!("/d /v:off /s /c \"{user_command}\""));
        assert!(arguments.contains(r#"-Command "Write-Output 'quoted path'""#));
    }

    #[test]
    fn windows_poll_reads_only_bounded_file_tail() {
        let script = make_windows_poll_script(r"%TEMP%\miaominal-agent-test.status");
        assert!(script.contains("Read-MiaominalTail"));
        assert!(script.contains("[IO.File]::Open"));
        assert!(script.contains(&DEFAULT_MAX_OUTPUT_BYTES.to_string()));
        assert!(script.contains("start_ticks"));
        assert!(script.contains("stdout_b64"));
        assert!(!script.contains("ReadAllBytes"));
        assert!(!script.contains("Get-Job"));
        assert!(!script.contains("Receive-Job"));
    }

    #[test]
    fn windows_stop_validates_pid_and_kills_process_tree() {
        let script = make_windows_stop_script(r"%TEMP%\miaominal-agent-test.status");
        assert!(script.contains("start_ticks"));
        assert!(script.contains("taskkill.exe /T /F /PID"));
        assert!(script.contains("WriteAllText"));
        assert!(!script.contains("Stop-Job"));
        let command = make_stop_command(
            r"%TEMP%\miaominal-agent-00000000-0000-0000-0000-000000000000.status",
            ShellType::Cmd,
        );
        assert!(
            command.len() < WINDOWS_CMD_MAX_COMMAND_BYTES,
            "stop command was {} bytes",
            command.len()
        );
    }

    #[test]
    fn parses_exited_job_poll_output() {
        let job_id = AgentJobId::new();
        let result = parse_poll_output(
            job_id.clone(),
            "status=exited\nexit=0\ntruncated=0\nstdout<<EOF\nhello\nEOF\nstderr<<EOF\nwarn\nEOF",
        )
        .unwrap();

        assert_eq!(result.job_id, job_id);
        assert_eq!(result.status, JobStatus::Exited);
        assert_eq!(result.exit_status, Some(0));
        assert_eq!(result.stdout, "hello");
        assert_eq!(result.stderr, "warn");
        assert!(!result.truncated);
    }

    #[test]
    fn parses_crlf_and_truncation_flag() {
        let result = parse_poll_output(
            AgentJobId::new(),
            "status=running\r\ntruncated=1\r\nstdout<<EOF\r\nlatest\r\nEOF\r\nstderr<<EOF\r\n\r\nEOF\r\n",
        )
        .unwrap();

        assert_eq!(result.status, JobStatus::Running);
        assert_eq!(result.stdout, "latest");
        assert!(result.truncated);
    }

    #[test]
    fn base64_framing_preserves_eof_lines_and_diagnostics() {
        let stdout = base64::engine::general_purpose::STANDARD.encode(b"before\nEOF\nafter");
        let stderr = base64::engine::general_purpose::STANDARD.encode(b"warning");
        let diagnostic = base64::engine::general_purpose::STANDARD.encode(b"process disappeared");
        let output = format!(
            "status=exited\ntruncated=0\nstdout_b64={stdout}\nstderr_b64={stderr}\ndiagnostic_b64={diagnostic}\n"
        );
        let result = parse_poll_output(AgentJobId::new(), &output).unwrap();

        assert_eq!(result.stdout, "before\nEOF\nafter");
        assert_eq!(result.stderr, "warning\nprocess disappeared");
    }

    #[test]
    fn base64_tail_drops_partial_utf8_prefix_and_stays_bounded() {
        let mut bytes = vec![0x82, 0xac];
        let valid_prefix = "最新日志🚀".as_bytes();
        bytes.extend_from_slice(valid_prefix);
        bytes.extend(std::iter::repeat_n(
            b'x',
            DEFAULT_MAX_OUTPUT_BYTES - 2 - valid_prefix.len(),
        ));
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        let output = format!("stdout_b64={encoded}\n");
        let text = encoded_output_field(&output, "stdout_b64")
            .unwrap()
            .unwrap();

        assert!(text.starts_with("最新日志🚀"));
        assert!(text.ends_with('x'));
        assert!(!text.contains('\u{fffd}'));
        assert!(text.len() <= DEFAULT_MAX_OUTPUT_BYTES);
        assert!(text.is_char_boundary(text.len()));
    }

    #[test]
    fn parses_missing_job_poll_output() {
        let result = parse_poll_output(AgentJobId::new(), "status=not_found\n").unwrap();

        assert_eq!(result.status, JobStatus::NotFound);
        assert_eq!(result.exit_status, None);
        assert_eq!(result.stdout, "");
        assert_eq!(result.stderr, "");
        assert!(!result.truncated);
    }

    #[test]
    fn parses_stopped_job_poll_output() {
        let result = parse_poll_output(
            AgentJobId::new(),
            "status=stopped\ntruncated=0\nstdout<<EOF\npartial\nEOF\nstderr<<EOF\n\nEOF",
        )
        .unwrap();

        assert_eq!(result.status, JobStatus::Stopped);
        assert_eq!(result.exit_status, None);
        assert_eq!(result.stdout, "partial");
    }

    #[test]
    fn heredoc_extracts_named_section() {
        let output = "stdout<<EOF\nline1\nline2\nEOF\nstderr<<EOF\nerr1\nEOF";
        assert_eq!(heredoc_section(output, "stdout").unwrap(), "line1\nline2");
        assert_eq!(heredoc_section(output, "stderr").unwrap(), "err1");
        assert!(heredoc_section(output, "missing").is_none());
    }

    #[test]
    fn fish_start_job_wraps_posix_management_in_sh() {
        let cmd = make_start_job_command(
            ShellType::Fish,
            "/home/user/project",
            "echo fish",
            "/tmp/miaominal-agent-00000000-0000-0000-0000-000000000000/status",
        );
        assert!(cmd.starts_with("sh -lc "));
        assert!(cmd.contains("nohup sh"));
        assert!(cmd.contains("setsid sh"));
    }

    #[cfg(unix)]
    fn execute_posix_command(command: &str) -> std::process::Output {
        std::process::Command::new("sh")
            .args(["-lc", command])
            .output()
            .expect("execute generated POSIX command")
    }

    #[cfg(unix)]
    #[test]
    fn posix_job_uses_private_permissions_and_stops_the_process_group() {
        use std::os::unix::fs::PermissionsExt;
        use std::path::Path;
        use std::time::{Duration, Instant};

        if !execute_posix_command("command -v setsid >/dev/null 2>&1")
            .status
            .success()
        {
            return;
        }

        let job_id = AgentJobId::new();
        let marker = job_id.remote_marker_for_shell(ShellType::Posix).unwrap();
        let start = make_start_job_command(
            ShellType::Posix,
            ".",
            "umask; sh -c 'sleep 30 & wait'",
            &marker,
        );
        let start_output = execute_posix_command(&format!("umask 022; {start}"));
        assert!(
            start_output.status.success(),
            "job start failed: {}",
            String::from_utf8_lossy(&start_output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&start_output.stdout).trim(), marker);

        let paths = PosixJobPaths::from_marker(&marker).unwrap();
        assert_eq!(
            std::fs::metadata(&paths.root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for path in [&paths.stdout, &paths.stderr, &paths.pid, &paths.ready] {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600,
                "unexpected permissions for {path}"
            );
        }

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let stdout = std::fs::read_to_string(&paths.stdout).unwrap_or_default();
            if stdout.contains("0022") || stdout.lines().any(|line| line.trim() == "022") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "user command did not preserve umask 022"
            );
            std::thread::sleep(Duration::from_millis(50));
        }

        let metadata = std::fs::read_to_string(&paths.pid).unwrap();
        let child_pgid = metadata
            .lines()
            .find_map(|line| line.strip_prefix("child_pgid="))
            .unwrap()
            .parse::<u32>()
            .unwrap();
        assert!(Path::new(&paths.runner).exists());

        let stop_output = execute_posix_command(&make_stop_command(&marker, ShellType::Posix));
        assert!(
            stop_output.status.success(),
            "job stop failed: {}",
            String::from_utf8_lossy(&stop_output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&stop_output.stdout).trim(),
            "stopped"
        );
        assert_eq!(std::fs::read_to_string(&paths.status).unwrap(), "stopped");
        assert!(
            !execute_posix_command(&format!("kill -0 -- -{child_pgid} 2>/dev/null"))
                .status
                .success(),
            "job process group survived stop"
        );

        let cleanup = execute_posix_command(&make_cleanup_command(&marker, ShellType::Posix));
        assert!(cleanup.status.success());
        assert!(!Path::new(&paths.root).exists());
    }

    #[cfg(windows)]
    fn windows_command_output(command: &str, shell_type: ShellType) -> std::process::Output {
        windows_command_output_with_user_profile(command, shell_type, None)
    }

    #[cfg(windows)]
    fn windows_command_output_with_user_profile(
        command: &str,
        shell_type: ShellType,
        user_profile: Option<&std::path::Path>,
    ) -> std::process::Output {
        let mut process = match shell_type {
            ShellType::Cmd => {
                let mut process = std::process::Command::new("cmd.exe");
                process.args(["/d", "/c", command]);
                process
            }
            ShellType::PowerShell => {
                let payload = command
                    .strip_prefix("powershell.exe -NoProfile -EncodedCommand ")
                    .expect("generated PowerShell command prefix");
                let mut process = std::process::Command::new("powershell.exe");
                process.args(["-NoProfile", "-EncodedCommand", payload]);
                process
            }
            ShellType::Posix | ShellType::Fish => unreachable!("Windows integration shell"),
        };
        if let Some(user_profile) = user_profile {
            process.env("USERPROFILE", user_profile);
        }
        process.output().expect("execute generated command")
    }

    #[cfg(windows)]
    fn execute_windows_command(command: &str, shell_type: ShellType) -> String {
        let output = windows_command_output(command, shell_type);
        assert!(
            output.status.success(),
            "command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[cfg(windows)]
    fn run_windows_job(shell_type: ShellType, command: &str, expected_exit: i32) -> JobPollResult {
        run_windows_job_in_cwd(shell_type, ".", command, expected_exit)
    }

    #[cfg(windows)]
    fn run_windows_job_in_cwd(
        shell_type: ShellType,
        cwd: &str,
        command: &str,
        expected_exit: i32,
    ) -> JobPollResult {
        run_windows_job_in_cwd_with_user_profile(shell_type, cwd, command, expected_exit, None)
    }

    #[cfg(windows)]
    fn run_windows_job_in_cwd_with_user_profile(
        shell_type: ShellType,
        cwd: &str,
        command: &str,
        expected_exit: i32,
        user_profile: Option<&std::path::Path>,
    ) -> JobPollResult {
        use std::time::{Duration, Instant};

        let job_id = AgentJobId::new();
        let logical_marker = job_id.remote_marker_for_shell(shell_type).unwrap();
        let start = make_start_job_command(shell_type, cwd, command, &logical_marker);
        if shell_type == ShellType::Cmd {
            assert!(
                start.len() < WINDOWS_CMD_MAX_COMMAND_BYTES,
                "start command was {} bytes",
                start.len()
            );
        }
        let start_output =
            windows_command_output_with_user_profile(&start, shell_type, user_profile);
        assert!(
            start_output.status.success(),
            "job start failed: {}",
            String::from_utf8_lossy(&start_output.stderr)
        );
        let marker = String::from_utf8_lossy(&start_output.stdout)
            .trim()
            .to_string();
        assert!(
            marker.to_ascii_lowercase().contains("\\temp\\"),
            "unexpected launcher output: {marker:?}"
        );
        assert!(
            marker
                .to_ascii_lowercase()
                .starts_with(&std::env::temp_dir().to_string_lossy().to_ascii_lowercase()),
            "marker was outside the Windows temp directory: {marker}"
        );

        let deadline = Instant::now() + Duration::from_secs(10);
        let result = loop {
            let poll_command = make_poll_command(&marker, shell_type);
            if shell_type == ShellType::Cmd {
                assert!(
                    poll_command.len() < WINDOWS_CMD_MAX_COMMAND_BYTES,
                    "poll command was {} bytes",
                    poll_command.len()
                );
            }
            let poll = execute_windows_command(&poll_command, shell_type);
            let result = parse_poll_output(job_id.clone(), &poll).unwrap();
            if result.status == JobStatus::Exited {
                break result;
            }
            assert_eq!(result.status, JobStatus::Running);
            assert!(Instant::now() < deadline, "job did not exit in time");
            std::thread::sleep(Duration::from_millis(100));
        };

        assert!(std::path::Path::new(&marker).exists());
        let cleanup = make_cleanup_command(&marker, shell_type);
        if shell_type == ShellType::Cmd {
            assert!(
                cleanup.len() < WINDOWS_CMD_MAX_COMMAND_BYTES,
                "cleanup command was {} bytes",
                cleanup.len()
            );
        }
        execute_windows_command(&cleanup, shell_type);
        assert!(!std::path::Path::new(&marker).exists());
        assert!(!std::path::Path::new(&format!("{marker}.out")).exists());
        assert!(!std::path::Path::new(&format!("{marker}.err")).exists());
        assert_eq!(
            result.exit_status,
            Some(expected_exit),
            "unexpected job result: status={:?}, stdout={:?}, stderr={:?}",
            result.status,
            result.stdout,
            result.stderr
        );
        result
    }

    #[cfg(windows)]
    #[test]
    fn windows_job_survives_separate_powershell_poll_processes() {
        let result = run_windows_job(
            ShellType::PowerShell,
            "Start-Sleep -Milliseconds 500; Write-Output 'hello'; exit 7",
            7,
        );
        assert_eq!(result.stdout.trim(), "hello");
        assert!(!result.truncated);
    }

    #[cfg(windows)]
    #[test]
    fn windows_job_dot_cwd_is_resolved_from_user_profile() {
        let user_profile =
            std::env::temp_dir().join(format!("miaominal-job-home-{}", AgentJobId::new().0));
        std::fs::create_dir_all(&user_profile).unwrap();
        let expected_cwd = user_profile.canonicalize().unwrap();
        let result = run_windows_job_in_cwd_with_user_profile(
            ShellType::PowerShell,
            ".",
            "[Console]::Out.Write([Environment]::CurrentDirectory); exit 0",
            0,
            Some(&user_profile),
        );
        let actual_cwd = std::path::Path::new(result.stdout.trim())
            .canonicalize()
            .unwrap();
        let _ = std::fs::remove_dir_all(&user_profile);

        assert_eq!(actual_cwd, expected_cwd);
    }

    #[cfg(windows)]
    #[test]
    fn windows_job_relative_cwd_is_resolved_from_user_profile() {
        let user_profile =
            std::env::temp_dir().join(format!("miaominal-job-home-{}", AgentJobId::new().0));
        let relative_cwd = format!("miaominal-job-cwd-{}", AgentJobId::new().0);
        let directory = user_profile.join(&relative_cwd);
        std::fs::create_dir_all(&directory).unwrap();
        let expected_cwd = directory.canonicalize().unwrap();
        let result = run_windows_job_in_cwd_with_user_profile(
            ShellType::PowerShell,
            &relative_cwd,
            "[Console]::Out.Write([Environment]::CurrentDirectory); exit 0",
            0,
            Some(&user_profile),
        );
        let actual_cwd = std::path::Path::new(result.stdout.trim())
            .canonicalize()
            .unwrap();
        let _ = std::fs::remove_dir_all(&user_profile);

        assert_eq!(actual_cwd, expected_cwd);
    }

    #[cfg(windows)]
    #[test]
    fn windows_launcher_timeout_cleans_monitor_and_artifacts() {
        use std::path::PathBuf;
        use std::process::Command;

        let job_id = AgentJobId::new();
        let marker = std::env::temp_dir()
            .join(format!("miaominal-agent-{}.status", job_id.0))
            .to_string_lossy()
            .into_owned();
        let monitor_pid_probe = format!("{marker}.test-monitor-pid");
        let mut launcher = make_windows_start_script(
            ShellType::PowerShell,
            ".",
            "Write-Output 'never reached'",
            &marker,
        );

        let monitor_start = "$ErrorActionPreference=''Stop''; $marker=";
        let stalled_monitor = "$ErrorActionPreference=''Stop''; Start-Sleep -Seconds 30; $marker=";
        let stalled_launcher = launcher.replacen(monitor_start, stalled_monitor, 1);
        assert_ne!(
            stalled_launcher, launcher,
            "monitor stall hook was not injected"
        );
        launcher = stalled_launcher;

        let detached_start = "$monitorPid=[MiaominalDetachedProcess]::Start($powershell,$monitorArgs,(Split-Path -Parent $runner)); ";
        let monitor_pid_probe_q = shell_quote(&monitor_pid_probe, ShellType::PowerShell);
        let instrumented_start = format!(
            "{detached_start}[IO.File]::WriteAllText({monitor_pid_probe_q},[string]$monitorPid); "
        );
        let instrumented_launcher = launcher.replacen(detached_start, &instrumented_start, 1);
        assert_ne!(
            instrumented_launcher, launcher,
            "monitor PID probe was not injected"
        );

        let command = super::super::windows::powershell_compressed_command(&instrumented_launcher);
        let output = windows_command_output(&command, ShellType::PowerShell);
        let diagnostic = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let timed_out = !output.status.success()
            && diagnostic.contains("job monitor failed to publish metadata");
        let monitor_pid: u32 = std::fs::read_to_string(&monitor_pid_probe)
            .expect("launcher should publish the test monitor PID")
            .trim()
            .parse()
            .expect("monitor PID should be numeric");
        let process_probe = format!(
            "if (Get-Process -Id {monitor_pid} -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}"
        );
        let monitor_alive = Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &process_probe])
            .status()
            .expect("probe monitor process")
            .success();
        let artifact_paths = [
            PathBuf::from(&marker),
            PathBuf::from(format!("{marker}.out")),
            PathBuf::from(format!("{marker}.err")),
            PathBuf::from(format!("{marker}.pid")),
            PathBuf::from(format!("{marker}.ctl.out")),
            PathBuf::from(format!("{marker}.ctl.err")),
            PathBuf::from(format!("{marker}.runner.ps1")),
        ];
        let leftovers = artifact_paths
            .iter()
            .filter(|path| path.exists())
            .cloned()
            .collect::<Vec<_>>();

        let _ = Command::new("taskkill.exe")
            .args(["/T", "/F", "/PID", &monitor_pid.to_string()])
            .output();
        for path in artifact_paths {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_file(&monitor_pid_probe);

        assert!(
            timed_out,
            "launcher did not enter the publication timeout path"
        );
        assert!(
            !monitor_alive && leftovers.is_empty(),
            "launcher timeout leaked monitor_alive={monitor_alive}, artifacts={leftovers:?}, diagnostic={diagnostic}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_launcher_create_process_failure_removes_runner() {
        let job_id = AgentJobId::new();
        let marker = std::env::temp_dir()
            .join(format!("miaominal-agent-{}.status", job_id.0))
            .to_string_lossy()
            .into_owned();
        let mut launcher = make_windows_start_script(
            ShellType::PowerShell,
            ".",
            "Write-Output 'never reached'",
            &marker,
        );
        let real_powershell = "$powershell=Join-Path $env:SystemRoot 'System32\\WindowsPowerShell\\v1.0\\powershell.exe'; ";
        let missing_powershell = format!(
            "$powershell=Join-Path $env:TEMP 'miaominal-agent-missing-{}.exe'; ",
            job_id.0
        );
        let replaced = launcher.replacen(real_powershell, &missing_powershell, 1);
        assert_ne!(replaced, launcher, "PowerShell path hook was not injected");
        launcher = replaced;

        let command = super::super::windows::powershell_compressed_command(&launcher);
        let output = windows_command_output(&command, ShellType::PowerShell);
        let diagnostic = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let artifacts = [
            std::path::PathBuf::from(&marker),
            std::path::PathBuf::from(format!("{marker}.out")),
            std::path::PathBuf::from(format!("{marker}.err")),
            std::path::PathBuf::from(format!("{marker}.pid")),
            std::path::PathBuf::from(format!("{marker}.runner.ps1")),
        ];
        let leftovers = artifacts
            .iter()
            .filter(|path| path.exists())
            .cloned()
            .collect::<Vec<_>>();
        for path in artifacts {
            let _ = std::fs::remove_file(path);
        }

        assert!(!output.status.success());
        assert!(diagnostic.contains("CreateProcess failed"), "{diagnostic}");
        assert!(
            leftovers.is_empty(),
            "CreateProcess failure leaked artifacts: {leftovers:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn cmd_job_uses_temp_artifacts_and_survives_separate_polls() {
        let result = run_windows_job(
            ShellType::Cmd,
            "ping -n 2 127.0.0.1 >nul & echo cmd-output & exit /b 9",
            9,
        );
        assert_eq!(result.stdout.trim(), "cmd-output");
    }

    #[cfg(windows)]
    #[test]
    fn cmd_job_preserves_nested_powershell_command_quotes_and_exit_status() {
        let result = run_windows_job(
            ShellType::Cmd,
            "powershell.exe -NoProfile -Command \"Start-Sleep -Milliseconds 1200; Write-Output 'JOB_STDOUT_OK'; [Console]::Error.WriteLine('JOB_STDERR_OK'); exit 7\"",
            7,
        );

        assert_eq!(result.stdout.trim(), "JOB_STDOUT_OK");
        assert_eq!(result.stderr.trim(), "JOB_STDERR_OK");
    }

    #[cfg(windows)]
    #[test]
    fn powershell_job_preserves_explicit_nested_powershell_exit_status() {
        let result = run_windows_job(
            ShellType::PowerShell,
            "powershell.exe -NoProfile -Command \"Start-Sleep -Milliseconds 1200; Write-Output 'JOB_STDOUT_OK'; [Console]::Error.WriteLine('JOB_STDERR_OK'); exit 7\"",
            7,
        );

        assert_eq!(result.stdout.trim(), "JOB_STDOUT_OK");
        assert!(result.stderr.contains("JOB_STDERR_OK"));
    }

    #[cfg(windows)]
    #[test]
    fn cmd_job_supports_a_quoted_executable_path() {
        let result = run_windows_job(
            ShellType::Cmd,
            r#""%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -Command "Write-Output 'QUOTED_EXE_OK'; exit 5""#,
            5,
        );

        assert_eq!(result.stdout.trim(), "QUOTED_EXE_OK");
    }

    #[cfg(windows)]
    #[test]
    fn windows_job_poll_caps_both_output_streams() {
        let result = run_windows_job(
            ShellType::PowerShell,
            "[Console]::Out.Write(('A'*70000)); [Console]::Error.Write(('B'*70000)); exit 0",
            0,
        );

        assert!(result.truncated);
        assert_eq!(result.stdout.len(), DEFAULT_MAX_OUTPUT_BYTES);
        assert_eq!(result.stderr.len(), DEFAULT_MAX_OUTPUT_BYTES);
        assert!(result.stdout.bytes().all(|byte| byte == b'A'));
        assert!(result.stderr.bytes().all(|byte| byte == b'B'));
    }

    #[cfg(windows)]
    #[test]
    fn windows_stop_kills_job_tree_and_preserves_stopped_status() {
        use std::time::{Duration, Instant};

        let job_id = AgentJobId::new();
        let logical_marker = job_id
            .remote_marker_for_shell(ShellType::PowerShell)
            .unwrap();
        let start = make_start_job_command(
            ShellType::PowerShell,
            ".",
            "Start-Sleep -Seconds 30",
            &logical_marker,
        );
        let started_at = Instant::now();
        let marker = execute_windows_command(&start, ShellType::PowerShell);
        assert!(
            started_at.elapsed() < Duration::from_secs(5),
            "background launcher waited for the 30-second job"
        );
        let metadata: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(format!("{marker}.pid")).unwrap())
                .unwrap();
        let monitor_pid = metadata["pid"].as_i64().unwrap();
        let stopped = execute_windows_command(
            &make_stop_command(&marker, ShellType::PowerShell),
            ShellType::PowerShell,
        );
        assert_eq!(stopped.trim(), "stopped");
        let process_gone = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "if (Get-Process -Id {monitor_pid} -ErrorAction SilentlyContinue) {{ exit 1 }} else {{ exit 0 }}"
                ),
            ])
            .status()
            .unwrap();
        assert!(process_gone.success(), "monitor process survived stop_job");

        let poll = execute_windows_command(
            &make_poll_command(&marker, ShellType::PowerShell),
            ShellType::PowerShell,
        );
        let result = parse_poll_output(job_id, &poll).unwrap();
        assert_eq!(result.status, JobStatus::Stopped);
        assert_eq!(result.exit_status, None);
        execute_windows_command(
            &make_cleanup_command(&marker, ShellType::PowerShell),
            ShellType::PowerShell,
        );
        std::thread::sleep(Duration::from_millis(250));
        assert!(!std::path::Path::new(&marker).exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_scavenger_removes_old_terminal_artifacts() {
        use std::fs;
        use std::process::Command;

        let _guard = WINDOWS_SCAVENGE_TEST_LOCK.lock().unwrap();

        let job_id = AgentJobId::new();
        let marker = std::env::temp_dir().join(format!("miaominal-agent-{}.status", job_id.0));
        let out = format!("{}.out", marker.display());
        fs::write(&marker, b"0").unwrap();
        fs::write(&out, b"old output").unwrap();
        let age_script = format!(
            "(Get-Item -LiteralPath '{}').LastWriteTimeUtc=[DateTime]::UtcNow.AddHours(-25)",
            marker.display().to_string().replace('\'', "''")
        );
        let aged = Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &age_script])
            .status()
            .unwrap();
        assert!(aged.success());

        let output = execute_windows_command(
            &make_scavenge_command(ShellType::PowerShell),
            ShellType::PowerShell,
        );
        assert!(output.contains(&format!("cleaned={}", job_id.0)));
        assert!(!marker.exists());
        assert!(!std::path::Path::new(&out).exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_scavenger_removes_old_runner_only_and_pid_temp_artifacts() {
        use std::fs;
        use std::process::Command;

        let _guard = WINDOWS_SCAVENGE_TEST_LOCK.lock().unwrap();

        let job_id = AgentJobId::new();
        let marker = std::env::temp_dir().join(format!("miaominal-agent-{}.status", job_id.0));
        let runner = format!("{}.runner.ps1", marker.display());
        let pid_tmp = format!("{}.pid.tmp-deadbeef", marker.display());
        fs::write(&runner, b"stale runner").unwrap();
        fs::write(&pid_tmp, b"stale metadata").unwrap();
        let runner_q = runner.replace('\'', "''");
        let pid_tmp_q = pid_tmp.replace('\'', "''");
        let age_script = format!(
            "Get-Item -LiteralPath @('{runner_q}','{pid_tmp_q}') | ForEach-Object {{ $_.LastWriteTimeUtc=[DateTime]::UtcNow.AddHours(-25) }}"
        );
        let aged = Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &age_script])
            .status()
            .unwrap();
        assert!(aged.success());

        let output = execute_windows_command(
            &make_scavenge_command(ShellType::PowerShell),
            ShellType::PowerShell,
        );

        assert!(output.contains(&format!("cleaned={}", job_id.0)));
        assert!(!std::path::Path::new(&runner).exists());
        assert!(!std::path::Path::new(&pid_tmp).exists());
    }
}
