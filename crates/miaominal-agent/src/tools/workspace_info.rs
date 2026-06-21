use crate::backend::BackendRoute;
use crate::capabilities::CapabilityProbe;
use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::AgentResult;

pub async fn workspace_info(channel: &AgentExecChannel) -> AgentResult<ToolOutput> {
    let output = channel.exec(CapabilityProbe::posix_command()).await?;
    let probe = CapabilityProbe::parse_posix(&output, BackendRoute::SshExec);

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
