use super::{
    AgentController, ChatPanelView, PendingChatSessionDeleteState, PendingChatSessionRenameState,
    SessionAgentMessage, SessionAgentMessageRole, SessionAgentState, SessionAgentToolCall,
    SessionAgentToolStatus,
};
use crate::ui::i18n;
use crate::ui::shell::{AppCommand, DialogOverlaySnapshot};
use gpui::{Context, Window};
use miaominal_storage::chat_store::{ChatMessageRecord, ChatMessageRole};
use std::time::{SystemTime, UNIX_EPOCH};

impl AgentController {
    pub(in crate::ui::shell) fn show_chat_history(&mut self, cx: &mut Context<Self>) {
        self.release_conversation_view(cx);
        self.clear_conversation_search_state(cx);
        self.stash_foreground_session(cx);
        self.refresh_chat_sessions(cx);
        self.runtime.get_mut().foreground = SessionAgentState {
            panel_view: ChatPanelView::SessionList,
            ..Default::default()
        };
        self.reset_session_filter(cx);
        self.forms.editing_title = false;
        cx.notify();
    }

    pub(in crate::ui::shell) fn start_new_conversation(
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

    pub(in crate::ui::shell) fn open_chat_session(
        &mut self,
        session_id: String,
        cx: &mut Context<Self>,
    ) {
        self.release_conversation_view(cx);
        self.reset_conversation_search(cx);

        if self.runtime.get_mut().foreground.session_id.as_deref() == Some(session_id.as_str()) {
            self.runtime.get_mut().foreground.panel_view = ChatPanelView::Conversation;
            cx.notify();
            return;
        }

        self.stash_foreground_session(cx);
        if let Some(mut state) = self.runtime.get_mut().take_background_session(&session_id) {
            state.panel_view = ChatPanelView::Conversation;
            self.runtime.get_mut().foreground = state;
            self.clear_conversation_search_state(cx);
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

        let messages = match self.load_chat_session_messages(&session_id) {
            Ok(messages) => messages,
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

        self.runtime.get_mut().foreground = SessionAgentState {
            messages: messages
                .into_iter()
                .map(session_agent_message_from_record)
                .collect(),
            session_id: Some(session_id),
            title,
            active_request_id: 1,
            panel_view: ChatPanelView::Conversation,
            ..Default::default()
        };
        cx.emit(AppCommand::Feedback(i18n::string(
            "workspace.panel.agent.messages.history_loaded",
        )));
        cx.notify();
    }

    pub(in crate::ui::shell) fn request_chat_session_delete(
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
        self.pending_chat_session_delete =
            Some(PendingChatSessionDeleteState { session_id, title });
        cx.notify();
    }

    pub(in crate::ui::shell) fn cancel_chat_session_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.pending_chat_session_delete.take() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::ChatSessionDelete(pending),
            ));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn confirm_chat_session_delete(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_chat_session_delete.take() else {
            return;
        };
        cx.emit(AppCommand::OverlayDismissed(
            DialogOverlaySnapshot::ChatSessionDelete(pending.clone()),
        ));
        self.delete_chat_session_and_runtime(&pending.session_id, cx);
    }

    pub(in crate::ui::shell) fn request_chat_session_rename(
        &mut self,
        session_id: String,
        current_title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session_is_busy(&session_id) {
            return;
        }
        self.forms.rename_title_input.update(cx, |input, cx| {
            input.set_value(current_title.clone(), window, cx);
        });
        self.pending_chat_session_rename = Some(PendingChatSessionRenameState {
            session_id,
            current_title,
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn cancel_chat_session_rename(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.pending_chat_session_rename.take() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::ChatSessionRename(pending),
            ));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn confirm_chat_session_rename(
        &mut self,
        new_title: String,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending_chat_session_rename.take() else {
            return;
        };
        cx.emit(AppCommand::OverlayDismissed(
            DialogOverlaySnapshot::ChatSessionRename(pending.clone()),
        ));
        self.rename_chat_session_and_runtime(&pending.session_id, &new_title, cx);
    }

    pub(in crate::ui::shell) fn pending_chat_session_delete(
        &self,
    ) -> Option<PendingChatSessionDeleteState> {
        self.pending_chat_session_delete.clone()
    }

    pub(in crate::ui::shell) fn pending_chat_session_rename(
        &self,
    ) -> Option<PendingChatSessionRenameState> {
        self.pending_chat_session_rename.clone()
    }

    pub(in crate::ui::shell) fn session_is_busy(&self, session_id: &str) -> bool {
        let runtime = self.runtime.borrow();
        if runtime.foreground.session_id.as_deref() == Some(session_id) {
            runtime.foreground.is_busy()
        } else {
            runtime.background_session_is_busy(session_id)
        }
    }

    pub(in crate::ui::shell) fn ensure_foreground_session(
        &mut self,
        _cx: &mut Context<Self>,
    ) -> String {
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

    pub(in crate::ui::shell) fn persist_foreground_chat(&mut self, cx: &mut Context<Self>) {
        if !self.chat_history_available() || self.runtime.get_mut().foreground.messages.is_empty() {
            return;
        }

        let now = unix_timestamp();
        let session_id = match self.runtime.get_mut().foreground.session_id.clone() {
            Some(session_id) => session_id,
            None => {
                let session_id = uuid::Uuid::new_v4().to_string();
                if let Err(error) = self.create_chat_session(&session_id, now) {
                    log::warn!("failed to create chat session: {error:?}");
                    return;
                }
                self.runtime.get_mut().foreground.session_id = Some(session_id.clone());
                session_id
            }
        };

        let (records, title) = {
            let runtime = self.runtime.borrow();
            let records = runtime
                .foreground
                .messages
                .iter()
                .enumerate()
                .filter_map(|(index, message)| {
                    chat_record_from_session_agent_message(&session_id, index, now, message)
                })
                .collect::<Vec<_>>();
            (records, runtime.foreground.title.clone())
        };
        if let Err(error) = self.persist_chat_session(&session_id, &records, title.as_deref(), cx) {
            log::warn!("failed to persist chat messages: {error:?}");
        }
    }

    pub(in crate::ui::shell) fn persist_session_chat(
        &mut self,
        session_id: &str,
        cx: &mut Context<Self>,
    ) {
        if !self.chat_history_available() {
            return;
        }
        let now = unix_timestamp();
        let Some((records, title)) = ({
            let runtime = self.runtime.borrow();
            runtime.session(session_id).and_then(|state| {
                (!state.messages.is_empty()).then(|| {
                    let records = state
                        .messages
                        .iter()
                        .enumerate()
                        .filter_map(|(index, message)| {
                            chat_record_from_session_agent_message(session_id, index, now, message)
                        })
                        .collect::<Vec<_>>();
                    (records, state.title.clone())
                })
            })
        }) else {
            return;
        };
        if let Err(error) = self.persist_chat_session(session_id, &records, title.as_deref(), cx) {
            log::warn!("failed to persist chat messages: {error:?}");
        }
    }

    pub(in crate::ui::shell) fn update_session_title_for_session(
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

    pub(in crate::ui::shell) fn refresh_chat_sessions(&mut self, cx: &mut Context<Self>) {
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

    pub(in crate::ui::shell) fn load_chat_session_messages(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<ChatMessageRecord>> {
        self.chat_service
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("chat history unavailable"))?
            .load_session_messages(session_id)
    }

    pub(in crate::ui::shell) fn chat_session_title(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Option<String>> {
        self.chat_service
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("chat history unavailable"))?
            .session_title(session_id)
    }

    pub(in crate::ui::shell) fn create_chat_session(
        &self,
        session_id: &str,
        now: i64,
    ) -> anyhow::Result<()> {
        self.chat_service
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("chat history unavailable"))?
            .create_session(session_id, now)
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

    pub(in crate::ui::shell) fn rename_chat_session_record(
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

    fn persist_chat_session(
        &mut self,
        session_id: &str,
        records: &[ChatMessageRecord],
        title: Option<&str>,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let chat_service = self
            .chat_service
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("chat history unavailable"))?;
        chat_service.replace_session_messages(session_id, records)?;
        if let Some(title) = title
            && let Err(error) = chat_service.update_session_title(session_id, title)
        {
            log::warn!("failed to persist chat title: {error:?}");
        }
        self.refresh_chat_sessions(cx);
        Ok(())
    }

    fn reset_foreground_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_conversation_view(cx);
        {
            let state = &mut self.runtime.get_mut().foreground;
            state.messages.clear();
            state.conversation_view = None;
            state.conversation_view_observation = None;
            state.conversation_viewport = None;
            state.session_id = None;
            state.last_error = None;
            state.active_request_id = state.active_request_id.wrapping_add(1);
            state.pending_stream_stop = None;
            state.pending_agent_cancellation = None;
            state.pending_task = None;
            state.selected_at_targets.clear();
            state.active_at_targets.clear();
            state.active_exec_context = None;
            state.title = None;
            state.panel_view = ChatPanelView::Conversation;
        }
        self.clear_prompt_input(window, cx);
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

pub(in crate::ui::shell) fn chat_record_from_session_agent_message(
    session_id: &str,
    index: usize,
    now: i64,
    message: &SessionAgentMessage,
) -> Option<ChatMessageRecord> {
    let role = match message.role {
        SessionAgentMessageRole::User => ChatMessageRole::User,
        SessionAgentMessageRole::Assistant => ChatMessageRole::Assistant,
        SessionAgentMessageRole::Thinking => ChatMessageRole::Thinking,
        SessionAgentMessageRole::ToolCall => ChatMessageRole::ToolCall,
        SessionAgentMessageRole::Error => ChatMessageRole::Error,
    };
    let tool_call = message.tool_call.as_ref();
    let content = tool_call
        .map(|tool| tool.arguments.clone())
        .unwrap_or_else(|| message.content.clone());
    Some(ChatMessageRecord {
        id: format!("{session_id}:{index}"),
        session_id: session_id.to_string(),
        role,
        content,
        tool_name: tool_call.map(|tool| tool.name.clone()),
        tool_summary: tool_call
            .and_then(|tool| tool.confirmation_note.clone())
            .or_else(|| tool_call.map(|tool| tool.summary.clone())),
        tool_status: tool_call.map(|tool| tool_status_as_str(tool.status).to_string()),
        sort_order: index as i64,
        created_at: now,
        attachments: serialize_message_attachments(message),
    })
}

pub(in crate::ui::shell) fn session_agent_message_from_record(
    record: ChatMessageRecord,
) -> SessionAgentMessage {
    let attachments: Vec<miaominal_core::chat_attachment::ChatAttachment> = record
        .attachments
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();
    match record.role {
        ChatMessageRole::User => {
            if attachments.is_empty() {
                SessionAgentMessage::user(record.content)
            } else {
                SessionAgentMessage::user_with_attachments(record.content, attachments)
            }
        }
        ChatMessageRole::Assistant => SessionAgentMessage::assistant_raw(record.content),
        ChatMessageRole::Thinking => SessionAgentMessage::thinking_from_history(record.content),
        ChatMessageRole::Error => SessionAgentMessage::error(record.content),
        ChatMessageRole::ToolCall => {
            let summary = record
                .tool_summary
                .clone()
                .filter(|summary| !summary.trim().is_empty())
                .unwrap_or_else(|| record.content.clone());
            let (status, confirmation_note) =
                restored_tool_status_and_note(record.tool_status.as_deref(), record.tool_summary);
            SessionAgentMessage::tool_call(SessionAgentToolCall {
                id: record.id,
                name: record
                    .tool_name
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| i18n::string("workspace.panel.agent.tool")),
                arguments: record.content,
                summary,
                status,
                requires_confirmation: false,
                confirmation_note,
                expanded: false,
            })
        }
    }
}

pub(in crate::ui::shell) fn tool_status_as_str(status: SessionAgentToolStatus) -> &'static str {
    match status {
        SessionAgentToolStatus::Pending => "pending",
        SessionAgentToolStatus::WaitingForConfirmation => "waiting_for_confirmation",
        SessionAgentToolStatus::InProgress => "in_progress",
        SessionAgentToolStatus::Completed => "completed",
        SessionAgentToolStatus::Failed => "failed",
        SessionAgentToolStatus::Rejected => "rejected",
    }
}

pub(in crate::ui::shell) fn restored_tool_status_and_note(
    status: Option<&str>,
    note: Option<String>,
) -> (SessionAgentToolStatus, Option<String>) {
    match status.unwrap_or("completed") {
        "pending" | "waiting_for_confirmation" | "in_progress" => (
            SessionAgentToolStatus::Rejected,
            Some(i18n::string(
                "workspace.panel.agent.messages.tool_interrupted_before_completion",
            )),
        ),
        "failed" => (SessionAgentToolStatus::Failed, note),
        "rejected" => (SessionAgentToolStatus::Rejected, note),
        _ => (SessionAgentToolStatus::Completed, note),
    }
}

fn serialize_message_attachments(message: &SessionAgentMessage) -> Option<String> {
    if message.role != SessionAgentMessageRole::User || message.attachments.is_empty() {
        return None;
    }
    serde_json::to_string(&message.attachments).ok()
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}
