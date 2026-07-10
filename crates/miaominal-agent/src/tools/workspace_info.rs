use crate::backend::BackendRoute;
use crate::capabilities::CapabilityProbe;
use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::{AgentError, AgentResult};
use anyhow::anyhow;
use miaominal_core::profile::ShellType;

pub async fn workspace_info(channel: &AgentExecChannel) -> AgentResult<ToolOutput> {
    ensure_exec_shell_detected(channel).await;
    /// Try each probe in order, returning (output, effective_shell_label).
    /// Falls back when the primary probe fails (e.g., POSIX probe on a Windows host).
    /// Also validates probe output to detect when a probe "succeeds" but
    /// produces garbage (e.g. a CMD batch sent to a PowerShell shell).
    async fn probe_with_fallback(
        channel: &AgentExecChannel,
    ) -> AgentResult<(String, &'static str)> {
        let mut failures = Vec::new();
        for (command, effective_label, validator) in probe_attempts(channel.shell_label()) {
            match channel.exec(command).await {
                Ok(out) if validator(&out) => return Ok((out, effective_label)),
                Ok(out) => {
                    failures.push(format!(
                        "{effective_label}: rejected output {}",
                        probe_preview(&out)
                    ));
                }
                Err(err) => {
                    failures.push(format!("{effective_label}: {err}"));
                }
            }
        }

        Err(AgentError::Backend(anyhow!(
            "unable to detect a valid remote shell probe ({})",
            failures.join("; ")
        )))
    }

    let (output, effective_label) = probe_with_fallback(channel).await?;

    // When the probe detected a shell different from the configured profile,
    // record it so subsequent tools use the correct shell type.
    if effective_label != channel.shell_label() {
        let detected = match effective_label {
            "powershell" => miaominal_core::profile::ShellType::PowerShell,
            "cmd" => miaominal_core::profile::ShellType::Cmd,
            "fish" => miaominal_core::profile::ShellType::Fish,
            _ => miaominal_core::profile::ShellType::Posix,
        };
        channel.set_detected_shell(detected);
    }

    let probe = match effective_label {
        "powershell" => CapabilityProbe::parse_powershell(&output, BackendRoute::SshExec),
        "cmd" => CapabilityProbe::parse_cmd(&output, BackendRoute::SshExec),
        _ => CapabilityProbe::parse_posix(&output, BackendRoute::SshExec),
    };
    let shell = match effective_label {
        "cmd" | "powershell" => effective_label.to_string(),
        _ => probe.shell,
    };

    Ok(ToolOutput::WorkspaceInfo {
        host: channel.profile_name().to_string(),
        user: probe.user,
        platform: probe.platform,
        arch: probe.arch,
        shell,
        cwd: probe.cwd,
        workspace_roots: vec![probe.home.clone()],
        trusted_read_roots: vec![probe.home.clone()],
        sensitive_paths: vec![
            "/root".into(),
            "/home/*/.ssh".into(),
            "*.env".into(),
            "*.pem".into(),
            "*.key".into(),
            "/etc/shadow".into(),
            // Windows-sensitive path patterns
            r#"C:\Windows\System32\config\*"#.into(),
            r#"%USERPROFILE%\.ssh\*"#.into(),
            "*.rdp".into(),
            "*.kdbx".into(),
        ],
        capabilities: probe.capabilities,
        route: probe.route,
        supported_tools: super::TOOL_NAMES
            .iter()
            .copied()
            .filter(|tool| *tool != "web_search" || channel.web_search_enabled())
            .map(str::to_string)
            .collect(),
    })
}

// ── Exec-channel shell detection ──
// Sends a raw command via `channel.exec()` (no wrapper) that produces
// different stdout depending on whether CMD or PowerShell is interpreting
// it.  This is necessary because Windows OpenSSH can have different shells
// for exec channels and interactive login sessions.

type ProbeValidator = fn(&str) -> bool;

pub(crate) async fn ensure_exec_shell_detected(channel: &AgentExecChannel) {
    if channel.detected_shell_type().is_some() {
        return;
    }

    refresh_exec_shell_detected(channel).await;
}

pub(crate) async fn refresh_exec_shell_detected(channel: &AgentExecChannel) {
    if let Some(actual) = detect_exec_shell(channel).await {
        let before = channel.shell_type();
        if actual != before {
            #[cfg(debug_assertions)]
            log::info!(
                "[exec_shell] exec shell mismatch: profile={:?} actual={:?}",
                before,
                actual,
            );
        }
        channel.set_detected_shell(actual);
    }
}

fn probe_attempts(label: &'static str) -> Vec<(&'static str, &'static str, ProbeValidator)> {
    match label {
        "powershell" => vec![
            (
                CapabilityProbe::powershell_native_command(),
                "powershell",
                is_valid_powershell_probe,
            ),
            (CapabilityProbe::cmd_command(), "cmd", is_valid_cmd_probe),
        ],
        "cmd" => vec![
            (CapabilityProbe::cmd_command(), "cmd", is_valid_cmd_probe),
            (
                CapabilityProbe::powershell_command(),
                "powershell",
                is_valid_powershell_probe,
            ),
        ],
        _ => vec![
            (
                CapabilityProbe::posix_command(),
                label,
                is_valid_posix_probe,
            ),
            (
                CapabilityProbe::powershell_native_command(),
                "powershell",
                is_valid_powershell_probe,
            ),
            (CapabilityProbe::cmd_command(), "cmd", is_valid_cmd_probe),
        ],
    }
}

async fn detect_exec_shell(channel: &AgentExecChannel) -> Option<ShellType> {
    async fn is_cmd(channel: &AgentExecChannel) -> bool {
        channel
            .exec("if defined ComSpec echo MIAOMINAL_EXEC_SHELL=cmd")
            .await
            .is_ok_and(|out| {
                out.lines()
                    .any(|line| line.trim() == "MIAOMINAL_EXEC_SHELL=cmd")
            })
    }

    async fn is_powershell(channel: &AgentExecChannel) -> bool {
        channel
            .exec("if ($PSVersionTable) { Write-Output 'MIAOMINAL_EXEC_SHELL=powershell' }")
            .await
            .is_ok_and(|out| {
                out.lines()
                    .any(|line| line.trim() == "MIAOMINAL_EXEC_SHELL=powershell")
            })
    }

    match channel.shell_type() {
        ShellType::PowerShell => {
            if is_powershell(channel).await {
                Some(ShellType::PowerShell)
            } else if is_cmd(channel).await {
                Some(ShellType::Cmd)
            } else {
                None
            }
        }
        _ => {
            if is_cmd(channel).await {
                Some(ShellType::Cmd)
            } else if is_powershell(channel).await {
                Some(ShellType::PowerShell)
            } else {
                None
            }
        }
    }
}

// ── Probe output validation ──
// A successful probe must contain the `shell=<label>` marker so we know
// the probe script actually ran on the expected shell interpreter.
//
// Additionally, `is_valid_cmd_probe` checks that `%USERPROFILE%` was
// actually expanded (i.e. the `home=<value>` line does not contain `%`).
// When PowerShell runs a CMD batch script, `echo home=%USERPROFILE%`
// outputs the literal string `home=%USERPROFILE%` (no expansion), which
// would otherwise pass a simple `shell=cmd` substring check.

fn is_valid_powershell_probe(output: &str) -> bool {
    output.contains("shell=powershell")
        && probe_value(output, "pwd").is_some_and(is_expanded_non_empty)
        && probe_value(output, "home").is_none_or(is_expanded_or_empty)
        && probe_value(output, "user").is_none_or(is_expanded_or_empty)
}

fn is_valid_cmd_probe(output: &str) -> bool {
    if !output.contains("shell=cmd") {
        return false;
    }
    // Real CMD expands %CD%; PowerShell running a CMD probe leaves it literal.
    probe_value(output, "pwd").is_some_and(is_expanded_non_empty)
        && probe_value(output, "home").is_none_or(is_expanded_or_empty)
        && probe_value(output, "user").is_none_or(is_expanded_or_empty)
}

fn is_valid_posix_probe(output: &str) -> bool {
    output.contains("shell=")
        && probe_value(output, "pwd").is_some_and(|value| !value.trim().is_empty())
}

fn probe_value<'a>(output: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    output.lines().find_map(|line| line.strip_prefix(&prefix))
}

fn is_expanded_non_empty(value: &str) -> bool {
    !value.trim().is_empty() && !has_unexpanded_windows_variable(value)
}

fn is_expanded_or_empty(value: &str) -> bool {
    value.trim().is_empty() || !has_unexpanded_windows_variable(value)
}

fn has_unexpanded_windows_variable(value: &str) -> bool {
    value.contains('%')
}

fn probe_preview(output: &str) -> String {
    let mut preview = output
        .lines()
        .take(4)
        .collect::<Vec<_>>()
        .join("\\n")
        .replace('\r', "\\r");
    if preview.len() > 240 {
        preview.truncate(preview.floor_char_boundary(240));
        preview.push_str("...");
    }
    format!("{preview:?}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::profile::ShellType;
    use miaominal_secrets::SecretStore;
    use miaominal_storage::known_hosts_store::KnownHostsStore;

    // ── validator tests ──

    #[test]
    fn cmd_probe_rejects_unexpanded_percent() {
        // PowerShell running the CMD batch: echo home=%USERPROFILE% → literal
        let ps_output = "\
home=%USERPROFILE%
pwd=%CD%
user=%USERNAME%
shell=cmd";
        assert!(
            !is_valid_cmd_probe(ps_output),
            "PowerShell running CMD script should be rejected (unexpanded %%)"
        );
    }

    #[test]
    fn cmd_probe_accepts_expanded_home() {
        // Real CMD expands the variable
        let cmd_output = "\
home=C:\\Users\\vboxuser
pwd=C:\\Users\\vboxuser
user=vboxuser
shell=cmd";
        assert!(
            is_valid_cmd_probe(cmd_output),
            "Real CMD output should be accepted"
        );
    }

    #[test]
    fn cmd_probe_accepts_empty_home_and_user_when_pwd_is_expanded() {
        let cmd_output = "\
home=
pwd=C:\\Windows\\System32
user=
shell=cmd";
        assert!(is_valid_cmd_probe(cmd_output));
    }

    #[test]
    fn powershell_probe_validation() {
        assert!(is_valid_powershell_probe(
            "home=C:\\Users\\me\npwd=C:\\Users\\me\nuser=me\nshell=powershell"
        ));
        assert!(!is_valid_powershell_probe(
            "home=C:\\Users\\me\npwd=C:\\Users\\me\nuser=me\nshell=cmd"
        ));
    }

    #[test]
    fn powershell_probe_accepts_empty_home_and_user_when_pwd_is_expanded() {
        assert!(is_valid_powershell_probe(
            "home=\npwd=C:\\Windows\\System32\nuser=\nshell=powershell"
        ));
    }

    #[test]
    fn powershell_probe_rejects_split_concat_output() {
        let output = "\
home=C:\\Users\\me
pwd=
+
C:\\Users\\me
user=me
shell=powershell";
        assert!(!is_valid_powershell_probe(output));
    }

    #[test]
    fn workspace_powershell_attempt_uses_native_probe_not_wrapper() {
        let attempts = probe_attempts("powershell");
        assert_eq!(attempts[0].1, "powershell");
        assert_eq!(attempts[0].0, CapabilityProbe::powershell_native_command());
        assert!(!attempts[0].0.contains("powershell.exe"));
        assert!(attempts.iter().any(|(_, label, _)| *label == "cmd"));
    }

    #[test]
    fn workspace_cmd_attempt_prefers_cmd_probe() {
        let attempts = probe_attempts("cmd");
        assert_eq!(attempts[0].1, "cmd");
        assert_eq!(attempts[0].0, CapabilityProbe::cmd_command());
        assert_eq!(attempts[1].1, "powershell");
        assert_eq!(attempts[1].0, CapabilityProbe::powershell_command());
        assert!(attempts[1].0.contains("powershell.exe"));
    }

    /// Build a minimal channel with the given shell type for label/name access.
    fn channel_with_shell(shell_type: ShellType) -> AgentExecChannel {
        let mut profile = miaominal_core::profile::SessionProfile::blank("test-host", 1);
        profile.host = "test-host".into();
        profile.shell_type = shell_type;
        AgentExecChannel::for_profile(
            profile,
            vec![],
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(
                std::env::temp_dir().join(format!("workspace-info-dispatch-{shell_type:?}")),
            ),
        )
    }

    #[test]
    fn shell_label_dispatch_matches_all_shell_types() {
        // workspace_info() dispatches on channel.shell_label() — verify the mapping
        assert_eq!(
            channel_with_shell(ShellType::Posix).shell_label(),
            "posix-sh",
            "Posix shell should map to posix-sh label"
        );
        assert_eq!(
            channel_with_shell(ShellType::Fish).shell_label(),
            "fish",
            "Fish shell should map to fish label"
        );
        assert_eq!(
            channel_with_shell(ShellType::PowerShell).shell_label(),
            "powershell",
            "PowerShell shell should map to powershell label"
        );
        assert_eq!(
            channel_with_shell(ShellType::Cmd).shell_label(),
            "cmd",
            "Cmd shell should map to cmd label"
        );
    }
}
