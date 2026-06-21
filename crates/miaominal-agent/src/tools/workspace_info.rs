use crate::backend::BackendRoute;
use crate::capabilities::CapabilityProbe;
use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::AgentResult;

pub async fn workspace_info(channel: &AgentExecChannel) -> AgentResult<ToolOutput> {
    // Dispatch probe command + parser based on shell type
    let output = match channel.shell_label() {
        "powershell" => channel.exec(CapabilityProbe::powershell_command()).await?,
        "cmd" => channel.exec(CapabilityProbe::cmd_command()).await?,
        _ => channel.exec(CapabilityProbe::posix_command()).await?,
    };

    let probe = match channel.shell_label() {
        "powershell" => CapabilityProbe::parse_powershell(&output, BackendRoute::SshExec),
        "cmd" => CapabilityProbe::parse_cmd(&output, BackendRoute::SshExec),
        _ => CapabilityProbe::parse_posix(&output, BackendRoute::SshExec),
    };

    Ok(ToolOutput::WorkspaceInfo {
        host: channel.profile_name().to_string(),
        user: probe.user,
        platform: probe.platform,
        arch: probe.arch,
        shell: probe.shell,
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

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::profile::ShellType;
    use miaominal_secrets::SecretStore;
    use miaominal_storage::known_hosts_store::KnownHostsStore;

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
