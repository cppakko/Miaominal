use super::*;
use crate::ui::i18n;
use miaominal_agent::{AgentMode, TerminalOutputTap};
use std::collections::VecDeque;
use std::time::Instant;

const SESSION_MONITOR_HISTORY_LIMIT: usize = 900;
pub(in crate::ui::shell) const SFTP_TRANSFER_CHILD_HISTORY_LIMIT: usize = 500;

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct MonitorChartPoint {
    pub(in crate::ui::shell) label: String,
    pub(in crate::ui::shell) value: f64,
}

pub(in crate::ui::shell) struct SessionMonitoringState {
    pub(in crate::ui::shell) auto_collect_enabled: bool,
    pub(in crate::ui::shell) last_snapshot: Option<SessionMonitorSnapshot>,
    pub(in crate::ui::shell) last_error: Option<String>,
    pub(in crate::ui::shell) cpu_history: Vec<MonitorChartPoint>,
    pub(in crate::ui::shell) memory_history: Vec<MonitorChartPoint>,
    pub(in crate::ui::shell) swap_history: Vec<MonitorChartPoint>,
    pub(in crate::ui::shell) disk_history: Vec<MonitorChartPoint>,
    pub(in crate::ui::shell) network_history: Vec<MonitorChartPoint>,
    pub(in crate::ui::shell) load_history: Vec<MonitorChartPoint>,
    sample_count: usize,
}

impl SessionMonitoringState {
    pub(in crate::ui::shell) fn new(auto_collect_enabled: bool) -> Self {
        Self {
            auto_collect_enabled,
            last_snapshot: None,
            last_error: None,
            cpu_history: Vec::new(),
            memory_history: Vec::new(),
            swap_history: Vec::new(),
            disk_history: Vec::new(),
            network_history: Vec::new(),
            load_history: Vec::new(),
            sample_count: 0,
        }
    }

    pub(in crate::ui::shell) fn set_enabled(&mut self, enabled: bool) {
        self.auto_collect_enabled = enabled;
        if enabled {
            self.last_error = None;
        }
    }

    pub(in crate::ui::shell) fn apply_snapshot(&mut self, snapshot: SessionMonitorSnapshot) {
        self.sample_count = self.sample_count.saturating_add(1);
        let label = self.sample_count.to_string();
        let network_total = snapshot.network_rx_kbps + snapshot.network_tx_kbps;

        Self::push_history_point(&mut self.cpu_history, &label, snapshot.cpu_percent);
        Self::push_history_point(&mut self.memory_history, &label, snapshot.memory_percent);
        Self::push_history_point(&mut self.swap_history, &label, snapshot.swap_percent);
        Self::push_history_point(&mut self.disk_history, &label, snapshot.disk_percent);
        Self::push_history_point(&mut self.network_history, &label, network_total);
        Self::push_history_point(&mut self.load_history, &label, snapshot.load);

        self.last_snapshot = Some(snapshot);
        self.last_error = None;
    }

    pub(in crate::ui::shell) fn report_error(&mut self, error: String) {
        self.last_error = Some(error);
    }

    fn push_history_point(history: &mut Vec<MonitorChartPoint>, label: &str, value: f64) {
        history.push(MonitorChartPoint {
            label: label.to_string(),
            value,
        });
        if history.len() > SESSION_MONITOR_HISTORY_LIMIT {
            let overflow = history.len() - SESSION_MONITOR_HISTORY_LIMIT;
            history.drain(0..overflow);
        }
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

pub(in crate::ui::shell) struct SessionAgentMessage {
    pub(in crate::ui::shell) role: SessionAgentMessageRole,
    pub(in crate::ui::shell) content: String,
    pub(in crate::ui::shell) tool_call: Option<SessionAgentToolCall>,
    pub(in crate::ui::shell) thinking: Option<SessionAgentThinking>,
    pub(in crate::ui::shell) motion: SessionAgentMessageMotion,
    pub(in crate::ui::shell) attachments: Vec<miaominal_core::chat_attachment::ChatAttachment>,
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
            attachments: Vec::new(),
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
            attachments,
        }
    }

    pub(in crate::ui::shell) fn assistant_raw(content: impl Into<String>) -> Self {
        Self {
            role: SessionAgentMessageRole::Assistant,
            content: content.into(),
            tool_call: None,
            thinking: None,
            motion: SessionAgentMessageMotion::default(),
            attachments: Vec::new(),
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
            attachments: Vec::new(),
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
            attachments: Vec::new(),
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
            attachments: Vec::new(),
        }
    }

    pub(in crate::ui::shell) fn error(content: impl Into<String>) -> Self {
        Self {
            role: SessionAgentMessageRole::Error,
            content: content.into(),
            tool_call: None,
            thinking: None,
            motion: SessionAgentMessageMotion::default(),
            attachments: Vec::new(),
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
    pub(in crate::ui::shell) terminal_tab_id: Option<usize>,
}

#[derive(Default)]
pub(in crate::ui::shell) struct SessionAgentState {
    pub(in crate::ui::shell) session_id: Option<String>,
    pub(in crate::ui::shell) messages: Vec<SessionAgentMessage>,
    pub(in crate::ui::shell) next_message_motion_key: u64,
    pub(in crate::ui::shell) pending_task: Option<gpui::Task<()>>,
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
    /// One-shot target block that should be brought into view after layout.
    pub(in crate::ui::shell) search_scroll_target: Option<(usize, usize)>,
    /// Agent mode controlling tool availability, policy enforcement, and confirmation behavior.
    pub(in crate::ui::shell) agent_mode: AgentMode,
    /// Default execution target captured when the user submitted the prompt.
    pub(in crate::ui::shell) active_exec_context: Option<SessionAgentExecutionContext>,
    /// Attachments staged in the composer but not yet sent.
    pub(in crate::ui::shell) pending_attachments:
        Vec<miaominal_core::chat_attachment::ChatAttachment>,
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

    pub(in crate::ui::shell) fn has_pending_task(&self) -> bool {
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
        if let Some(tool_call) = self
            .messages
            .iter_mut()
            .rev()
            .filter_map(|message| message.tool_call.as_mut())
            .find(|tool_call| tool_call.id == id)
        {
            tool_call.status = SessionAgentToolStatus::Rejected;
            tool_call.requires_confirmation = false;
            tool_call.confirmation_note =
                Some(i18n::string("workspace.panel.agent.messages.tool_denied"));
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

    fn finish_active_thinking(&mut self) {
        if let Some(message) = self.messages.last_mut()
            && message.role == SessionAgentMessageRole::Thinking
            && let Some(thinking) = message.thinking.as_mut()
            && thinking.elapsed_ms.is_none()
        {
            thinking.elapsed_ms = Some(thinking.started_at.elapsed().as_millis());
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

pub(in crate::ui::shell) struct TabState {
    pub(in crate::ui::shell) id: usize,
    pub(in crate::ui::shell) title: String,
    pub(in crate::ui::shell) status: String,
    pub(in crate::ui::shell) kind: TabKind,
    pub(in crate::ui::shell) workspace: Option<TabWorkspaceState>,
    pub(in crate::ui::shell) hidden_from_topbar: bool,
}

pub(in crate::ui::shell) struct SessionTabState {
    pub(in crate::ui::shell) profile_id: String,
    pub(in crate::ui::shell) port_forward_rule_id: Option<String>,
    pub(in crate::ui::shell) terminal: TerminalState,
    pub(in crate::ui::shell) connection_state: SessionConnectionState,
    pub(in crate::ui::shell) preserved_history_popup_hidden: bool,
    pub(in crate::ui::shell) pending_profile: Option<SessionProfile>,
    pub(in crate::ui::shell) commands: Option<SessionCommandSender>,
    pub(in crate::ui::shell) pty_output_tap: Option<TerminalOutputTap>,
    pub(in crate::ui::shell) bytes_in: u64,
    pub(in crate::ui::shell) bytes_out: u64,
    pub(in crate::ui::shell) pending_host_key: Option<HostKeyPrompt>,
    pub(in crate::ui::shell) pending_keyboard_interactive: Option<KbiChallenge>,
    pub(in crate::ui::shell) reconnect_task: Option<gpui::Task<()>>,
    pub(in crate::ui::shell) reconnect_attempt: u32,
    pub(in crate::ui::shell) has_activity: bool,
    pub(in crate::ui::shell) monitoring: SessionMonitoringState,
    pub(in crate::ui::shell) purpose: SessionPurpose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum SessionPurpose {
    Terminal,
    PortForwarding,
    ConnectionTest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum SessionFailureStatus {
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum SessionConnectionState {
    Connecting,
    Ready,
    Reconnecting {
        error: String,
        attempt: u32,
    },
    Failed {
        error: String,
        status: Option<SessionFailureStatus>,
    },
    Disconnected,
}

impl SessionConnectionState {
    pub(in crate::ui::shell) fn preserves_terminal_history(&self) -> bool {
        matches!(self, Self::Failed { .. } | Self::Disconnected)
    }
}

impl SessionTabState {
    pub(in crate::ui::shell) fn set_connection_state(
        &mut self,
        connection_state: SessionConnectionState,
    ) {
        self.connection_state = connection_state;
        self.preserved_history_popup_hidden = false;
    }

    pub(in crate::ui::shell) fn preserves_terminal_history(&self) -> bool {
        self.purpose == SessionPurpose::Terminal
            && self.connection_state.preserves_terminal_history()
    }

    pub(in crate::ui::shell) fn uses_blocking_placeholder(&self) -> bool {
        !matches!(self.connection_state, SessionConnectionState::Ready)
            && !self.preserves_terminal_history()
    }

    pub(in crate::ui::shell) fn is_terminal_read_only(&self) -> bool {
        self.preserves_terminal_history()
    }

    pub(in crate::ui::shell) fn preserved_history_popup_hidden(&self) -> bool {
        self.preserves_terminal_history() && self.preserved_history_popup_hidden
    }

    pub(in crate::ui::shell) fn hide_preserved_history_popup(&mut self) {
        if self.preserves_terminal_history() {
            self.preserved_history_popup_hidden = true;
        }
    }
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct LocalSftpEntry {
    pub(in crate::ui::shell) filename: String,
    pub(in crate::ui::shell) path: PathBuf,
    pub(in crate::ui::shell) is_directory: bool,
    pub(in crate::ui::shell) size: Option<u64>,
    pub(in crate::ui::shell) modified: Option<SystemTime>,
    pub(in crate::ui::shell) attributes: Option<String>,
    pub(in crate::ui::shell) owner: Option<String>,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) enum SftpTransferStatus {
    Queued,
    Running,
    Paused,
    Done,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) enum SftpTransferChildStatus {
    Running,
    Paused,
    Done,
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct SftpTransferChildRow {
    pub(in crate::ui::shell) child_id: TransferChildId,
    pub(in crate::ui::shell) relative_path: String,
    pub(in crate::ui::shell) bytes_complete: u64,
    pub(in crate::ui::shell) bytes_total: Option<u64>,
    pub(in crate::ui::shell) status: SftpTransferChildStatus,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct SftpTransferRow {
    pub(in crate::ui::shell) transfer_id: TransferId,
    pub(in crate::ui::shell) direction: TransferDirection,
    pub(in crate::ui::shell) source: PathBuf,
    pub(in crate::ui::shell) destination: String,
    pub(in crate::ui::shell) bytes_complete: u64,
    pub(in crate::ui::shell) bytes_total: Option<u64>,
    pub(in crate::ui::shell) status: SftpTransferStatus,
    pub(in crate::ui::shell) bytes_per_second: Option<u64>,
    pub(in crate::ui::shell) last_progress_at: Option<Instant>,
    pub(in crate::ui::shell) last_bytes_complete: u64,
    pub(in crate::ui::shell) is_directory: bool,
    pub(in crate::ui::shell) expanded: bool,
    pub(in crate::ui::shell) children: VecDeque<SftpTransferChildRow>,
    pub(in crate::ui::shell) child_count: u64,
}

impl SftpTransferRow {
    pub(in crate::ui::shell) fn push_child(&mut self, child: SftpTransferChild) {
        self.is_directory = true;
        self.child_count = self.child_count.saturating_add(1);
        if self.children.len() == SFTP_TRANSFER_CHILD_HISTORY_LIMIT {
            self.children.pop_front();
        }
        self.children.push_back(SftpTransferChildRow {
            child_id: child.child_id,
            relative_path: child.relative_path,
            bytes_complete: 0,
            bytes_total: child.bytes_total,
            status: SftpTransferChildStatus::Running,
        });
    }

    pub(in crate::ui::shell) fn omitted_child_count(&self) -> u64 {
        self.child_count.saturating_sub(self.children.len() as u64)
    }

    pub(in crate::ui::shell) fn apply_child_update(&mut self, update: SftpTransferChildUpdate) {
        let Some(child) = self
            .children
            .iter_mut()
            .find(|child| child.child_id == update.child_id)
        else {
            return;
        };
        child.bytes_complete = update.bytes_complete;
        child.status = match update.state {
            SftpTransferChildState::Running => SftpTransferChildStatus::Running,
            SftpTransferChildState::Done => {
                if let Some(total) = child.bytes_total {
                    child.bytes_complete = total;
                }
                SftpTransferChildStatus::Done
            }
            SftpTransferChildState::Cancelled => SftpTransferChildStatus::Cancelled,
            SftpTransferChildState::Failed(message) => SftpTransferChildStatus::Failed(message),
        };
    }

    pub(in crate::ui::shell) fn pause_active_child(&mut self) {
        if let Some(child) = self
            .children
            .iter_mut()
            .find(|child| matches!(child.status, SftpTransferChildStatus::Running))
        {
            child.status = SftpTransferChildStatus::Paused;
        }
    }

    pub(in crate::ui::shell) fn resume_active_child(&mut self) {
        if let Some(child) = self
            .children
            .iter_mut()
            .find(|child| matches!(child.status, SftpTransferChildStatus::Paused))
        {
            child.status = SftpTransferChildStatus::Running;
        }
    }

    pub(in crate::ui::shell) fn cancel_unfinished_children(&mut self) {
        for child in &mut self.children {
            if matches!(
                child.status,
                SftpTransferChildStatus::Running | SftpTransferChildStatus::Paused
            ) {
                child.status = SftpTransferChildStatus::Cancelled;
            }
        }
    }

    pub(in crate::ui::shell) fn fail_unfinished_children(&mut self, message: &str) {
        if !self
            .children
            .iter()
            .any(|child| matches!(child.status, SftpTransferChildStatus::Failed(_)))
            && let Some(child) = self.children.iter_mut().find(|child| {
                matches!(
                    child.status,
                    SftpTransferChildStatus::Running | SftpTransferChildStatus::Paused
                )
            })
        {
            child.status = SftpTransferChildStatus::Failed(message.to_string());
        }
    }
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) enum SftpPromptKind {
    CreateRemoteDirectory {
        parent: String,
    },
    ConfirmOverwrite {
        conflict_count: usize,
        pending_uploads: Vec<(PathBuf, String)>,
        pending_downloads: Vec<(String, PathBuf)>,
    },
    ConfirmDelete {
        entries: Vec<(String, bool)>,
        refresh_path: String,
    },
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct SftpPromptState {
    pub(in crate::ui::shell) kind: SftpPromptKind,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingProfileDeleteState {
    pub(in crate::ui::shell) profile_id: String,
    pub(in crate::ui::shell) profile_name: String,
    pub(in crate::ui::shell) reload_inputs_after_delete: bool,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingManagedKeyDeleteState {
    pub(in crate::ui::shell) key_id: String,
    pub(in crate::ui::shell) key_name: String,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingKnownHostDeleteState {
    pub(in crate::ui::shell) host: String,
    pub(in crate::ui::shell) port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::ui::shell) enum TrustedHostFilter {
    #[default]
    All,
    Linked,
    Orphaned,
    DefaultPort,
    CustomPort,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingSnippetDeleteState {
    pub(in crate::ui::shell) snippet_id: String,
    pub(in crate::ui::shell) snippet_description: String,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingPortForwardRuleDeleteState {
    pub(in crate::ui::shell) profile_id: String,
    pub(in crate::ui::shell) rule_id: String,
    pub(in crate::ui::shell) profile_label: String,
    pub(in crate::ui::shell) rule_label: String,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingChatSessionDeleteState {
    pub(in crate::ui::shell) session_id: String,
    pub(in crate::ui::shell) title: String,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingChatSessionRenameState {
    pub(in crate::ui::shell) session_id: String,
    pub(in crate::ui::shell) current_title: String,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingSyncDirectionState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::shell) enum SyncPullConfirmReason {
    Manual,
    RemoteNewer,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingSyncPullConfirmState {
    pub(in crate::ui::shell) reason: SyncPullConfirmReason,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingLocalVaultDisableConfirmState;

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingLocalDataResetConfirmState;

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingLocalDataResetConfirmationPopupState;

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingSyncPassphraseClearConfirmPopupState;

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingSyncPassphrasePopupState;

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingAiProviderPopupState;

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingWebSearchConfigPopupState;

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct PendingSyncProviderConfigPopupState {
    pub(in crate::ui::shell) provider: SyncProvider,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) enum DialogOverlaySnapshot {
    HostKey(HostKeyPrompt),
    KeyboardInteractive(KbiChallenge),
    ProfileDelete(PendingProfileDeleteState),
    ManagedKeyDelete(PendingManagedKeyDeleteState),
    KnownHostDelete(PendingKnownHostDeleteState),
    SnippetDelete(PendingSnippetDeleteState),
    PortForwardRuleDelete(PendingPortForwardRuleDeleteState),
    ChatSessionDelete(PendingChatSessionDeleteState),
    ChatSessionRename(PendingChatSessionRenameState),
    SyncDirection(PendingSyncDirectionState),
    SyncPullConfirm(PendingSyncPullConfirmState),
    LocalVaultDisableConfirm(PendingLocalVaultDisableConfirmState),
    LocalDataResetConfirm(PendingLocalDataResetConfirmState),
    LocalDataResetConfirmationPopup(PendingLocalDataResetConfirmationPopupState),
    SyncPassphraseClearConfirmPopup(PendingSyncPassphraseClearConfirmPopupState),
    SyncPassphrasePopup(PendingSyncPassphrasePopupState),
    AiProviderPopup(PendingAiProviderPopupState),
    WebSearchConfigPopup(PendingWebSearchConfigPopupState),
    SyncProviderConfigPopup(PendingSyncProviderConfigPopupState),
    LocalVaultPassphrasePopup(LocalVaultPassphrasePopupMode),
    SftpPrompt {
        tab_id: usize,
        prompt: SftpPromptState,
    },
}

impl DialogOverlaySnapshot {
    pub(in crate::ui::shell) fn stable_key(&self) -> String {
        match self {
            Self::HostKey(_) => "trusted-host-key".to_string(),
            Self::KeyboardInteractive(_) => "keyboard-interactive".to_string(),
            Self::ProfileDelete(_) => "profile-delete".to_string(),
            Self::ManagedKeyDelete(_) => "managed-key-delete".to_string(),
            Self::KnownHostDelete(_) => "known-host-delete".to_string(),
            Self::SnippetDelete(_) => "snippet-delete".to_string(),
            Self::PortForwardRuleDelete(_) => "port-forward-rule-delete".to_string(),
            Self::ChatSessionDelete(_) => "chat-session-delete".to_string(),
            Self::ChatSessionRename(_) => "chat-session-rename".to_string(),
            Self::SyncDirection(_) => "sync-direction".to_string(),
            Self::SyncPullConfirm(_) => "sync-pull-confirm".to_string(),
            Self::LocalVaultDisableConfirm(_) => "local-vault-disable-confirm".to_string(),
            Self::LocalDataResetConfirm(_) => "local-data-reset-confirm".to_string(),
            Self::LocalDataResetConfirmationPopup(_) => "local-data-reset-confirmation".to_string(),
            Self::SyncPassphraseClearConfirmPopup(_) => "sync-passphrase-clear-confirm".to_string(),
            Self::SyncPassphrasePopup(_) => "sync-passphrase".to_string(),
            Self::AiProviderPopup(_) => "ai-provider".to_string(),
            Self::WebSearchConfigPopup(_) => "web-search-config".to_string(),
            Self::SyncProviderConfigPopup(popup) => match popup.provider {
                SyncProvider::None => "sync-provider-none".to_string(),
                SyncProvider::GithubGist => "sync-provider-gist".to_string(),
                SyncProvider::WebDav => "sync-provider-webdav".to_string(),
            },
            Self::LocalVaultPassphrasePopup(_) => "local-vault-passphrase".to_string(),
            Self::SftpPrompt { tab_id, .. } => format!("sftp-prompt-{tab_id}"),
        }
    }
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct ExitingDialogState {
    pub(in crate::ui::shell) snapshot: DialogOverlaySnapshot,
    pub(in crate::ui::shell) started_at: Instant,
}

pub(in crate::ui::shell) struct ClosedSessionTabState {
    pub(in crate::ui::shell) profile: SessionProfile,
    pub(in crate::ui::shell) hidden_from_topbar: bool,
}

pub(in crate::ui::shell) enum ClosedTabBundle {
    Hosts,
    Sftp {
        profile: SessionProfile,
    },
    SessionWorkspace {
        tabs: Vec<ClosedSessionTabState>,
        workspace: Option<TabWorkspaceState>,
    },
}

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct SftpDragSelectionState {
    pub(in crate::ui::shell) start: Point<Pixels>,
    pub(in crate::ui::shell) current: Point<Pixels>,
    pub(in crate::ui::shell) last_row_range: Option<(usize, usize)>,
}

impl SftpDragSelectionState {
    pub(in crate::ui::shell) fn new(start: Point<Pixels>) -> Self {
        Self {
            start,
            current: start,
            last_row_range: None,
        }
    }

    pub(in crate::ui::shell) fn update(&mut self, current: Point<Pixels>) {
        self.current = current;
    }

    pub(in crate::ui::shell) fn bounds(&self) -> Bounds<Pixels> {
        let left = if self.start.x <= self.current.x {
            self.start.x
        } else {
            self.current.x
        };
        let top = if self.start.y <= self.current.y {
            self.start.y
        } else {
            self.current.y
        };
        let right = if self.start.x >= self.current.x {
            self.start.x
        } else {
            self.current.x
        };
        let bottom = if self.start.y >= self.current.y {
            self.start.y
        } else {
            self.current.y
        };

        Bounds::from_corners(Point::new(left, top), Point::new(right, bottom))
    }

    pub(in crate::ui::shell) fn exceeds_threshold(&self, threshold: Pixels) -> bool {
        let bounds = self.bounds();
        bounds.size.width >= threshold || bounds.size.height >= threshold
    }

    pub(in crate::ui::shell) fn set_last_row_range(
        &mut self,
        row_range: Option<(usize, usize)>,
    ) -> bool {
        if self.last_row_range == row_range {
            return false;
        }

        self.last_row_range = row_range;
        true
    }
}

#[derive(Debug, Clone, Copy)]
pub(in crate::ui::shell) struct SftpDragSelectionContext {
    pub(in crate::ui::shell) side: SftpBrowserSide,
    pub(in crate::ui::shell) tab_id: usize,
    pub(in crate::ui::shell) last_position: Point<Pixels>,
    pub(in crate::ui::shell) panel_bounds: Bounds<Pixels>,
    pub(in crate::ui::shell) row_height: Pixels,
    pub(in crate::ui::shell) anchor_content_y: f32,
    pub(in crate::ui::shell) generation: u64,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct InlineRenameState {
    pub(in crate::ui::shell) from: String,
    pub(in crate::ui::shell) parent: String,
}

pub(in crate::ui::shell) struct SftpEditSession {
    pub(in crate::ui::shell) temp_path: PathBuf,
    // Keeps the filesystem watcher alive; dropping it unregisters the watch.
    pub(in crate::ui::shell) _watcher: notify::RecommendedWatcher,
    // Pending debounced upload task; dropping it cancels the pending upload.
    pub(in crate::ui::shell) debounce_task: Option<gpui::Task<()>>,
    // Background task that reads watcher events; dropping it cancels the loop.
    pub(in crate::ui::shell) _watch_task: gpui::Task<()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::shell) enum SftpSplitDivider {
    BrowserPanels,
    ProgressCenter,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct SftpSplitDragState {
    pub(in crate::ui::shell) divider: SftpSplitDivider,
    pub(in crate::ui::shell) initial_pointer: f32,
    pub(in crate::ui::shell) initial_flex_a: f32,
    pub(in crate::ui::shell) container_size: f32,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct SftpLayoutState {
    pub(in crate::ui::shell) local_panel_flex: Option<f32>,
    pub(in crate::ui::shell) browser_area_flex: Option<f32>,
    pub(in crate::ui::shell) progress_center_visible: bool,
    pub(in crate::ui::shell) progress_center_transition: Option<SftpProgressCenterTransition>,
    pub(in crate::ui::shell) browser_container_width: Pixels,
    pub(in crate::ui::shell) page_container_height: Pixels,
    pub(in crate::ui::shell) drag: Option<SftpSplitDragState>,
}

impl Default for SftpLayoutState {
    fn default() -> Self {
        Self {
            local_panel_flex: None,
            browser_area_flex: None,
            progress_center_visible: false,
            progress_center_transition: None,
            browser_container_width: px(0.0),
            page_container_height: px(0.0),
            drag: None,
        }
    }
}

pub(in crate::ui::shell) struct SftpTabState {
    pub(in crate::ui::shell) profile_id: String,
    pub(in crate::ui::shell) owner_session_tab_id: Option<usize>,
    pub(in crate::ui::shell) commands: Option<SftpCommandSender>,
    pub(in crate::ui::shell) local_path: PathBuf,
    pub(in crate::ui::shell) local_entries: Vec<LocalSftpEntry>,
    pub(in crate::ui::shell) selected_local_path: Option<PathBuf>,
    pub(in crate::ui::shell) selected_local_paths: Vec<PathBuf>,
    pub(in crate::ui::shell) local_selection_anchor: Option<PathBuf>,
    pub(in crate::ui::shell) remote_path: String,
    pub(in crate::ui::shell) remote_entries: Vec<SftpEntry>,
    pub(in crate::ui::shell) selected_remote_path: Option<String>,
    pub(in crate::ui::shell) selected_remote_paths: Vec<String>,
    pub(in crate::ui::shell) remote_selection_anchor: Option<String>,
    pub(in crate::ui::shell) transfers: Vec<SftpTransferRow>,
    pub(in crate::ui::shell) last_status: String,
    pub(in crate::ui::shell) last_error: Option<String>,
    pub(in crate::ui::shell) loading_remote: bool,
    pub(in crate::ui::shell) prompt: Option<SftpPromptState>,
    pub(in crate::ui::shell) local_drag_candidate: Option<Point<Pixels>>,
    pub(in crate::ui::shell) remote_drag_candidate: Option<Point<Pixels>>,
    pub(in crate::ui::shell) local_drag_selection: Option<SftpDragSelectionState>,
    pub(in crate::ui::shell) remote_drag_selection: Option<SftpDragSelectionState>,
    pub(in crate::ui::shell) drag_selection_context: Option<SftpDragSelectionContext>,
    pub(in crate::ui::shell) drag_selection_generation: u64,
    pub(in crate::ui::shell) suppress_local_clear_click: bool,
    pub(in crate::ui::shell) suppress_remote_clear_click: bool,
    pub(in crate::ui::shell) inline_rename: Option<InlineRenameState>,
    pub(in crate::ui::shell) edit_pending_downloads: std::collections::HashMap<TransferId, String>,
    pub(in crate::ui::shell) edit_sessions: std::collections::HashMap<String, SftpEditSession>,
    pub(in crate::ui::shell) layout: SftpLayoutState,
}

pub(in crate::ui::shell) enum TabKind {
    Hosts,
    Session(Box<SessionTabState>),
    Sftp(Box<SftpTabState>),
}

pub(in crate::ui::shell) struct HostEditorEnvironmentVariableRow {
    pub(in crate::ui::shell) name_input: Entity<InputState>,
    pub(in crate::ui::shell) value_input: Entity<InputState>,
}

fn localized_port_forward_kind_label(kind: PortForwardKind) -> String {
    match kind {
        PortForwardKind::Local => i18n::string("forwarding.editor.local"),
        PortForwardKind::Remote => i18n::string("forwarding.editor.remote"),
    }
}

impl TabState {
    pub(in crate::ui::shell) fn new_hosts(id: usize) -> Self {
        let title = i18n::string("tabs.initial.hosts_tab_title");
        let status = i18n::string("navigation.section.hosts.title");

        Self {
            id,
            title,
            status,
            kind: TabKind::Hosts,
            workspace: None,
            hidden_from_topbar: false,
        }
    }

    pub(in crate::ui::shell) fn new_session_pending(
        id: usize,
        profile: SessionProfile,
        terminal: TerminalState,
        auto_collect_monitoring: bool,
    ) -> Self {
        let title = profile.name.clone();
        let connection = profile.connection_label();
        let status = i18n::string_args(
            "tabs.initial.session_connecting_to",
            &[("connection", &connection)],
        );
        let profile_id = profile.id.clone();

        Self {
            id,
            title,
            status,
            kind: TabKind::Session(Box::new(SessionTabState {
                profile_id,
                port_forward_rule_id: None,
                terminal,
                connection_state: SessionConnectionState::Connecting,
                preserved_history_popup_hidden: false,
                pending_profile: Some(profile),
                commands: None,
                pty_output_tap: None,
                bytes_in: 0,
                bytes_out: 0,
                pending_host_key: None,
                pending_keyboard_interactive: None,
                reconnect_task: None,
                reconnect_attempt: 0,
                has_activity: false,
                monitoring: SessionMonitoringState::new(auto_collect_monitoring),
                purpose: SessionPurpose::Terminal,
            })),
            workspace: None,
            hidden_from_topbar: false,
        }
    }

    pub(in crate::ui::shell) fn new_port_forwarding(
        id: usize,
        profile: &SessionProfile,
        rule: &PortForwardRule,
        commands: SessionCommandSender,
    ) -> Self {
        let kind = localized_port_forward_kind_label(rule.kind);
        let listen_port = rule.listen_port.to_string();
        let target_port = rule.target_port.to_string();
        let title = if rule.label.trim().is_empty() {
            i18n::string_args(
                "tabs.initial.port_forward_title",
                &[
                    ("kind", &kind),
                    ("listen_host", &rule.listen_host),
                    ("listen_port", &listen_port),
                    ("target_host", &rule.target_host),
                    ("target_port", &target_port),
                ],
            )
        } else {
            i18n::string_args(
                "tabs.initial.port_forward_named_title",
                &[("label", &rule.label)],
            )
        };
        let profile_summary = profile.summary();

        Self {
            id,
            title,
            status: i18n::string_args(
                "tabs.initial.port_forward_connecting_to",
                &[("profile", &profile_summary)],
            ),
            kind: TabKind::Session(Box::new(SessionTabState {
                profile_id: profile.id.clone(),
                port_forward_rule_id: Some(rule.id.clone()),
                terminal: TerminalState::default(),
                connection_state: SessionConnectionState::Connecting,
                preserved_history_popup_hidden: false,
                pending_profile: None,
                commands: Some(commands),
                pty_output_tap: None,
                bytes_in: 0,
                bytes_out: 0,
                pending_host_key: None,
                pending_keyboard_interactive: None,
                reconnect_task: None,
                reconnect_attempt: 0,
                has_activity: false,
                monitoring: SessionMonitoringState::new(false),
                purpose: SessionPurpose::PortForwarding,
            })),
            workspace: None,
            hidden_from_topbar: true,
        }
    }

    pub(in crate::ui::shell) fn new_connection_test(
        id: usize,
        profile: &SessionProfile,
        commands: SessionCommandSender,
    ) -> Self {
        let profile_summary = profile.summary();

        Self {
            id,
            title: i18n::string_args(
                "tabs.initial.connection_test_title",
                &[("profile", &profile_summary)],
            ),
            status: i18n::string_args(
                "tabs.initial.connection_test_status",
                &[("profile", &profile_summary)],
            ),
            kind: TabKind::Session(Box::new(SessionTabState {
                profile_id: profile.id.clone(),
                port_forward_rule_id: None,
                terminal: TerminalState::default(),
                connection_state: SessionConnectionState::Connecting,
                preserved_history_popup_hidden: false,
                pending_profile: None,
                commands: Some(commands),
                pty_output_tap: None,
                bytes_in: 0,
                bytes_out: 0,
                pending_host_key: None,
                pending_keyboard_interactive: None,
                reconnect_task: None,
                reconnect_attempt: 0,
                has_activity: false,
                monitoring: SessionMonitoringState::new(false),
                purpose: SessionPurpose::ConnectionTest,
            })),
            workspace: None,
            hidden_from_topbar: true,
        }
    }

    pub(in crate::ui::shell) fn new_sftp(id: usize, profile: &SessionProfile) -> Self {
        let local_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        Self {
            id,
            title: profile.connection_label(),
            status: i18n::string("tabs.initial.sftp_connecting"),
            kind: TabKind::Sftp(Box::new(SftpTabState {
                profile_id: profile.id.clone(),
                owner_session_tab_id: None,
                commands: None,
                local_path,
                local_entries: Vec::new(),
                selected_local_path: None,
                selected_local_paths: Vec::new(),
                local_selection_anchor: None,
                remote_path: ".".into(),
                remote_entries: Vec::new(),
                selected_remote_path: None,
                selected_remote_paths: Vec::new(),
                remote_selection_anchor: None,
                transfers: Vec::new(),
                last_status: i18n::string("tabs.initial.sftp_starting_worker"),
                last_error: None,
                loading_remote: true,
                prompt: None,
                local_drag_candidate: None,
                remote_drag_candidate: None,
                local_drag_selection: None,
                remote_drag_selection: None,
                drag_selection_context: None,
                drag_selection_generation: 0,
                suppress_local_clear_click: false,
                suppress_remote_clear_click: false,
                inline_rename: None,
                edit_pending_downloads: std::collections::HashMap::new(),
                edit_sessions: std::collections::HashMap::new(),
                layout: SftpLayoutState::default(),
            })),
            workspace: None,
            hidden_from_topbar: false,
        }
    }

    pub(in crate::ui::shell) fn is_hosts(&self) -> bool {
        matches!(self.kind, TabKind::Hosts)
    }

    pub(in crate::ui::shell) fn as_session(&self) -> Option<&SessionTabState> {
        match &self.kind {
            TabKind::Session(session) => Some(session.as_ref()),
            TabKind::Hosts | TabKind::Sftp(_) => None,
        }
    }

    pub(in crate::ui::shell) fn as_session_mut(&mut self) -> Option<&mut SessionTabState> {
        match &mut self.kind {
            TabKind::Session(session) => Some(session.as_mut()),
            TabKind::Hosts | TabKind::Sftp(_) => None,
        }
    }

    pub(in crate::ui::shell) fn as_sftp(&self) -> Option<&SftpTabState> {
        match &self.kind {
            TabKind::Sftp(sftp) => Some(sftp.as_ref()),
            TabKind::Hosts | TabKind::Session(_) => None,
        }
    }

    pub(in crate::ui::shell) fn as_sftp_mut(&mut self) -> Option<&mut SftpTabState> {
        match &mut self.kind {
            TabKind::Sftp(sftp) => Some(sftp.as_mut()),
            TabKind::Hosts | TabKind::Session(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(in crate::ui::shell) struct DraggedTab {
    pub(in crate::ui::shell) source_tab_id: usize,
    pub(in crate::ui::shell) source_index: usize,
    pub(in crate::ui::shell) source_pane_id: PaneId,
    pub(in crate::ui::shell) is_active: bool,
    pub(in crate::ui::shell) title: String,
    pub(in crate::ui::shell) status_color: Option<u32>,
}

impl Render for DraggedTab {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let roles = miaominal_settings::current_theme().material.roles;

        h_flex()
            .w(px(TOPBAR_TAB_WIDTH))
            .h(px(36.0))
            .px_3()
            .gap_2()
            .items_center()
            .rounded(px(14.0))
            .bg(rgb(if self.is_active {
                roles.secondary_container
            } else {
                roles.surface_container_high
            }))
            .opacity(0.92)
            .text_size(miaominal_settings::FontSize::Body.scaled())
            .text_color(rgb(if self.is_active {
                roles.on_secondary_container
            } else {
                roles.on_surface_variant
            }))
            .child(
                h_flex()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap_2()
                    .items_center()
                    .when_some(self.status_color, |this, color| {
                        this.child(div().size(px(7.0)).rounded(px(999.0)).bg(rgb(color)))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .child(self.title.clone()),
                    ),
            )
    }
}

#[derive(Default)]
pub(in crate::ui::shell) struct DialogState {
    pub(in crate::ui::shell) pending_profile_delete: Option<PendingProfileDeleteState>,
    pub(in crate::ui::shell) pending_managed_key_delete: Option<PendingManagedKeyDeleteState>,
    pub(in crate::ui::shell) pending_known_host_delete: Option<PendingKnownHostDeleteState>,
    pub(in crate::ui::shell) pending_snippet_delete: Option<PendingSnippetDeleteState>,
    pub(in crate::ui::shell) pending_port_forward_rule_delete:
        Option<PendingPortForwardRuleDeleteState>,
    pub(in crate::ui::shell) pending_chat_session_delete: Option<PendingChatSessionDeleteState>,
    pub(in crate::ui::shell) pending_chat_session_rename: Option<PendingChatSessionRenameState>,
    pub(in crate::ui::shell) pending_sync_direction: Option<PendingSyncDirectionState>,
    pub(in crate::ui::shell) pending_sync_pull_confirm: Option<PendingSyncPullConfirmState>,
    pub(in crate::ui::shell) pending_local_vault_disable_confirm:
        Option<PendingLocalVaultDisableConfirmState>,
    pub(in crate::ui::shell) pending_local_data_reset_confirm:
        Option<PendingLocalDataResetConfirmState>,
    pub(in crate::ui::shell) pending_local_data_reset_confirmation_popup:
        Option<PendingLocalDataResetConfirmationPopupState>,
    pub(in crate::ui::shell) pending_sync_passphrase_clear_confirm_popup:
        Option<PendingSyncPassphraseClearConfirmPopupState>,
    pub(in crate::ui::shell) exiting_dialogs: Vec<ExitingDialogState>,
}

pub(in crate::ui::shell) struct OnboardingState {
    pub(in crate::ui::shell) show_onboarding: bool,
    pub(in crate::ui::shell) onboarding_step: OnboardingStep,
    pub(in crate::ui::shell) visible_onboarding_step: OnboardingStep,
    pub(in crate::ui::shell) onboarding_step_transition: Option<OnboardingStepTransition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::shell) enum SyncPassphraseOperation {
    Save,
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::shell) enum SyncProviderConfigSaveOperation {
    GithubGist,
    WebDav,
}

pub(in crate::ui::shell) struct SyncUiState {
    pub(in crate::ui::shell) sync_engine: SyncEngine,
    pub(in crate::ui::shell) sync_status: SyncStatus,
    pub(in crate::ui::shell) active_sync_task: Option<gpui::Task<()>>,
    pub(in crate::ui::shell) sync_provider_config_save_operation:
        Option<SyncProviderConfigSaveOperation>,
    pub(in crate::ui::shell) sync_passphrase_operation: Option<SyncPassphraseOperation>,
    pub(in crate::ui::shell) sync_passphrase_configured: bool,
}

#[derive(Default)]
pub(in crate::ui::shell) struct SecretVisibilityState {
    sync_github_token: bool,
    sync_webdav_password: bool,
    host_password: bool,
    sync_passphrase: bool,
    sync_passphrase_confirmation: bool,
    local_vault_passphrase: bool,
    local_vault_passphrase_confirmation: bool,
    web_search_api_key: bool,
    ai_provider_api_keys: std::collections::HashSet<String>,
}

impl SecretVisibilityState {
    pub(in crate::ui::shell) fn is_visible(&self, target: &SecretRevealTarget) -> bool {
        match target {
            SecretRevealTarget::SyncGithubToken => self.sync_github_token,
            SecretRevealTarget::SyncWebdavPassword => self.sync_webdav_password,
            SecretRevealTarget::HostPassword => self.host_password,
            SecretRevealTarget::SyncPassphrase => self.sync_passphrase,
            SecretRevealTarget::SyncPassphraseConfirmation => self.sync_passphrase_confirmation,
            SecretRevealTarget::LocalVaultPassphrase => self.local_vault_passphrase,
            SecretRevealTarget::LocalVaultPassphraseConfirmation => {
                self.local_vault_passphrase_confirmation
            }
            SecretRevealTarget::WebSearchApiKey => self.web_search_api_key,
            SecretRevealTarget::AiProviderApiKey(provider_id) => {
                self.ai_provider_api_keys.contains(provider_id)
            }
        }
    }

    pub(in crate::ui::shell) fn set_visible(&mut self, target: SecretRevealTarget, visible: bool) {
        match target {
            SecretRevealTarget::SyncGithubToken => self.sync_github_token = visible,
            SecretRevealTarget::SyncWebdavPassword => self.sync_webdav_password = visible,
            SecretRevealTarget::HostPassword => self.host_password = visible,
            SecretRevealTarget::SyncPassphrase => self.sync_passphrase = visible,
            SecretRevealTarget::SyncPassphraseConfirmation => {
                self.sync_passphrase_confirmation = visible;
            }
            SecretRevealTarget::LocalVaultPassphrase => self.local_vault_passphrase = visible,
            SecretRevealTarget::LocalVaultPassphraseConfirmation => {
                self.local_vault_passphrase_confirmation = visible;
            }
            SecretRevealTarget::WebSearchApiKey => self.web_search_api_key = visible,
            SecretRevealTarget::AiProviderApiKey(provider_id) => {
                if visible {
                    self.ai_provider_api_keys.insert(provider_id);
                } else {
                    self.ai_provider_api_keys.remove(&provider_id);
                }
            }
        }
    }

    pub(in crate::ui::shell) fn clear_ai_provider_visibility(&mut self) {
        self.ai_provider_api_keys.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::ui::shell) enum SessionSidePanelView {
    #[default]
    Monitor,
    Snippets,
    Sftp,
}

#[derive(Default)]
pub(in crate::ui::shell) struct PanelState {
    pub(in crate::ui::shell) session_side_panel_open: bool,
    pub(in crate::ui::shell) session_side_panel_view: SessionSidePanelView,
    pub(in crate::ui::shell) session_agent_panel_open: bool,
    pub(in crate::ui::shell) session_sftp_progress_center_visible: bool,
    pub(in crate::ui::shell) visible_session_side_panel: bool,
    pub(in crate::ui::shell) visible_session_agent_panel: bool,
    pub(in crate::ui::shell) session_side_panel_transition: Option<WorkspaceSidePanelTransition>,
    pub(in crate::ui::shell) session_agent_panel_transition: Option<WorkspaceSidePanelTransition>,
    pub(in crate::ui::shell) session_sftp_progress_center_transition:
        Option<SftpProgressCenterTransition>,
    pub(in crate::ui::shell) selected_known_host: Option<(String, u16, String)>,
}

pub(in crate::ui::shell) struct WorkspaceState {
    pub(in crate::ui::shell) tabs: Vec<TabState>,
    pub(in crate::ui::shell) shared_profile_monitoring: HashMap<String, SessionMonitoringState>,
    pub(in crate::ui::shell) monitor_source_tabs: HashMap<String, usize>,
    pub(in crate::ui::shell) active_topbar_tab: Option<usize>,
    pub(in crate::ui::shell) topbar_tab_scroll_handle: ScrollHandle,
    pub(in crate::ui::shell) session_monitor_scroll_handle: ScrollHandle,
    pub(in crate::ui::shell) sftp_progress_center_scroll_handle: ScrollHandle,
    pub(in crate::ui::shell) session_agent_scroll_handle: ScrollHandle,
    pub(in crate::ui::shell) session_agent_history_scroll_handle: VirtualListScrollHandle,
    pub(in crate::ui::shell) topbar_previous_visible_tabs: Vec<TopbarTabSnapshot>,
    pub(in crate::ui::shell) topbar_entering_tabs: Vec<TopbarTabEnterTransition>,
    pub(in crate::ui::shell) topbar_exiting_tabs: Vec<TopbarTabExitTransition>,
    pub(in crate::ui::shell) topbar_active_transition: Option<TopbarActiveTabTransition>,
    pub(in crate::ui::shell) topbar_visible_active_tab_id: Option<usize>,
    pub(in crate::ui::shell) next_tab_id: usize,
    pub(in crate::ui::shell) workspace: TabWorkspaceState,
    pub(in crate::ui::shell) recently_closed_tabs: Vec<ClosedTabBundle>,
    pub(in crate::ui::shell) renaming_tab: Option<usize>,
    pub(in crate::ui::shell) reported_terminal_focus_tab_id: Option<usize>,
    pub(in crate::ui::shell) primary_view_transition: Option<PrimaryViewTransition>,
    pub(in crate::ui::shell) visible_primary_view: Option<PrimaryViewKind>,
    pub(in crate::ui::shell) session_agent_panel_width: f32,
    pub(in crate::ui::shell) session_agent_panel_drag: Option<SessionAgentPanelDragState>,
    pub(in crate::ui::shell) session_sftp_progress_center_flex: f32,
    pub(in crate::ui::shell) session_sftp_progress_center_drag:
        Option<SessionSftpProgressCenterDragState>,
    pub(in crate::ui::shell) terminal_originated_selection_drag: Option<PaneId>,
    pub(in crate::ui::shell) session_agent_auto_scroll: Option<SessionAgentAutoScrollState>,
    pub(in crate::ui::shell) session_agent_auto_scroll_generation: u64,
    pub(in crate::ui::shell) session_agent_follow_bottom_generation: u64,
    pub(in crate::ui::shell) session_agent_follow_bottom_disabled_until: Option<Instant>,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct SessionAgentPanelDragState {
    pub(in crate::ui::shell) initial_pointer: f32,
    pub(in crate::ui::shell) initial_width: f32,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct SessionSftpProgressCenterDragState {
    pub(in crate::ui::shell) initial_pointer: f32,
    pub(in crate::ui::shell) initial_flex: f32,
    pub(in crate::ui::shell) container_height: Pixels,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct SessionAgentAutoScrollState {
    pub(in crate::ui::shell) anchor_y: f32,
    pub(in crate::ui::shell) pointer_y: f32,
    pub(in crate::ui::shell) generation: u64,
}

#[derive(Default)]
pub(in crate::ui::shell) struct ShellState {
    pub(in crate::ui::shell) page_editor_sidebar_transition: Option<PageEditorSidebarTransition>,
    pub(in crate::ui::shell) visible_page_editor_sidebar: Option<PageEditorSidebarKind>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_state(
        purpose: SessionPurpose,
        connection_state: SessionConnectionState,
    ) -> SessionTabState {
        SessionTabState {
            profile_id: "profile".to_string(),
            port_forward_rule_id: None,
            terminal: TerminalState::default(),
            connection_state,
            preserved_history_popup_hidden: false,
            pending_profile: None,
            commands: None,
            pty_output_tap: None,
            bytes_in: 0,
            bytes_out: 0,
            pending_host_key: None,
            pending_keyboard_interactive: None,
            reconnect_task: None,
            reconnect_attempt: 0,
            has_activity: false,
            monitoring: SessionMonitoringState::new(false),
            purpose,
        }
    }

    fn transfer_row() -> SftpTransferRow {
        SftpTransferRow {
            transfer_id: TransferId(1),
            direction: TransferDirection::Upload,
            source: PathBuf::from("root"),
            destination: "/remote/root".to_string(),
            bytes_complete: 0,
            bytes_total: None,
            status: SftpTransferStatus::Queued,
            bytes_per_second: None,
            last_progress_at: None,
            last_bytes_complete: 0,
            is_directory: false,
            expanded: false,
            children: VecDeque::new(),
            child_count: 0,
        }
    }

    fn transfer_children() -> Vec<SftpTransferChild> {
        vec![
            SftpTransferChild {
                child_id: TransferChildId(0),
                relative_path: "done.txt".to_string(),
                bytes_total: Some(3),
            },
            SftpTransferChild {
                child_id: TransferChildId(1),
                relative_path: "active.txt".to_string(),
                bytes_total: Some(5),
            },
        ]
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
    fn terminal_disconnected_preserves_history_and_is_read_only() {
        let session = session_state(
            SessionPurpose::Terminal,
            SessionConnectionState::Disconnected,
        );

        assert!(session.preserves_terminal_history());
        assert!(session.is_terminal_read_only());
        assert!(!session.uses_blocking_placeholder());
    }

    #[test]
    fn terminal_failed_preserves_history_and_is_read_only() {
        let session = session_state(
            SessionPurpose::Terminal,
            SessionConnectionState::Failed {
                error: "boom".to_string(),
                status: Some(SessionFailureStatus::Closed),
            },
        );

        assert!(session.preserves_terminal_history());
        assert!(session.is_terminal_read_only());
        assert!(!session.uses_blocking_placeholder());
    }

    #[test]
    fn connecting_session_keeps_blocking_placeholder() {
        let session = session_state(SessionPurpose::Terminal, SessionConnectionState::Connecting);

        assert!(!session.preserves_terminal_history());
        assert!(!session.is_terminal_read_only());
        assert!(session.uses_blocking_placeholder());
    }

    #[test]
    fn non_terminal_disconnected_session_does_not_preserve_history() {
        let session = session_state(
            SessionPurpose::PortForwarding,
            SessionConnectionState::Disconnected,
        );

        assert!(!session.preserves_terminal_history());
        assert!(!session.is_terminal_read_only());
        assert!(session.uses_blocking_placeholder());
    }

    #[test]
    fn changing_connection_state_resets_hidden_history_popup() {
        let mut session = session_state(
            SessionPurpose::Terminal,
            SessionConnectionState::Disconnected,
        );
        session.hide_preserved_history_popup();

        session.set_connection_state(SessionConnectionState::Connecting);

        assert!(!session.preserved_history_popup_hidden());
        assert!(session.uses_blocking_placeholder());
    }

    #[test]
    fn pending_tool_call_counts_as_active_and_can_be_rejected() {
        let mut state = SessionAgentState::default();
        state.push_tool_call(
            "tool-1".to_string(),
            "read".to_string(),
            "{\"path\":\"Cargo.toml\"}".to_string(),
            SessionAgentToolStatus::Pending,
        );

        assert!(state.has_active_tool_call());
        let stopped_by_user = i18n::string("workspace.panel.agent.messages.stopped_by_user");
        assert!(state.reject_active_tool_calls(&stopped_by_user));

        let tool = state.tool_call("tool-1").expect("tool should exist");
        assert_eq!(tool.status, SessionAgentToolStatus::Rejected);
        assert_eq!(
            tool.confirmation_note.as_deref(),
            Some(stopped_by_user.as_str())
        );
        assert!(!state.has_active_tool_call());
    }

    #[test]
    fn session_agent_running_tool_execution_mode_uses_captured_context_over_current_ui_mode() {
        let mut state = SessionAgentState {
            exec_mode: AgentExecMode::Pty,
            active_exec_context: Some(SessionAgentExecutionContext {
                profile_id: "profile-a".to_string(),
                exec_mode: AgentExecMode::ExecChannel,
                terminal_tab_id: None,
            }),
            ..Default::default()
        };

        assert_eq!(
            state.execution_mode_for_running_tools(),
            AgentExecMode::ExecChannel
        );

        state.exec_mode = AgentExecMode::ExecChannel;
        state.active_exec_context = Some(SessionAgentExecutionContext {
            profile_id: "profile-a".to_string(),
            exec_mode: AgentExecMode::Pty,
            terminal_tab_id: Some(42),
        });

        assert_eq!(state.execution_mode_for_running_tools(), AgentExecMode::Pty);
    }

    #[test]
    fn session_agent_running_tool_execution_mode_falls_back_to_current_ui_mode_without_context() {
        let mut state = SessionAgentState {
            exec_mode: AgentExecMode::Pty,
            ..Default::default()
        };

        assert_eq!(state.execution_mode_for_running_tools(), AgentExecMode::Pty);

        state.exec_mode = AgentExecMode::ExecChannel;

        assert_eq!(
            state.execution_mode_for_running_tools(),
            AgentExecMode::ExecChannel
        );
    }

    #[test]
    fn waiting_confirmation_tool_call_is_detected_separately() {
        let mut state = SessionAgentState::default();
        state.push_tool_call(
            "tool-1".to_string(),
            "run_shell".to_string(),
            "{\"command\":\"cargo test\"}".to_string(),
            SessionAgentToolStatus::InProgress,
        );

        assert!(!state.has_tool_call_waiting_for_confirmation());

        state.require_tool_call_confirmation("tool-1", "approval required".to_string());

        assert!(state.has_active_tool_call());
        assert!(state.has_tool_call_waiting_for_confirmation());
    }

    #[test]
    fn realtime_session_agent_messages_receive_enter_motion_keys() {
        let mut state = SessionAgentState::default();

        state.push_message_with_enter_motion(SessionAgentMessage::user("hello"));
        state.append_assistant_delta("hi");
        state.append_thinking_delta("checking");
        state.push_tool_call(
            "tool-1".to_string(),
            "read".to_string(),
            "{\"path\":\"Cargo.toml\"}".to_string(),
            SessionAgentToolStatus::InProgress,
        );

        let keys = state
            .messages
            .iter()
            .map(|message| message.motion.enter_key)
            .collect::<Vec<_>>();
        assert_eq!(keys, vec![Some(1), Some(2), Some(3), Some(4)]);
    }

    #[test]
    fn sftp_transfer_child_updates_and_failure_propagation_preserve_completed_files() {
        let mut transfer = transfer_row();
        let mut children = transfer_children().into_iter();
        transfer.push_child(children.next().expect("completed child"));

        assert!(transfer.is_directory);
        assert!(!transfer.expanded);
        assert_eq!(transfer.children.len(), 1);

        transfer.apply_child_update(SftpTransferChildUpdate {
            child_id: TransferChildId(0),
            bytes_complete: 3,
            state: SftpTransferChildState::Done,
        });
        transfer.push_child(children.next().expect("active child"));
        transfer.apply_child_update(SftpTransferChildUpdate {
            child_id: TransferChildId(1),
            bytes_complete: 2,
            state: SftpTransferChildState::Running,
        });
        transfer.pause_active_child();
        assert!(matches!(
            &transfer.children[1].status,
            SftpTransferChildStatus::Paused
        ));
        transfer.resume_active_child();
        assert!(matches!(
            &transfer.children[1].status,
            SftpTransferChildStatus::Running
        ));

        transfer.fail_unfinished_children("boom");

        assert!(matches!(
            &transfer.children[0].status,
            SftpTransferChildStatus::Done
        ));
        assert!(matches!(
            &transfer.children[1].status,
            SftpTransferChildStatus::Failed(message) if message == "boom"
        ));
    }

    #[test]
    fn sftp_transfer_cancel_marks_only_unfinished_children() {
        let mut transfer = transfer_row();
        let mut children = transfer_children().into_iter();
        transfer.push_child(children.next().expect("completed child"));
        transfer.apply_child_update(SftpTransferChildUpdate {
            child_id: TransferChildId(0),
            bytes_complete: 3,
            state: SftpTransferChildState::Done,
        });
        transfer.push_child(children.next().expect("active child"));
        transfer.apply_child_update(SftpTransferChildUpdate {
            child_id: TransferChildId(1),
            bytes_complete: 2,
            state: SftpTransferChildState::Running,
        });

        transfer.cancel_unfinished_children();

        assert!(matches!(
            &transfer.children[0].status,
            SftpTransferChildStatus::Done
        ));
        assert!(matches!(
            &transfer.children[1].status,
            SftpTransferChildStatus::Cancelled
        ));
    }

    #[test]
    fn sftp_transfer_child_history_is_bounded() {
        let mut transfer = transfer_row();
        let total = SFTP_TRANSFER_CHILD_HISTORY_LIMIT + 7;
        for index in 0..total {
            let child_id = TransferChildId(index as u64);
            transfer.push_child(SftpTransferChild {
                child_id,
                relative_path: format!("file-{index}.txt"),
                bytes_total: Some(1),
            });
            transfer.apply_child_update(SftpTransferChildUpdate {
                child_id,
                bytes_complete: 1,
                state: SftpTransferChildState::Done,
            });
        }

        assert_eq!(transfer.children.len(), SFTP_TRANSFER_CHILD_HISTORY_LIMIT);
        assert_eq!(transfer.child_count, total as u64);
        assert_eq!(transfer.omitted_child_count(), 7);
        assert_eq!(
            transfer.children.front().map(|child| child.child_id),
            Some(TransferChildId(7))
        );
    }

    #[test]
    fn session_agent_streaming_deltas_reuse_existing_message_enter_motion_key() {
        let mut state = SessionAgentState::default();

        state.append_assistant_delta("hello");
        let assistant_key = state.messages[0].motion.enter_key;
        state.append_assistant_delta(" world");

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].content, "hello world");
        assert_eq!(state.messages[0].motion.enter_key, assistant_key);

        state.append_thinking_delta("reason");
        let thinking_key = state.messages[1].motion.enter_key;
        state.append_thinking_delta("ing");

        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[1].content, "reasoning");
        assert_eq!(state.messages[1].motion.enter_key, thinking_key);
    }

    #[test]
    fn stopped_turn_replaces_empty_assistant_placeholder() {
        let mut state = SessionAgentState::default();
        state.messages.push(SessionAgentMessage::user("hello"));
        state.messages.push(SessionAgentMessage::assistant_raw(""));

        state.finish_stopped_turn();

        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[1].role, SessionAgentMessageRole::Assistant);
        assert_eq!(
            state.messages[1].content,
            i18n::string("workspace.panel.agent.messages.stopped_by_user")
        );
    }
}
