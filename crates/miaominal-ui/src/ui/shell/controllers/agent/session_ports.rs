use super::{
    AgentController, AgentExecMode, SessionAgentExecutionContext, SessionAgentTargetCandidate,
};
use crate::ui::{
    i18n,
    shell::{SessionProfile, SessionTerminalTarget, TabId, TerminalLeaseError, TerminalLeaseGrant},
};

impl AgentController {
    pub(in crate::ui::shell) fn session_profiles(&self) -> Vec<SessionProfile> {
        self.session_query.profiles()
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
        match self.session_agent().exec_mode {
            AgentExecMode::ExecChannel => self
                .session_query
                .profiles()
                .into_iter()
                .map(|profile| SessionAgentTargetCandidate {
                    name: profile.name,
                    detail: format!("{}@{}", profile.username, profile.host),
                    resolved: true,
                })
                .collect(),
            AgentExecMode::Pty => self
                .session_terminal
                .targets()
                .into_iter()
                .map(|target| {
                    let detail = self
                        .session_query
                        .profile(&target.profile_id)
                        .map(|profile| format!("{}@{}", profile.username, profile.host))
                        .unwrap_or_else(|| {
                            i18n::string("workspace.panel.agent.messages.terminal_session")
                        });
                    SessionAgentTargetCandidate {
                        name: target.title,
                        detail,
                        resolved: target.command_available,
                    }
                })
                .collect(),
        }
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
