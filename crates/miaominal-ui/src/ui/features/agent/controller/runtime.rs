use crate::ui::contracts::workspace::TabId;
use crate::ui::features::agent::conversation_view::SessionAgentConversationView;
use gpui_kit::{App, AppContext as _, Entity, ListOffset, Subscription, px};
use miaominal_agent::engine::ApprovalRequest;
use miaominal_agent::model::TokenUsage;
use miaominal_agent::policy::ApprovalMode;
use miaominal_agent::transcript::Item;
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
pub(in crate::ui) enum SessionAgentMessageRole {
    User,
    Assistant,
    Thinking,
    ToolCall,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(in crate::ui) enum SessionAgentToolStatus {
    Pending,
    WaitingForConfirmation,
    InProgress,
    Completed,
    Failed,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct SessionAgentToolCall {
    pub(in crate::ui) id: String,
    pub(in crate::ui) name: String,
    pub(in crate::ui) arguments: String,
    pub(in crate::ui) summary: String,
    pub(in crate::ui) status: SessionAgentToolStatus,
    pub(in crate::ui) requires_confirmation: bool,
    pub(in crate::ui) confirmation_note: Option<String>,
    pub(in crate::ui) expanded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct SessionAgentThinking {
    pub(in crate::ui) started_at: Instant,
    pub(in crate::ui) elapsed_ms: Option<u128>,
    pub(in crate::ui) expanded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::ui) struct SessionAgentMessageMotion {
    pub(in crate::ui) enter_key: Option<u64>,
}

#[derive(Clone)]
pub(in crate::ui) struct SessionAgentMessage {
    pub(in crate::ui) role: SessionAgentMessageRole,
    pub(in crate::ui) content: String,
    pub(in crate::ui) tool_call: Option<SessionAgentToolCall>,
    pub(in crate::ui) thinking: Option<SessionAgentThinking>,
    pub(in crate::ui) motion: SessionAgentMessageMotion,
    pub(in crate::ui) attachments: Arc<[miaominal_core::chat_attachment::ChatAttachment]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::ui) enum ChatPanelView {
    #[default]
    SessionList,
    Conversation,
}

impl SessionAgentMessage {
    pub(in crate::ui) fn user(content: impl Into<String>) -> Self {
        Self {
            role: SessionAgentMessageRole::User,
            content: content.into(),
            tool_call: None,
            thinking: None,
            motion: SessionAgentMessageMotion::default(),
            attachments: Arc::default(),
        }
    }

    pub(in crate::ui) fn user_with_attachments(
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

    pub(in crate::ui) fn assistant_raw(content: impl Into<String>) -> Self {
        Self {
            role: SessionAgentMessageRole::Assistant,
            content: content.into(),
            tool_call: None,
            thinking: None,
            motion: SessionAgentMessageMotion::default(),
            attachments: Arc::default(),
        }
    }

    pub(in crate::ui) fn thinking_raw(content: impl Into<String>) -> Self {
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
    pub(in crate::ui) fn thinking_from_history(content: impl Into<String>) -> Self {
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
    pub(in crate::ui) fn tool_call(tool_call: SessionAgentToolCall) -> Self {
        Self {
            role: SessionAgentMessageRole::ToolCall,
            content: tool_call.summary.clone(),
            tool_call: Some(tool_call),
            thinking: None,
            motion: SessionAgentMessageMotion::default(),
            attachments: Arc::default(),
        }
    }

    pub(in crate::ui) fn error(content: impl Into<String>) -> Self {
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
pub(in crate::ui) enum AgentExecMode {
    #[default]
    ExecChannel,
    Pty,
}

impl AgentExecMode {
    pub(in crate::ui) fn toggle(self) -> Self {
        match self {
            Self::ExecChannel => Self::Pty,
            Self::Pty => Self::ExecChannel,
        }
    }

    pub(in crate::ui) fn is_pty(self) -> bool {
        matches!(self, Self::Pty)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct SessionAgentExecutionContext {
    pub(in crate::ui) profile_id: String,
    pub(in crate::ui) exec_mode: AgentExecMode,
    pub(in crate::ui) terminal_tab_id: Option<TabId>,
}

#[derive(Default)]
pub(in crate::ui) struct SessionAgentState {
    pub(in crate::ui) session_id: Option<String>,
    pub(in crate::ui) messages: Vec<SessionAgentMessage>,
    pub(in crate::ui) conversation_view: Option<Entity<SessionAgentConversationView>>,
    pub(in crate::ui) conversation_view_observation: Option<Subscription>,
    /// Lightweight viewport state retained while the expensive conversation projection is
    /// released for a hidden session.
    pub(in crate::ui) conversation_viewport: Option<SessionAgentConversationViewport>,
    pub(in crate::ui) next_message_motion_key: u64,
    /// Running engine for this conversation (spawned on first submit).
    pub(in crate::ui) engine: Option<super::engine::SessionEngine>,
    /// Persisted history handed to the engine when it starts.
    pub(in crate::ui) transcript_seed: Option<Vec<Item>>,
    /// The engine is streaming, running tools or waiting for the user.
    pub(in crate::ui) engine_busy: bool,
    /// Engine item id → message index.
    pub(in crate::ui) item_messages: HashMap<String, usize>,
    /// Tool call id → message index.
    pub(in crate::ui) call_messages: HashMap<String, usize>,
    pub(in crate::ui) pending_approvals: HashMap<String, ApprovalRequest>,
    /// (tokens used, context window) from the latest completion.
    pub(in crate::ui) context_usage: Option<(u64, u64)>,
    pub(in crate::ui) last_usage: Option<TokenUsage>,
    /// Terminal text shared with the engine's `terminal_read` tool.
    pub(in crate::ui) terminal_context: super::actions::SharedTerminalContext,
    /// Terminal leases held while the current turn runs.
    pub(in crate::ui) turn_leases: Vec<crate::ui::contracts::session::TerminalLease>,
    pub(in crate::ui) last_error: Option<String>,
    pub(in crate::ui) exec_mode: AgentExecMode,
    pub(in crate::ui) at_mention_query: Option<String>,
    pub(in crate::ui) at_mention_anchor: usize,
    pub(in crate::ui) selected_at_targets: Vec<String>,
    pub(in crate::ui) active_at_targets: Vec<String>,
    pub(in crate::ui) prompt_history: Vec<String>,
    pub(in crate::ui) prompt_history_cursor: Option<usize>,
    pub(in crate::ui) prompt_history_draft: Option<String>,
    pub(in crate::ui) title: Option<String>,
    pub(in crate::ui) panel_view: ChatPanelView,
    /// Active search query for filtering messages in the conversation view.
    pub(in crate::ui) search_query: Option<String>,
    /// Matching blocks as (message_index, block_index) pairs.
    pub(in crate::ui) search_match_indices: Vec<(usize, usize)>,
    /// Current position in the match navigation (index into search_match_indices).
    pub(in crate::ui) search_current_match: Option<usize>,
    /// One-shot block target used for the second phase of virtual-list search scrolling.
    pub(in crate::ui) search_scroll_target: Option<(usize, usize)>,
    /// Message rows whose streaming content needs a throttled search-index refresh.
    pub(in crate::ui) search_refresh_pending_messages: Vec<usize>,
    /// Invalidates detached search-refresh timers when the query/session changes.
    pub(in crate::ui) search_refresh_generation: u64,
    /// Agent mode controlling tool availability, policy enforcement, and confirmation behavior.
    pub(in crate::ui) agent_mode: ApprovalMode,
    /// Default execution target captured when the user submitted the prompt.
    pub(in crate::ui) active_exec_context: Option<SessionAgentExecutionContext>,
    /// Attachments staged in the composer but not yet sent.
    pub(in crate::ui) pending_attachments: Vec<miaominal_core::chat_attachment::ChatAttachment>,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::ui) struct SessionAgentConversationViewport {
    pub(in crate::ui) offset: ListOffset,
    pub(in crate::ui) following_tail: bool,
    /// Whether the cached offset was measured against the per-block conversation-search layout.
    pub(in crate::ui) search_layout_active: bool,
}

impl SessionAgentConversationViewport {
    pub(in crate::ui) fn offset_for_search_layout(self, search_layout_active: bool) -> ListOffset {
        let mut offset = self.offset;
        if self.search_layout_active != search_layout_active {
            offset.offset_in_item = px(0.0);
        }
        offset
    }
}

/// Split message content into logical blocks for per-block search matching.
/// Code fences are kept as single blocks; paragraphs are separated by blank lines.
pub(in crate::ui) fn split_message_into_blocks(content: &str) -> Vec<String> {
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
    pub(in crate::ui) fn ensure_conversation_view_projection(
        &mut self,
        cx: &mut App,
    ) -> Entity<SessionAgentConversationView> {
        if let Some(view) = self.conversation_view.as_ref() {
            return view.clone();
        }

        let search_layout_active = self
            .search_query
            .as_ref()
            .is_some_and(|query| !query.trim().is_empty());
        let messages = self.messages.clone();
        let generating = self.has_pending_task();
        let viewport = self.conversation_viewport.take();
        let view =
            cx.new(move |cx| SessionAgentConversationView::from_messages(messages, generating, cx));
        if let Some(viewport) = viewport
            && !viewport.following_tail
        {
            let offset = viewport.offset_for_search_layout(search_layout_active);
            view.read(cx)
                .scroll_to(offset.item_ix, offset.offset_in_item);
        }
        self.conversation_view = Some(view.clone());
        view
    }

    pub(in crate::ui) fn reconcile_conversation_view_projection(&mut self, cx: &mut App) -> bool {
        if self.conversation_view.is_none() {
            return false;
        }
        self.release_conversation_view(cx);
        self.ensure_conversation_view_projection(cx);
        true
    }

    pub(in crate::ui) fn assign_enter_motion(&mut self, message: &mut SessionAgentMessage) {
        self.next_message_motion_key = self.next_message_motion_key.wrapping_add(1).max(1);
        message.motion.enter_key = Some(self.next_message_motion_key);
    }

    pub(in crate::ui) fn push_message_with_enter_motion(
        &mut self,
        mut message: SessionAgentMessage,
    ) {
        self.assign_enter_motion(&mut message);
        self.messages.push(message);
    }

    /// Changes whenever a new user turn starts (guards async UI work).
    pub(in crate::ui) fn turn_marker(&self) -> u64 {
        self.messages
            .iter()
            .filter(|message| message.role == SessionAgentMessageRole::User)
            .count() as u64
    }

    pub(super) fn has_pending_task(&self) -> bool {
        self.engine_busy
    }

    pub(in crate::ui) fn is_busy(&self) -> bool {
        self.engine_busy
    }

    pub(in crate::ui) fn has_tool_call_waiting_for_confirmation(&self) -> bool {
        self.messages.iter().rev().any(|message| {
            message.tool_call.as_ref().is_some_and(|tool_call| {
                tool_call.status == SessionAgentToolStatus::WaitingForConfirmation
            })
        })
    }

    pub(in crate::ui) fn active_ask_user_tool_call(&self) -> Option<SessionAgentToolCall> {
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

    pub(in crate::ui) fn toggle_thinking_expanded(&mut self, index: usize) {
        if let Some(message) = self.messages.get_mut(index)
            && let Some(thinking) = message.thinking.as_mut()
        {
            thinking.expanded = !thinking.expanded;
        }
    }

    pub(in crate::ui) fn toggle_tool_call_expanded(&mut self, id: &str) {
        if let Some(tool_call) = self
            .messages
            .iter_mut()
            .filter_map(|message| message.tool_call.as_mut())
            .find(|tool_call| tool_call.id == id)
        {
            tool_call.expanded = !tool_call.expanded;
        }
    }

    pub(in crate::ui) fn push_conversation_message_view(
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

    pub(in crate::ui) fn sync_conversation_message_view(&self, index: usize, cx: &mut App) {
        let Some(message) = self.messages.get(index).cloned() else {
            return;
        };
        if let Some(view) = self.conversation_view.as_ref().cloned() {
            view.update(cx, |view, cx| {
                view.set_message_snapshot(index, message, cx);
            });
        }
    }

    pub(in crate::ui) fn append_conversation_message_view_delta(
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

    pub(in crate::ui) fn clear_conversation_view(&self, cx: &mut App) {
        if let Some(view) = self.conversation_view.as_ref().cloned() {
            view.update(cx, |view, cx| view.clear(cx));
        }
    }

    pub(in crate::ui) fn release_conversation_view(&mut self, cx: &mut App) {
        if let Some(view) = self.conversation_view.as_ref() {
            let view = view.read(cx);
            let motion_keys = view.enter_motion_keys_for_rebuild(cx);
            if motion_keys.len() == self.messages.len() {
                for (message, motion_key) in self.messages.iter_mut().zip(motion_keys) {
                    message.motion.enter_key = motion_key;
                }
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

pub(in crate::ui) fn trailing_at_mention_query(value: &str) -> Option<(usize, String)> {
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
    use gpui_kit::TestAppContext;

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
    fn conversation_projection_reconciliation_rebuilds_from_complete_state() {
        let cx = TestAppContext::single();
        let stale_view = cx.update(|cx| {
            cx.new(|cx| {
                SessionAgentConversationView::from_messages(
                    vec![SessionAgentMessage::error("stale error-only projection")],
                    true,
                    cx,
                )
            })
        });
        let mut first = SessionAgentMessage::user("earlier question");
        first.motion.enter_key = Some(41);
        let mut state = SessionAgentState {
            messages: vec![
                first,
                SessionAgentMessage::assistant_raw("earlier answer"),
                SessionAgentMessage::user("current question"),
                SessionAgentMessage::assistant_raw("partial answer"),
                SessionAgentMessage::error("provider failed"),
            ],
            conversation_view: Some(stale_view),
            ..Default::default()
        };

        cx.update(|cx| {
            assert!(state.reconcile_conversation_view_projection(cx));
            let view = state
                .conversation_view
                .as_ref()
                .expect("conversation projection should be rebuilt");
            let snapshots = view.read(cx).message_snapshots(cx);
            assert_eq!(snapshots.len(), state.messages.len());
            assert_eq!(
                snapshots
                    .iter()
                    .map(|message| (message.role, message.content.as_str()))
                    .collect::<Vec<_>>(),
                state
                    .messages
                    .iter()
                    .map(|message| (message.role, message.content.as_str()))
                    .collect::<Vec<_>>()
            );
            assert_eq!(state.messages[0].motion.enter_key, Some(41));
            assert!(!view.read(cx).is_generating());
        });
        cx.quit();
    }

    #[test]
    fn split_message_into_blocks_skips_blank_segments() {
        assert!(split_message_into_blocks("").is_empty());
        assert!(split_message_into_blocks("\n\n  \n\t\n").is_empty());

        assert_eq!(
            split_message_into_blocks("hello\n\n\nworld"),
            vec!["hello".to_string(), "world".to_string()]
        );
    }

    #[test]
    fn split_message_into_blocks_keeps_code_fences_together() {
        assert_eq!(
            split_message_into_blocks("before\n\n```rust\nfn main() {}\n```\n\nafter"),
            vec![
                "before".to_string(),
                "```rust\nfn main() {}\n```".to_string(),
                "after".to_string()
            ]
        );
    }

    #[test]
    fn background_state_tracks_busy_and_approval_from_engine_events() {
        use miaominal_agent::engine::{AgentEvent, ApprovalRequest, ItemHint, SessionStatus};
        use miaominal_agent::policy::AgentRiskLevel;

        let mut runtime = AgentRuntimeStore::default();
        let mut state = SessionAgentState {
            session_id: Some("session-1".to_string()),
            ..Default::default()
        };
        state.apply_engine_event(AgentEvent::Status(SessionStatus::Streaming));
        state.apply_engine_event(AgentEvent::ItemStarted {
            item_id: "tool-1".into(),
            hint: ItemHint::ToolCall,
            name: Some("shell".into()),
        });
        runtime.store_background_session("session-1".to_string(), state);
        assert!(runtime.background_session_is_busy("session-1"));
        assert!(!runtime.background_session_needs_approval("session-1"));

        let background = runtime
            .session_mut("session-1")
            .expect("background session should be addressable by id");
        background.apply_engine_event(AgentEvent::ApprovalRequested(ApprovalRequest {
            call_id: "tool-1".into(),
            tool: "shell".into(),
            target: "web".into(),
            risk: AgentRiskLevel::L2ServiceImpacting,
            reason: "restart".into(),
            summary: "systemctl restart nginx".into(),
            editable: None,
            grant_prefix: None,
        }));
        assert!(runtime.background_session_needs_approval("session-1"));
        assert!(!runtime.session_is_foreground("session-1"));
        runtime.remove_background_session("session-1");
        assert!(!runtime.background_session_is_busy("session-1"));
    }
}
