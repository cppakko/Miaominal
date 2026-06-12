use crate::channel::{AgentExecChannel, DEFAULT_MAX_OUTPUT_BYTES, ShellCommandResult, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::path_guard::{resolve_workspace_path, shell_quote};
use anyhow::anyhow;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct RunShellArgs {
    pub command: String,
    pub cwd: Option<String>,
    pub timeout_seconds: Option<u64>,
    pub max_bytes: Option<usize>,
    pub shell: Option<String>,
}

pub async fn run_shell(channel: &AgentExecChannel, args: RunShellArgs) -> AgentResult<ToolOutput> {
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
    let timeout_secs = args.timeout_seconds.unwrap_or(20).max(1);
    let max_bytes = args.max_bytes.unwrap_or(DEFAULT_MAX_OUTPUT_BYTES);
    let shell = args.shell.as_deref().unwrap_or("posix-sh");
    if shell != "posix-sh" && shell != "sh" {
        return Err(AgentError::PosixOnly(
            "run_shell v1 only supports posix-sh".into(),
        ));
    }
    let command = format!(
        concat!(
            "cd \"$HOME\" && cd {cwd} && ",
            "export PAGER=cat SYSTEMD_PAGER= GIT_PAGER=cat LESS= LANG=C.UTF-8; ",
            "out=$(mktemp) && err=$(mktemp) && ",
            "timeout {timeout_secs} sh -lc {user_command} >\"$out\" 2>\"$err\"; ",
            "miaominal_status=$?; ",
            "printf 'MIAOMINAL_STATUS=%s\\n' \"$miaominal_status\"; ",
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
        cwd = shell_quote(&cwd),
        timeout_secs = timeout_secs,
        user_command = shell_quote(&args.command),
        max = max_bytes,
    );
    let output = if channel.has_pty_handle() {
        channel.exec_pty(command, timeout_secs + 5).await?
    } else {
        channel.exec(command).await?
    };
    let result = parse_shell_result(&output)?;
    Ok(ToolOutput::Shell { result })
}

pub fn parse_shell_result(output: &str) -> AgentResult<ShellCommandResult> {
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

    #[test]
    fn shell_result_parser_marks_timeout_exit_code() {
        let output = concat!(
            "MIAOMINAL_STATUS=124\n",
            "MIAOMINAL_STDOUT_BEGIN\n",
            "\nMIAOMINAL_STDOUT_END\n",
            "MIAOMINAL_STDERR_BEGIN\n",
            "\nMIAOMINAL_STDERR_END\n",
            "MIAOMINAL_TRUNCATED=0\n"
        );

        let result = parse_shell_result(output).unwrap();

        assert!(result.timed_out);
        assert!(!result.truncated);
    }
}
