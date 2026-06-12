use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::jobs::{AgentJobId, JobPollResult, JobStatus};
use crate::path_guard::{resolve_workspace_path, shell_quote};
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
    let command = format!(
        "cd \"$HOME\" && cd {cwd} && nohup sh -lc {} >{}.out 2>{}.err; printf $? >{}",
        shell_quote(&args.command),
        shell_quote(&marker),
        shell_quote(&marker),
        shell_quote(&marker),
        cwd = shell_quote(&cwd),
    );
    let launch = format!(
        "({command}) >/dev/null 2>&1 & printf '%s' {marker}",
        marker = shell_quote(&marker)
    );
    let marker = channel.exec(launch).await?.trim_matches('\'').to_string();
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
    let output = channel.exec(poll_command(&marker)).await?;
    Ok(ToolOutput::JobPoll {
        result: parse_poll_output(args.job_id, &output)?,
    })
}

fn poll_command(marker: &str) -> String {
    format!(
        "emit_streams() {{ printf 'stdout<<EOF\\n'; cat {out} 2>/dev/null; printf '\\nEOF\\nstderr<<EOF\\n'; cat {err} 2>/dev/null; printf '\\nEOF\\n'; }}; if [ -f {status} ]; then exit_status=$(cat {status}); if [ \"$exit_status\" = stopped ]; then printf 'status=stopped\\n'; else printf 'status=exited\\nexit=%s\\n' \"$exit_status\"; fi; emit_streams; elif [ -f {out} ] || [ -f {err} ]; then printf 'status=running\\n'; emit_streams; else printf 'status=not_found\\n'; fi",
        status = shell_quote(&marker),
        out = shell_quote(&format!("{marker}.out")),
        err = shell_quote(&format!("{marker}.err")),
    )
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
    let command = format!(
        "pkill -f {marker} 2>/dev/null || true; printf 'stopped' >{status}; printf 'stopped\\n'",
        marker = shell_quote(&marker),
        status = shell_quote(&marker),
    );
    Ok(ToolOutput::Text {
        content: channel.exec(command).await?,
        truncated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_command_distinguishes_missing_job_from_running_job() {
        let command = poll_command("/tmp/miaominal-agent-job.status");

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
}
