use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::path_guard::{cd_prefix, resolve_workspace_path, shell_quote};
use miaominal_core::profile::ShellType;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ApplyPatchArgs {
    pub patch: String,
    #[serde(default = "default_dot")]
    pub base_dir: String,
    pub validator: Option<PatchValidator>,
}

#[derive(Debug, Deserialize)]
pub struct PatchValidator {
    pub command: String,
}

pub async fn apply_patch(
    channel: &AgentExecChannel,
    args: ApplyPatchArgs,
) -> AgentResult<ToolOutput> {
    let base_dir = resolve_workspace_path(&args.base_dir)?;
    if crate::policy::is_sensitive_path(&base_dir) {
        channel
            .policy()
            .enforce_path(crate::policy::AgentPathAccess::Edit, &base_dir, false)?;
    }

    let shell = channel.shell_type();
    let command = build_patch_command(shell, &base_dir, &args.patch)?;

    let patch_output = match channel.exec(command).await {
        Ok(output) => output,
        Err(_) if matches!(shell, ShellType::PowerShell) => {
            return Err(AgentError::PosixOnly(
                "patch command not found on remote Windows host. \
                 Install Git for Windows (includes patch.exe) or \
                 use run_shell to apply changes manually."
                    .into(),
            ));
        }
        Err(e) => return Err(e),
    };

    if let Some(validator) = args.validator {
        channel.policy().enforce_command(&validator.command, true)?;
        let validation = super::run_shell::run_shell(
            channel,
            super::run_shell::RunShellArgs {
                command: validator.command,
                cwd: Some(base_dir),
                timeout_seconds: Some(60),
                max_bytes: None,
                shell: None,
            },
        )
        .await?;
        Ok(ToolOutput::Patch {
            summary: patch_output,
            validation: Some(Box::new(validation)),
        })
    } else {
        Ok(ToolOutput::Patch {
            summary: patch_output,
            validation: None,
        })
    }
}

fn build_patch_command(
    shell: ShellType,
    base_dir: &str,
    patch: &str,
) -> AgentResult<String> {
    match shell {
        ShellType::Posix | ShellType::Fish => {
            Ok(format!(
                "cd \"$HOME\" && cd {base_dir} && patch -p0 <<'MIAOMINAL_AGENT_PATCH'\n{patch}\nMIAOMINAL_AGENT_PATCH",
                base_dir = shell_quote(base_dir, shell),
                patch = patch,
            ))
        }
        ShellType::PowerShell => {
            let ps_script = format!(
                "{cd_prefix}\n@'\n{patch}\n'@ | & patch -p0",
                cd_prefix = cd_prefix(shell, base_dir),
                patch = patch,
            );
            Ok(format!("powershell.exe -NoProfile -Command \"{ps_script}\""))
        }
        ShellType::Cmd => {
            Err(AgentError::PosixOnly(
                "apply_patch is not supported in CMD sessions. \
                 Use a PowerShell session or install Git for Windows."
                    .into(),
            ))
        }
    }
}

fn default_dot() -> String {
    ".".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── build_patch_command tests ──

    #[test]
    fn posix_apply_patch_unchanged() {
        let cmd = build_patch_command(
            ShellType::Posix,
            "/home/user/project",
            "--- a/file.txt\n+++ b/file.txt\n@@ -1 +1 @@\n-old\n+new",
        )
        .unwrap();

        assert!(cmd.contains("<<'MIAOMINAL_AGENT_PATCH'"), "POSIX should use heredoc");
        assert!(cmd.contains("patch -p0"), "POSIX should call patch -p0");
        assert!(cmd.contains("cd \"$HOME\""), "POSIX should cd to HOME first");
        assert!(cmd.contains("MIAOMINAL_AGENT_PATCH"), "POSIX should use MIAOMINAL_AGENT_PATCH sentinel");
        // Verify heredoc content is embedded
        assert!(cmd.contains("--- a/file.txt"));
        assert!(cmd.contains("+++ b/file.txt"));
    }

    #[test]
    fn fish_apply_patch_uses_heredoc() {
        let cmd = build_patch_command(
            ShellType::Fish,
            "/home/user/project",
            "--- a/file.txt\n+++ b/file.txt\n@@ -1 +1 @@\n-old\n+new",
        )
        .unwrap();

        assert!(cmd.contains("<<'MIAOMINAL_AGENT_PATCH'"), "Fish should use heredoc");
        assert!(cmd.contains("patch -p0"), "Fish should call patch -p0");
        assert!(cmd.contains("cd \"$HOME\""), "Fish should cd to HOME first");
    }

    #[test]
    fn powershell_patch_unavailable_error() {
        // Test that PowerShell command uses here-string and patch invocation
        let cmd = build_patch_command(
            ShellType::PowerShell,
            "C:\\Users\\user\\project",
            "--- a/file.txt\n+++ b/file.txt\n@@ -1 +1 @@\n-old\n+new",
        )
        .unwrap();

        assert!(
            cmd.contains("@'\n"),
            "PowerShell should use here-string (@')"
        );
        assert!(cmd.contains("\n'@ | & patch -p0"), "PowerShell should pipe to patch");
        assert!(
            cmd.contains("Set-Location $env:USERPROFILE"),
            "PowerShell should use Set-Location to user profile"
        );
        assert!(
            !cmd.contains("<<'MIAOMINAL_AGENT_PATCH'"),
            "PowerShell should NOT use POSIX heredoc"
        );
    }

    #[test]
    fn cmd_returns_posix_only_error() {
        let result = build_patch_command(
            ShellType::Cmd,
            "C:\\Users\\user\\project",
            "some diff",
        );

        match result {
            Err(AgentError::PosixOnly(msg)) => {
                assert!(
                    msg.contains("CMD"),
                    "Error should mention CMD: {msg}"
                );
                assert!(
                    msg.contains("PowerShell"),
                    "Error should suggest PowerShell: {msg}"
                );
                assert!(
                    msg.contains("Git for Windows"),
                    "Error should suggest Git for Windows: {msg}"
                );
            }
            other => panic!("Expected PosixOnly error for CMD, got: {other:?}"),
        }
    }

    #[test]
    fn powershell_here_string_contains_patch_content() {
        let patch = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@\n-foo\n+bar";
        let cmd = build_patch_command(
            ShellType::PowerShell,
            "C:\\project",
            patch,
        )
        .unwrap();

        // The patch content should appear verbatim between @' and '@
        assert!(cmd.contains(patch), "PowerShell here-string should contain patch content verbatim");
    }

    #[test]
    fn posix_quotes_path_with_spaces() {
        let cmd = build_patch_command(
            ShellType::Posix,
            "/home/user/my project",
            "diff",
        )
        .unwrap();

        assert!(
            cmd.contains("'/home/user/my project'"),
            "POSIX should single-quote paths with spaces"
        );
    }

    #[test]
    fn powershell_quotes_path_with_spaces() {
        let cmd = build_patch_command(
            ShellType::PowerShell,
            "C:\\Users\\user\\my project",
            "diff",
        )
        .unwrap();

        assert!(
            cmd.contains("'C:\\Users\\user\\my project'"),
            "PowerShell should single-quote paths with spaces"
        );
    }
}
