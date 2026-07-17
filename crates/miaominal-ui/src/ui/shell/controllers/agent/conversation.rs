use gpui::Context;

use super::{AgentController, SessionAgentMessage};
use crate::ui::{
    i18n,
    shell::{
        AgentFinishStreamOutcome, AppCommand, SessionAgentMessageRole, SessionAgentToolStatus,
    },
};

impl AgentController {
    pub(in crate::ui::shell) fn push_message(
        &mut self,
        message: SessionAgentMessage,
        cx: &mut Context<Self>,
    ) {
        let previous_message_count = {
            let mut state = self.session_agent_mut();
            let previous_message_count = state.messages.len();
            state.push_message_with_enter_motion(message);
            previous_message_count
        };
        self.push_message_views_from(previous_message_count, cx);
    }

    pub(in crate::ui::shell) fn set_execution_context_error(
        &mut self,
        message: String,
        cx: &mut Context<Self>,
    ) {
        self.session_agent_mut().last_error = Some(message.clone());
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_execution_context_error_for_session(
        &mut self,
        session_id: &str,
        message: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(state) = self.runtime.get_mut().session_mut(session_id) else {
            return false;
        };
        state.last_error = Some(message.clone());
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn fail_tool_start_for_session(
        &mut self,
        session_id: &str,
        tool_id: &str,
        message: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let is_foreground = self.runtime.borrow().session_is_foreground(session_id);
        let index = {
            let Some(state) = self.runtime.get_mut().session_mut(session_id) else {
                return false;
            };
            state.fail_tool_call(tool_id, message.clone());
            state.last_error = Some(message.clone());
            state.messages.iter().position(|message| {
                message
                    .tool_call
                    .as_ref()
                    .is_some_and(|tool_call| tool_call.id == tool_id)
            })
        };
        if is_foreground && let Some(index) = index {
            self.sync_conversation_message_view(index, cx);
        }
        cx.emit(AppCommand::Feedback(message));
        self.persist_session_chat(session_id, cx);
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn finish_stream_for_session(
        &mut self,
        session_id: &str,
        request_id: u64,
        cx: &mut Context<Self>,
    ) -> Option<AgentFinishStreamOutcome> {
        let is_foreground = self.runtime.borrow().session_is_foreground(session_id);
        let thinking_index = self.active_thinking_index_for_session(session_id);
        {
            let state = self.runtime.get_mut().session_mut(session_id)?;
            if state.active_request_id != request_id {
                return None;
            }
            state.finish_active_thinking();
        }
        if is_foreground && let Some(index) = thinking_index {
            self.sync_conversation_message_view(index, cx);
        }
        self.take_pending_task_for_session(session_id, cx);

        let (previous_message_count, waiting_for_confirmation, title_seed) = {
            let state = self.runtime.get_mut().session_mut(session_id)?;
            state.active_request_id = 0;
            let previous_message_count = state.messages.len();
            let turn_has_output = state
                .messages
                .iter()
                .rev()
                .take_while(|message| message.role != SessionAgentMessageRole::User)
                .any(|message| {
                    matches!(message.role, SessionAgentMessageRole::ToolCall)
                        || (message.role == SessionAgentMessageRole::Assistant
                            && !message.content.trim().is_empty())
                });
            if !turn_has_output {
                state.push_message_with_enter_motion(SessionAgentMessage::assistant_raw(
                    i18n::string("workspace.panel.agent.empty_reply"),
                ));
            }
            state.last_error = None;
            let waiting_for_confirmation = state
                .messages
                .iter()
                .rev()
                .take_while(|message| message.role != SessionAgentMessageRole::User)
                .any(|message| {
                    message.tool_call.as_ref().is_some_and(|tool_call| {
                        tool_call.status == SessionAgentToolStatus::WaitingForConfirmation
                    })
                });
            let title_seed = if state.title.is_none() && !waiting_for_confirmation {
                let user_count = state
                    .messages
                    .iter()
                    .filter(|message| message.role == SessionAgentMessageRole::User)
                    .count();
                (user_count == 1)
                    .then(|| {
                        let user = state
                            .messages
                            .iter()
                            .find(|message| message.role == SessionAgentMessageRole::User)?
                            .content
                            .clone();
                        let assistant = state
                            .messages
                            .iter()
                            .filter(|message| message.role == SessionAgentMessageRole::Assistant)
                            .find(|message| !message.content.trim().is_empty())?
                            .content
                            .clone();
                        Some((user, assistant))
                    })
                    .flatten()
            } else {
                None
            };
            (previous_message_count, waiting_for_confirmation, title_seed)
        };
        if is_foreground {
            self.push_message_views_from(previous_message_count, cx);
        }
        if is_foreground {
            cx.emit(AppCommand::Feedback(if waiting_for_confirmation {
                i18n::string("workspace.panel.agent.messages.waiting_for_tool_approval")
            } else {
                i18n::string("workspace.panel.agent.reply_ready")
            }));
        }
        self.persist_session_chat(session_id, cx);
        cx.notify();
        Some(AgentFinishStreamOutcome { title_seed })
    }

    pub(in crate::ui::shell) fn fail_stream_for_session(
        &mut self,
        session_id: &str,
        request_id: u64,
        message: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let is_foreground = self.runtime.borrow().session_is_foreground(session_id);
        let thinking_index = self.active_thinking_index_for_session(session_id);
        let previous_message_count = {
            let Some(state) = self.runtime.get_mut().session_mut(session_id) else {
                return false;
            };
            if state.active_request_id != request_id {
                return false;
            }
            state.finish_active_thinking();
            let previous_message_count = state.messages.len();
            state.active_request_id = 0;
            state.last_error = Some(message.clone());
            state.push_message_with_enter_motion(SessionAgentMessage::error(message.clone()));
            previous_message_count
        };
        if is_foreground && let Some(index) = thinking_index {
            self.sync_conversation_message_view(index, cx);
        }
        self.take_pending_task_for_session(session_id, cx);
        if is_foreground {
            self.push_message_views_from(previous_message_count, cx);
        }
        if is_foreground {
            cx.emit(AppCommand::Feedback(message));
        }
        self.persist_session_chat(session_id, cx);
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn request_matches(&self, expected_request_id: Option<u64>) -> bool {
        expected_request_id
            .is_none_or(|request_id| self.session_agent().active_request_id == request_id)
    }

    pub(in crate::ui::shell) fn request_matches_for_session(
        &self,
        session_id: &str,
        expected_request_id: Option<u64>,
    ) -> bool {
        self.runtime
            .borrow()
            .session(session_id)
            .is_some_and(|state| {
                expected_request_id.is_none_or(|request_id| state.active_request_id == request_id)
            })
    }

    pub(in crate::ui::shell) fn finalize_stopped(&mut self, cx: &mut Context<Self>) -> bool {
        let thinking_index = self.active_thinking_index();
        let active_tool_indices = self.active_tool_message_indices();
        let previous_message_count = self.session_agent().messages.len();
        let had_pending_task = self.take_pending_task(cx);
        let had_active_tool = self
            .session_agent_mut()
            .reject_active_tool_calls(&i18n::string(
                "workspace.panel.agent.messages.stopped_by_user",
            ));
        if !had_pending_task && !had_active_tool {
            return false;
        }

        {
            let mut state = self.session_agent_mut();
            state.active_request_id = state.active_request_id.wrapping_add(1);
            state.finish_stopped_turn();
        }
        if let Some(index) = thinking_index {
            self.sync_conversation_message_view(index, cx);
        }
        for index in active_tool_indices {
            self.sync_conversation_message_view(index, cx);
        }
        if self.session_agent().messages.len() > previous_message_count {
            self.push_message_views_from(previous_message_count, cx);
        } else {
            let assistant_index = {
                let state = self.session_agent();
                state.messages.len().checked_sub(1).filter(|index| {
                    state.messages[*index].role == SessionAgentMessageRole::Assistant
                })
            };
            if let Some(index) = assistant_index {
                self.sync_conversation_message_view(index, cx);
                self.refresh_conversation_search_message(index, cx);
            }
        }
        {
            let mut state = self.session_agent_mut();
            state.active_exec_context = None;
            state.last_error = None;
        }
        cx.emit(AppCommand::Feedback(i18n::string(
            "workspace.panel.agent.messages.stopped",
        )));
        self.persist_foreground_chat(cx);
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn finalize_stopped_for_session(
        &mut self,
        session_id: &str,
        expected_request_id: Option<u64>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.request_matches_for_session(session_id, expected_request_id) {
            return false;
        }
        let is_foreground = self.runtime.borrow().session_is_foreground(session_id);
        let thinking_index = self.active_thinking_index_for_session(session_id);
        let active_tool_indices = self.active_tool_message_indices_for_session(session_id);
        let (previous_message_count, had_active_tool, assistant_index) = {
            let Some(state) = self.runtime.get_mut().session_mut(session_id) else {
                return false;
            };
            let previous_message_count = state.messages.len();
            let had_active_tool = state.reject_active_tool_calls(&i18n::string(
                "workspace.panel.agent.messages.stopped_by_user",
            ));
            let assistant_index =
                state.messages.len().checked_sub(1).filter(|index| {
                    state.messages[*index].role == SessionAgentMessageRole::Assistant
                });
            (previous_message_count, had_active_tool, assistant_index)
        };
        let had_pending_task = self.take_pending_task_for_session(session_id, cx);
        if !had_pending_task && !had_active_tool {
            return false;
        }
        let appended_message = {
            let state = self
                .runtime
                .get_mut()
                .session_mut(session_id)
                .expect("agent session disappeared during stop finalization");
            state.active_request_id = state.active_request_id.wrapping_add(1);
            state.finish_stopped_turn();
            state.active_exec_context = None;
            state.last_error = None;
            state.messages.len() > previous_message_count
        };
        if is_foreground {
            if let Some(index) = thinking_index {
                self.sync_conversation_message_view(index, cx);
            }
            for index in active_tool_indices {
                self.sync_conversation_message_view(index, cx);
            }
            if appended_message {
                self.push_message_views_from(previous_message_count, cx);
            } else if let Some(index) = assistant_index {
                self.sync_conversation_message_view(index, cx);
                self.refresh_conversation_search_message(index, cx);
            }
        }
        if is_foreground {
            cx.emit(AppCommand::Feedback(i18n::string(
                "workspace.panel.agent.messages.stopped",
            )));
        }
        self.persist_session_chat(session_id, cx);
        cx.notify();
        true
    }
}
