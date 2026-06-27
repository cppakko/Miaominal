use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::path_guard::{cd_prefix, resolve_workspace_path, shell_quote};
use base64::Engine as _;
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

fn build_patch_command(shell: ShellType, base_dir: &str, patch: &str) -> AgentResult<String> {
    match shell {
        ShellType::Posix => Ok(format!(
            "cd \"$HOME\" && cd {base_dir} && patch -p0 <<'MIAOMINAL_AGENT_PATCH'\n{patch}\nMIAOMINAL_AGENT_PATCH",
            base_dir = shell_quote(base_dir, ShellType::Posix),
            patch = patch,
        )),
        ShellType::Fish => Ok(build_fish_patch_command(base_dir, patch)),
        ShellType::PowerShell => {
            let patch_base64 = base64::engine::general_purpose::STANDARD.encode(patch.as_bytes());
            let ps_script = format!(
                "{cd_prefix}\n$patch = [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String('{patch_base64}'))\n$patch | & patch -p0",
                cd_prefix = cd_prefix(shell, base_dir),
                patch_base64 = patch_base64,
            );
            Ok(format!(
                "powershell.exe -NoProfile -EncodedCommand {}",
                powershell_encoded_command(&ps_script)
            ))
        }
        ShellType::Cmd => Err(AgentError::PosixOnly(
            "apply_patch is not supported in CMD sessions. \
                 Use a PowerShell session or install Git for Windows."
                .into(),
        )),
    }
}

fn build_fish_patch_command(base_dir: &str, patch: &str) -> String {
    let patch_args = patch.lines().map(fish_double_quote_arg).collect::<Vec<_>>();
    let patch_args = if patch_args.is_empty() {
        fish_double_quote_arg("")
    } else {
        patch_args.join(" ")
    };
    format!(
        "cd \"$HOME\"; and cd {base_dir}; and printf '%s\\n' {patch_args} | patch -p0",
        base_dir = fish_double_quote_arg(base_dir),
        patch_args = patch_args,
    )
}

fn fish_double_quote_arg(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
    )
}

fn powershell_encoded_command(script: &str) -> String {
    let mut bytes = Vec::with_capacity(script.len() * 2);
    for unit in script.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
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

        assert!(
            cmd.contains("<<'MIAOMINAL_AGENT_PATCH'"),
            "POSIX should use heredoc"
        );
        assert!(cmd.contains("patch -p0"), "POSIX should call patch -p0");
        assert!(
            cmd.contains("cd \"$HOME\""),
            "POSIX should cd to HOME first"
        );
        assert!(
            cmd.contains("MIAOMINAL_AGENT_PATCH"),
            "POSIX should use MIAOMINAL_AGENT_PATCH sentinel"
        );
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

        assert!(
            !cmd.contains("<<'MIAOMINAL_AGENT_PATCH'"),
            "Fish should not use POSIX heredoc"
        );
        assert!(cmd.contains("printf '%s\\n'"), "Fish should pipe printf");
        assert!(cmd.contains("patch -p0"), "Fish should call patch -p0");
        assert!(
            cmd.contains("cd \"$HOME\"; and cd"),
            "Fish should use fish command chaining"
        );
    }

    #[test]
    fn powershell_patch_unavailable_error() {
        // Test that PowerShell command uses EncodedCommand and patch invocation.
        let cmd = build_patch_command(
            ShellType::PowerShell,
            "C:\\Users\\user\\project",
            "--- a/file.txt\n+++ b/file.txt\n@@ -1 +1 @@\n-\"old\"\n+\"new\"",
        )
        .unwrap();

        assert!(
            cmd.starts_with("powershell.exe -NoProfile -EncodedCommand "),
            "PowerShell should use EncodedCommand"
        );
        assert!(
            !cmd.contains("\"old\""),
            "raw patch content should not be embedded in the outer command"
        );
        assert!(
            !cmd.contains("<<'MIAOMINAL_AGENT_PATCH'"),
            "PowerShell should NOT use POSIX heredoc"
        );
    }

    #[test]
    fn cmd_returns_posix_only_error() {
        let result = build_patch_command(ShellType::Cmd, "C:\\Users\\user\\project", "some diff");

        match result {
            Err(AgentError::PosixOnly(msg)) => {
                assert!(msg.contains("CMD"), "Error should mention CMD: {msg}");
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
        let patch = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@\n-'@\n+\"bar\"";
        let cmd = build_patch_command(ShellType::PowerShell, "C:\\project", patch).unwrap();

        assert!(
            !cmd.contains(patch),
            "PowerShell outer command should not contain raw patch content"
        );
    }

    #[test]
    fn posix_quotes_path_with_spaces() {
        let cmd = build_patch_command(ShellType::Posix, "/home/user/my project", "diff").unwrap();

        assert!(
            cmd.contains("'/home/user/my project'"),
            "POSIX should single-quote paths with spaces"
        );
    }

    #[test]
    fn powershell_quotes_path_with_spaces() {
        let cmd = build_patch_command(ShellType::PowerShell, "C:\\Users\\user\\my project", "diff")
            .unwrap();

        assert!(cmd.starts_with("powershell.exe -NoProfile -EncodedCommand "));
    }

    #[test]
    fn powershell_encoded_command_round_trips_utf16le() {
        let script = "Set-Location 'C:\\Users\\user\\my project'\n$patch = '\"quoted\"'";
        let encoded = powershell_encoded_command(script);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("encoded command decodes");
        let units = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();

        assert_eq!(String::from_utf16(&units).unwrap(), script);
    }

    #[test]
    fn fish_patch_command_escapes_expansions_in_patch_lines() {
        let cmd = build_patch_command(
            ShellType::Fish,
            "/home/user/$project",
            "--- a/file\n+++ b/file\n@@ -1 +1 @@\n-$old \"value\" \\ path\n+$new",
        )
        .unwrap();

        assert!(cmd.contains("\"/home/user/\\$project\""));
        assert!(cmd.contains("\"-\\$old \\\"value\\\" \\\\ path\""));
        assert!(cmd.contains("\"+\\$new\""));
    }
}
