use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::AgentResult;
use crate::jobs::AgentJobId;
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
    let marker = format!("/tmp/miaominal-agent-{}.status", uuid::Uuid::new_v4());
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
    let job_id = channel.jobs().insert_remote_job(args.command, marker);
    Ok(ToolOutput::JobStarted { job_id })
}

pub async fn poll_job(channel: &AgentExecChannel, args: PollJobArgs) -> AgentResult<ToolOutput> {
    let marker = channel.jobs().remote_marker(&args.job_id)?;
    let command = format!(
        "if [ -f {status} ]; then printf 'status=exited\\nexit='; cat {status}; printf '\\nstdout<<EOF\\n'; cat {out} 2>/dev/null; printf '\\nEOF\\nstderr<<EOF\\n'; cat {err} 2>/dev/null; printf '\\nEOF\\n'; else printf 'status=running\\n'; fi",
        status = shell_quote(&marker),
        out = shell_quote(&format!("{marker}.out")),
        err = shell_quote(&format!("{marker}.err")),
    );
    Ok(ToolOutput::Text {
        content: channel.exec(command).await?,
        truncated: false,
    })
}

pub async fn stop_job(channel: &AgentExecChannel, args: StopJobArgs) -> AgentResult<ToolOutput> {
    let marker = channel.jobs().remote_marker(&args.job_id)?;
    let command = format!(
        "pkill -f {marker} 2>/dev/null || true; printf 'stopped\\n'",
        marker = shell_quote(&marker),
    );
    channel.jobs().remove(&args.job_id)?;
    Ok(ToolOutput::Text {
        content: channel.exec(command).await?,
        truncated: false,
    })
}
