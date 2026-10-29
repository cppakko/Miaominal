use super::*;
use super::{
    AgentController, ChatPanelView, PendingChatSessionDeleteState, PendingChatSessionRenameState,
    SessionAgentState,
};
use crate::ui::i18n;
use gpui_kit::{Context, Window};
use std::time::{SystemTime, UNIX_EPOCH};

impl AgentController {
    pub(in crate::ui) fn show_chat_history(&mut self, cx: &mut Context<Self>) {
        self.release_conversation_view(cx);
        self.clear_conversation_search_state(cx);
        self.stash_foreground_session(cx);
        self.refresh_chat_sessions(cx);
        self.runtime.get_mut().foreground = SessionAgentState {
            panel_view: ChatPanelView::SessionList,
            agent_mode: self.preferred_agent_mode,
            ..Default::default()
        };
        self.reset_session_filter(cx);
        self.forms.editing_title = false;
        cx.notify();
    }

    pub(in crate::ui) fn start_new_conversation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.release_conversation_view(cx);
        self.reset_conversation_search(cx);
        self.stash_foreground_session(cx);
        self.reset_foreground_chat(window, cx);
        cx.emit(AppCommand::Feedback(i18n::string(
            "workspace.panel.agent.new_chat_started",
        )));
        cx.notify();
    }

    pub(in crate::ui) fn open_chat_session(&mut self, session_id: String, cx: &mut Context<Self>) {
        self.release_conversation_view(cx);
        self.reset_conversation_search(cx);
        self.ensure_conversation_view(cx);

        if self.runtime.get_mut().foreground.session_id.as_deref() == Some(session_id.as_str()) {
            self.runtime.get_mut().foreground.panel_view = ChatPanelView::Conversation;
            cx.notify();
            return;
        }

        self.stash_foreground_session(cx);
        if let Some(mut state) = self.runtime.get_mut().take_background_session(&session_id) {
            state.panel_view = ChatPanelView::Conversation;
            state.agent_mode = self.preferred_agent_mode;
            self.runtime.get_mut().foreground = state;
            self.clear_conversation_search_state(cx);
            self.ensure_conversation_view(cx);
            cx.emit(AppCommand::Feedback(i18n::string(
                "workspace.panel.agent.messages.restored",
            )));
            cx.notify();
            return;
        }

        if !self.chat_history_available() {
            cx.emit(AppCommand::Feedback(i18n::string(
                "workspace.panel.agent.messages.history_unavailable",
            )));
            return;
        }

        let items = match self
            .chat_service
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("chat history unavailable"))
            .and_then(|service| service.load_transcript(&session_id))
        {
            Ok(items) => items,
            Err(error) => {
                let message = i18n::string_args(
                    "workspace.panel.agent.messages.load_failed",
                    &[("error", &error.to_string())],
                );
                self.runtime.get_mut().foreground.last_error = Some(message.clone());
                cx.emit(AppCommand::Feedback(message));
                cx.notify();
                return;
            }
        };
        let title = self
            .chat_session_title(&session_id)
            .unwrap_or_else(|error| {
                log::warn!("failed to load chat session title: {error:?}");
                None
            })
            .filter(|title| !title.trim().is_empty());

        let mut state = SessionAgentState {
            session_id: Some(session_id),
            title,
            panel_view: ChatPanelView::Conversation,
            agent_mode: self.preferred_agent_mode,
            ..Default::default()
        };
        for item in &items {
            state.apply_committed_item(item.clone(), false);
        }
        state.transcript_seed = Some(items);
        self.runtime.get_mut().foreground = state;
        self.ensure_conversation_view(cx);
        cx.emit(AppCommand::Feedback(i18n::string(
            "workspace.panel.agent.messages.history_loaded",
        )));
        cx.notify();
    }

    pub(in crate::ui) fn request_chat_session_delete(
        &mut self,
        session_id: String,
        title: String,
        cx: &mut Context<Self>,
    ) {
        if self.session_is_busy(&session_id) {
            cx.emit(AppCommand::Feedback(i18n::string(
                "workspace.panel.agent.messages.stop_before_delete",
            )));
            return;
        }
        self.overlay_revisions.begin("chat-session-delete");
        self.pending_chat_session_delete =
            Some(PendingChatSessionDeleteState { session_id, title });
        cx.notify();
    }

    pub(in crate::ui) fn cancel_chat_session_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.pending_chat_session_delete.take() {
            cx.emit(AppCommand::OverlayDismissed(self.overlay_snapshot(
                AgentOverlay::ChatSessionDelete(pending),
                cx.entity(),
                cx,
            )));
            cx.notify();
        }
    }

    pub(in crate::ui) fn confirm_chat_session_delete(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_chat_session_delete.take() else {
            return;
        };
        cx.emit(AppCommand::OverlayDismissed(self.overlay_snapshot(
            AgentOverlay::ChatSessionDelete(pending.clone()),
            cx.entity(),
            cx,
        )));
        self.delete_chat_session_and_runtime(&pending.session_id, cx);
    }

    pub(in crate::ui) fn request_chat_session_rename(
        &mut self,
        session_id: String,
        current_title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session_is_busy(&session_id) {
            return;
        }
        self.forms.rename_title_input = new_input_state(
            i18n::string("dialogs.chat_rename.title_label"),
            "",
            false,
            window,
            cx,
        );
        self.forms.rename_title_input.update(cx, |input, cx| {
            input.set_value(current_title.clone(), window, cx);
        });
        self.overlay_revisions.begin("chat-session-rename");
        self.pending_chat_session_rename = Some(PendingChatSessionRenameState {
            session_id,
            current_title,
        });
        cx.notify();
    }

    pub(in crate::ui) fn cancel_chat_session_rename(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.pending_chat_session_rename.take() {
            cx.emit(AppCommand::OverlayDismissed(self.overlay_snapshot(
                AgentOverlay::ChatSessionRename(pending),
                cx.entity(),
                cx,
            )));
            cx.notify();
        }
    }

    pub(in crate::ui) fn confirm_chat_session_rename(
        &mut self,
        new_title: String,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending_chat_session_rename.take() else {
            return;
        };
        cx.emit(AppCommand::OverlayDismissed(self.overlay_snapshot(
            AgentOverlay::ChatSessionRename(pending.clone()),
            cx.entity(),
            cx,
        )));
        self.rename_chat_session_and_runtime(&pending.session_id, &new_title, cx);
    }

    pub(in crate::ui) fn session_is_busy(&self, session_id: &str) -> bool {
        let runtime = self.runtime.borrow();
        if runtime.foreground.session_id.as_deref() == Some(session_id) {
            runtime.foreground.is_busy()
        } else {
            runtime.background_session_is_busy(session_id)
        }
    }

    pub(in crate::ui) fn ensure_foreground_session(&mut self, _cx: &mut Context<Self>) -> String {
        if let Some(session_id) = self.runtime.get_mut().foreground.session_id.clone() {
            return session_id;
        }

        let session_id = uuid::Uuid::new_v4().to_string();
        if self.chat_history_available()
            && let Err(error) = self.create_chat_session(&session_id, unix_timestamp())
        {
            log::warn!("failed to create chat session: {error:?}");
        }
        self.runtime.get_mut().foreground.session_id = Some(session_id.clone());
        session_id
    }

    pub(in crate::ui) fn update_session_title_for_session(
        &mut self,
        session_id: &str,
        title: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.runtime.get_mut().session_mut(session_id) else {
            return;
        };
        state.title = title.clone();
        if self.chat_history_available()
            && let Err(error) =
                self.rename_chat_session_record(session_id, title.as_deref().unwrap_or(""), cx)
        {
            log::warn!("failed to update chat title: {error:?}");
        }
        cx.notify();
    }

    pub(in crate::ui) fn refresh_chat_sessions(&mut self, cx: &mut Context<Self>) {
        let Some(chat_service) = self.chat_service.as_ref() else {
            self.chat_sessions.clear();
            cx.notify();
            return;
        };
        match chat_service.list_sessions() {
            Ok(sessions) => {
                self.chat_sessions = sessions;
                cx.notify();
            }
            Err(error) => log::warn!("failed to refresh chat sessions: {error:?}"),
        }
    }

    pub(in crate::ui) fn chat_session_title(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Option<String>> {
        self.chat_service
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("chat history unavailable"))?
            .session_title(session_id)
    }

    pub(in crate::ui) fn create_chat_session(
        &self,
        session_id: &str,
        now: i64,
    ) -> anyhow::Result<()> {
        self.chat_service
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("chat history unavailable"))?
            .create_session_v2(session_id, now)
    }

    fn delete_chat_session_and_runtime(&mut self, session_id: &str, cx: &mut Context<Self>) {
        if self.session_is_busy(session_id) {
            cx.emit(AppCommand::Feedback(i18n::string(
                "workspace.panel.agent.messages.stop_before_delete",
            )));
            return;
        }
        if !self.chat_history_available() {
            cx.emit(AppCommand::Feedback(i18n::string(
                "workspace.panel.agent.messages.history_unavailable",
            )));
            return;
        }
        if let Err(error) = self.delete_chat_session_record(session_id, cx) {
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "workspace.panel.agent.messages.delete_failed",
                &[("error", &error.to_string())],
            )));
            return;
        }

        if self.runtime.get_mut().foreground.session_id.as_deref() == Some(session_id) {
            let state = &mut self.runtime.get_mut().foreground;
            state.clear_conversation_view(cx);
            state.session_id = None;
            state.messages.clear();
            state.conversation_view = None;
            state.conversation_view_observation = None;
            state.conversation_viewport = None;
            state.title = None;
        }
        self.runtime.get_mut().remove_background_session(session_id);
        cx.emit(AppCommand::Feedback(i18n::string(
            "workspace.panel.agent.messages.deleted",
        )));
        cx.notify();
    }

    fn rename_chat_session_and_runtime(
        &mut self,
        session_id: &str,
        title: &str,
        cx: &mut Context<Self>,
    ) {
        if self.session_is_busy(session_id) {
            return;
        }
        let new_title = title.trim().to_string();
        if new_title.is_empty() {
            return;
        }
        if !self.chat_history_available() {
            cx.emit(AppCommand::Feedback(i18n::string(
                "workspace.panel.agent.messages.history_unavailable",
            )));
            return;
        }
        if let Err(error) = self.rename_chat_session_record(session_id, &new_title, cx) {
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "workspace.panel.agent.messages.rename_failed",
                &[("error", &error.to_string())],
            )));
            return;
        }
        if self.runtime.get_mut().foreground.session_id.as_deref() == Some(session_id) {
            self.runtime.get_mut().foreground.title = Some(new_title);
        }
        cx.emit(AppCommand::Feedback(i18n::string(
            "workspace.panel.agent.messages.renamed",
        )));
        cx.notify();
    }

    fn delete_chat_session_record(
        &mut self,
        session_id: &str,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        self.chat_service
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("chat history unavailable"))?
            .delete_session(session_id)?;
        self.refresh_chat_sessions(cx);
        Ok(())
    }

    pub(in crate::ui) fn rename_chat_session_record(
        &mut self,
        session_id: &str,
        title: &str,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        self.chat_service
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("chat history unavailable"))?
            .update_session_title(session_id, title)?;
        self.refresh_chat_sessions(cx);
        Ok(())
    }

    fn reset_foreground_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_conversation_view(cx);
        reset_session_agent_state(
            &mut self.runtime.get_mut().foreground,
            self.preferred_agent_mode,
        );
        self.clear_prompt_input(window, cx);
        self.ensure_conversation_view(cx);
    }

    fn stash_foreground_session(&mut self, cx: &mut Context<Self>) {
        let should_stash = {
            let runtime = self.runtime.borrow();
            let state = &runtime.foreground;
            state.session_id.is_some() && (!state.messages.is_empty() || state.is_busy())
        };
        if !should_stash {
            return;
        }
        self.release_conversation_view(cx);
        let runtime = self.runtime.get_mut();
        let session_id = runtime
            .foreground
            .session_id
            .clone()
            .expect("stashed agent session should have an id");
        let state = std::mem::take(&mut runtime.foreground);
        runtime.store_background_session(session_id, state);
    }
}

fn reset_session_agent_state(state: &mut SessionAgentState, agent_mode: ApprovalMode) {
    state.messages.clear();
    state.conversation_view = None;
    state.conversation_view_observation = None;
    state.conversation_viewport = None;
    state.session_id = None;
    state.last_error = None;
    state.engine = None;
    state.transcript_seed = None;
    state.engine_busy = false;
    state.item_messages.clear();
    state.call_messages.clear();
    state.pending_approvals.clear();
    state.context_usage = None;
    state.last_usage = None;
    super::retire_session_agent_pty_leases(&std::mem::take(&mut state.turn_leases));
    state.selected_at_targets.clear();
    state.active_at_targets.clear();
    state.active_exec_context = None;
    state.title = None;
    state.panel_view = ChatPanelView::Conversation;
    state.agent_mode = agent_mode;
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_conversation_keeps_the_last_agent_mode() {
        let mut state = SessionAgentState {
            session_id: Some("previous-session".into()),
            messages: vec![SessionAgentMessage::user("previous question")],
            agent_mode: ApprovalMode::Standard,
            engine_busy: true,
            ..Default::default()
        };

        reset_session_agent_state(&mut state, ApprovalMode::FullAuto);

        assert_eq!(state.agent_mode, ApprovalMode::FullAuto);
        assert!(state.session_id.is_none());
        assert!(state.messages.is_empty());
        assert!(!state.engine_busy);
        assert_eq!(state.panel_view, ChatPanelView::Conversation);
    }
}
