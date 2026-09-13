use super::{
    AgentController, AgentExecMode, SessionAgentExecutionContext, SessionAgentTargetCandidate,
};
use crate::ui::{
    i18n,
    shell::{SessionProfile, SessionTerminalTarget, TabId, TerminalLeaseError, TerminalLeaseGrant},
};
use miaominal_core::proxy::ProxyProfile;

impl AgentController {
    pub(in crate::ui::shell) fn session_profiles(&self) -> Vec<SessionProfile> {
        self.session_query.profiles()
    }

    pub(in crate::ui::shell) fn session_proxies(&self) -> Vec<ProxyProfile> {
        self.session_query.proxies()
    }

    pub(in crate::ui::shell) fn terminal_targets(&self) -> Vec<SessionTerminalTarget> {
        self.session_terminal.targets()
    }

    pub(in crate::ui::shell) fn acquire_terminal(
        &self,
        tab_id: TabId,
    ) -> Result<TerminalLeaseGrant, TerminalLeaseError> {
        self.session_terminal.acquire(tab_id)
    }

    pub(in crate::ui::shell) fn target_candidates(&self) -> Vec<SessionAgentTargetCandidate> {
        agent_target_candidates(
            self.session_agent().exec_mode,
            &self.session_query.profiles(),
            &self.session_terminal.targets(),
        )
    }

    pub(in crate::ui::shell) fn capture_execution_context(
        &self,
    ) -> Option<SessionAgentExecutionContext> {
        match self.session_agent().exec_mode {
            AgentExecMode::ExecChannel => {
                self.session_query
                    .active_profile()
                    .map(|profile| SessionAgentExecutionContext {
                        profile_id: profile.id,
                        exec_mode: AgentExecMode::ExecChannel,
                        terminal_tab_id: None,
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

    pub(in crate::ui::shell) fn profile_for_execution_context(
        &self,
        context: &SessionAgentExecutionContext,
    ) -> Option<SessionProfile> {
        self.session_query.profile(&context.profile_id)
    }

    pub(in crate::ui::shell) fn terminal_target_marker_for_execution_context(
        &self,
        context: &SessionAgentExecutionContext,
    ) -> Option<String> {
        (context.exec_mode == AgentExecMode::Pty)
            .then_some(context.terminal_tab_id)
            .flatten()
            .and_then(|tab_id| self.session_terminal.target(tab_id))
            .map(|target| format!("@{}", target.title))
    }

    pub(in crate::ui::shell) fn acquire_terminal_lease_for_execution_context(
        &self,
        context: &SessionAgentExecutionContext,
    ) -> Result<Option<TerminalLeaseGrant>, String> {
        if context.exec_mode != AgentExecMode::Pty {
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

pub(in crate::ui::shell) fn agent_target_candidates(
    exec_mode: AgentExecMode,
    profiles: &[SessionProfile],
    terminal_targets: &[SessionTerminalTarget],
) -> Vec<SessionAgentTargetCandidate> {
    match exec_mode {
        AgentExecMode::ExecChannel => profiles
            .iter()
            .filter(|profile| !profile.is_local())
            .map(|profile| SessionAgentTargetCandidate {
                name: profile.name.clone(),
                detail: format!("{}@{}", profile.username, profile.host),
                resolved: true,
            })
            .collect(),
        AgentExecMode::Pty => terminal_targets
            .iter()
            .filter(|target| {
                profiles
                    .iter()
                    .find(|profile| profile.id == target.profile_id)
                    .is_none_or(|profile| !profile.is_local())
            })
            .map(|target| {
                let detail = profiles
                    .iter()
                    .find(|profile| profile.id == target.profile_id)
                    .map(|profile| format!("{}@{}", profile.username, profile.host))
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
}
