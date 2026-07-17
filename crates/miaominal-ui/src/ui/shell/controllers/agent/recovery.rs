use super::{AgentController, SessionAgentMessage};
use crate::ui::{i18n, shell::AppCommand};
use gpui::Context;

pub(in crate::ui::shell) struct AgentRecoveryPreparation {
    pub(in crate::ui::shell) session_id: String,
    pub(in crate::ui::shell) request_id: u64,
    pub(in crate::ui::shell) history_messages: Vec<SessionAgentMessage>,
}

impl AgentController {
    pub(in crate::ui::shell) fn begin_prompt_recovery_for_session(
        &mut self,
        session_id: &str,
        request_id: u64,
        message: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let is_foreground = self.runtime.borrow().session_is_foreground(session_id);
        let thinking_index = self.active_thinking_index_for_session(session_id);
        {
            let Some(state) = self.runtime.get_mut().session_mut(session_id) else {
                return false;
            };
            if state.active_request_id != request_id {
                return false;
            }
            state.finish_active_thinking();
        }
        if is_foreground && let Some(index) = thinking_index {
            self.sync_conversation_message_view(index, cx);
        }
        self.take_pending_task_for_session(session_id, cx);
        let previous_message_count = {
            let state = self
                .runtime
                .get_mut()
                .session_mut(session_id)
                .expect("agent session disappeared during prompt recovery");
            state.active_request_id = 0;
            state.last_error = None;
            let previous_message_count = state.messages.len();
            state.push_message_with_enter_motion(SessionAgentMessage::error(i18n::string_args(
                "workspace.panel.agent.messages.tool_loop_error_message",
                &[("message", message)],
            )));
            previous_message_count
        };
        if is_foreground {
            self.push_message_views_from(previous_message_count, cx);
        }
        if is_foreground {
            cx.emit(AppCommand::Feedback(i18n::string(
                "workspace.panel.agent.messages.tool_loop_error_returned",
            )));
        }
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn record_recovery_setup_error_for_session(
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
        self.persist_session_chat(session_id, cx);
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn prepare_recovery_request_for_session(
        &mut self,
        session_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<AgentRecoveryPreparation> {
        let is_foreground = self.runtime.borrow().session_is_foreground(session_id);
        let thinking_index = self.active_thinking_index_for_session(session_id);
        let (request_id, history_messages, previous_message_count) = {
            let state = self.runtime.get_mut().session_mut(session_id)?;
            let history_messages = state.messages.clone();
            let request_id = state.next_request_id();
            state.active_request_id = request_id;
            let previous_message_count = state.messages.len();
            state.start_assistant_reply();
            (request_id, history_messages, previous_message_count)
        };
        if is_foreground {
            if let Some(index) = thinking_index {
                self.sync_conversation_message_view(index, cx);
            }
            self.push_message_views_from(previous_message_count, cx);
        }
        if is_foreground {
            cx.emit(AppCommand::Feedback(i18n::string(
                "workspace.panel.agent.thinking",
            )));
        }
        cx.notify();
        Some(AgentRecoveryPreparation {
            session_id: session_id.to_string(),
            request_id,
            history_messages,
        })
    }
}
