use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::jobs::{AgentJobId, JobPollResult, JobStatus};
use crate::path_guard::{cd_prefix, resolve_workspace_path, shell_quote};
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

// ── Shell-dispatch helpers ──

/// Build the background job command for the given shell type.
///
/// Posix/Fish: `nohup sh -lc` with file redirection.
/// PowerShell: `Start-Job -Name {marker} -ScriptBlock {…}`.
/// CMD: `start /b` with file redirection.
fn make_start_job_command(
    shell_type: ShellType,
    cwd: &str,
    user_command: &str,
    marker: &str,
) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => {
            let cwd_q = shell_quote(cwd, shell_type);
            let cmd_q = shell_quote(user_command, shell_type);
            let marker_q = shell_quote(marker, shell_type);
            format!(
                "cd \"$HOME\" && cd {cwd_q} && nohup sh -lc {cmd_q} >{marker_q}.out 2>{marker_q}.err; printf $? >{marker_q}"
            )
        }
        ShellType::PowerShell => {
            let cd = cd_prefix(shell_type, cwd);
            let marker_q = shell_quote(marker, shell_type);
            let cmd_q = shell_quote(user_command, shell_type);
            let ps_script = format!(
                "{cd}; $job = Start-Job -Name {marker_q} -ScriptBlock ([ScriptBlock]::Create({cmd_q}))"
            );
            format!("powershell.exe -NoProfile -Command \"{ps_script}\"")
        }
        ShellType::Cmd => {
            let cd = cd_prefix(shell_type, cwd);
            // Use backslash separators so CMD built-ins (if exist, type, etc.)
            // don't misinterpret the leading / as a switch.
            let marker_win = marker.replace('/', "\\");
            format!(
                "start \"{marker}\" /b cmd /v:on /c \"{cd} && ({user_command}) > {m}.out 2> {m}.err & echo !ERRORLEVEL! > {m}\"",
                m = marker_win,
            )
        }
    }
}

/// Build the launch wrapper that runs the background command and echoes the
/// marker so the registry can store it.
fn make_start_job_launch(
    shell_type: ShellType,
    job_command: &str,
    marker: &str,
) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => {
            let marker_q = shell_quote(marker, shell_type);
            format!("({job_command}) >/dev/null 2>&1 & printf '%s' {marker_q}")
        }
        ShellType::PowerShell => {
            let marker_q = shell_quote(marker, shell_type);
            let ps_script = format!("{job_command}; Write-Output {marker_q}");
            format!("powershell.exe -NoProfile -Command \"{ps_script}\"")
        }
        ShellType::Cmd => {
            format!("{job_command} & echo {marker}")
        }
    }
}

/// Build the poll command for the given shell type.
/// All variants MUST emit the same structured output so `parse_poll_output`
/// can consume them: `status=…`, optional `exit=…`, `stdout<<EOF\n…\nEOF`,
/// `stderr<<EOF\n…\nEOF`.
fn make_poll_command(marker: &str, shell_type: ShellType) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => {
            let status = shell_quote(marker, shell_type);
            let out = shell_quote(&format!("{marker}.out"), shell_type);
            let err = shell_quote(&format!("{marker}.err"), shell_type);
            format!(
                "emit_streams() {{ printf 'stdout<<EOF\\n'; cat {out} 2>/dev/null; printf '\\nEOF\\nstderr<<EOF\\n'; cat {err} 2>/dev/null; printf '\\nEOF\\n'; }}; if [ -f {status} ]; then exit_status=$(cat {status}); if [ \"$exit_status\" = stopped ]; then printf 'status=stopped\\n'; else printf 'status=exited\\nexit=%s\\n' \"$exit_status\"; fi; emit_streams; elif [ -f {out} ] || [ -f {err} ]; then printf 'status=running\\n'; emit_streams; else printf 'status=not_found\\n'; fi"
            )
        }
        ShellType::PowerShell => {
            let marker_q = shell_quote(marker, shell_type);
            let ps_script = format!(
                "$job = Get-Job -Name {marker_q} -ErrorAction SilentlyContinue; if (-not $job) {{ Write-Output 'status=not_found' }} elseif ($job.State -eq 'Running') {{ Write-Output 'status=running'; Write-Output 'stdout<<EOF'; Receive-Job -Name {marker_q} -Keep; Write-Output 'EOF' }} else {{ $result = Receive-Job -Name {marker_q} -ErrorAction SilentlyContinue; Write-Output 'status=exited'; Write-Output ('exit=' + ($job.ExitCode ?? 0)); Write-Output 'stdout<<EOF'; $result; Write-Output 'EOF'; Write-Output 'stderr<<EOF'; Write-Output 'EOF'; Remove-Job -Name {marker_q} }}"
            );
            format!("powershell.exe -NoProfile -Command \"{ps_script}\"")
        }
        ShellType::Cmd => {
            let marker_win = marker.replace('/', "\\");
            format!(
                "cmd /v:on /c \"if exist {m} (set /p ec=<{m} & if \\!ec\\!==stopped (echo status=stopped) else (echo status=exited & echo exit=\\!ec\\!) & echo stdout<<EOF & type {m}.out 2>nul & echo EOF & echo stderr<<EOF & type {m}.err 2>nul & echo EOF) else (if exist {m}.out (echo status=running & echo stdout<<EOF & type {m}.out 2>nul & echo EOF & echo stderr<<EOF & type {m}.err 2>nul & echo EOF) else echo status=not_found)\"",
                m = marker_win,
            )
        }
    }
}

/// Build the stop command for the given shell type.
fn make_stop_command(marker: &str, shell_type: ShellType) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => {
            let marker_q = shell_quote(marker, shell_type);
            let status_q = shell_quote(marker, shell_type);
            format!(
                "pkill -f {marker_q} 2>/dev/null || true; printf 'stopped' >{status_q}; printf 'stopped\\n'"
            )
        }
        ShellType::PowerShell => {
            let marker_q = shell_quote(marker, shell_type);
            let ps_script = format!(
                "$job = Get-Job -Name {marker_q} -ErrorAction SilentlyContinue; if ($job) {{ Stop-Job -Name {marker_q}; Remove-Job -Name {marker_q} }}; Write-Output 'stopped'"
            );
            format!("powershell.exe -NoProfile -Command \"{ps_script}\"")
        }
        ShellType::Cmd => {
            let marker_win = marker.replace('/', "\\");
            format!(
                "taskkill /F /FI \"WINDOWTITLE eq {marker}\" 2>nul & echo stopped > {m} & echo stopped",
                marker = marker,
                m = marker_win,
            )
        }
    }
}

// ── Tool implementations ──

pub async fn start_job(channel: &AgentExecChannel, args: StartJobArgs) -> AgentResult<ToolOutput> {
    let cwd = resolve_workspace_path(args.cwd.as_deref().unwrap_or("."))?;
    channel
        .policy()
        .enforce_path(crate::policy::AgentPathAccess::Read, &cwd, true)?;
    if matches!(
        channel.policy().decide_command(&args.command, true),
        crate::policy::AgentPolicyDecision::Deny { .. }
    ) {
        channel.policy().enforce_command(&args.command, true)?;
    }
    let job_id = AgentJobId::new();
    let marker = job_id.remote_marker()?;
    let st = channel.shell_type();

    let command = make_start_job_command(st, &cwd, &args.command, &marker);
    let launch = make_start_job_launch(st, &command, &marker);

    let marker = channel
        .exec(launch)
        .await?
        .trim_matches('\'')
        .trim()
        .to_string();
    let job_id = channel
        .jobs()
        .insert_remote_job_with_id(job_id, args.command, marker);
    Ok(ToolOutput::JobStarted {
        job_id,
        poll_after_ms: DEFAULT_POLL_AFTER_MS,
        next_action: "Poll this job with poll_job until status is exited, or use list_jobs if you lose the job_id. Use run_shell instead of start_job for short commands."
            .into(),
    })
}

pub async fn list_jobs(channel: &AgentExecChannel) -> AgentResult<ToolOutput> {
    Ok(ToolOutput::JobList {
        jobs: channel.jobs().list()?,
    })
}

pub async fn poll_job(channel: &AgentExecChannel, args: PollJobArgs) -> AgentResult<ToolOutput> {
    let marker = channel.jobs().remote_marker(&args.job_id)?;
    let output = channel
        .exec(make_poll_command(&marker, channel.shell_type()))
        .await?;
    Ok(ToolOutput::JobPoll {
        result: parse_poll_output(args.job_id, &output)?,
    })
}

fn parse_poll_output(job_id: AgentJobId, output: &str) -> AgentResult<JobPollResult> {
    let status = output
        .lines()
        .find_map(|line| line.strip_prefix("status="))
        .ok_or_else(|| {
            AgentError::Backend(anyhow::anyhow!("job poll response is missing status"))
        })?;
    let exit_status = output
        .lines()
        .find_map(|line| line.strip_prefix("exit="))
        .and_then(|value| value.trim().parse::<i32>().ok());

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
        stdout: heredoc_section(output, "stdout").unwrap_or_default(),
        stderr: heredoc_section(output, "stderr").unwrap_or_default(),
    })
}

fn heredoc_section(output: &str, name: &str) -> Option<String> {
    let start = format!("{name}<<EOF\n");
    let after_start = output.split_once(&start)?.1;
    let section = after_start.split_once("\nEOF")?.0;
    Some(section.to_string())
}

pub async fn stop_job(channel: &AgentExecChannel, args: StopJobArgs) -> AgentResult<ToolOutput> {
    let marker = channel.jobs().remote_marker(&args.job_id)?;
    let command = make_stop_command(&marker, channel.shell_type());
    Ok(ToolOutput::Text {
        content: channel.exec(command).await?,
        truncated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── POSIX / Fish (unchanged paths) ──

    #[test]
    fn posix_start_job_command_uses_nohup_sh() {
        let cmd = make_start_job_command(
            ShellType::Posix,
            "/home/user/project",
            "echo hello",
            "/tmp/miaominal-agent-test.status",
        );
        assert!(cmd.contains("nohup sh -lc"), "expected nohup: {cmd}");
        // shell_quote wraps the marker in single quotes; .out / .err
        // are appended after the closing quote (valid shell concatenation).
        assert!(cmd.contains(".status'.out"), "expected .out redirect: {cmd}");
        assert!(cmd.contains(".status'.err"), "expected .err redirect: {cmd}");
    }

    #[test]
    fn posix_start_job_launch_backgrounds_and_prints_marker() {
        let launch = make_start_job_launch(
            ShellType::Posix,
            "nohup sh -lc 'echo hi'",
            "/tmp/marker.status",
        );
        assert!(launch.contains("&"), "expected background &: {launch}");
        assert!(launch.contains("printf '%s'"), "expected printf: {launch}");
    }

    #[test]
    fn poll_command_distinguishes_missing_job_from_running_job() {
        let command = make_poll_command("/tmp/miaominal-agent-job.status", ShellType::Posix);

        assert!(command.contains("status=running"));
        assert!(command.contains("status=not_found"));
        assert!(command.contains("elif [ -f '/tmp/miaominal-agent-job.status.out' ]"));
    }

    #[test]
    fn parses_exited_job_poll_output() {
        let job_id = AgentJobId::new();
        let result = parse_poll_output(
            job_id.clone(),
            "status=exited\nexit=0\nstdout<<EOF\nhello\nEOF\nstderr<<EOF\nwarn\nEOF",
        )
        .unwrap();

        assert_eq!(result.job_id, job_id);
        assert_eq!(result.status, JobStatus::Exited);
        assert_eq!(result.exit_status, Some(0));
        assert_eq!(result.stdout, "hello");
        assert_eq!(result.stderr, "warn");
    }

    #[test]
    fn parses_missing_job_poll_output() {
        let result = parse_poll_output(AgentJobId::new(), "status=not_found\n").unwrap();

        assert_eq!(result.status, JobStatus::NotFound);
        assert_eq!(result.exit_status, None);
        assert_eq!(result.stdout, "");
        assert_eq!(result.stderr, "");
    }

    #[test]
    fn parses_stopped_job_poll_output() {
        let job_id = AgentJobId::new();
        let result = parse_poll_output(job_id.clone(), "status=stopped\nstdout<<EOF\npartial\nEOF\nstderr<<EOF\n\nEOF").unwrap();

        assert_eq!(result.job_id, job_id);
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

    // ── PowerShell ──

    #[test]
    fn powershell_start_job_command_uses_start_job() {
        let cmd = make_start_job_command(
            ShellType::PowerShell,
            "C:\\Users\\user\\project",
            "Get-Process",
            "/tmp/miaominal-agent-ps.status",
        );
        assert!(cmd.contains("Start-Job"), "expected Start-Job: {cmd}");
        assert!(cmd.contains("[ScriptBlock]::Create"), "expected ScriptBlock: {cmd}");
        assert!(cmd.contains("Set-Location $env:USERPROFILE"), "expected cd prefix: {cmd}");
        assert!(!cmd.contains("nohup"), "PowerShell must not use nohup: {cmd}");
        assert!(!cmd.contains(">/dev/null"), "PowerShell must not use posix redirects: {cmd}");
    }

    #[test]
    fn powershell_start_job_launch_chains_write_output() {
        let launch = make_start_job_launch(
            ShellType::PowerShell,
            "Set-Location 'C:\\'; Start-Job -Name '/tmp/j.status' -ScriptBlock ([ScriptBlock]::Create('whoami'))",
            "/tmp/j.status",
        );
        assert!(launch.contains("Write-Output"), "expected Write-Output: {launch}");
        assert!(!launch.contains("&"), "PowerShell launch must not use posix &: {launch}");
    }

    #[test]
    fn powershell_poll_command_outputs_structured_format() {
        let cmd = make_poll_command("/tmp/j.status", ShellType::PowerShell);
        assert!(cmd.contains("Get-Job"), "expected Get-Job: {cmd}");
        assert!(cmd.contains("status=not_found"), "expected not_found: {cmd}");
        assert!(cmd.contains("status=running"), "expected running: {cmd}");
        assert!(cmd.contains("status=exited"), "expected exited: {cmd}");
        assert!(cmd.contains("Receive-Job"), "expected Receive-Job: {cmd}");
        assert!(cmd.contains("Remove-Job"), "expected Remove-Job: {cmd}");
        assert!(cmd.contains("stdout<<EOF"), "expected stdout heredoc: {cmd}");
        assert!(cmd.contains("stderr<<EOF"), "expected stderr heredoc: {cmd}");
    }

    #[test]
    fn powershell_stop_job_command_uses_stop_job() {
        let cmd = make_stop_command("/tmp/j.status", ShellType::PowerShell);
        assert!(cmd.contains("Stop-Job"), "expected Stop-Job: {cmd}");
        assert!(cmd.contains("Remove-Job"), "expected Remove-Job: {cmd}");
        assert!(cmd.contains("Write-Output 'stopped'"), "expected stopped output: {cmd}");
    }

    // ── CMD ──

    #[test]
    fn cmd_start_job_command_uses_start_b() {
        let cmd = make_start_job_command(
            ShellType::Cmd,
            "C:\\Users\\user\\project",
            "dir",
            "/tmp/miaominal-agent-cmd.status",
        );
        assert!(cmd.contains("start \"/tmp/miaominal-agent-cmd.status\" /b"), "expected start /b with title: {cmd}");
        assert!(cmd.contains("cmd /v:on /c"), "expected delayed expansion: {cmd}");
        assert!(cmd.contains("!ERRORLEVEL!"), "expected delayed ERRORLEVEL: {cmd}");
        // Backslash conversion for CMD built-ins
        assert!(cmd.contains("\\tmp\\miaominal-agent-cmd.status"), "expected backslash marker: {cmd}");
    }

    #[test]
    fn cmd_start_job_launch_echos_marker() {
        let launch = make_start_job_launch(
            ShellType::Cmd,
            "start ... /b cmd /c \"...\"",
            "/tmp/marker.status",
        );
        assert!(launch.contains("& echo /tmp/marker.status"), "expected echo marker: {launch}");
    }

    #[test]
    fn cmd_poll_command_outputs_structured_format() {
        let cmd = make_poll_command("/tmp/j.status", ShellType::Cmd);
        assert!(cmd.contains("cmd /v:on /c"), "expected delayed expansion wrapper: {cmd}");
        assert!(cmd.contains("status=not_found"), "expected not_found: {cmd}");
        assert!(cmd.contains("status=running"), "expected running: {cmd}");
        assert!(cmd.contains("status=exited"), "expected exited: {cmd}");
        assert!(cmd.contains("status=stopped"), "expected stopped: {cmd}");
        assert!(cmd.contains("stdout<<EOF"), "expected stdout heredoc: {cmd}");
        assert!(cmd.contains("stderr<<EOF"), "expected stderr heredoc: {cmd}");
    }

    #[test]
    fn cmd_stop_job_command_uses_taskkill() {
        let cmd = make_stop_command("/tmp/j.status", ShellType::Cmd);
        assert!(cmd.contains("taskkill"), "expected taskkill: {cmd}");
        assert!(cmd.contains("WINDOWTITLE eq /tmp/j.status"), "expected WINDOWTITLE filter: {cmd}");
        assert!(cmd.contains("echo stopped"), "expected stopped output: {cmd}");
    }

    // ── Cross-shell parse compat ──

    #[test]
    fn parse_poll_output_works_with_powershell_style_output() {
        // Simulate what the PowerShell poll command emits for an exited job.
        let job_id = AgentJobId::new();
        let output = "status=exited\nexit=0\nstdout<<EOF\nPS output\nEOF\nstderr<<EOF\n\nEOF\n";
        let result = parse_poll_output(job_id.clone(), output).unwrap();
        assert_eq!(result.status, JobStatus::Exited);
        assert_eq!(result.exit_status, Some(0));
        assert_eq!(result.stdout, "PS output");
    }

    #[test]
    fn parse_poll_output_works_with_running_no_exit_code() {
        // Running jobs have no exit= line.
        let job_id = AgentJobId::new();
        let output = "status=running\nstdout<<EOF\nlive\nEOF\nstderr<<EOF\n\nEOF\n";
        let result = parse_poll_output(job_id.clone(), output).unwrap();
        assert_eq!(result.status, JobStatus::Running);
        assert_eq!(result.exit_status, None);
        assert_eq!(result.stdout, "live");
    }

    #[test]
    fn fish_start_job_uses_nohup_like_posix() {
        let cmd = make_start_job_command(
            ShellType::Fish,
            "/home/user/project",
            "echo fish",
            "/tmp/fish.status",
        );
        assert!(cmd.contains("nohup sh -lc"), "Fish also uses nohup: {cmd}");
        // Fish uses `; and` for chaining in cd_prefix, but the job command
        // itself is still sh-based.
        assert!(cmd.contains("cd \"$HOME\""), "expected HOME cd: {cmd}");
    }
}
