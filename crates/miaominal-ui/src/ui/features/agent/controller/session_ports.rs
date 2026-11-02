use super::*;
use super::{
    AgentController, AgentExecMode, SessionAgentExecutionContext, SessionAgentTargetCandidate,
};
use miaominal_core::proxy::ProxyProfile;

impl AgentController {
    pub(in crate::ui) fn session_profiles(&self) -> Vec<SessionProfile> {
        self.catalog_query.profiles()
    }

    pub(in crate::ui) fn session_proxies(&self) -> Vec<ProxyProfile> {
        self.catalog_query.proxies()
    }

    pub(in crate::ui) fn terminal_targets(&self) -> Vec<SessionTerminalTarget> {
        self.session_terminal.targets()
    }

    /// Types a suggested one-line command into the active terminal without
    /// pressing Enter, so the user reviews and runs it themselves.
    pub(in crate::ui) fn insert_command_into_terminal(&self, code: &str, cx: &mut App) -> bool {
        let Some(command) = insertable_command(code) else {
            return false;
        };
        let Some(target) = self.session_terminal.active_target() else {
            return false;
        };
        self.session_terminal.paste_text(target.tab_id, command, cx)
    }

    pub(in crate::ui) fn acquire_terminal(
        &self,
        tab_id: TabId,
    ) -> Result<TerminalLeaseGrant, TerminalLeaseError> {
        self.session_terminal.acquire(tab_id)
    }

    pub(in crate::ui) fn target_candidates(&self) -> Vec<SessionAgentTargetCandidate> {
        agent_target_candidates(
            self.session_agent().exec_mode,
            &self.catalog_query.profiles(),
            &self.session_terminal.targets(),
        )
    }

    pub(in crate::ui) fn capture_execution_context(&self) -> Option<SessionAgentExecutionContext> {
        match self.session_agent().exec_mode {
            AgentExecMode::ExecChannel => {
                let active_tab_id = self
                    .session_terminal
                    .active_target()
                    .map(|target| target.tab_id);
                let profile = active_tab_id
                    .and_then(|tab_id| {
                        self.session_query
                            .resolved_profile_for_session(tab_id, &self.catalog_query)
                    })
                    .or_else(|| {
                        self.session_query
                            .active_profile_id()
                            .and_then(|id| self.catalog_query.profile(&id))
                    });
                profile.map(|profile| SessionAgentExecutionContext {
                    // Telnet has no exec channel: commands always run in its
                    // terminal tab, whichever mode the composer selects.
                    exec_mode: if profile.is_telnet() {
                        AgentExecMode::Pty
                    } else {
                        AgentExecMode::ExecChannel
                    },
                    profile_id: profile.id,
                    terminal_tab_id: active_tab_id,
                })
            }
            AgentExecMode::Pty => {
                self.session_terminal
                    .active_target()
                    .map(|target| SessionAgentExecutionContext {
                        profile_id: target.profile_id,
                        exec_mode: AgentExecMode::Pty,
                        terminal_tab_id: Some(target.tab_id),
                    })
            }
        }
    }

    pub(in crate::ui) fn profile_for_execution_context(
        &self,
        context: &SessionAgentExecutionContext,
    ) -> Option<SessionProfile> {
        context
            .terminal_tab_id
            .and_then(|tab_id| {
                self.session_query
                    .resolved_profile_for_session(tab_id, &self.catalog_query)
            })
            .or_else(|| self.catalog_query.profile(&context.profile_id))
    }

    pub(in crate::ui) fn terminal_target_marker_for_execution_context(
        &self,
        context: &SessionAgentExecutionContext,
    ) -> Option<String> {
        (context.exec_mode == AgentExecMode::Pty)
            .then_some(context.terminal_tab_id)
            .flatten()
            .and_then(|tab_id| self.session_terminal.target(tab_id))
            .map(|target| format!("@{}", target.title))
    }

    pub(in crate::ui) fn acquire_terminal_lease_for_execution_context(
        &self,
        context: &SessionAgentExecutionContext,
    ) -> Result<Option<TerminalLeaseGrant>, String> {
        if context.exec_mode != AgentExecMode::Pty {
            return Ok(None);
        }
        // Read-only Telnet targets run nothing, so they never take the terminal.
        if self
            .profile_for_execution_context(context)
            .is_some_and(|profile| profile.agent_suggests_only())
        {
            return Ok(None);
        }

        let Some(tab_id) = context.terminal_tab_id else {
            return Err(i18n::string(
                "workspace.panel.agent.messages.pty_requires_active_session",
            ));
        };
        self.session_terminal
            .acquire(tab_id)
            .map(Some)
            .map_err(|error| match error {
                TerminalLeaseError::Busy => {
                    i18n::string("workspace.panel.agent.messages.pty_terminal_busy")
                }
                TerminalLeaseError::Disconnected => {
                    i18n::string("workspace.panel.agent.messages.pty_requires_connected_session")
                }
                TerminalLeaseError::Missing => {
                    i18n::string("workspace.panel.agent.messages.pty_requires_active_session")
                }
            })
    }
}

/// A code block that can be typed into a terminal without running anything:
/// exactly one non-empty line (a newline would act as Enter on devices that
/// do not support bracketed paste).
pub(in crate::ui) fn insertable_command(code: &str) -> Option<String> {
    let command = code.trim_matches(['\r', '\n']).trim_end();
    (!command.trim().is_empty() && !command.contains(['\r', '\n'])).then(|| command.to_string())
}

/// `user@host` for SSH; Telnet logins are optional, so its summary is used.
fn candidate_detail(profile: &SessionProfile) -> String {
    if profile.is_telnet() {
        profile.summary()
    } else {
        format!("{}@{}", profile.username, profile.host)
    }
}

pub(in crate::ui) fn agent_target_candidates(
    exec_mode: AgentExecMode,
    profiles: &[SessionProfile],
    terminal_targets: &[SessionTerminalTarget],
) -> Vec<SessionAgentTargetCandidate> {
    match exec_mode {
        AgentExecMode::ExecChannel => profiles
            .iter()
            .filter_map(|profile| {
                // Telnet hosts are reachable only through their open terminal tab.
                let resolved = if profile.is_ssh() {
                    true
                } else if profile.is_telnet() {
                    terminal_targets
                        .iter()
                        .find(|target| target.profile_id == profile.id)?
                        .command_available
                } else {
                    return None;
                };
                Some(SessionAgentTargetCandidate {
                    name: profile.name.clone(),
                    detail: candidate_detail(profile),
                    resolved,
                })
            })
            .collect(),
        AgentExecMode::Pty => terminal_targets
            .iter()
            .filter(|target| {
                profiles
                    .iter()
                    .find(|profile| profile.id == target.profile_id)
                    .is_none_or(|profile| profile.is_ssh() || profile.is_telnet())
            })
            .map(|target| {
                let detail = profiles
                    .iter()
                    .find(|profile| profile.id == target.profile_id)
                    .map(candidate_detail)
                    .unwrap_or_else(|| {
                        i18n::string("workspace.panel.agent.messages.terminal_session")
                    });
                SessionAgentTargetCandidate {
                    name: target.title.clone(),
                    detail,
                    resolved: target.command_available,
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh_profile(id: &str, name: &str) -> SessionProfile {
        let mut profile = SessionProfile::blank(id, 1);
        profile.name = name.to_string();
        profile.host = "example.test".to_string();
        profile.username = "user".to_string();
        profile
    }

    fn local_profile(id: &str) -> SessionProfile {
        SessionProfile::blank_local(id, 1)
    }

    fn terminal(tab_id: usize, profile_id: &str, title: &str) -> SessionTerminalTarget {
        SessionTerminalTarget {
            tab_id: TabId::new(tab_id),
            title: title.to_string(),
            profile_id: profile_id.to_string(),
            profile: None,
            command_available: true,
        }
    }

    #[test]
    fn exec_channel_candidates_skip_local_terminal_profiles() {
        let profiles = vec![ssh_profile("ssh-a", "A"), local_profile("local-a")];

        let candidates = agent_target_candidates(AgentExecMode::ExecChannel, &profiles, &[]);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name, "A");
        assert_eq!(candidates[0].detail, "user@example.test");
    }

    #[test]
    fn pty_candidates_skip_local_terminal_sessions() {
        let profiles = vec![ssh_profile("ssh-a", "A"), local_profile("local-a")];
        let targets = vec![
            terminal(7, "ssh-a", "A"),
            terminal(9, "local-a", "Local terminal"),
        ];

        let candidates = agent_target_candidates(AgentExecMode::Pty, &profiles, &targets);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name, "A");
        assert_eq!(candidates[0].detail, "user@example.test");
    }

    #[test]
    fn only_single_line_commands_can_be_inserted_without_running() {
        assert_eq!(
            insertable_command("show running-config\n").as_deref(),
            Some("show running-config")
        );
        assert_eq!(
            insertable_command("  ls -la  ").as_deref(),
            Some("  ls -la")
        );
        assert_eq!(insertable_command("conf t\ninterface g0/1\n"), None);
        assert_eq!(insertable_command("echo a\r\necho b"), None);
        assert_eq!(insertable_command("\n  \n"), None);
    }

    fn telnet_profile(id: &str, name: &str) -> SessionProfile {
        let mut profile = SessionProfile::blank(id, 1);
        profile.kind = miaominal_core::profile::ProfileKind::Telnet;
        profile.name = name.to_string();
        profile.host = "switch.lan".to_string();
        profile.port = 23;
        profile
    }

    #[test]
    fn telnet_targets_need_their_open_terminal_tab() {
        let profiles = vec![
            ssh_profile("ssh-a", "A"),
            telnet_profile("telnet-open", "Open"),
            telnet_profile("telnet-closed", "Closed"),
        ];
        let targets = vec![terminal(7, "telnet-open", "Open")];

        let exec = agent_target_candidates(AgentExecMode::ExecChannel, &profiles, &targets);
        let names: Vec<_> = exec.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["A", "Open"]);
        assert_eq!(exec[1].detail, "switch.lan:23");
        assert!(exec[1].resolved);

        let pty = agent_target_candidates(AgentExecMode::Pty, &profiles, &targets);
        assert_eq!(pty.len(), 1);
        assert_eq!(pty[0].name, "Open");
        assert_eq!(pty[0].detail, "switch.lan:23");
    }
}
