use super::{
    AgentController, ChatPanelView, SessionAgentExecutionContext, SessionAgentMessage,
    trailing_at_mention_query,
};
use crate::ui::shell::set_input_value;
use gpui::{App, Context, Window};
use miaominal_core::chat_attachment::ChatAttachment;

const SESSION_AGENT_PROMPT_HISTORY_LIMIT: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum PromptHistoryDirection {
    Previous,
    Next,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct AgentPromptDraft {
    pub(in crate::ui::shell) prompt: String,
    pub(in crate::ui::shell) has_pending_attachments: bool,
    pub(in crate::ui::shell) target_names: Vec<String>,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) enum AgentPromptDraftOutcome {
    Busy,
    Empty,
    Ready(AgentPromptDraft),
}

pub(in crate::ui::shell) struct AgentPromptRequestPreparation {
    pub(in crate::ui::shell) session_id: String,
    pub(in crate::ui::shell) request_id: u64,
    pub(in crate::ui::shell) attachments: Vec<ChatAttachment>,
    pub(in crate::ui::shell) history_messages: Vec<SessionAgentMessage>,
}

impl AgentController {
    pub(in crate::ui::shell) fn prompt_submission_draft(
        &self,
        cx: &App,
    ) -> AgentPromptDraftOutcome {
        let state = self.session_agent();
        if state.is_busy() {
            return AgentPromptDraftOutcome::Busy;
        }

        let prompt = self.forms.prompt_input.read(cx).value().trim().to_string();
        let has_pending_attachments = !state.pending_attachments.is_empty();
        if prompt.is_empty() && !has_pending_attachments {
            return AgentPromptDraftOutcome::Empty;
        }

        AgentPromptDraftOutcome::Ready(AgentPromptDraft {
            prompt,
            has_pending_attachments,
            target_names: state.selected_at_targets.clone(),
        })
    }

    pub(in crate::ui::shell) fn capture_prompt_execution_context(&mut self) {
        let context = self.capture_execution_context();
        self.runtime.get_mut().foreground.active_exec_context = context;
    }

    pub(in crate::ui::shell) fn clear_prompt_execution_context(&mut self) {
        self.runtime.get_mut().foreground.active_exec_context = None;
    }

    pub(in crate::ui::shell) fn record_prompt_submission_error(
        &mut self,
        message: String,
        clear_execution_context: bool,
        cx: &mut Context<Self>,
    ) {
        let state = &mut self.runtime.get_mut().foreground;
        if clear_execution_context {
            state.active_exec_context = None;
        }
        state.last_error = Some(message.clone());
        cx.emit(crate::ui::shell::AppCommand::Feedback(message));
        cx.notify();
    }

    pub(in crate::ui::shell) fn prepare_prompt_request(
        &mut self,
        target_names: Vec<String>,
        cx: &mut Context<Self>,
    ) -> AgentPromptRequestPreparation {
        let session_id = self.ensure_foreground_session(cx);
        let state = &mut self.runtime.get_mut().foreground;
        state.panel_view = ChatPanelView::Conversation;
        state.active_at_targets = target_names;
        let request_id = state.next_request_id();
        let attachments = std::mem::take(&mut state.pending_attachments);
        let history_messages = state.messages.clone();
        AgentPromptRequestPreparation {
            session_id,
            request_id,
            attachments,
            history_messages,
        }
    }

    pub(in crate::ui::shell) fn commit_prompt_request(
        &mut self,
        prompt: &str,
        model_prompt: String,
        attachments: Vec<ChatAttachment>,
        request_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.push_message(
            SessionAgentMessage::user_with_attachments(model_prompt, attachments),
            cx,
        );
        if !prompt.is_empty() {
            self.record_prompt_history(prompt);
        }
        self.persist_foreground_chat(cx);
        {
            let state = &mut self.runtime.get_mut().foreground;
            state.active_request_id = request_id;
            state.last_error = None;
        }
        self.clear_prompt_after_submission(window, cx);
    }

    pub(in crate::ui::shell) fn prompt_execution_context(
        &self,
    ) -> Option<SessionAgentExecutionContext> {
        self.session_agent().active_exec_context.clone()
    }

    pub(in crate::ui::shell) fn clear_prompt_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        set_input_value(&self.forms.prompt_input, String::new(), window, cx);
        let state = &mut self.runtime.get_mut().foreground;
        state.at_mention_query = None;
        state.at_mention_anchor = 0;
        state.prompt_history_cursor = None;
        state.prompt_history_draft = None;
        cx.notify();
    }

    pub(in crate::ui::shell) fn record_prompt_history(&mut self, prompt: &str) {
        if prompt.trim().is_empty() {
            return;
        }

        let state = &mut self.runtime.get_mut().foreground;
        if state
            .prompt_history
            .last()
            .is_some_and(|previous| previous == prompt)
        {
            state.prompt_history_cursor = None;
            state.prompt_history_draft = None;
            return;
        }

        state.prompt_history.push(prompt.to_string());
        if state.prompt_history.len() > SESSION_AGENT_PROMPT_HISTORY_LIMIT {
            let overflow = state.prompt_history.len() - SESSION_AGENT_PROMPT_HISTORY_LIMIT;
            state.prompt_history.drain(0..overflow);
        }
        state.prompt_history_cursor = None;
        state.prompt_history_draft = None;
    }

    pub(in crate::ui::shell) fn browse_prompt_history(
        &mut self,
        direction: PromptHistoryDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.runtime.get_mut().foreground.prompt_history.is_empty() {
            return false;
        }

        let current_value = self.forms.prompt_input.read(cx).value().to_string();
        let next_value = {
            let state = &mut self.runtime.get_mut().foreground;
            let next_cursor = match (direction, state.prompt_history_cursor) {
                (PromptHistoryDirection::Previous, None) => {
                    state.prompt_history_draft = Some(current_value);
                    Some(state.prompt_history.len() - 1)
                }
                (PromptHistoryDirection::Previous, Some(cursor)) => Some(cursor.saturating_sub(1)),
                (PromptHistoryDirection::Next, Some(cursor))
                    if cursor + 1 < state.prompt_history.len() =>
                {
                    Some(cursor + 1)
                }
                (PromptHistoryDirection::Next, Some(_)) => None,
                (PromptHistoryDirection::Next, None) => return true,
            };
            let next_value = next_cursor
                .and_then(|cursor| state.prompt_history.get(cursor).cloned())
                .unwrap_or_else(|| state.prompt_history_draft.take().unwrap_or_default());
            state.prompt_history_cursor = next_cursor;
            next_value
        };
        set_input_value(&self.forms.prompt_input, next_value, window, cx);
        let value = self.forms.prompt_input.read(cx).value().to_string();
        self.update_at_mention_state_from_value(&value);
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn insert_at_mention(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = self.forms.prompt_input.read(cx).value().to_string();
        let anchor = self
            .runtime
            .get_mut()
            .foreground
            .at_mention_anchor
            .min(value.len());
        let mut next = String::new();
        next.push_str(&value[..anchor]);
        set_input_value(&self.forms.prompt_input, next, window, cx);

        let state = &mut self.runtime.get_mut().foreground;
        if !state
            .selected_at_targets
            .iter()
            .any(|target| target == &name)
        {
            state.selected_at_targets.push(name);
        }
        state.at_mention_query = None;
        state.at_mention_anchor = 0;
        cx.notify();
    }

    pub(in crate::ui::shell) fn remove_at_target(&mut self, name: &str, cx: &mut Context<Self>) {
        let state = &mut self.runtime.get_mut().foreground;
        state.selected_at_targets.retain(|target| target != name);
        state.active_at_targets.retain(|target| target != name);
        cx.notify();
    }

    pub(in crate::ui::shell) fn clear_prompt_after_submission(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        set_input_value(&self.forms.prompt_input, String::new(), window, cx);
        let state = &mut self.runtime.get_mut().foreground;
        state.at_mention_query = None;
        state.at_mention_anchor = 0;
        state.selected_at_targets.clear();
        cx.notify();
    }

    pub(super) fn prompt_input_changed(&mut self, value: &str) {
        let state = &mut self.runtime.get_mut().foreground;
        state.prompt_history_cursor = None;
        state.prompt_history_draft = None;
        update_at_mention_state(state, value);
    }

    fn update_at_mention_state_from_value(&mut self, value: &str) {
        update_at_mention_state(&mut self.runtime.get_mut().foreground, value);
    }
}

fn update_at_mention_state(state: &mut super::SessionAgentState, value: &str) {
    if let Some((anchor, query)) = trailing_at_mention_query(value) {
        state.at_mention_anchor = anchor;
        state.at_mention_query = Some(query);
    } else {
        state.at_mention_query = None;
        state.at_mention_anchor = 0;
    }
}
