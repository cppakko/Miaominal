use super::super::*;
use crate::ui::i18n;
use crate::ui::shell::state::TokenUsage;
use gpui_component::WindowExt as _;
use miaominal_agent::{
    AgentChatEvent, AgentChatMessage, AgentChatProvider, AgentChatProviderKind, AgentChatRequest,
    AgentChatRole, AgentChatToolEvent, AgentExecChannel, AgentMode, AgentToolCallRequest,
    AgentToolResultContinuationRequest, AgentToolSet, TerminalExecHandle,
};
use miaominal_secrets::SecretKind;
use miaominal_settings::{AiProviderConfig, AiProviderKind};
use miaominal_storage::chat_store::{ChatMessageRecord, ChatMessageRole};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, mpsc};

const SESSION_AGENT_FOLLOW_BOTTOM_INTERVAL: Duration = Duration::from_millis(16);
const SESSION_AGENT_FOLLOW_BOTTOM_TICKS: usize = 50;
const SESSION_AGENT_FOLLOW_BOTTOM_USER_SCROLL_COOLDOWN: Duration = Duration::from_millis(1000);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum PromptHistoryDirection {
    Previous,
    Next,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SessionAgentBackgroundNotificationKind {
    ToolApprovalRequired { tool_name: String },
    ReplyReady,
}

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
        let mut content = message.content.clone();
        let images: Vec<miaominal_core::chat_attachment::ChatImage> = message
            .attachments
            .iter()
            .filter_map(|attachment| match &attachment.content {
                miaominal_core::chat_attachment::ChatAttachmentContent::Image(image) => {
                    Some(image.clone())
                }
                miaominal_core::chat_attachment::ChatAttachmentContent::TextFile(_text_file) => {
                    embed_text_attachments_into_content(
                        &mut content,
                        std::slice::from_ref(attachment),
                    );
                    None
                }
            })
            .collect();
        Self {
            role: match message.role {
                SessionAgentMessageRole::User => AgentChatRole::User,
                SessionAgentMessageRole::Assistant
                | SessionAgentMessageRole::Thinking
                | SessionAgentMessageRole::ToolCall
                | SessionAgentMessageRole::Error => AgentChatRole::Assistant,
            },
            content,
            images,
        }
    }
}

/// Appends text-file attachment content blocks into the given string
/// using fenced code blocks (e.g. ```` ```rust\n...\n``` ````).
/// Image attachments are ignored — callers must handle those separately.
fn embed_text_attachments_into_content(
    content: &mut String,
    attachments: &[miaominal_core::chat_attachment::ChatAttachment],
) {
    for attachment in attachments {
        if let miaominal_core::chat_attachment::ChatAttachmentContent::TextFile(text_file) =
            &attachment.content
        {
            let language = text_file.language.as_deref().unwrap_or("");
            let fence = if language.is_empty() {
                String::new()
            } else {
                format!("\n```{language}")
            };
            let close_fence = if language.is_empty() {
                String::new()
            } else {
                "\n```".to_string()
            };
            let block = format!(
                "\n\n[Attached file: {}]{fence}\n{}\n{close_fence}",
                attachment.filename, text_file.text
            );
            content.push_str(&block);
        }
    }
}

impl AppView {
    pub(in crate::ui::shell) fn is_session_agent_prompt_input_focused(
        &self,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        window.focused_input(cx).is_some_and(|input| {
            input.entity_id() == self.workspace_forms.agent.prompt_input.entity_id()
        })
    }

    fn clear_session_agent_prompt_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        set_input_value(
            &self.workspace_forms.agent.prompt_input,
            String::new(),
            window,
            cx,
        );
        self.session_agent.at_mention_query = None;
        self.session_agent.at_mention_anchor = 0;
        self.session_agent.prompt_history_cursor = None;
        self.session_agent.prompt_history_draft = None;
        cx.notify();
    }

    pub(in crate::ui::shell) fn clear_focused_session_agent_prompt_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_session_agent_prompt_input(window, cx);
    }

    fn record_session_agent_prompt_history(&mut self, prompt: &str) {
        const SESSION_AGENT_PROMPT_HISTORY_LIMIT: usize = 100;

        if prompt.trim().is_empty() {
            return;
        }

        if self
            .session_agent
            .prompt_history
            .last()
            .is_some_and(|previous| previous == prompt)
        {
            self.session_agent.prompt_history_cursor = None;
            self.session_agent.prompt_history_draft = None;
            return;
        }

        self.session_agent.prompt_history.push(prompt.to_string());
        if self.session_agent.prompt_history.len() > SESSION_AGENT_PROMPT_HISTORY_LIMIT {
            let overflow =
                self.session_agent.prompt_history.len() - SESSION_AGENT_PROMPT_HISTORY_LIMIT;
            self.session_agent.prompt_history.drain(0..overflow);
        }
        self.session_agent.prompt_history_cursor = None;
        self.session_agent.prompt_history_draft = None;
    }

    pub(in crate::ui::shell) fn reset_session_agent_prompt_history_cursor(&mut self) {
        self.session_agent.prompt_history_cursor = None;
        self.session_agent.prompt_history_draft = None;
    }

    pub(in crate::ui::shell) fn browse_session_agent_prompt_history(
        &mut self,
        direction: PromptHistoryDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.session_agent.prompt_history.is_empty() {
            return false;
        }

        let current_value = self
            .workspace_forms
            .agent
            .prompt_input
            .read(cx)
            .value()
            .to_string();
        let next_cursor = match (direction, self.session_agent.prompt_history_cursor) {
            (PromptHistoryDirection::Previous, None) => {
                self.session_agent.prompt_history_draft = Some(current_value);
                Some(self.session_agent.prompt_history.len() - 1)
            }
            (PromptHistoryDirection::Previous, Some(cursor)) => Some(cursor.saturating_sub(1)),
            (PromptHistoryDirection::Next, Some(cursor))
                if cursor + 1 < self.session_agent.prompt_history.len() =>
            {
                Some(cursor + 1)
            }
            (PromptHistoryDirection::Next, Some(_)) => None,
            (PromptHistoryDirection::Next, None) => return true,
        };

        let next_value = next_cursor
            .and_then(|cursor| self.session_agent.prompt_history.get(cursor).cloned())
            .unwrap_or_else(|| {
                self.session_agent
                    .prompt_history_draft
                    .take()
                    .unwrap_or_default()
            });
        self.session_agent.prompt_history_cursor = next_cursor;
        set_input_value(
            &self.workspace_forms.agent.prompt_input,
            next_value,
            window,
            cx,
        );
        self.update_session_agent_at_mention_state(cx);
        true
    }

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
                            .unwrap_or_else(|| {
                                i18n::string("workspace.panel.agent.messages.terminal_session")
                            });
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
        if self.session_agent_is_physically_scrolled_to_bottom() {
            return true;
        }

        if self
            .workspace_state
            .session_agent_follow_bottom_disabled_until
            .is_some_and(|until| Instant::now() < until)
        {
            return false;
        }

        false
    }

    fn session_agent_is_physically_scrolled_to_bottom(&self) -> bool {
        let scroll_handle = &self.workspace_state.session_agent_scroll_handle;
        let offset = scroll_handle.offset();
        let max_offset = scroll_handle.max_offset();
        if max_offset.y <= px(2.0) {
            return true;
        }
        (offset.y + max_offset.y).abs() <= px(2.0)
    }

    fn clear_expired_session_agent_follow_bottom_cooldown(&mut self) {
        if self.session_agent_is_physically_scrolled_to_bottom()
            || self
                .workspace_state
                .session_agent_follow_bottom_disabled_until
                .is_some_and(|until| Instant::now() >= until)
        {
            self.workspace_state
                .session_agent_follow_bottom_disabled_until = None;
        }
    }

    pub(in crate::ui::shell) fn stop_session_agent_follow_bottom(&mut self, user_initiated: bool) {
        self.workspace_state.session_agent_follow_bottom_generation = self
            .workspace_state
            .session_agent_follow_bottom_generation
            .wrapping_add(1);
        if user_initiated {
            self.workspace_state
                .session_agent_follow_bottom_disabled_until =
                Some(Instant::now() + SESSION_AGENT_FOLLOW_BOTTOM_USER_SCROLL_COOLDOWN);
        }
    }

    pub(in crate::ui::shell) fn handle_session_agent_scroll_wheel(
        &mut self,
        _event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop_session_agent_follow_bottom(true);
        cx.on_next_frame(window, |this, _window, cx| {
            if this.session_agent_is_physically_scrolled_to_bottom() {
                this.keep_session_agent_following_bottom_for_layout(cx);
            }
        });
    }

    fn keep_session_agent_following_bottom_for_layout(&mut self, cx: &mut Context<Self>) {
        self.workspace_state
            .session_agent_follow_bottom_disabled_until = None;
        self.workspace_state.session_agent_follow_bottom_generation = self
            .workspace_state
            .session_agent_follow_bottom_generation
            .wrapping_add(1);
        let generation = self.workspace_state.session_agent_follow_bottom_generation;
        self.workspace_state
            .session_agent_scroll_handle
            .scroll_to_bottom();

        cx.spawn(async move |this, cx| {
            for _ in 0..SESSION_AGENT_FOLLOW_BOTTOM_TICKS {
                cx.background_executor()
                    .timer(SESSION_AGENT_FOLLOW_BOTTOM_INTERVAL)
                    .await;

                let keep_following = this
                    .update(cx, |this, cx| {
                        if this.workspace_state.session_agent_follow_bottom_generation != generation
                            || !this.panels.session_agent_panel_open
                            || this.session_agent.panel_view != ChatPanelView::Conversation
                        {
                            return false;
                        }

                        this.workspace_state
                            .session_agent_scroll_handle
                            .scroll_to_bottom();
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);

                if !keep_following {
                    break;
                }
            }
        })
        .detach();
    }

    fn reset_session_agent_scroll(&self) {
        self.workspace_state
            .session_agent_scroll_handle
            .set_offset(Point::new(px(0.0), px(0.0)));
    }

    fn scroll_session_agent_to_bottom_if_following(
        &mut self,
        previous_message_count: usize,
        was_scrolled_to_bottom: bool,
        content_may_have_grown: bool,
        cx: &mut Context<Self>,
    ) {
        let new_block_added = self.session_agent.messages.len() > previous_message_count;
        if was_scrolled_to_bottom && (new_block_added || content_may_have_grown) {
            self.keep_session_agent_following_bottom_for_layout(cx);
        }
    }

    fn push_session_agent_message(&mut self, message: SessionAgentMessage, cx: &mut Context<Self>) {
        let previous_message_count = self.session_agent.messages.len();
        let was_scrolled_to_bottom = self.session_agent_is_scrolled_to_bottom();
        self.session_agent.push_message_with_enter_motion(message);
        self.scroll_session_agent_to_bottom_if_following(
            previous_message_count,
            was_scrolled_to_bottom,
            false,
            cx,
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
        self.reset_session_agent_scroll();
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
        self.workspace_forms.chat_search.session_filter_open = false;
        self.workspace_forms.chat_search.session_filter_visible = false;
        self.workspace_forms.chat_search.session_filter_visibility = 0.0;
        self.workspace_forms.chat_search.session_filter_animation = None;
        self.reset_session_agent_scroll();
        self.workspace_forms.agent.editing_title = false;
        cx.notify();
    }

    pub(in crate::ui::shell) fn start_session_agent_conversation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Clear any active search state
        self.session_agent.search_query = None;
        self.session_agent.search_match_indices.clear();
        self.session_agent.search_current_match = None;
        self.session_agent.search_scroll_target = None;
        let chat_search = &mut self.workspace_forms.chat_search;
        chat_search.conversation_search_open = false;
        chat_search.conversation_search_visible = false;
        chat_search.conversation_search_visibility = 0.0;
        chat_search.conversation_search_animation = None;
        chat_search.match_count = 0;
        chat_search.current_match = None;
        chat_search.status = None;

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
        // Clear any active search state when loading a session
        self.session_agent.search_query = None;
        self.session_agent.search_match_indices.clear();
        self.session_agent.search_current_match = None;
        self.session_agent.search_scroll_target = None;
        let chat_search = &mut self.workspace_forms.chat_search;
        chat_search.conversation_search_open = false;
        chat_search.conversation_search_visible = false;
        chat_search.conversation_search_visibility = 0.0;
        chat_search.conversation_search_animation = None;
        chat_search.match_count = 0;
        chat_search.current_match = None;
        chat_search.status = None;

        if self.session_agent.session_id.as_deref() == Some(session_id.as_str()) {
            self.session_agent.panel_view = ChatPanelView::Conversation;
            self.reset_session_agent_scroll();
            cx.notify();
            return;
        }

        self.stash_current_session_agent();
        if let Some(mut state) = self.session_agent_sessions.remove(&session_id) {
            state.panel_view = ChatPanelView::Conversation;
            self.session_agent = state;
            self.reset_session_agent_scroll();
            self.status_message = i18n::string("workspace.panel.agent.messages.restored");
            cx.notify();
            return;
        }

        let Some(chat_service) = self.services.chat_service.as_ref() else {
            self.status_message =
                i18n::string("workspace.panel.agent.messages.history_unavailable");
            cx.notify();
            return;
        };

        let messages = match chat_service.load_session_messages(&session_id) {
            Ok(messages) => messages,
            Err(error) => {
                let message = i18n::string_args(
                    "workspace.panel.agent.messages.load_failed",
                    &[("error", &error.to_string())],
                );
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
        self.reset_session_agent_scroll();
        self.status_message = i18n::string("workspace.panel.agent.messages.history_loaded");
        cx.notify();
    }

    pub(in crate::ui::shell) fn delete_session_agent_chat(
        &mut self,
        session_id: String,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent_session_is_busy(&session_id) {
            self.status_message = i18n::string("workspace.panel.agent.messages.stop_before_delete");
            cx.notify();
            return;
        }

        let Some(chat_service) = self.services.chat_service.as_ref() else {
            self.status_message =
                i18n::string("workspace.panel.agent.messages.history_unavailable");
            cx.notify();
            return;
        };

        if let Err(error) = chat_service.delete_session(&session_id) {
            self.status_message = i18n::string_args(
                "workspace.panel.agent.messages.delete_failed",
                &[("error", &error.to_string())],
            );
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
        self.status_message = i18n::string("workspace.panel.agent.messages.deleted");
        cx.notify();
    }

    pub(in crate::ui::shell) fn request_session_agent_chat_delete(
        &mut self,
        session_id: String,
        title: String,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent_session_is_busy(&session_id) {
            self.status_message = i18n::string("workspace.panel.agent.messages.stop_before_delete");
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

    pub(in crate::ui::shell) fn rename_session_agent_chat(
        &mut self,
        session_id: String,
        title: String,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent_session_is_busy(&session_id) {
            return;
        }

        let new_title = title.trim().to_string();
        if new_title.is_empty() {
            return;
        }

        let Some(chat_service) = self.services.chat_service.as_ref() else {
            self.status_message =
                i18n::string("workspace.panel.agent.messages.history_unavailable");
            cx.notify();
            return;
        };

        if let Err(error) = chat_service.update_session_title(&session_id, &new_title) {
            self.status_message = i18n::string_args(
                "workspace.panel.agent.messages.rename_failed",
                &[("error", &error.to_string())],
            );
            cx.notify();
            return;
        }

        // Update in-memory title if this is the current session
        if self.session_agent.session_id.as_deref() == Some(session_id.as_str()) {
            self.session_agent.title = Some(new_title);
        }

        self.refresh_chat_sessions();
        self.status_message = i18n::string("workspace.panel.agent.messages.renamed");
        cx.notify();
    }

    pub(in crate::ui::shell) fn request_session_agent_chat_rename(
        &mut self,
        session_id: String,
        current_title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent_session_is_busy(&session_id) {
            return;
        }

        self.workspace_forms
            .agent
            .rename_title_input
            .update(cx, |input, cx| {
                input.set_value(current_title.clone(), window, cx);
            });

        self.dialogs.pending_chat_session_rename = Some(PendingChatSessionRenameState {
            session_id,
            current_title,
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn cancel_session_agent_chat_rename(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if let Some(pending) = self.dialogs.pending_chat_session_rename.take() {
            self.start_dialog_exit(DialogOverlaySnapshot::ChatSessionRename(pending), cx);
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn confirm_session_agent_chat_rename(
        &mut self,
        new_title: String,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.dialogs.pending_chat_session_rename.take() else {
            return;
        };
        self.start_dialog_exit(
            DialogOverlaySnapshot::ChatSessionRename(pending.clone()),
            cx,
        );
        self.rename_session_agent_chat(pending.session_id, new_title, cx);
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

    pub(in crate::ui::shell) fn session_agent_session_needs_approval(
        &self,
        session_id: &str,
    ) -> bool {
        if self.session_agent.session_id.as_deref() == Some(session_id) {
            return self.session_agent.has_tool_call_waiting_for_confirmation();
        }
        self.session_agent_sessions
            .get(session_id)
            .is_some_and(SessionAgentState::has_tool_call_waiting_for_confirmation)
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
            let targets = mentions
                .unresolved
                .iter()
                .map(|name| format!("@{name}"))
                .collect::<Vec<_>>()
                .join(", ");
            let message = i18n::string_args(
                "workspace.panel.agent.messages.unknown_target",
                &[("targets", &targets)],
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
        let attachments = std::mem::take(&mut self.session_agent.pending_attachments);
        let prompt_images: Vec<miaominal_core::chat_attachment::ChatImage> = attachments
            .iter()
            .filter_map(|attachment| attachment.as_image().cloned())
            .collect();
        let mut llm_prompt = model_prompt.clone();
        embed_text_attachments_into_content(&mut llm_prompt, &attachments);
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

        self.push_session_agent_message(
            SessionAgentMessage::user_with_attachments(model_prompt.clone(), attachments),
            cx,
        );
        self.record_session_agent_prompt_history(&prompt);
        self.persist_session_agent_chat();
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
                        prompt: llm_prompt,
                        prompt_images,
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
        let use_pty = self.session_agent.exec_mode == AgentExecMode::Pty;
        let pty_commands = if use_pty {
            let Some(index) = self.active_terminal_session_index() else {
                let message =
                    i18n::string("workspace.panel.agent.messages.pty_requires_active_session");
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
                let message =
                    i18n::string("workspace.panel.agent.messages.pty_requires_connected_session");
                self.session_agent.last_error = Some(message.clone());
                self.status_message = message;
                cx.notify();
                return None;
            };
            Some(commands)
        } else {
            None
        };

        let pty_tap_active = pty_commands.is_some();
        let tools = self.active_profile().cloned().map(|profile| {
            let mut channel = self.agent_exec_channel_for_profile(profile);
            if let Some(command_sender) = pty_commands.clone() {
                let (sender, receiver) = mpsc::unbounded_channel();
                self.set_active_session_pty_tap(Some(sender));
                channel = channel.with_terminal_exec(TerminalExecHandle {
                    command_sender,
                    output_tap: Arc::new(Mutex::new(Some(receiver))),
                });
            }
            channel = channel.with_aux_channels(aux_channels);
            let mode = self.session_agent.agent_mode;
            AgentToolSet::for_channel(channel, mode)
        });

        Some((tools, pty_tap_active))
    }

    pub(in crate::ui::shell) fn stop_session_agent_stream(&mut self, cx: &mut Context<Self>) {
        let had_pending_task = self.session_agent.pending_task.take().is_some();
        let had_active_tool = self.session_agent.reject_active_tool_calls(&i18n::string(
            "workspace.panel.agent.messages.stopped_by_user",
        ));
        if !had_pending_task && !had_active_tool {
            return;
        }

        self.session_agent.active_request_id = self.session_agent.active_request_id.wrapping_add(1);
        self.session_agent.finish_stopped_turn();
        self.set_active_session_pty_tap(None);
        for tab in &mut self.workspace_state.tabs {
            if let Some(session) = tab.as_session_mut() {
                session.pty_output_tap = None;
            }
        }
        self.status_message = i18n::string("workspace.panel.agent.messages.stopped");
        self.session_agent.last_error = None;
        self.persist_session_agent_chat();
        cx.notify();
    }

    pub(in crate::ui::shell) fn approve_session_agent_tool_call(
        &mut self,
        tool_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(tool_call) = self.session_agent.tool_call(&tool_id) else {
            self.status_message = i18n::string("workspace.panel.agent.messages.tool_not_found");
            cx.notify();
            return;
        };
        let Some(profile) = self.active_profile().cloned() else {
            self.status_message =
                i18n::string("workspace.panel.agent.messages.no_active_session_for_approval");
            cx.notify();
            return;
        };

        let arguments = parse_tool_arguments(&tool_call.arguments);
        let reasoning = self.session_agent.reasoning_before_tool_call(&tool_id);
        self.session_agent.approve_tool_call(&tool_id);
        self.status_message = i18n::string("workspace.panel.agent.messages.tool_approved_running");
        let approval_session_id = self.ensure_session_agent_session();

        let use_pty = self.session_agent.exec_mode == AgentExecMode::Pty;
        let pty_handle = if use_pty {
            let Some(index) = self.active_terminal_session_index() else {
                let message =
                    i18n::string("workspace.panel.agent.messages.pty_requires_active_session");
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
                let message =
                    i18n::string("workspace.panel.agent.messages.pty_requires_connected_session");
                self.session_agent.fail_tool_call(&tool_id, message.clone());
                self.status_message = message;
                cx.notify();
                return;
            };
            let (sender, receiver) = mpsc::unbounded_channel();
            self.set_active_session_pty_tap(Some(sender));
            Some(TerminalExecHandle {
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
            let worker_tool_name = tool_name.clone();
            let handle = miaominal_agent::agent_runtime().spawn_blocking(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| anyhow::anyhow!(error))?;
                runtime.block_on(async move {
                    let mut channel = AgentExecChannel::for_profile(
                        profile,
                        sessions,
                        secrets.clone(),
                        known_hosts,
                    );
                    if web_search_config.enabled {
                        let web_search_api_key =
                            secrets.get("web_search", SecretKind::WebSearchApiKey)?;
                        channel =
                            channel.with_web_search_config(web_search_config, web_search_api_key);
                    }
                    if let Some(ref pty_handle) = pty_handle {
                        channel = channel.with_terminal_exec(pty_handle.clone());
                    }
                    channel = channel.with_aux_channels(approval_mentions.aux_channels);
                    channel
                        .call_tool(AgentToolCallRequest {
                            tool_name: worker_tool_name,
                            arguments,
                            approved: true,
                            route: None,
                            skip_policy: false,
                        })
                        .await
                        .map_err(anyhow::Error::from)
                        .and_then(|response| {
                            serde_json::to_string(&response).map_err(|error| anyhow::anyhow!(error))
                        })
                })
            });

            let result = match handle.await {
                Ok(result) => result,
                Err(e) => Err(anyhow::anyhow!("agent tool task failed: {e}")),
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
                                this.status_message =
                                    i18n::string("workspace.panel.agent.messages.stopped");
                                cx.notify();
                                return;
                            }
                            this.session_agent
                                .complete_tool_call(&tool_id, result.clone());
                            this.status_message = i18n::string(
                                "workspace.panel.agent.messages.tool_finished_continuing",
                            );
                            (result, false)
                        }
                        Err(error) => {
                            if !matches!(
                                this.session_agent
                                    .tool_call(&tool_id)
                                    .map(|tool_call| tool_call.status),
                                Some(SessionAgentToolStatus::InProgress)
                            ) {
                                this.status_message =
                                    i18n::string("workspace.panel.agent.messages.stopped");
                                cx.notify();
                                return;
                            }
                            let result = format!("tool failed after approval: {error}");
                            this.session_agent.fail_tool_call(&tool_id, result.clone());
                            this.status_message = i18n::string(
                                "workspace.panel.agent.messages.tool_failed_continuing",
                            );
                            (result, true)
                        }
                    };
                    this.scroll_session_agent_to_bottom_if_following(
                        previous_message_count,
                        was_scrolled_to_bottom,
                        true,
                        cx,
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
            self.push_session_agent_message(
                SessionAgentMessage::error(i18n::string(
                    "workspace.panel.agent.messages.approved_tool_result_skipped",
                )),
                cx,
            );
            self.status_message = i18n::string("workspace.panel.agent.messages.already_processing");
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
        self.session_agent.start_assistant_reply();
        self.scroll_session_agent_to_bottom_if_following(
            previous_message_count,
            was_scrolled_to_bottom,
            false,
            cx,
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
        self.status_message = i18n::string("workspace.panel.agent.messages.tool_denied");
        self.persist_session_agent_chat();
        cx.notify();
    }

    pub(in crate::ui::shell) fn copy_session_agent_text(
        &mut self,
        label: String,
        text: String,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.status_message = i18n::string_args(
            "workspace.panel.agent.messages.copy_success",
            &[("label", &label)],
        );
        cx.notify();
    }

    fn session_agent_session_is_foreground(&self, session_id: &str) -> bool {
        self.panels.session_agent_panel_open
            && self.active_terminal_session_index().is_some()
            && self.session_agent.panel_view == ChatPanelView::Conversation
            && self.session_agent.session_id.as_deref() == Some(session_id)
    }

    fn session_agent_notification_chat_label(&self) -> String {
        if let Some(title) = self
            .session_agent
            .title
            .as_ref()
            .map(|title| title.trim())
            .filter(|title| !title.is_empty())
        {
            return truncate_with_ellipsis(title, 48);
        }

        let fallback_title = i18n::string("workspace.panel.agent.sidebar_title");
        self.session_agent
            .messages
            .iter()
            .find(|message| message.role == SessionAgentMessageRole::User)
            .map(|message| {
                truncate_with_ellipsis(
                    message
                        .content
                        .lines()
                        .next()
                        .unwrap_or(fallback_title.as_str())
                        .trim(),
                    48,
                )
            })
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| fallback_title.clone())
    }

    fn notify_background_session_agent(
        &mut self,
        chat_label: String,
        kind: SessionAgentBackgroundNotificationKind,
        cx: &mut Context<Self>,
    ) {
        let (title, message, notification) = match kind {
            SessionAgentBackgroundNotificationKind::ToolApprovalRequired { tool_name } => {
                let title = i18n::string("workspace.panel.agent.notifications.tool_approval_title");
                let message = i18n::string_args(
                    "workspace.panel.agent.notifications.tool_approval",
                    &[("chat", &chat_label), ("tool", &tool_name)],
                );
                let notification = Self::warning_notification(title.clone(), message.clone());
                (title, message, notification)
            }
            SessionAgentBackgroundNotificationKind::ReplyReady => {
                let title = i18n::string("workspace.panel.agent.notifications.reply_ready_title");
                let message = i18n::string_args(
                    "workspace.panel.agent.notifications.reply_ready",
                    &[("chat", &chat_label)],
                );
                let notification = Self::success_notification(title.clone(), message.clone());
                (title, message, notification)
            }
        };

        self.status_message = format!("{title}: {message}");
        self.with_active_window(cx, move |window, cx| {
            window.push_notification(notification, cx);
        });
    }

    fn apply_session_agent_event(
        &mut self,
        request_id: u64,
        event: AgentChatEvent,
        cx: &mut Context<Self>,
    ) -> Option<SessionAgentBackgroundNotificationKind> {
        if self.session_agent.active_request_id != request_id {
            return None;
        }

        self.clear_expired_session_agent_follow_bottom_cooldown();
        let previous_message_count = self.session_agent.messages.len();
        let was_scrolled_to_bottom = self.session_agent_is_scrolled_to_bottom();
        let content_may_have_grown = matches!(
            &event,
            AgentChatEvent::TextDelta(_)
                | AgentChatEvent::ThinkingDelta(_)
                | AgentChatEvent::ToolCallDelta { .. }
                | AgentChatEvent::ToolCallCompleted { .. }
                | AgentChatEvent::ToolCallApprovalRequired { .. }
                | AgentChatEvent::Finished(_)
        );
        let notification_kind = match event {
            AgentChatEvent::TextDelta(delta) => {
                self.session_agent.append_assistant_delta(delta);
                self.session_agent.last_error = None;
                None
            }
            AgentChatEvent::ThinkingDelta(delta) => {
                self.session_agent.append_thinking_delta(delta);
                self.status_message = i18n::string("workspace.panel.agent.thinking");
                None
            }
            AgentChatEvent::ToolCallStarted(tool) => {
                self.session_agent.push_tool_call(
                    tool.id,
                    tool.name,
                    tool.arguments,
                    SessionAgentToolStatus::InProgress,
                );
                None
            }
            AgentChatEvent::ToolCallDelta { id, delta } => {
                self.session_agent.append_tool_call_delta(&id, delta);
                None
            }
            AgentChatEvent::ToolCallCompleted { id, result } => {
                self.session_agent.complete_tool_call(&id, result);
                self.session_agent.tool_call(&id).and_then(|tool_call| {
                    if tool_call.status == SessionAgentToolStatus::WaitingForConfirmation {
                        Some(
                            SessionAgentBackgroundNotificationKind::ToolApprovalRequired {
                                tool_name: tool_call.name,
                            },
                        )
                    } else {
                        None
                    }
                })
            }
            AgentChatEvent::ToolCallApprovalRequired { id, message } => {
                if matches!(
                    self.session_agent.agent_mode,
                    AgentMode::NonBlocking | AgentMode::FullAuto
                ) {
                    self.approve_session_agent_tool_call(id, cx);
                    None
                } else {
                    let tool_name = self
                        .session_agent
                        .tool_call(&id)
                        .map(|tool_call| tool_call.name)
                        .unwrap_or_else(|| i18n::string("workspace.panel.agent.tool"));
                    self.session_agent
                        .require_tool_call_confirmation(&id, message);
                    self.finish_session_agent_stream(request_id, cx);
                    Some(SessionAgentBackgroundNotificationKind::ToolApprovalRequired { tool_name })
                }
            }
            AgentChatEvent::Finished(reply) => {
                self.session_agent.finish_assistant_reply(reply);
                if self.finish_session_agent_stream(request_id, cx)
                    && !self.session_agent.has_active_tool_call()
                {
                    Some(SessionAgentBackgroundNotificationKind::ReplyReady)
                } else {
                    None
                }
            }
            AgentChatEvent::TokenUsage {
                input_tokens,
                output_tokens,
            } => {
                self.session_agent.last_usage = Some(TokenUsage {
                    input_tokens,
                    output_tokens,
                });
                None
            }
        };
        self.scroll_session_agent_to_bottom_if_following(
            previous_message_count,
            was_scrolled_to_bottom,
            content_may_have_grown,
            cx,
        );

        cx.notify();
        notification_kind
    }

    fn apply_session_agent_event_for_session(
        &mut self,
        session_id: &str,
        request_id: u64,
        event: AgentChatEvent,
        cx: &mut Context<Self>,
    ) {
        let is_loaded_session = self.session_agent.session_id.as_deref() == Some(session_id);
        let should_notify = !self.session_agent_session_is_foreground(session_id);
        let mut notification = None;
        let updated = self.with_session_agent_state(session_id, |this| {
            let kind = this.apply_session_agent_event(request_id, event, cx);
            if should_notify {
                notification =
                    kind.map(|kind| (this.session_agent_notification_chat_label(), kind));
            }
        });
        if !updated {
            return;
        }

        if let Some((chat_label, kind)) = notification {
            self.notify_background_session_agent(chat_label, kind, cx);
        }

        if !is_loaded_session {
            self.refresh_chat_sessions();
            cx.notify();
        }
    }

    fn finish_session_agent_stream(&mut self, request_id: u64, cx: &mut Context<Self>) -> bool {
        if self.session_agent.active_request_id != request_id {
            return false;
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
            i18n::string("workspace.panel.agent.messages.waiting_for_tool_approval")
        } else {
            i18n::string("workspace.panel.agent.reply_ready")
        };

        self.persist_session_agent_chat();

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
        true
    }

    fn finish_session_agent_stream_for_session(
        &mut self,
        session_id: &str,
        request_id: u64,
        cx: &mut Context<Self>,
    ) {
        let is_loaded_session = self.session_agent.session_id.as_deref() == Some(session_id);
        let should_notify = !self.session_agent_session_is_foreground(session_id);
        let mut notification = None;
        let updated = self.with_session_agent_state(session_id, |this| {
            if this.finish_session_agent_stream(request_id, cx)
                && should_notify
                && !this.session_agent.has_active_tool_call()
            {
                notification = Some((
                    this.session_agent_notification_chat_label(),
                    SessionAgentBackgroundNotificationKind::ReplyReady,
                ));
            }
        });
        if !updated {
            return;
        }

        if let Some((chat_label, kind)) = notification {
            self.notify_background_session_agent(chat_label, kind, cx);
        }

        if !is_loaded_session {
            self.refresh_chat_sessions();
            cx.notify();
        }
    }

    fn fail_session_agent_stream(
        &mut self,
        request_id: u64,
        error: anyhow::Error,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.session_agent.active_request_id != request_id {
            return false;
        }

        self.session_agent.pending_task = None;
        self.session_agent.active_request_id = 0;
        let message = error.to_string();
        self.session_agent.last_error = Some(message.clone());
        self.push_session_agent_message(SessionAgentMessage::error(message.clone()), cx);
        self.status_message = message;
        self.persist_session_agent_chat();
        cx.notify();
        true
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
        let is_loaded_session = self.session_agent.session_id.as_deref() == Some(session_id);
        let updated = self.with_session_agent_state(session_id, |this| {
            this.handle_session_agent_stream_error(request_id, error, cx);
        });
        if !updated {
            return;
        }

        if !is_loaded_session {
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
            SessionAgentMessage::error(i18n::string_args(
                "workspace.panel.agent.messages.tool_loop_error_message",
                &[("message", &message)],
            )),
            cx,
        );
        self.status_message =
            i18n::string("workspace.panel.agent.messages.tool_loop_error_returned");

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
        self.session_agent.start_assistant_reply();
        self.scroll_session_agent_to_bottom_if_following(
            previous_message_count,
            was_scrolled_to_bottom,
            false,
            cx,
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
                        prompt_images: Vec::new(),
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
            temperature: provider.temperature,
            max_tokens: provider.max_tokens,
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
                        .with_terminal_exec(TerminalExecHandle {
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
            .filter(|name| {
                !resolved_names.iter().any(|resolved| {
                    resolved == *name
                        || resolved
                            .strip_prefix(*name)
                            .is_some_and(|suffix| suffix.starts_with(' '))
                })
            })
            .cloned()
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

#[derive(Default)]
struct ResolvedSessionAgentMentions {
    aux_channels: HashMap<String, AgentExecChannel>,
    guidance: Option<String>,
    unresolved: Vec<String>,
    pty_tap_tab_ids: Vec<usize>,
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

/// Serializes a message's attachments to JSON for persistence. Only user
/// messages with non-empty attachments are serialized; all other roles and
/// empty-attachment messages return `None` so the DB column stays NULL.
fn serialize_message_attachments(message: &SessionAgentMessage) -> Option<String> {
    if message.role != SessionAgentMessageRole::User || message.attachments.is_empty() {
        return None;
    }
    serde_json::to_string(&message.attachments).ok()
}

fn session_agent_message_from_record(record: ChatMessageRecord) -> SessionAgentMessage {
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

fn tool_status_as_str(status: SessionAgentToolStatus) -> &'static str {
    match status {
        SessionAgentToolStatus::Pending => "pending",
        SessionAgentToolStatus::WaitingForConfirmation => "waiting_for_confirmation",
        SessionAgentToolStatus::InProgress => "in_progress",
        SessionAgentToolStatus::Completed => "completed",
        SessionAgentToolStatus::Failed => "failed",
        SessionAgentToolStatus::Rejected => "rejected",
    }
}

fn restored_tool_status_and_note(
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

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_status_strings_round_trip_completed_failed_and_rejected() {
        assert_eq!(
            restored_tool_status_and_note(
                Some(tool_status_as_str(SessionAgentToolStatus::Completed)),
                Some("done".to_string()),
            ),
            (SessionAgentToolStatus::Completed, Some("done".to_string()))
        );
        assert_eq!(
            restored_tool_status_and_note(
                Some(tool_status_as_str(SessionAgentToolStatus::Failed)),
                Some("boom".to_string()),
            ),
            (SessionAgentToolStatus::Failed, Some("boom".to_string()))
        );
        assert_eq!(
            restored_tool_status_and_note(
                Some(tool_status_as_str(SessionAgentToolStatus::Rejected)),
                Some("nope".to_string()),
            ),
            (SessionAgentToolStatus::Rejected, Some("nope".to_string()))
        );
    }

    #[test]
    fn unfinished_tool_statuses_restore_as_interrupted_rejected_tools() {
        let interrupted =
            i18n::string("workspace.panel.agent.messages.tool_interrupted_before_completion");
        for status in [
            SessionAgentToolStatus::Pending,
            SessionAgentToolStatus::WaitingForConfirmation,
            SessionAgentToolStatus::InProgress,
        ] {
            assert_eq!(
                restored_tool_status_and_note(
                    Some(tool_status_as_str(status)),
                    Some("old note".to_string()),
                ),
                (SessionAgentToolStatus::Rejected, Some(interrupted.clone()),)
            );
        }
    }

    #[test]
    fn missing_tool_status_keeps_legacy_tool_calls_completed() {
        assert_eq!(
            restored_tool_status_and_note(None, Some("legacy result".to_string())),
            (
                SessionAgentToolStatus::Completed,
                Some("legacy result".to_string()),
            )
        );
    }

    #[test]
    fn restored_chat_messages_do_not_receive_enter_motion_keys() {
        let message = session_agent_message_from_record(ChatMessageRecord {
            id: "message-1".to_string(),
            session_id: "session-1".to_string(),
            role: ChatMessageRole::Assistant,
            content: "hello".to_string(),
            tool_name: None,
            tool_summary: None,
            tool_status: None,
            sort_order: 0,
            created_at: 1,
            attachments: None,
        });

        assert_eq!(message.motion.enter_key, None);

        let tool_message = session_agent_message_from_record(ChatMessageRecord {
            id: "tool-1".to_string(),
            session_id: "session-1".to_string(),
            role: ChatMessageRole::ToolCall,
            content: "{\"path\":\"Cargo.toml\"}".to_string(),
            tool_name: Some("read".to_string()),
            tool_summary: Some("read Cargo.toml".to_string()),
            tool_status: Some(tool_status_as_str(SessionAgentToolStatus::Completed).to_string()),
            sort_order: 1,
            created_at: 2,
            attachments: None,
        });

        assert_eq!(tool_message.motion.enter_key, None);
    }

    #[test]
    fn user_message_with_attachments_round_trips_through_record() {
        let attachment = miaominal_core::chat_attachment::ChatAttachment {
            id: "att-1".to_string(),
            filename: "main.rs".to_string(),
            mime_type: "text/plain".to_string(),
            size_bytes: 42,
            content: miaominal_core::chat_attachment::ChatAttachmentContent::TextFile(
                miaominal_core::chat_attachment::ChatTextFile {
                    text: "fn main() {}".to_string(),
                    language: Some("rust".to_string()),
                },
            ),
        };
        let original =
            SessionAgentMessage::user_with_attachments("look at this", vec![attachment.clone()]);
        let record = chat_record_from_session_agent_message("session-1", 0, 100, &original)
            .expect("record should be produced");
        assert_eq!(record.role, ChatMessageRole::User);
        assert!(record.attachments.is_some());

        let restored = session_agent_message_from_record(record);
        assert_eq!(restored.role, SessionAgentMessageRole::User);
        assert_eq!(restored.content, "look at this");
        assert_eq!(restored.attachments.len(), 1);
        assert_eq!(restored.attachments[0].filename, "main.rs");
    }

    #[test]
    fn plain_text_user_message_has_no_attachments_in_record() {
        let original = SessionAgentMessage::user("hello");
        let record = chat_record_from_session_agent_message("session-1", 0, 100, &original)
            .expect("record should be produced");
        assert!(record.attachments.is_none());

        let restored = session_agent_message_from_record(record);
        assert!(restored.attachments.is_empty());
    }

    #[test]
    fn assistant_message_has_no_attachments_in_record() {
        let original = SessionAgentMessage::assistant_raw("sure");
        let record = chat_record_from_session_agent_message("session-1", 0, 100, &original)
            .expect("record should be produced");
        assert!(record.attachments.is_none());
    }
}
