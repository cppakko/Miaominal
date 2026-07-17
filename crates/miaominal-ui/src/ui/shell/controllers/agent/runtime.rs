use crate::ui::i18n;
use crate::ui::shell::TabId;
use crate::ui::shell::session_agent_view::SessionAgentConversationView;
use gpui::{App, Entity, ListOffset, Subscription, px};
use miaominal_agent::{AgentMode, AgentToolCancellation};
use std::collections::HashMap;
use std::{sync::Arc, time::Instant};

#[derive(Default)]
pub(super) struct AgentRuntimeStore {
    pub(super) foreground: SessionAgentState,
    background_sessions: HashMap<String, SessionAgentState>,
}

impl AgentRuntimeStore {
    pub(super) fn session(&self, session_id: &str) -> Option<&SessionAgentState> {
        if self.foreground.session_id.as_deref() == Some(session_id) {
            Some(&self.foreground)
        } else {
            self.background_sessions.get(session_id)
        }
    }

    pub(super) fn take_background_session(
        &mut self,
        session_id: &str,
    ) -> Option<SessionAgentState> {
        self.background_sessions.remove(session_id)
    }

    pub(super) fn store_background_session(
        &mut self,
        session_id: String,
        state: SessionAgentState,
    ) {
        self.background_sessions.insert(session_id, state);
    }

    pub(super) fn remove_background_session(&mut self, session_id: &str) {
        self.background_sessions.remove(session_id);
    }

    pub(super) fn background_session_is_busy(&self, session_id: &str) -> bool {
        self.background_sessions
            .get(session_id)
            .is_some_and(SessionAgentState::is_busy)
    }

    pub(super) fn background_session_needs_approval(&self, session_id: &str) -> bool {
        self.background_sessions
            .get(session_id)
            .is_some_and(SessionAgentState::has_tool_call_waiting_for_confirmation)
    }

    pub(super) fn session_mut(&mut self, session_id: &str) -> Option<&mut SessionAgentState> {
        if self.foreground.session_id.as_deref() == Some(session_id) {
            Some(&mut self.foreground)
        } else {
            self.background_sessions.get_mut(session_id)
        }
    }

    pub(super) fn session_is_foreground(&self, session_id: &str) -> bool {
        self.foreground.session_id.as_deref() == Some(session_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::shell) enum SessionAgentMessageRole {
    User,
    Assistant,
    Thinking,
    ToolCall,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(in crate::ui::shell) enum SessionAgentToolStatus {
    Pending,
    WaitingForConfirmation,
    InProgress,
    Completed,
    Failed,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui::shell) struct SessionAgentToolCall {
    pub(in crate::ui::shell) id: String,
    pub(in crate::ui::shell) name: String,
    pub(in crate::ui::shell) arguments: String,
    pub(in crate::ui::shell) summary: String,
    pub(in crate::ui::shell) status: SessionAgentToolStatus,
    pub(in crate::ui::shell) requires_confirmation: bool,
    pub(in crate::ui::shell) confirmation_note: Option<String>,
    pub(in crate::ui::shell) expanded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui::shell) struct SessionAgentThinking {
    pub(in crate::ui::shell) started_at: Instant,
    pub(in crate::ui::shell) elapsed_ms: Option<u128>,
    pub(in crate::ui::shell) expanded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::ui::shell) struct SessionAgentMessageMotion {
    pub(in crate::ui::shell) enter_key: Option<u64>,
}

#[derive(Clone)]
pub(in crate::ui::shell) struct SessionAgentMessage {
    pub(in crate::ui::shell) role: SessionAgentMessageRole,
    pub(in crate::ui::shell) content: String,
    pub(in crate::ui::shell) tool_call: Option<SessionAgentToolCall>,
    pub(in crate::ui::shell) thinking: Option<SessionAgentThinking>,
    pub(in crate::ui::shell) motion: SessionAgentMessageMotion,
    pub(in crate::ui::shell) attachments: Arc<[miaominal_core::chat_attachment::ChatAttachment]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::ui::shell) enum ChatPanelView {
    #[default]
    SessionList,
    Conversation,
}

impl SessionAgentMessage {
    pub(in crate::ui::shell) fn user(content: impl Into<String>) -> Self {
        Self {
            role: SessionAgentMessageRole::User,
            content: content.into(),
            tool_call: None,
            thinking: None,
            motion: SessionAgentMessageMotion::default(),
            attachments: Arc::default(),
        }
    }

    pub(in crate::ui::shell) fn user_with_attachments(
        content: impl Into<String>,
        attachments: Vec<miaominal_core::chat_attachment::ChatAttachment>,
    ) -> Self {
        Self {
            role: SessionAgentMessageRole::User,
            content: content.into(),
            tool_call: None,
            thinking: None,
            motion: SessionAgentMessageMotion::default(),
            attachments: attachments.into(),
        }
    }

    pub(in crate::ui::shell) fn assistant_raw(content: impl Into<String>) -> Self {
        Self {
            role: SessionAgentMessageRole::Assistant,
            content: content.into(),
            tool_call: None,
            thinking: None,
            motion: SessionAgentMessageMotion::default(),
            attachments: Arc::default(),
        }
    }

    pub(in crate::ui::shell) fn thinking_raw(content: impl Into<String>) -> Self {
        Self {
            role: SessionAgentMessageRole::Thinking,
            content: content.into(),
            tool_call: None,
            thinking: Some(SessionAgentThinking {
                started_at: Instant::now(),
                elapsed_ms: None,
                expanded: false,
            }),
            motion: SessionAgentMessageMotion::default(),
            attachments: Arc::default(),
        }
    }

    /// Creates a thinking message from a historical record.
    /// elapsed_ms is set to Some(0) to signal this is a completed/historical
    /// message — the renderer hides the live timer for these.
    pub(in crate::ui::shell) fn thinking_from_history(content: impl Into<String>) -> Self {
        Self {
            role: SessionAgentMessageRole::Thinking,
            content: content.into(),
            tool_call: None,
            thinking: Some(SessionAgentThinking {
                started_at: Instant::now(),
                elapsed_ms: Some(0),
                expanded: false,
            }),
            motion: SessionAgentMessageMotion::default(),
            attachments: Arc::default(),
        }
    }

    #[allow(dead_code)]
    pub(in crate::ui::shell) fn tool_call(tool_call: SessionAgentToolCall) -> Self {
        Self {
            role: SessionAgentMessageRole::ToolCall,
            content: tool_call.summary.clone(),
            tool_call: Some(tool_call),
            thinking: None,
            motion: SessionAgentMessageMotion::default(),
            attachments: Arc::default(),
        }
    }

    pub(in crate::ui::shell) fn error(content: impl Into<String>) -> Self {
        Self {
            role: SessionAgentMessageRole::Error,
            content: content.into(),
            tool_call: None,
            thinking: None,
            motion: SessionAgentMessageMotion::default(),
            attachments: Arc::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::ui::shell) enum AgentExecMode {
    #[default]
    ExecChannel,
    Pty,
}

impl AgentExecMode {
    pub(in crate::ui::shell) fn toggle(self) -> Self {
        match self {
            Self::ExecChannel => Self::Pty,
            Self::Pty => Self::ExecChannel,
        }
    }

    pub(in crate::ui::shell) fn is_pty(self) -> bool {
        matches!(self, Self::Pty)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui::shell) struct SessionAgentExecutionContext {
    pub(in crate::ui::shell) profile_id: String,
    pub(in crate::ui::shell) exec_mode: AgentExecMode,
    pub(in crate::ui::shell) terminal_tab_id: Option<TabId>,
}

#[derive(Default)]
pub(in crate::ui::shell) struct SessionAgentState {
    pub(in crate::ui::shell) session_id: Option<String>,
    pub(in crate::ui::shell) messages: Vec<SessionAgentMessage>,
    pub(in crate::ui::shell) conversation_view: Option<Entity<SessionAgentConversationView>>,
    pub(in crate::ui::shell) conversation_view_observation: Option<Subscription>,
    /// Lightweight viewport state retained while the expensive conversation projection is
    /// released for a hidden session.
    pub(in crate::ui::shell) conversation_viewport: Option<SessionAgentConversationViewport>,
    pub(in crate::ui::shell) next_message_motion_key: u64,
    pub(super) pending_task: Option<gpui::Task<()>>,
    pub(super) pending_stream_stop: Option<tokio::sync::watch::Sender<bool>>,
    pub(super) pending_agent_cancellation: Option<AgentToolCancellation>,
    pub(in crate::ui::shell) active_request_id: u64,
    pub(in crate::ui::shell) request_counter: u64,
    pub(in crate::ui::shell) last_error: Option<String>,
    pub(in crate::ui::shell) exec_mode: AgentExecMode,
    pub(in crate::ui::shell) at_mention_query: Option<String>,
    pub(in crate::ui::shell) at_mention_anchor: usize,
    pub(in crate::ui::shell) selected_at_targets: Vec<String>,
    pub(in crate::ui::shell) active_at_targets: Vec<String>,
    pub(in crate::ui::shell) prompt_history: Vec<String>,
    pub(in crate::ui::shell) prompt_history_cursor: Option<usize>,
    pub(in crate::ui::shell) prompt_history_draft: Option<String>,
    pub(in crate::ui::shell) title: Option<String>,
    pub(in crate::ui::shell) panel_view: ChatPanelView,
    /// Active search query for filtering messages in the conversation view.
    pub(in crate::ui::shell) search_query: Option<String>,
    /// Matching blocks as (message_index, block_index) pairs.
    pub(in crate::ui::shell) search_match_indices: Vec<(usize, usize)>,
    /// Current position in the match navigation (index into search_match_indices).
    pub(in crate::ui::shell) search_current_match: Option<usize>,
    /// One-shot block target used for the second phase of virtual-list search scrolling.
    pub(in crate::ui::shell) search_scroll_target: Option<(usize, usize)>,
    /// Message rows whose streaming content needs a throttled search-index refresh.
    pub(in crate::ui::shell) search_refresh_pending_messages: Vec<usize>,
    /// Invalidates detached search-refresh timers when the query/session changes.
    pub(in crate::ui::shell) search_refresh_generation: u64,
    /// Agent mode controlling tool availability, policy enforcement, and confirmation behavior.
    pub(in crate::ui::shell) agent_mode: AgentMode,
    /// Default execution target captured when the user submitted the prompt.
    pub(in crate::ui::shell) active_exec_context: Option<SessionAgentExecutionContext>,
    /// Attachments staged in the composer but not yet sent.
    pub(in crate::ui::shell) pending_attachments:
        Vec<miaominal_core::chat_attachment::ChatAttachment>,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct SessionAgentConversationViewport {
    pub(in crate::ui::shell) offset: ListOffset,
    pub(in crate::ui::shell) following_tail: bool,
    /// Whether the cached offset was measured against the per-block conversation-search layout.
    pub(in crate::ui::shell) search_layout_active: bool,
}

impl SessionAgentConversationViewport {
    pub(in crate::ui::shell) fn offset_for_search_layout(
        self,
        search_layout_active: bool,
    ) -> ListOffset {
        let mut offset = self.offset;
        if self.search_layout_active != search_layout_active {
            offset.offset_in_item = px(0.0);
        }
        offset
    }
}

/// Split message content into logical blocks for per-block search matching.
/// Code fences are kept as single blocks; paragraphs are separated by blank lines.
pub(in crate::ui::shell) fn split_message_into_blocks(content: &str) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_code_fence = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if !in_code_fence && !current.is_empty() {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    blocks.push(trimmed);
                }
                current.clear();
            }
            current.push_str(line);
            current.push('\n');
            in_code_fence = !in_code_fence;
            if !in_code_fence {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    blocks.push(trimmed);
                }
                current.clear();
            }
        } else if !in_code_fence && trimmed.is_empty() {
            if !current.is_empty() {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    blocks.push(trimmed);
                }
                current.clear();
            }
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    if !current.is_empty() {
        let trimmed = current.trim().to_string();
        if !trimmed.is_empty() {
            blocks.push(trimmed);
        }
    }
    blocks
}

impl SessionAgentState {
    pub(in crate::ui::shell) fn execution_mode_for_running_tools(&self) -> AgentExecMode {
        self.active_exec_context
            .as_ref()
            .map(|context| context.exec_mode)
            .unwrap_or(self.exec_mode)
    }

    pub(in crate::ui::shell) fn assign_enter_motion(&mut self, message: &mut SessionAgentMessage) {
        self.next_message_motion_key = self.next_message_motion_key.wrapping_add(1).max(1);
        message.motion.enter_key = Some(self.next_message_motion_key);
    }

    pub(in crate::ui::shell) fn push_message_with_enter_motion(
        &mut self,
        mut message: SessionAgentMessage,
    ) {
        self.assign_enter_motion(&mut message);
        self.messages.push(message);
    }

    pub(super) fn has_pending_task(&self) -> bool {
        self.pending_task.is_some()
    }

    pub(in crate::ui::shell) fn is_busy(&self) -> bool {
        self.has_pending_task() || self.has_active_tool_call()
    }

    pub(in crate::ui::shell) fn has_active_tool_call(&self) -> bool {
        self.messages.iter().rev().any(|message| {
            message.tool_call.as_ref().is_some_and(|tool_call| {
                matches!(
                    tool_call.status,
                    SessionAgentToolStatus::Pending
                        | SessionAgentToolStatus::WaitingForConfirmation
                        | SessionAgentToolStatus::InProgress
                )
            })
        })
    }

    pub(in crate::ui::shell) fn has_tool_call_waiting_for_confirmation(&self) -> bool {
        self.messages.iter().rev().any(|message| {
            message.tool_call.as_ref().is_some_and(|tool_call| {
                tool_call.status == SessionAgentToolStatus::WaitingForConfirmation
            })
        })
    }

    pub(in crate::ui::shell) fn next_request_id(&mut self) -> u64 {
        self.request_counter = self.request_counter.wrapping_add(1).max(1);
        self.request_counter
    }

    pub(in crate::ui::shell) fn append_assistant_delta(&mut self, delta: impl AsRef<str>) {
        self.finish_active_thinking();
        let delta = delta.as_ref();

        if let Some(message) = self.messages.last_mut()
            && message.role == SessionAgentMessageRole::Assistant
        {
            // Always append to an existing assistant message, including
            // whitespace-only deltas (newlines are structurally significant
            // for markdown headings and tables).
            if delta.is_empty() {
                return;
            }
            message.content.push_str(delta);
        } else {
            // Don't create a new assistant message for whitespace-only content.
            if delta.trim().is_empty() {
                return;
            }
            self.push_message_with_enter_motion(SessionAgentMessage::assistant_raw(delta));
        }
    }

    pub(in crate::ui::shell) fn start_assistant_reply(&mut self) {
        self.finish_active_thinking();
        if !self.messages.last().is_some_and(|message| {
            message.role == SessionAgentMessageRole::Assistant && message.content.is_empty()
        }) {
            self.push_message_with_enter_motion(SessionAgentMessage::assistant_raw(""));
        }
    }

    pub(in crate::ui::shell) fn append_thinking_delta(&mut self, delta: impl AsRef<str>) {
        let delta = delta.as_ref();
        if delta.trim().is_empty() {
            return;
        }

        if let Some(message) = self.messages.last_mut()
            && message.role == SessionAgentMessageRole::Thinking
        {
            message.content.push_str(delta);
        } else {
            self.push_message_with_enter_motion(SessionAgentMessage::thinking_raw(delta));
        }
    }

    pub(in crate::ui::shell) fn push_tool_call(
        &mut self,
        id: String,
        name: String,
        arguments: String,
        status: SessionAgentToolStatus,
    ) {
        self.finish_active_thinking();
        let summary = if arguments.trim().is_empty() || arguments.trim() == "null" {
            "No arguments".to_string()
        } else {
            arguments
        };
        self.push_message_with_enter_motion(SessionAgentMessage::tool_call(SessionAgentToolCall {
            id,
            name,
            arguments: summary.clone(),
            summary,
            status,
            requires_confirmation: false,
            confirmation_note: None,
            expanded: false,
        }));
    }

    pub(in crate::ui::shell) fn append_tool_call_delta(&mut self, id: &str, delta: String) {
        if delta.trim().is_empty() {
            return;
        }

        if let Some(tool_call) = self
            .messages
            .iter_mut()
            .rev()
            .filter_map(|message| message.tool_call.as_mut())
            .find(|tool_call| tool_call.id == id)
        {
            if tool_call.summary == "No arguments" {
                tool_call.summary.clear();
                tool_call.arguments.clear();
            }
            tool_call.summary.push_str(&delta);
            tool_call.arguments.push_str(&delta);
        }
    }

    pub(in crate::ui::shell) fn complete_tool_call(&mut self, id: &str, result: String) {
        if let Some(tool_call) = self
            .messages
            .iter_mut()
            .rev()
            .filter_map(|message| message.tool_call.as_mut())
            .find(|tool_call| tool_call.id == id)
        {
            tool_call.status = SessionAgentToolStatus::Completed;
            if !result.trim().is_empty() {
                tool_call.confirmation_note = Some(result);
            }
        }
    }

    pub(in crate::ui::shell) fn fail_tool_call(&mut self, id: &str, result: String) {
        if let Some(tool_call) = self
            .messages
            .iter_mut()
            .rev()
            .filter_map(|message| message.tool_call.as_mut())
            .find(|tool_call| tool_call.id == id)
        {
            tool_call.status = SessionAgentToolStatus::Failed;
            tool_call.requires_confirmation = false;
            tool_call.confirmation_note = Some(result);
        }
    }

    pub(in crate::ui::shell) fn approve_tool_call(&mut self, id: &str) {
        if let Some(tool_call) = self
            .messages
            .iter_mut()
            .rev()
            .filter_map(|message| message.tool_call.as_mut())
            .find(|tool_call| tool_call.id == id)
        {
            tool_call.status = SessionAgentToolStatus::InProgress;
            tool_call.requires_confirmation = false;
            tool_call.confirmation_note = Some(i18n::string(
                "workspace.panel.agent.messages.tool_approved_running",
            ));
        }
    }

    pub(in crate::ui::shell) fn reject_tool_call(&mut self, id: &str) {
        self.reject_tool_call_with_message(
            id,
            i18n::string("workspace.panel.agent.messages.tool_denied"),
        );
    }

    pub(in crate::ui::shell) fn reject_tool_call_with_message(
        &mut self,
        id: &str,
        message: String,
    ) {
        if let Some(tool_call) = self
            .messages
            .iter_mut()
            .rev()
            .filter_map(|message| message.tool_call.as_mut())
            .find(|tool_call| tool_call.id == id)
        {
            tool_call.status = SessionAgentToolStatus::Rejected;
            tool_call.requires_confirmation = false;
            tool_call.confirmation_note = Some(message);
        }
    }

    pub(in crate::ui::shell) fn reject_active_tool_calls(&mut self, message: &str) -> bool {
        let mut rejected = false;
        for tool_call in self
            .messages
            .iter_mut()
            .filter_map(|message| message.tool_call.as_mut())
        {
            if matches!(
                tool_call.status,
                SessionAgentToolStatus::Pending
                    | SessionAgentToolStatus::WaitingForConfirmation
                    | SessionAgentToolStatus::InProgress
            ) {
                tool_call.status = SessionAgentToolStatus::Rejected;
                tool_call.requires_confirmation = false;
                tool_call.confirmation_note = Some(message.into());
                rejected = true;
            }
        }
        rejected
    }

    pub(in crate::ui::shell) fn fail_active_tool_calls(&mut self, message: &str) -> bool {
        let mut failed = false;
        for tool_call in self
            .messages
            .iter_mut()
            .filter_map(|message| message.tool_call.as_mut())
        {
            if matches!(
                tool_call.status,
                SessionAgentToolStatus::Pending
                    | SessionAgentToolStatus::WaitingForConfirmation
                    | SessionAgentToolStatus::InProgress
            ) {
                tool_call.status = SessionAgentToolStatus::Failed;
                tool_call.requires_confirmation = false;
                tool_call.confirmation_note = Some(message.into());
                failed = true;
            }
        }
        failed
    }

    pub(in crate::ui::shell) fn tool_call(&self, id: &str) -> Option<SessionAgentToolCall> {
        self.messages
            .iter()
            .rev()
            .filter_map(|message| message.tool_call.as_ref())
            .find(|tool_call| tool_call.id == id)
            .cloned()
    }

    pub(in crate::ui::shell) fn active_ask_user_tool_call(&self) -> Option<SessionAgentToolCall> {
        self.messages
            .iter()
            .rev()
            .filter_map(|message| message.tool_call.as_ref())
            .find(|tool_call| {
                tool_call.name == "ask_user"
                    && tool_call.status == SessionAgentToolStatus::WaitingForConfirmation
            })
            .cloned()
    }

    pub(in crate::ui::shell) fn reasoning_before_tool_call(&self, id: &str) -> Option<String> {
        let tool_index = self.messages.iter().position(|message| {
            message
                .tool_call
                .as_ref()
                .is_some_and(|tool_call| tool_call.id == id)
        })?;

        self.messages[..tool_index]
            .iter()
            .rev()
            .take_while(|message| message.role != SessionAgentMessageRole::User)
            .find(|message| message.role == SessionAgentMessageRole::Thinking)
            .map(|message| message.content.clone())
            .filter(|content| !content.trim().is_empty())
    }

    pub(in crate::ui::shell) fn require_tool_call_confirmation(
        &mut self,
        id: &str,
        message: String,
    ) {
        if let Some(tool_call) = self
            .messages
            .iter_mut()
            .rev()
            .filter_map(|message| message.tool_call.as_mut())
            .find(|tool_call| tool_call.id == id)
        {
            tool_call.status = SessionAgentToolStatus::WaitingForConfirmation;
            tool_call.requires_confirmation = true;
            tool_call.confirmation_note = Some(message);
            tool_call.expanded = true;
        }
    }

    pub(in crate::ui::shell) fn toggle_thinking_expanded(&mut self, index: usize) {
        if let Some(message) = self.messages.get_mut(index)
            && let Some(thinking) = message.thinking.as_mut()
        {
            thinking.expanded = !thinking.expanded;
        }
    }

    pub(in crate::ui::shell) fn toggle_tool_call_expanded(&mut self, id: &str) {
        if let Some(tool_call) = self
            .messages
            .iter_mut()
            .filter_map(|message| message.tool_call.as_mut())
            .find(|tool_call| tool_call.id == id)
        {
            tool_call.expanded = !tool_call.expanded;
        }
    }

    pub(in crate::ui::shell) fn finish_assistant_reply(&mut self, reply: String) {
        self.finish_active_thinking();
        let reply = reply.trim().to_string();
        if reply.is_empty() {
            return;
        }

        if let Some(message) = self.messages.last_mut()
            && message.role == SessionAgentMessageRole::Assistant
        {
            message.content = reply;
            return;
        }

        if self
            .messages
            .iter()
            .rev()
            .take_while(|message| message.role != SessionAgentMessageRole::User)
            .any(|message| {
                message.role == SessionAgentMessageRole::Assistant
                    && message.content.trim() == reply.trim()
            })
        {
            return;
        }

        self.push_message_with_enter_motion(SessionAgentMessage::assistant_raw(reply));
    }

    pub(in crate::ui::shell) fn finish_stopped_turn(&mut self) {
        self.finish_active_thinking();
        if let Some(message) = self.messages.last_mut()
            && message.role == SessionAgentMessageRole::Assistant
            && message.content.trim().is_empty()
        {
            message.content = i18n::string("workspace.panel.agent.messages.stopped_by_user");
            return;
        }

        let turn_has_visible_output = self
            .messages
            .iter()
            .rev()
            .take_while(|message| message.role != SessionAgentMessageRole::User)
            .any(|message| {
                matches!(message.role, SessionAgentMessageRole::ToolCall)
                    || (message.role == SessionAgentMessageRole::Assistant
                        && !message.content.trim().is_empty())
            });
        if !turn_has_visible_output {
            self.push_message_with_enter_motion(SessionAgentMessage::assistant_raw(i18n::string(
                "workspace.panel.agent.messages.stopped_by_user",
            )));
        }
    }

    pub(in crate::ui::shell) fn finish_active_thinking(&mut self) {
        if let Some(message) = self.messages.last_mut()
            && message.role == SessionAgentMessageRole::Thinking
            && let Some(thinking) = message.thinking.as_mut()
            && thinking.elapsed_ms.is_none()
        {
            thinking.elapsed_ms = Some(thinking.started_at.elapsed().as_millis());
        }
    }

    pub(in crate::ui::shell) fn push_conversation_message_view(
        &self,
        message: SessionAgentMessage,
        cx: &mut App,
    ) {
        if let Some(view) = self.conversation_view.as_ref().cloned() {
            view.update(cx, |view, cx| {
                view.push_message(message, cx);
            });
        }
    }

    pub(in crate::ui::shell) fn sync_conversation_message_view(&self, index: usize, cx: &mut App) {
        let Some(message) = self.messages.get(index).cloned() else {
            return;
        };
        if let Some(view) = self.conversation_view.as_ref().cloned() {
            view.update(cx, |view, cx| {
                view.set_message_snapshot(index, message, cx);
            });
        }
    }

    pub(in crate::ui::shell) fn append_conversation_message_view_delta(
        &self,
        index: usize,
        delta: &str,
        cx: &mut App,
    ) {
        if delta.is_empty() {
            return;
        }
        if let Some(view) = self.conversation_view.as_ref().cloned() {
            view.update(cx, |view, cx| {
                view.append_to_message(index, delta, cx);
            });
        }
    }

    pub(in crate::ui::shell) fn clear_conversation_view(&self, cx: &mut App) {
        if let Some(view) = self.conversation_view.as_ref().cloned() {
            view.update(cx, |view, cx| view.clear(cx));
        }
    }

    pub(in crate::ui::shell) fn release_conversation_view(&mut self, cx: &mut App) {
        if let Some(view) = self.conversation_view.as_ref() {
            let view = view.read(cx);
            let motion_keys = view.enter_motion_keys_for_rebuild(cx);
            for (message, motion_key) in self.messages.iter_mut().zip(motion_keys) {
                message.motion.enter_key = motion_key;
            }
            let search_layout_active = self
                .search_query
                .as_ref()
                .is_some_and(|query| !query.trim().is_empty());
            self.conversation_viewport = Some(SessionAgentConversationViewport {
                offset: view.list_state().logical_scroll_top(),
                following_tail: view.is_following_tail(),
                search_layout_active,
            });
        }
        self.conversation_view_observation = None;
        self.conversation_view = None;
    }

    pub(super) fn sync_conversation_generating_view(&self, cx: &mut App) {
        let generating = self.has_pending_task();
        if let Some(view) = self.conversation_view.as_ref().cloned() {
            view.update(cx, |view, cx| view.set_generating(generating, cx));
        }
    }
}

pub(in crate::ui::shell) fn trailing_at_mention_query(value: &str) -> Option<(usize, String)> {
    let mut start = None;
    for (index, ch) in value.char_indices().rev() {
        if ch == '@' {
            start = Some(index);
            break;
        }
        if ch.is_whitespace() || matches!(ch, ',' | ';' | ':' | '"' | '\'' | '(' | ')') {
            return None;
        }
    }
    let start = start?;
    let query = &value[start + 1..];
    if query
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        Some((start, query.to_string()))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_viewport_preserves_search_offset_on_reopen_and_drops_it_after_clear() {
        let viewport = SessionAgentConversationViewport {
            offset: ListOffset {
                item_ix: 4,
                offset_in_item: px(72.0),
            },
            following_tail: false,
            search_layout_active: true,
        };

        assert_eq!(
            viewport.offset_for_search_layout(true).offset_in_item,
            px(72.0)
        );
        assert_eq!(
            viewport.offset_for_search_layout(false).offset_in_item,
            px(0.0)
        );
        assert_eq!(viewport.offset_for_search_layout(false).item_ix, 4);
    }

    #[test]
    fn background_runtime_store_round_trips_busy_and_approval_state() {
        let mut runtime = AgentRuntimeStore::default();
        let mut state = SessionAgentState {
            session_id: Some("session-1".to_string()),
            ..Default::default()
        };
        state.push_tool_call(
            "tool-1".to_string(),
            "run_shell".to_string(),
            "{}".to_string(),
            SessionAgentToolStatus::InProgress,
        );
        runtime.store_background_session("session-1".to_string(), state);

        assert!(runtime.background_session_is_busy("session-1"));
        assert!(!runtime.background_session_needs_approval("session-1"));

        let mut restored = runtime
            .take_background_session("session-1")
            .expect("background session should be restored");
        restored.require_tool_call_confirmation("tool-1", "approval required".to_string());
        runtime.store_background_session("session-1".to_string(), restored);

        assert!(runtime.background_session_needs_approval("session-1"));
        runtime.remove_background_session("session-1");
        assert!(!runtime.background_session_is_busy("session-1"));
    }

    #[test]
    fn session_lookup_mutates_background_state_without_swapping_foreground() {
        let mut runtime = AgentRuntimeStore {
            foreground: SessionAgentState {
                session_id: Some("foreground".to_string()),
                active_request_id: 11,
                last_error: Some("foreground status".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        runtime.store_background_session(
            "background".to_string(),
            SessionAgentState {
                session_id: Some("background".to_string()),
                active_request_id: 22,
                ..Default::default()
            },
        );

        let background = runtime
            .session_mut("background")
            .expect("background session should be addressable by id");
        background.active_request_id = 23;
        background.last_error = Some("background error".to_string());

        assert_eq!(runtime.foreground.active_request_id, 11);
        assert_eq!(
            runtime.foreground.last_error.as_deref(),
            Some("foreground status")
        );
        assert!(runtime.session_is_foreground("foreground"));
        assert!(!runtime.session_is_foreground("background"));
        assert_eq!(
            runtime
                .session("background")
                .map(|state| state.active_request_id),
            Some(23)
        );
    }
}
