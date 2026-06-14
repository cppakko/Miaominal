use super::super::*;
use crate::ui::i18n;
use crate::ui::shell::state::TokenUsage;
use miaominal_agent::{
    AgentChatEvent, AgentChatMessage, AgentChatProvider, AgentChatProviderKind, AgentChatRequest,
    AgentChatRole, AgentChatToolEvent, AgentExecChannel, AgentPtyHandle, AgentToolCallRequest,
    AgentToolResultContinuationRequest, AgentToolSet,
};
use miaominal_secrets::SecretKind;
use miaominal_settings::{AiProviderConfig, AiProviderKind};
use miaominal_storage::chat_store::{ChatMessageRecord, ChatMessageRole};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

fn agent_provider_kind(kind: AiProviderKind) -> AgentChatProviderKind {
    match kind {
        AiProviderKind::Anthropic => AgentChatProviderKind::Anthropic,
        AiProviderKind::ChatGpt => AgentChatProviderKind::ChatGpt,
        AiProviderKind::Cohere => AgentChatProviderKind::Cohere,
        AiProviderKind::Copilot => AgentChatProviderKind::Copilot,
        AiProviderKind::DeepSeek => AgentChatProviderKind::DeepSeek,
        AiProviderKind::Gemini => AgentChatProviderKind::Gemini,
        AiProviderKind::HuggingFace => AgentChatProviderKind::HuggingFace,
        AiProviderKind::Mistral => AgentChatProviderKind::Mistral,
        AiProviderKind::OpenAi => AgentChatProviderKind::OpenAi,
        AiProviderKind::OpenRouter => AgentChatProviderKind::OpenRouter,
        AiProviderKind::Together => AgentChatProviderKind::Together,
        AiProviderKind::Xai => AgentChatProviderKind::Xai,
        AiProviderKind::Custom => AgentChatProviderKind::Custom,
    }
}

impl From<&SessionAgentMessage> for AgentChatMessage {
    fn from(message: &SessionAgentMessage) -> Self {
        Self {
            role: match message.role {
                SessionAgentMessageRole::User => AgentChatRole::User,
                SessionAgentMessageRole::Assistant
                | SessionAgentMessageRole::Thinking
                | SessionAgentMessageRole::ToolCall
                | SessionAgentMessageRole::Error => AgentChatRole::Assistant,
            },
            content: message.content.clone(),
        }
    }
}

impl AppView {
    pub(in crate::ui::shell) fn update_session_agent_at_mention_state(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let value = self
            .workspace_forms
            .agent
            .prompt_input
            .read(cx)
            .value()
            .to_string();
        if let Some((anchor, query)) = trailing_at_mention_query(&value) {
            self.session_agent.at_mention_anchor = anchor;
            self.session_agent.at_mention_query = Some(query);
        } else {
            self.session_agent.at_mention_query = None;
            self.session_agent.at_mention_anchor = 0;
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn insert_session_agent_at_mention(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = self
            .workspace_forms
            .agent
            .prompt_input
            .read(cx)
            .value()
            .to_string();
        let anchor = self.session_agent.at_mention_anchor.min(value.len());
        let replacement_end = value.len();
        let mut next = String::new();
        next.push_str(&value[..anchor]);
        next.push_str(&value[replacement_end..]);
        set_input_value(&self.workspace_forms.agent.prompt_input, next, window, cx);
        if !self
            .session_agent
            .selected_at_targets
            .iter()
            .any(|target| target == &name)
        {
            self.session_agent.selected_at_targets.push(name);
        }
        self.session_agent.at_mention_query = None;
        self.session_agent.at_mention_anchor = 0;
        cx.notify();
    }

    pub(in crate::ui::shell) fn remove_session_agent_at_target(
        &mut self,
        name: String,
        cx: &mut Context<Self>,
    ) {
        self.session_agent
            .selected_at_targets
            .retain(|target| target != &name);
        self.session_agent
            .active_at_targets
            .retain(|target| target != &name);
        cx.notify();
    }

    pub(in crate::ui::shell) fn session_agent_target_candidates(
        &self,
    ) -> Vec<SessionAgentTargetCandidate> {
        match self.session_agent.exec_mode {
            AgentExecMode::ExecChannel => self
                .data
                .sessions
                .iter()
                .map(|profile| SessionAgentTargetCandidate {
                    name: profile.name.clone(),
                    detail: format!("{}@{}", profile.username, profile.host),
                    resolved: true,
                })
                .collect(),
            AgentExecMode::Pty => self
                .workspace_state
                .tabs
                .iter()
                .filter_map(|tab| {
                    let session = tab.as_session()?;
                    (session.purpose == SessionPurpose::Terminal).then(|| {
                        let detail = self
                            .data
                            .sessions
                            .iter()
                            .find(|profile| profile.id == session.profile_id)
                            .map(|profile| format!("{}@{}", profile.username, profile.host))
                            .unwrap_or_else(|| "terminal session".to_string());
                        SessionAgentTargetCandidate {
                            name: tab.title.clone(),
                            detail,
                            resolved: session.commands.is_some(),
                        }
                    })
                })
                .collect(),
        }
    }

    fn session_agent_is_scrolled_to_bottom(&self) -> bool {
        let scroll_handle = &self.workspace_state.session_agent_scroll_handle;
        let offset = scroll_handle.offset();
        let max_offset = scroll_handle.max_offset();
        (offset.y + max_offset.y).abs() <= px(2.0)
    }

    fn scroll_session_agent_to_bottom_if_following(
        &self,
        previous_message_count: usize,
        was_scrolled_to_bottom: bool,
        content_may_have_grown: bool,
    ) {
        let new_block_added = self.session_agent.messages.len() > previous_message_count;
        if was_scrolled_to_bottom && (new_block_added || content_may_have_grown) {
            self.workspace_state
                .session_agent_scroll_handle
                .scroll_to_bottom();
        }
    }

    fn push_session_agent_message(&mut self, message: SessionAgentMessage, cx: &mut Context<Self>) {
        let previous_message_count = self.session_agent.messages.len();
        let was_scrolled_to_bottom = self.session_agent_is_scrolled_to_bottom();
        self.session_agent.messages.push(message);
        let index = self.session_agent.messages.len() - 1;
        self.session_agent.ensure_plain_markdown(index, cx);
        self.scroll_session_agent_to_bottom_if_following(
            previous_message_count,
            was_scrolled_to_bottom,
            false,
        );
    }

    pub(in crate::ui::shell) fn reset_session_agent_chat(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.session_agent.messages.clear();
        self.session_agent.session_id = None;
        self.session_agent.last_error = None;
        self.session_agent.active_request_id = self.session_agent.active_request_id.wrapping_add(1);
        self.session_agent.pending_task = None;
        self.session_agent.selected_at_targets.clear();
        self.session_agent.active_at_targets.clear();
        self.session_agent.title = None;
        self.session_agent.last_usage = None;
        self.session_agent.panel_view = ChatPanelView::Conversation;
        set_input_value(
            &self.workspace_forms.agent.prompt_input,
            String::new(),
            window,
            cx,
        );
        self.session_agent.at_mention_query = None;
        self.session_agent.at_mention_anchor = 0;
        self.status_message = i18n::string("workspace.panel.agent.new_chat_started");
        cx.notify();
    }

    pub(in crate::ui::shell) fn show_session_agent_history(&mut self, cx: &mut Context<Self>) {
        self.stash_current_session_agent();
        self.refresh_chat_sessions();
        self.session_agent = SessionAgentState {
            panel_view: ChatPanelView::SessionList,
            ..Default::default()
        };
        self.session_agent.panel_view = ChatPanelView::SessionList;
        self.workspace_forms.agent.editing_title = false;
        cx.notify();
    }

    pub(in crate::ui::shell) fn start_session_agent_conversation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stash_current_session_agent();
        self.reset_session_agent_chat(window, cx);
        self.session_agent.panel_view = ChatPanelView::Conversation;
        cx.notify();
    }

    pub(in crate::ui::shell) fn load_session_agent_chat(
        &mut self,
        session_id: String,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent.session_id.as_deref() == Some(session_id.as_str()) {
            self.session_agent.panel_view = ChatPanelView::Conversation;
            cx.notify();
            return;
        }

        self.stash_current_session_agent();
        if let Some(mut state) = self.session_agent_sessions.remove(&session_id) {
            state.panel_view = ChatPanelView::Conversation;
            self.session_agent = state;
            self.status_message = "Chat restored.".into();
            cx.notify();
            return;
        }

        let Some(chat_service) = self.services.chat_service.as_ref() else {
            self.status_message = "Chat history is unavailable.".into();
            cx.notify();
            return;
        };

        let messages = match chat_service.load_session_messages(&session_id) {
            Ok(messages) => messages,
            Err(error) => {
                let message = format!("Failed to load chat history: {error}");
                self.session_agent.last_error = Some(message.clone());
                self.status_message = message;
                cx.notify();
                return;
            }
        };
        let title = chat_service
            .session_title(&session_id)
            .unwrap_or_else(|error| {
                log::warn!("failed to load chat session title: {error:?}");
                None
            })
            .filter(|title| !title.trim().is_empty());

        self.session_agent = SessionAgentState::default();
        self.session_agent.messages = messages
            .into_iter()
            .map(session_agent_message_from_record)
            .collect();
        self.session_agent.session_id = Some(session_id);
        self.session_agent.title = title;
        self.session_agent.pending_task = None;
        self.session_agent.active_request_id = self.session_agent.active_request_id.wrapping_add(1);
        self.session_agent.last_error = None;
        self.session_agent.selected_at_targets.clear();
        self.session_agent.active_at_targets.clear();
        self.session_agent.panel_view = ChatPanelView::Conversation;
        self.rebuild_session_agent_markdown(cx);
        self.status_message = "Chat history loaded.".into();
        cx.notify();
    }

    pub(in crate::ui::shell) fn delete_session_agent_chat(
        &mut self,
        session_id: String,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent_session_is_busy(&session_id) {
            self.status_message = "Stop the current chat before deleting it.".into();
            cx.notify();
            return;
        }

        let Some(chat_service) = self.services.chat_service.as_ref() else {
            self.status_message = "Chat history is unavailable.".into();
            cx.notify();
            return;
        };

        if let Err(error) = chat_service.delete_session(&session_id) {
            self.status_message = format!("Failed to delete chat: {error}");
            cx.notify();
            return;
        }

        if self.session_agent.session_id.as_deref() == Some(session_id.as_str()) {
            self.session_agent.session_id = None;
            self.session_agent.messages.clear();
            self.session_agent.title = None;
        }
        self.session_agent_sessions.remove(&session_id);
        self.refresh_chat_sessions();
        self.status_message = "Chat deleted.".into();
        cx.notify();
    }

    pub(in crate::ui::shell) fn request_session_agent_chat_delete(
        &mut self,
        session_id: String,
        title: String,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent_session_is_busy(&session_id) {
            self.status_message = "Stop the current chat before deleting it.".into();
            cx.notify();
            return;
        }

        self.dialogs.pending_chat_session_delete =
            Some(PendingChatSessionDeleteState { session_id, title });
        cx.notify();
    }

    pub(in crate::ui::shell) fn cancel_session_agent_chat_delete(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if let Some(pending) = self.dialogs.pending_chat_session_delete.take() {
            self.start_dialog_exit(DialogOverlaySnapshot::ChatSessionDelete(pending), cx);
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn confirm_session_agent_chat_delete(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.dialogs.pending_chat_session_delete.take() else {
            return;
        };
        self.start_dialog_exit(
            DialogOverlaySnapshot::ChatSessionDelete(pending.clone()),
            cx,
        );
        self.delete_session_agent_chat(pending.session_id, cx);
    }

    fn stash_current_session_agent(&mut self) {
        let Some(session_id) = self.session_agent.session_id.clone() else {
            return;
        };
        if self.session_agent.messages.is_empty() && !self.session_agent.is_busy() {
            return;
        }
        let state = std::mem::take(&mut self.session_agent);
        self.session_agent_sessions.insert(session_id, state);
    }

    fn ensure_session_agent_session(&mut self) -> String {
        if let Some(session_id) = self.session_agent.session_id.clone() {
            return session_id;
        }

        let session_id = uuid::Uuid::new_v4().to_string();
        if let Some(chat_service) = self.services.chat_service.as_ref()
            && let Err(error) = chat_service.create_session(&session_id, unix_timestamp())
        {
            log::warn!("failed to create chat session: {error:?}");
        }
        self.session_agent.session_id = Some(session_id.clone());
        session_id
    }

    pub(in crate::ui::shell) fn session_agent_session_is_busy(&self, session_id: &str) -> bool {
        if self.session_agent.session_id.as_deref() == Some(session_id) {
            return self.session_agent.is_busy();
        }
        self.session_agent_sessions
            .get(session_id)
            .is_some_and(SessionAgentState::is_busy)
    }

    fn with_session_agent_state(&mut self, session_id: &str, f: impl FnOnce(&mut Self)) -> bool {
        if self.session_agent.session_id.as_deref() == Some(session_id) {
            f(self);
            return true;
        }

        let Some(mut target) = self.session_agent_sessions.remove(session_id) else {
            return false;
        };
        std::mem::swap(&mut self.session_agent, &mut target);
        f(self);
        std::mem::swap(&mut self.session_agent, &mut target);
        self.session_agent_sessions
            .insert(session_id.to_string(), target);
        true
    }

    fn refresh_chat_sessions(&mut self) {
        let Some(chat_service) = self.services.chat_service.as_ref() else {
            self.data.chat_sessions.clear();
            return;
        };
        match chat_service.list_sessions() {
            Ok(sessions) => self.data.chat_sessions = sessions,
            Err(error) => log::warn!("failed to refresh chat sessions: {error:?}"),
        }
    }

    pub(in crate::ui::shell) fn update_session_agent_title(&mut self, title: Option<String>) {
        self.session_agent.title = title.clone();
        let Some(session_id) = self.session_agent.session_id.as_deref() else {
            return;
        };
        let Some(chat_service) = self.services.chat_service.as_ref() else {
            return;
        };
        if let Err(error) =
            chat_service.update_session_title(session_id, title.as_deref().unwrap_or(""))
        {
            log::warn!("failed to update chat title: {error:?}");
            return;
        }
        self.refresh_chat_sessions();
    }

    fn persist_session_agent_chat(&mut self) {
        let Some(chat_service) = self.services.chat_service.as_ref() else {
            return;
        };
        if self.session_agent.messages.is_empty() {
            return;
        }

        let now = unix_timestamp();
        let session_id = match self.session_agent.session_id.clone() {
            Some(session_id) => session_id,
            None => {
                let session_id = uuid::Uuid::new_v4().to_string();
                if let Err(error) = chat_service.create_session(&session_id, now) {
                    log::warn!("failed to create chat session: {error:?}");
                    return;
                }
                self.session_agent.session_id = Some(session_id.clone());
                session_id
            }
        };

        let records = self
            .session_agent
            .messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| {
                chat_record_from_session_agent_message(&session_id, index, now, message)
            })
            .collect::<Vec<_>>();

        for record in records {
            if let Err(error) = chat_service.insert_message(&record) {
                log::warn!("failed to persist chat message: {error:?}");
                return;
            }
        }

        if let Some(title) = self.session_agent.title.as_deref()
            && let Err(error) = chat_service.update_session_title(&session_id, title)
        {
            log::warn!("failed to persist chat title: {error:?}");
        }
        self.refresh_chat_sessions();
    }

    fn rebuild_session_agent_markdown(&mut self, cx: &mut Context<Self>) {
        for index in 0..self.session_agent.messages.len() {
            match self.session_agent.messages[index].role {
                SessionAgentMessageRole::User | SessionAgentMessageRole::Error => {
                    self.session_agent.ensure_plain_markdown(index, cx);
                }
                SessionAgentMessageRole::Assistant => {
                    self.session_agent.ensure_assistant_markdown(index, cx);
                }
                SessionAgentMessageRole::Thinking => {
                    self.session_agent.ensure_thinking_markdown(index, cx);
                }
                SessionAgentMessageRole::ToolCall => {}
            }
        }
    }

    pub(in crate::ui::shell) fn submit_session_agent_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent.is_busy() {
            return;
        }

        let prompt = self
            .workspace_forms
            .agent
            .prompt_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if prompt.is_empty() {
            self.status_message = i18n::string("workspace.panel.agent.empty_prompt");
            cx.notify();
            return;
        }

        let Some(provider_id) = self.selected_ai_provider_id(cx) else {
            let message = i18n::string("workspace.panel.agent.no_provider_configured");
            self.session_agent.last_error = Some(message.clone());
            self.status_message = message;
            cx.notify();
            return;
        };

        let provider = match self.build_session_agent_provider(&provider_id) {
            Ok(provider) => provider,
            Err(error) => {
                let message = error.to_string();
                self.session_agent.last_error = Some(message.clone());
                self.status_message = message;
                cx.notify();
                return;
            }
        };

        let target_names = self.session_agent.selected_at_targets.clone();
        let mentions = self.resolve_session_agent_mentions(&target_names);
        if !mentions.unresolved.is_empty() {
            self.clear_session_pty_taps_by_tab_id(&mentions.pty_tap_tab_ids);
            let message = format!(
                "Unknown @ target: {}",
                mentions
                    .unresolved
                    .iter()
                    .map(|name| format!("@{name}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            self.session_agent.last_error = Some(message.clone());
            self.status_message = message;
            cx.notify();
            return;
        }

        let pty_target_tap_tab_ids = mentions.pty_tap_tab_ids.clone();
        let Some((tools, pty_tap_active)) =
            self.build_session_agent_tools(mentions.aux_channels, cx)
        else {
            return;
        };
        let target_guidance = mentions.guidance;
        self.session_agent.panel_view = ChatPanelView::Conversation;
        self.session_agent.active_at_targets = target_names.clone();
        let target_prefix = if target_names.is_empty() {
            String::new()
        } else {
            format!(
                "Targets: {}\n\n",
                target_names
                    .iter()
                    .map(|name| format!("@{name}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let model_prompt = format!("{target_prefix}{prompt}");
        let stream_session_id = self.ensure_session_agent_session();
        let request_id = self.session_agent.next_request_id();
        let history = self
            .session_agent
            .messages
            .iter()
            .filter(|message| {
                matches!(
                    message.role,
                    SessionAgentMessageRole::User | SessionAgentMessageRole::Assistant
                )
            })
            .map(AgentChatMessage::from)
            .collect::<Vec<_>>();

        self.push_session_agent_message(SessionAgentMessage::user(model_prompt.clone()), cx);
        self.persist_session_agent_chat();
        self.workspace_state
            .session_agent_scroll_handle
            .scroll_to_bottom();
        self.session_agent.active_request_id = request_id;
        self.session_agent.last_error = None;
        self.status_message = i18n::string("workspace.panel.agent.send_pending");
        set_input_value(
            &self.workspace_forms.agent.prompt_input,
            String::new(),
            window,
            cx,
        );
        self.session_agent.at_mention_query = None;
        self.session_agent.at_mention_anchor = 0;
        self.session_agent.selected_at_targets.clear();

        let runtime = self.services.runtime.clone();
        let task = cx.spawn(async move |this, cx| {
            let stream_session_id_for_error = stream_session_id.clone();
            let stream_result = runtime
                .spawn(async move {
                    miaominal_agent::stream_chat(AgentChatRequest {
                        provider,
                        messages: history,
                        prompt: model_prompt,
                        tools,
                        target_guidance,
                    })
                    .await
                    .map_err(anyhow::Error::from)
                })
                .await
                .unwrap_or_else(|error| {
                    Err(anyhow::anyhow!(
                        "session agent stream task cancelled: {error}"
                    ))
                });

            let mut receiver = match stream_result {
                Ok(receiver) => receiver,
                Err(error) => {
                    let _ = this.update(cx, move |this, cx| {
                        this.handle_session_agent_stream_error_for_session(
                            &stream_session_id_for_error,
                            request_id,
                            error,
                            cx,
                        );
                        if pty_tap_active {
                            this.set_active_session_pty_tap(None);
                        }
                        this.clear_session_pty_taps_by_tab_id(&pty_target_tap_tab_ids);
                    });
                    return;
                }
            };

            while let Some(event) = receiver.recv().await {
                match event {
                    Ok(event) => {
                        let done = matches!(event, AgentChatEvent::Finished(_));
                        let event_session_id = stream_session_id.clone();
                        if let Err(error) = this.update(cx, move |this, cx| {
                            this.apply_session_agent_event_for_session(
                                &event_session_id,
                                request_id,
                                event,
                                cx,
                            );
                        }) {
                            log::debug!("failed to apply session agent chat event: {error:?}");
                            break;
                        }
                        if done {
                            break;
                        }
                    }
                    Err(error) => {
                        let error_session_id = stream_session_id.clone();
                        let _ = this
                            .update(cx, move |this, cx| {
                                this.handle_session_agent_stream_error_for_session(
                                    &error_session_id,
                                    request_id,
                                    anyhow::Error::from(error),
                                    cx,
                                );
                                this.clear_session_pty_taps_by_tab_id(&pty_target_tap_tab_ids);
                            })
                            .map_err(|error| {
                                log::debug!("failed to apply session agent chat error: {error:?}");
                            });
                        return;
                    }
                }
            }

            let finish_session_id = stream_session_id.clone();
            let _ = this.update(cx, move |this, cx| {
                this.finish_session_agent_stream_for_session(&finish_session_id, request_id, cx);
                if pty_tap_active {
                    this.set_active_session_pty_tap(None);
                }
                this.clear_session_pty_taps_by_tab_id(&pty_target_tap_tab_ids);
            });
        });
        self.session_agent.pending_task = Some(task);
        cx.notify();
    }

    fn build_session_agent_tools(
        &mut self,
        aux_channels: HashMap<String, AgentExecChannel>,
        cx: &mut Context<Self>,
    ) -> Option<(Option<AgentToolSet>, bool)> {
        let active_pty_commands = if self.session_agent.exec_mode == AgentExecMode::Pty {
            let Some(index) = self.active_terminal_session_index() else {
                let message = "PTY mode requires an active terminal session.".to_string();
                self.session_agent.last_error = Some(message.clone());
                self.status_message = message;
                cx.notify();
                return None;
            };
            let Some(commands) = self
                .workspace_state
                .tabs
                .get(index)
                .and_then(TabState::as_session)
                .and_then(|session| session.commands.clone())
            else {
                let message = "PTY mode requires a connected terminal session.".to_string();
                self.session_agent.last_error = Some(message.clone());
                self.status_message = message;
                cx.notify();
                return None;
            };
            Some(commands)
        } else {
            None
        };

        let pty_tap_active = active_pty_commands.is_some();
        let tools = self.active_profile().cloned().map(|profile| {
            let mut channel = self.agent_exec_channel_for_profile(profile);
            if let Some(command_sender) = active_pty_commands.clone() {
                let (sender, receiver) = mpsc::unbounded_channel();
                self.set_active_session_pty_tap(Some(sender));
                channel = channel.with_pty_handle(AgentPtyHandle {
                    command_sender,
                    output_tap: Arc::new(Mutex::new(Some(receiver))),
                });
            }
            channel = channel.with_aux_channels(aux_channels);
            AgentToolSet::for_channel(channel)
        });

        Some((tools, pty_tap_active))
    }

    pub(in crate::ui::shell) fn stop_session_agent_stream(&mut self, cx: &mut Context<Self>) {
        let had_pending_task = self.session_agent.pending_task.take().is_some();
        let had_active_tool = self
            .session_agent
            .reject_active_tool_calls("Stopped by user.");
        if !had_pending_task && !had_active_tool {
            return;
        }

        self.session_agent.active_request_id = self.session_agent.active_request_id.wrapping_add(1);
        self.set_active_session_pty_tap(None);
        for tab in &mut self.workspace_state.tabs {
            if let Some(session) = tab.as_session_mut() {
                session.pty_output_tap = None;
            }
        }
        self.status_message = "Agent stopped.".into();
        self.session_agent.last_error = None;
        cx.notify();
    }

    pub(in crate::ui::shell) fn approve_session_agent_tool_call(
        &mut self,
        tool_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(tool_call) = self.session_agent.tool_call(&tool_id) else {
            self.status_message = "Tool call was not found.".into();
            cx.notify();
            return;
        };
        let Some(profile) = self.active_profile().cloned() else {
            self.status_message = "No active session for tool approval.".into();
            cx.notify();
            return;
        };

        let arguments = parse_tool_arguments(&tool_call.arguments);
        let reasoning = self.session_agent.reasoning_before_tool_call(&tool_id);
        self.session_agent.approve_tool_call(&tool_id);
        self.status_message = "Tool approved. Running...".into();
        let approval_session_id = self.ensure_session_agent_session();

        let pty_handle = if self.session_agent.exec_mode == AgentExecMode::Pty {
            let Some(index) = self.active_terminal_session_index() else {
                let message = "PTY mode requires an active terminal session.".to_string();
                self.session_agent.fail_tool_call(&tool_id, message.clone());
                self.status_message = message;
                cx.notify();
                return;
            };
            let Some(command_sender) = self
                .workspace_state
                .tabs
                .get(index)
                .and_then(TabState::as_session)
                .and_then(|session| session.commands.clone())
            else {
                let message = "PTY mode requires a connected terminal session.".to_string();
                self.session_agent.fail_tool_call(&tool_id, message.clone());
                self.status_message = message;
                cx.notify();
                return;
            };
            let (sender, receiver) = mpsc::unbounded_channel();
            self.set_active_session_pty_tap(Some(sender));
            Some(AgentPtyHandle {
                command_sender,
                output_tap: Arc::new(Mutex::new(Some(receiver))),
            })
        } else {
            None
        };
        let pty_tap_active = pty_handle.is_some();
        let approval_mentions = self.resolve_mentions_from_tool_arguments(&arguments);
        let approval_pty_target_tap_tab_ids = approval_mentions.pty_tap_tab_ids.clone();
        let sessions = self.data.sessions.clone();
        let secrets = self.services.secrets.clone();
        let known_hosts = self.services.known_hosts.clone();
        let web_search_config = self.settings_store.settings().web_search.clone();
        let tool_name = tool_call.name.clone();
        let tool_arguments = tool_call.arguments.clone();
        let task = cx.spawn(async move |this, cx| {
            let (sender, receiver) = tokio::sync::oneshot::channel();
            let worker_tool_name = tool_name.clone();
            let spawn_result = std::thread::Builder::new()
                .name(format!("session-agent-approved-tool-{worker_tool_name}"))
                .spawn(move || {
                    let result = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| anyhow::anyhow!(error))
                        .and_then(|runtime| {
                            runtime.block_on(async move {
                                let mut channel = AgentExecChannel::for_profile(
                                    profile,
                                    sessions,
                                    secrets.clone(),
                                    known_hosts,
                                );
                                if web_search_config.enabled {
                                    let web_search_api_key = secrets
                                        .get("web_search", SecretKind::WebSearchApiKey)
                                        .map_err(anyhow::Error::from)?;
                                    channel = channel.with_web_search_config(
                                        web_search_config,
                                        web_search_api_key,
                                    );
                                }
                                if let Some(pty_handle) = pty_handle {
                                    channel = channel.with_pty_handle(pty_handle);
                                }
                                channel = channel.with_aux_channels(approval_mentions.aux_channels);
                                channel
                                    .call_tool(AgentToolCallRequest {
                                        tool_name: worker_tool_name,
                                        arguments,
                                        approved: true,
                                        route: None,
                                    })
                                    .await
                                    .map_err(anyhow::Error::from)
                                    .and_then(|response| {
                                        serde_json::to_string(&response)
                                            .map_err(|error| anyhow::anyhow!(error))
                                    })
                            })
                        });
                    let _ = sender.send(result);
                });

            let result = match spawn_result {
                Ok(_) => receiver.await.unwrap_or_else(|_| {
                    Err(anyhow::anyhow!(
                        "approved tool worker stopped before returning a result"
                    ))
                }),
                Err(error) => Err(anyhow::anyhow!(error)),
            };

            let _ = this.update(cx, move |this, cx| {
                if pty_tap_active {
                    this.set_active_session_pty_tap(None);
                }
                this.clear_session_pty_taps_by_tab_id(&approval_pty_target_tap_tab_ids);
                let approval_session_id = approval_session_id.clone();
                this.with_session_agent_state(&approval_session_id, |this| {
                    let previous_message_count = this.session_agent.messages.len();
                    let was_scrolled_to_bottom = this.session_agent_is_scrolled_to_bottom();
                    let (tool_result, failed) = match result {
                        Ok(result) => {
                            if !matches!(
                                this.session_agent
                                    .tool_call(&tool_id)
                                    .map(|tool_call| tool_call.status),
                                Some(SessionAgentToolStatus::InProgress)
                            ) {
                                this.status_message = "Agent stopped.".into();
                                cx.notify();
                                return;
                            }
                            this.session_agent
                                .complete_tool_call(&tool_id, result.clone());
                            this.status_message = "Approved tool finished. Continuing...".into();
                            (result, false)
                        }
                        Err(error) => {
                            if !matches!(
                                this.session_agent
                                    .tool_call(&tool_id)
                                    .map(|tool_call| tool_call.status),
                                Some(SessionAgentToolStatus::InProgress)
                            ) {
                                this.status_message = "Agent stopped.".into();
                                cx.notify();
                                return;
                            }
                            let result = format!("tool failed after approval: {error}");
                            this.session_agent.fail_tool_call(&tool_id, result.clone());
                            this.status_message = "Approved tool failed. Continuing...".into();
                            (result, true)
                        }
                    };
                    this.scroll_session_agent_to_bottom_if_following(
                        previous_message_count,
                        was_scrolled_to_bottom,
                        true,
                    );
                    this.continue_session_agent_after_tool_result(
                        AgentChatToolEvent {
                            id: tool_id,
                            name: tool_name,
                            arguments: tool_arguments,
                        },
                        reasoning,
                        tool_result,
                        failed,
                        cx,
                    );
                });
                cx.notify();
            });
        });
        task.detach();
        cx.notify();
    }

    fn continue_session_agent_after_tool_result(
        &mut self,
        tool_call: AgentChatToolEvent,
        reasoning: Option<String>,
        result: String,
        failed: bool,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent.has_pending_task() {
            self.push_session_agent_message(SessionAgentMessage::error(
                "Agent is already processing another response; approved tool result was not sent to the model.",
            ), cx);
            self.status_message = "Agent is already processing.".into();
            return;
        }

        let Some(provider_id) = self.selected_ai_provider_id(cx) else {
            let message = i18n::string("workspace.panel.agent.no_provider_configured");
            self.session_agent.last_error = Some(message.clone());
            self.push_session_agent_message(SessionAgentMessage::error(message.clone()), cx);
            self.status_message = message;
            return;
        };

        let provider = match self.build_session_agent_provider(&provider_id) {
            Ok(provider) => provider,
            Err(error) => {
                let message = error.to_string();
                self.session_agent.last_error = Some(message.clone());
                self.push_session_agent_message(SessionAgentMessage::error(message.clone()), cx);
                self.status_message = message;
                return;
            }
        };

        let active_targets = self.session_agent.active_at_targets.clone();
        let mentions = self.resolve_session_agent_mentions(&active_targets);
        let pty_target_tap_tab_ids = mentions.pty_tap_tab_ids.clone();
        let target_guidance = mentions.guidance;
        let Some((tools, pty_tap_active)) =
            self.build_session_agent_tools(mentions.aux_channels, cx)
        else {
            return;
        };
        let stream_session_id = self.ensure_session_agent_session();
        let request_id = self.session_agent.next_request_id();
        let history = self
            .session_agent
            .messages
            .iter()
            .filter(|message| {
                matches!(
                    message.role,
                    SessionAgentMessageRole::User | SessionAgentMessageRole::Assistant
                )
            })
            .map(AgentChatMessage::from)
            .collect::<Vec<_>>();

        self.session_agent.active_request_id = request_id;
        self.session_agent.last_error = None;
        let previous_message_count = self.session_agent.messages.len();
        let was_scrolled_to_bottom = self.session_agent_is_scrolled_to_bottom();
        self.session_agent.start_assistant_reply(cx);
        self.scroll_session_agent_to_bottom_if_following(
            previous_message_count,
            was_scrolled_to_bottom,
            false,
        );
        self.status_message = i18n::string("workspace.panel.agent.thinking");

        let runtime = self.services.runtime.clone();
        let task = cx.spawn(async move |this, cx| {
            let stream_session_id_for_error = stream_session_id.clone();
            let stream_result = runtime
                .spawn(async move {
                    let result = if failed {
                        format!("ERROR: {result}")
                    } else {
                        result
                    };
                    miaominal_agent::stream_chat_after_tool_result(
                        AgentToolResultContinuationRequest {
                            provider,
                            messages: history,
                            tool_call,
                            reasoning,
                            result,
                            tools,
                            target_guidance,
                        },
                    )
                    .await
                    .map_err(anyhow::Error::from)
                })
                .await
                .unwrap_or_else(|error| {
                    Err(anyhow::anyhow!(
                        "session agent continuation task cancelled: {error}"
                    ))
                });

            let mut receiver = match stream_result {
                Ok(receiver) => receiver,
                Err(error) => {
                    let _ = this.update(cx, move |this, cx| {
                        this.handle_session_agent_stream_error_for_session(
                            &stream_session_id_for_error,
                            request_id,
                            error,
                            cx,
                        );
                        if pty_tap_active {
                            this.set_active_session_pty_tap(None);
                        }
                        this.clear_session_pty_taps_by_tab_id(&pty_target_tap_tab_ids);
                    });
                    return;
                }
            };

            while let Some(event) = receiver.recv().await {
                match event {
                    Ok(event) => {
                        let done = matches!(event, AgentChatEvent::Finished(_));
                        let event_session_id = stream_session_id.clone();
                        if let Err(error) = this.update(cx, move |this, cx| {
                            this.apply_session_agent_event_for_session(
                                &event_session_id,
                                request_id,
                                event,
                                cx,
                            );
                        }) {
                            log::debug!(
                                "failed to apply session agent continuation event: {error:?}"
                            );
                            break;
                        }
                        if done {
                            break;
                        }
                    }
                    Err(error) => {
                        let error_session_id = stream_session_id.clone();
                        let _ = this
                            .update(cx, move |this, cx| {
                                this.handle_session_agent_stream_error_for_session(
                                    &error_session_id,
                                    request_id,
                                    anyhow::Error::from(error),
                                    cx,
                                );
                                this.clear_session_pty_taps_by_tab_id(&pty_target_tap_tab_ids);
                            })
                            .map_err(|error| {
                                log::debug!(
                                    "failed to apply session agent continuation error: {error:?}"
                                );
                            });
                        return;
                    }
                }
            }

            let finish_session_id = stream_session_id.clone();
            let _ = this.update(cx, move |this, cx| {
                this.finish_session_agent_stream_for_session(&finish_session_id, request_id, cx);
                if pty_tap_active {
                    this.set_active_session_pty_tap(None);
                }
                this.clear_session_pty_taps_by_tab_id(&pty_target_tap_tab_ids);
            });
        });
        self.session_agent.pending_task = Some(task);
    }

    pub(in crate::ui::shell) fn deny_session_agent_tool_call(
        &mut self,
        tool_id: String,
        cx: &mut Context<Self>,
    ) {
        self.session_agent.reject_tool_call(&tool_id);
        self.status_message = "Tool denied.".into();
        cx.notify();
    }

    pub(in crate::ui::shell) fn copy_session_agent_text(
        &mut self,
        label: &str,
        text: String,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.status_message = format!("Copied {label}.");
        cx.notify();
    }

    fn apply_session_agent_event(
        &mut self,
        request_id: u64,
        event: AgentChatEvent,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent.active_request_id != request_id {
            return;
        }

        let previous_message_count = self.session_agent.messages.len();
        let was_scrolled_to_bottom = self.session_agent_is_scrolled_to_bottom();
        let content_may_have_grown = matches!(
            event,
            AgentChatEvent::TextDelta(_)
                | AgentChatEvent::ThinkingDelta(_)
                | AgentChatEvent::ToolCallDelta { .. }
                | AgentChatEvent::ToolCallCompleted { .. }
                | AgentChatEvent::ToolCallApprovalRequired { .. }
                | AgentChatEvent::Finished(_)
        );
        match event {
            AgentChatEvent::TextDelta(delta) => {
                self.session_agent.append_assistant_delta(delta, cx);
                self.session_agent.last_error = None;
            }
            AgentChatEvent::ThinkingDelta(delta) => {
                self.session_agent.append_thinking_delta(delta, cx);
                self.status_message = i18n::string("workspace.panel.agent.thinking");
            }
            AgentChatEvent::ToolCallStarted(tool) => {
                self.session_agent.push_tool_call(
                    tool.id,
                    tool.name,
                    tool.arguments,
                    SessionAgentToolStatus::InProgress,
                );
            }
            AgentChatEvent::ToolCallDelta { id, delta } => {
                self.session_agent.append_tool_call_delta(&id, delta);
            }
            AgentChatEvent::ToolCallCompleted { id, result } => {
                self.session_agent.complete_tool_call(&id, result);
            }
            AgentChatEvent::ToolCallApprovalRequired { id, message } => {
                self.session_agent
                    .require_tool_call_confirmation(&id, message);
                self.finish_session_agent_stream(request_id, cx);
            }
            AgentChatEvent::Finished(reply) => {
                self.session_agent.finish_assistant_reply(reply);
                // Sync the markdown entity with the final authoritative content.
                // finish_assistant_reply overwrites message.content but does not
                // update markdown_entity, so we need to do it here.
                let last_assistant_idx = self
                    .session_agent
                    .messages
                    .iter()
                    .rposition(|m| m.role == SessionAgentMessageRole::Assistant);
                if let Some(idx) = last_assistant_idx {
                    self.session_agent.ensure_assistant_markdown(idx, cx);
                }
                self.finish_session_agent_stream(request_id, cx);
            }
            AgentChatEvent::TokenUsage {
                input_tokens,
                output_tokens,
            } => {
                self.session_agent.last_usage = Some(TokenUsage {
                    input_tokens,
                    output_tokens,
                });
            }
        }
        self.scroll_session_agent_to_bottom_if_following(
            previous_message_count,
            was_scrolled_to_bottom,
            content_may_have_grown,
        );

        cx.notify();
    }

    fn apply_session_agent_event_for_session(
        &mut self,
        session_id: &str,
        request_id: u64,
        event: AgentChatEvent,
        cx: &mut Context<Self>,
    ) {
        let is_visible = self.session_agent.session_id.as_deref() == Some(session_id);
        if self.with_session_agent_state(session_id, |this| {
            this.apply_session_agent_event(request_id, event, cx);
        }) && !is_visible
        {
            self.refresh_chat_sessions();
            cx.notify();
        }
    }

    fn finish_session_agent_stream(&mut self, request_id: u64, cx: &mut Context<Self>) {
        if self.session_agent.active_request_id != request_id {
            return;
        }

        self.session_agent.pending_task = None;
        self.session_agent.active_request_id = 0;
        let turn_has_output = self
            .session_agent
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
            self.push_session_agent_message(
                SessionAgentMessage::assistant_raw(i18n::string(
                    "workspace.panel.agent.empty_reply",
                )),
                cx,
            );
        }
        self.session_agent.last_error = None;
        let waiting_for_confirmation = self
            .session_agent
            .messages
            .iter()
            .rev()
            .take_while(|message| message.role != SessionAgentMessageRole::User)
            .any(|message| {
                message.tool_call.as_ref().is_some_and(|tool_call| {
                    tool_call.status == SessionAgentToolStatus::WaitingForConfirmation
                })
            });
        self.status_message = if waiting_for_confirmation {
            "Waiting for tool approval.".into()
        } else {
            i18n::string("workspace.panel.agent.reply_ready")
        };

        if !waiting_for_confirmation {
            self.persist_session_agent_chat();
        }

        // --- title generation ---
        if self.session_agent.title.is_none() && !waiting_for_confirmation {
            let user_count = self
                .session_agent
                .messages
                .iter()
                .filter(|m| m.role == SessionAgentMessageRole::User)
                .count();
            if user_count == 1 {
                let first_user = self
                    .session_agent
                    .messages
                    .iter()
                    .find(|m| m.role == SessionAgentMessageRole::User)
                    .map(|m| m.content.clone());
                let first_assistant = self
                    .session_agent
                    .messages
                    .iter()
                    .filter(|m| m.role == SessionAgentMessageRole::Assistant)
                    .find(|m| !m.content.trim().is_empty())
                    .map(|m| m.content.clone());
                if let (Some(user_msg), Some(assistant_msg)) = (first_user, first_assistant) {
                    let provider_id = self.selected_ai_provider_id(cx);
                    let provider = match provider_id.as_ref() {
                        Some(id) => match self.build_session_agent_provider(id) {
                            Ok(p) => Some(p),
                            Err(e) => {
                                log::info!("skip title generation: {e:?}");
                                None
                            }
                        },
                        None => None,
                    };
                    if let Some(provider) = provider {
                        let runtime = self.services.runtime.clone();
                        let title_session_id = self.ensure_session_agent_session();
                        let task = cx.spawn(async move |this, cx| {
                            let title = runtime
                                .spawn(async move {
                                    miaominal_agent::generate_title(
                                        provider,
                                        &user_msg,
                                        &assistant_msg,
                                    )
                                    .await
                                })
                                .await
                                .unwrap_or_else(|error| {
                                    log::info!("title generation task cancelled: {error:?}");
                                    None
                                });
                            if let Some(title) = title {
                                let _ = this.update(cx, move |this, cx| {
                                    this.with_session_agent_state(&title_session_id, |this| {
                                        this.update_session_agent_title(Some(title));
                                    });
                                    cx.notify();
                                });
                            }
                        });
                        task.detach();
                    }
                }
            }
        }
        // --- end title generation ---

        cx.notify();
    }

    fn finish_session_agent_stream_for_session(
        &mut self,
        session_id: &str,
        request_id: u64,
        cx: &mut Context<Self>,
    ) {
        let is_visible = self.session_agent.session_id.as_deref() == Some(session_id);
        if self.with_session_agent_state(session_id, |this| {
            this.finish_session_agent_stream(request_id, cx);
        }) && !is_visible
        {
            self.refresh_chat_sessions();
            cx.notify();
        }
    }

    fn fail_session_agent_stream(
        &mut self,
        request_id: u64,
        error: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent.active_request_id != request_id {
            return;
        }

        self.session_agent.pending_task = None;
        self.session_agent.active_request_id = 0;
        let message = error.to_string();
        self.session_agent.last_error = Some(message.clone());
        self.push_session_agent_message(SessionAgentMessage::error(message.clone()), cx);
        self.status_message = message;
        cx.notify();
    }

    fn handle_session_agent_stream_error(
        &mut self,
        request_id: u64,
        error: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        let message = error.to_string();
        if is_recoverable_session_agent_prompt_error(&message) {
            self.recover_session_agent_prompt_error(request_id, message, cx);
        } else {
            self.fail_session_agent_stream(request_id, error, cx);
        }
    }

    fn handle_session_agent_stream_error_for_session(
        &mut self,
        session_id: &str,
        request_id: u64,
        error: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        let is_visible = self.session_agent.session_id.as_deref() == Some(session_id);
        if self.with_session_agent_state(session_id, |this| {
            this.handle_session_agent_stream_error(request_id, error, cx);
        }) && !is_visible
        {
            self.refresh_chat_sessions();
            cx.notify();
        }
    }

    fn recover_session_agent_prompt_error(
        &mut self,
        request_id: u64,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent.active_request_id != request_id {
            return;
        }

        self.session_agent.pending_task = None;
        self.session_agent.active_request_id = 0;
        self.session_agent.last_error = None;
        self.push_session_agent_message(
            SessionAgentMessage::error(format!(
                "Agent tool-loop error returned to model: {message}"
            )),
            cx,
        );
        self.status_message = "Agent tool-loop error returned to model.".into();

        let Some(provider_id) = self.selected_ai_provider_id(cx) else {
            self.session_agent.last_error = Some(message.clone());
            self.status_message = message;
            cx.notify();
            return;
        };

        let provider = match self.build_session_agent_provider(&provider_id) {
            Ok(provider) => provider,
            Err(error) => {
                let message = error.to_string();
                self.session_agent.last_error = Some(message.clone());
                self.status_message = message;
                cx.notify();
                return;
            }
        };

        let history = self
            .session_agent
            .messages
            .iter()
            .filter(|message| {
                matches!(
                    message.role,
                    SessionAgentMessageRole::User | SessionAgentMessageRole::Assistant
                )
            })
            .map(AgentChatMessage::from)
            .collect::<Vec<_>>();
        let tool_guidance = if message.contains("UnknownToolCall") {
            "\nTool correction: use only the listed Miaominal tools. There is no `write`, `edit`, or `replace` tool. For file creation or modification, use `apply_patch` with a unified patch."
        } else {
            ""
        };
        let prompt = format!(
            "The previous agent step failed before producing a final answer.\n\
             Error:\n{message}\n{tool_guidance}\n\n\
             Continue the conversation in plain text. Explain what happened and choose the next safe step. Do not call tools in this recovery response."
        );
        let request_id = self.session_agent.next_request_id();
        self.session_agent.active_request_id = request_id;
        let previous_message_count = self.session_agent.messages.len();
        let was_scrolled_to_bottom = self.session_agent_is_scrolled_to_bottom();
        self.session_agent.start_assistant_reply(cx);
        self.scroll_session_agent_to_bottom_if_following(
            previous_message_count,
            was_scrolled_to_bottom,
            false,
        );
        self.status_message = i18n::string("workspace.panel.agent.thinking");

        let runtime = self.services.runtime.clone();
        let recovery_session_id = self.ensure_session_agent_session();
        let task = cx.spawn(async move |this, cx| {
            let recovery_session_id_for_error = recovery_session_id.clone();
            let stream_result = runtime
                .spawn(async move {
                    miaominal_agent::stream_chat(AgentChatRequest {
                        provider,
                        messages: history,
                        prompt,
                        tools: None,
                        target_guidance: None,
                    })
                    .await
                    .map_err(anyhow::Error::from)
                })
                .await
                .unwrap_or_else(|error| {
                    Err(anyhow::anyhow!(
                        "session agent recovery task cancelled: {error}"
                    ))
                });

            let mut receiver = match stream_result {
                Ok(receiver) => receiver,
                Err(error) => {
                    let _ = this.update(cx, move |this, cx| {
                        this.handle_session_agent_stream_error_for_session(
                            &recovery_session_id_for_error,
                            request_id,
                            error,
                            cx,
                        );
                    });
                    return;
                }
            };

            while let Some(event) = receiver.recv().await {
                match event {
                    Ok(event) => {
                        let done = matches!(event, AgentChatEvent::Finished(_));
                        let event_session_id = recovery_session_id.clone();
                        if let Err(error) = this.update(cx, move |this, cx| {
                            this.apply_session_agent_event_for_session(
                                &event_session_id,
                                request_id,
                                event,
                                cx,
                            );
                        }) {
                            log::debug!("failed to apply session agent recovery event: {error:?}");
                            break;
                        }
                        if done {
                            break;
                        }
                    }
                    Err(error) => {
                        let error_session_id = recovery_session_id.clone();
                        let _ = this
                            .update(cx, move |this, cx| {
                                this.handle_session_agent_stream_error_for_session(
                                    &error_session_id,
                                    request_id,
                                    anyhow::Error::from(error),
                                    cx,
                                );
                            })
                            .map_err(|error| {
                                log::debug!(
                                    "failed to apply session agent recovery error: {error:?}"
                                );
                            });
                        return;
                    }
                }
            }

            let finish_session_id = recovery_session_id.clone();
            let _ = this.update(cx, move |this, cx| {
                this.finish_session_agent_stream_for_session(&finish_session_id, request_id, cx);
            });
        });
        self.session_agent.pending_task = Some(task);
        cx.notify();
    }

    fn build_session_agent_provider(&self, provider_id: &str) -> anyhow::Result<AgentChatProvider> {
        let provider = self
            .settings_store
            .settings()
            .ai_providers
            .iter()
            .find(|provider| provider.id == provider_id && provider.enabled)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(i18n::string("workspace.panel.agent.provider_missing"))
            })?;

        let api_key = self.resolve_ai_provider_api_key(&provider)?;
        Ok(AgentChatProvider {
            id: provider.id,
            name: provider.name,
            kind: agent_provider_kind(provider.kind),
            model: provider.model,
            base_url: provider.base_url,
            api_key,
        })
    }

    fn agent_exec_channel_for_profile(&self, profile: SessionProfile) -> AgentExecChannel {
        let mut channel = AgentExecChannel::for_profile(
            profile,
            self.data.sessions.clone(),
            self.services.secrets.clone(),
            self.services.known_hosts.clone(),
        );
        let web_search_config = self.settings_store.settings().web_search.clone();
        if web_search_config.enabled {
            let web_search_api_key = self
                .services
                .secrets
                .get("web_search", SecretKind::WebSearchApiKey)
                .unwrap_or_else(|error| {
                    log::warn!("failed to load web search API key: {error:?}");
                    None
                });
            channel = channel.with_web_search_config(web_search_config, web_search_api_key);
        }
        channel
    }

    fn resolve_session_agent_mentions(
        &mut self,
        targets: &[String],
    ) -> ResolvedSessionAgentMentions {
        let mut aux_channels = HashMap::new();
        let mut guidance_lines = Vec::new();
        let mut resolved_names = Vec::new();
        let mut pty_tap_tab_ids = Vec::new();
        let mut pending_pty_taps = Vec::new();

        match self.session_agent.exec_mode {
            AgentExecMode::ExecChannel => {
                for profile in &self.data.sessions {
                    let marker = format!("@{}", profile.name);
                    if !targets.iter().any(|target| target == &profile.name)
                        || resolved_names.contains(&profile.name)
                    {
                        continue;
                    }
                    aux_channels.insert(
                        marker.clone(),
                        self.agent_exec_channel_for_profile(profile.clone()),
                    );
                    guidance_lines.push(format!(
                        "- {marker}: SSH profile \"{}\" (host: {}, user: {})",
                        profile.name, profile.host, profile.username
                    ));
                    resolved_names.push(profile.name.clone());
                }
            }
            AgentExecMode::Pty => {
                for tab in &self.workspace_state.tabs {
                    let Some(session) = tab.as_session() else {
                        continue;
                    };
                    if session.purpose != SessionPurpose::Terminal {
                        continue;
                    }
                    let marker = format!("@{}", tab.title);
                    if !targets.iter().any(|target| target == &tab.title)
                        || resolved_names.contains(&tab.title)
                    {
                        continue;
                    }
                    let Some(command_sender) = session.commands.clone() else {
                        continue;
                    };
                    let Some(profile) = self
                        .data
                        .sessions
                        .iter()
                        .find(|profile| profile.id == session.profile_id)
                        .cloned()
                        .or_else(|| session.pending_profile.clone())
                    else {
                        continue;
                    };
                    let (sender, receiver) = mpsc::unbounded_channel();
                    let channel = self
                        .agent_exec_channel_for_profile(profile.clone())
                        .with_pty_handle(AgentPtyHandle {
                            command_sender,
                            output_tap: Arc::new(Mutex::new(Some(receiver))),
                        });
                    aux_channels.insert(marker.clone(), channel);
                    if !pty_tap_tab_ids.contains(&tab.id) {
                        pty_tap_tab_ids.push(tab.id);
                    }
                    pending_pty_taps.push((tab.id, sender));
                    guidance_lines.push(format!(
                        "- {marker}: terminal session \"{}\" (profile: {}, host: {}, user: {})",
                        tab.title, profile.name, profile.host, profile.username
                    ));
                    resolved_names.push(tab.title.clone());
                }
            }
        }

        for (tab_id, sender) in pending_pty_taps {
            self.set_session_pty_tap_by_tab_id(tab_id, Some(sender));
        }

        let unresolved = targets
            .iter()
            .cloned()
            .into_iter()
            .filter(|name| {
                !resolved_names.iter().any(|resolved| {
                    resolved == name
                        || resolved
                            .strip_prefix(name)
                            .is_some_and(|suffix| suffix.starts_with(' '))
                })
            })
            .collect::<Vec<_>>();
        let guidance = (!guidance_lines.is_empty()).then(|| {
            format!(
                "Available execution targets:\n{}\nTo run a tool on a specific target, add \"target\": \"@name\" to the tool arguments, using one of the exact @ targets above.",
                guidance_lines.join("\n")
            )
        });

        ResolvedSessionAgentMentions {
            aux_channels,
            guidance,
            unresolved,
            pty_tap_tab_ids,
        }
    }

    fn resolve_mentions_from_tool_arguments(
        &mut self,
        arguments: &Value,
    ) -> ResolvedSessionAgentMentions {
        let Some(target) = arguments.get("target").and_then(Value::as_str) else {
            return ResolvedSessionAgentMentions::default();
        };
        let target = target.trim_start_matches('@').to_string();
        self.resolve_session_agent_mentions(&[target])
    }

    fn resolve_ai_provider_api_key(&self, provider: &AiProviderConfig) -> anyhow::Result<String> {
        if !provider.api_key_env.trim().is_empty()
            && let Ok(value) = std::env::var(provider.api_key_env.trim())
            && !value.trim().is_empty()
        {
            return Ok(value);
        }

        let api_key = self
            .services
            .secrets
            .get(&provider.id, SecretKind::AiProviderApiKey)?
            .unwrap_or_default();
        if api_key.trim().is_empty() {
            return Err(anyhow::anyhow!(i18n::string_args(
                "workspace.panel.agent.api_key_missing",
                &[("provider", &provider.name)],
            )));
        }

        Ok(api_key)
    }
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct SessionAgentTargetCandidate {
    pub(in crate::ui::shell) name: String,
    pub(in crate::ui::shell) detail: String,
    pub(in crate::ui::shell) resolved: bool,
}

struct ResolvedSessionAgentMentions {
    aux_channels: HashMap<String, AgentExecChannel>,
    guidance: Option<String>,
    unresolved: Vec<String>,
    pty_tap_tab_ids: Vec<usize>,
}

impl Default for ResolvedSessionAgentMentions {
    fn default() -> Self {
        Self {
            aux_channels: HashMap::new(),
            guidance: None,
            unresolved: Vec::new(),
            pty_tap_tab_ids: Vec::new(),
        }
    }
}

fn parse_tool_arguments(arguments: &str) -> Value {
    let trimmed = arguments.trim();
    if trimmed.is_empty() || trimmed == "No arguments" || trimmed == "null" {
        Value::Null
    } else {
        let value =
            serde_json::from_str(trimmed).unwrap_or_else(|_| Value::String(arguments.to_string()));
        value.get("arguments").cloned().unwrap_or(value)
    }
}

fn is_recoverable_session_agent_prompt_error(message: &str) -> bool {
    message.contains("PromptError")
        || message.contains("UnknownToolCall")
        || message.contains("ToolCallError")
        || message.contains("ToolServerError")
        || message.contains("MaxTurnError")
}

fn chat_record_from_session_agent_message(
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
        SessionAgentMessageRole::Error => return None,
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
        sort_order: index as i64,
        created_at: now,
    })
}

fn session_agent_message_from_record(record: ChatMessageRecord) -> SessionAgentMessage {
    match record.role {
        ChatMessageRole::User => SessionAgentMessage::user(record.content),
        ChatMessageRole::Assistant => SessionAgentMessage::assistant_raw(record.content),
        ChatMessageRole::Thinking => SessionAgentMessage::thinking_raw(record.content),
        ChatMessageRole::ToolCall => {
            let summary = record
                .tool_summary
                .clone()
                .filter(|summary| !summary.trim().is_empty())
                .unwrap_or_else(|| record.content.clone());
            SessionAgentMessage::tool_call(SessionAgentToolCall {
                id: record.id,
                name: record
                    .tool_name
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| "tool".to_string()),
                arguments: record.content,
                summary,
                status: SessionAgentToolStatus::Completed,
                requires_confirmation: false,
                confirmation_note: record.tool_summary,
                expanded: false,
            })
        }
    }
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}
