use super::*;
use crate::ui::i18n;
use std::time::Instant;
use tokio::sync::mpsc;

const SESSION_MONITOR_HISTORY_LIMIT: usize = 900;

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

pub(in crate::ui::shell) struct SessionAgentMessage {
    pub(in crate::ui::shell) role: SessionAgentMessageRole,
    pub(in crate::ui::shell) content: String,
    pub(in crate::ui::shell) tool_call: Option<SessionAgentToolCall>,
    pub(in crate::ui::shell) thinking: Option<SessionAgentThinking>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::shell) struct TokenUsage {
    pub(in crate::ui::shell) input_tokens: u64,
    pub(in crate::ui::shell) output_tokens: u64,
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
        }
    }

    pub(in crate::ui::shell) fn assistant_raw(content: impl Into<String>) -> Self {
        Self {
            role: SessionAgentMessageRole::Assistant,
            content: content.into(),
            tool_call: None,
            thinking: None,
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
        }
    }

    #[allow(dead_code)]
    pub(in crate::ui::shell) fn tool_call(tool_call: SessionAgentToolCall) -> Self {
        Self {
            role: SessionAgentMessageRole::ToolCall,
            content: tool_call.summary.clone(),
            tool_call: Some(tool_call),
            thinking: None,
        }
    }

    pub(in crate::ui::shell) fn error(content: impl Into<String>) -> Self {
        Self {
            role: SessionAgentMessageRole::Error,
            content: content.into(),
            tool_call: None,
            thinking: None,
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

#[derive(Default)]
pub(in crate::ui::shell) struct SessionAgentState {
    pub(in crate::ui::shell) session_id: Option<String>,
    pub(in crate::ui::shell) messages: Vec<SessionAgentMessage>,
    pub(in crate::ui::shell) pending_task: Option<gpui::Task<()>>,
    pub(in crate::ui::shell) active_request_id: u64,
    pub(in crate::ui::shell) request_counter: u64,
    pub(in crate::ui::shell) last_error: Option<String>,
    pub(in crate::ui::shell) exec_mode: AgentExecMode,
    pub(in crate::ui::shell) at_mention_query: Option<String>,
    pub(in crate::ui::shell) at_mention_anchor: usize,
    pub(in crate::ui::shell) selected_at_targets: Vec<String>,
    pub(in crate::ui::shell) active_at_targets: Vec<String>,
    pub(in crate::ui::shell) title: Option<String>,
    pub(in crate::ui::shell) panel_view: ChatPanelView,
    /// Token usage from the most recent LLM completion request.
    pub(in crate::ui::shell) last_usage: Option<TokenUsage>,
}

impl SessionAgentState {
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
                    SessionAgentToolStatus::WaitingForConfirmation
                        | SessionAgentToolStatus::InProgress
                )
            })
        })
    }

    pub(in crate::ui::shell) fn next_request_id(&mut self) -> u64 {
        self.request_counter = self.request_counter.wrapping_add(1).max(1);
        self.request_counter
    }

    pub(in crate::ui::shell) fn append_assistant_delta(
        &mut self,
        delta: impl AsRef<str>,
        _cx: &mut gpui::Context<super::app_view::AppView>,
    ) {
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
            self.messages
                .push(SessionAgentMessage::assistant_raw(delta));
        }
    }

    pub(in crate::ui::shell) fn start_assistant_reply(
        &mut self,
        _cx: &mut gpui::Context<super::app_view::AppView>,
    ) {
        self.finish_active_thinking();
        if !self.messages.last().is_some_and(|message| {
            message.role == SessionAgentMessageRole::Assistant && message.content.is_empty()
        }) {
            self.messages.push(SessionAgentMessage::assistant_raw(""));
        }
    }

    pub(in crate::ui::shell) fn append_thinking_delta(
        &mut self,
        delta: impl AsRef<str>,
        _cx: &mut gpui::Context<super::app_view::AppView>,
    ) {
        let delta = delta.as_ref();
        if delta.trim().is_empty() {
            return;
        }

        if let Some(message) = self.messages.last_mut()
            && message.role == SessionAgentMessageRole::Thinking
        {
            message.content.push_str(delta);
        } else {
            self.messages.push(SessionAgentMessage::thinking_raw(delta));
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
        self.messages
            .push(SessionAgentMessage::tool_call(SessionAgentToolCall {
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
            if result.contains("requires user approval") {
                tool_call.status = SessionAgentToolStatus::WaitingForConfirmation;
                tool_call.requires_confirmation = true;
                tool_call.confirmation_note = Some(result);
                tool_call.expanded = true;
            } else {
                tool_call.status = SessionAgentToolStatus::Completed;
                if !result.trim().is_empty() {
                    tool_call.confirmation_note = Some(result);
                }
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
            tool_call.confirmation_note = Some("Approved. Running tool...".into());
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
            tool_call.confirmation_note = Some("Denied by user.".into());
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
                SessionAgentToolStatus::WaitingForConfirmation | SessionAgentToolStatus::InProgress
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

        self.messages
            .push(SessionAgentMessage::assistant_raw(reply));
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
    pub(in crate::ui::shell) pty_output_tap: Option<mpsc::UnboundedSender<Vec<u8>>>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::shell) enum TrustedHostFilter {
    All,
    Linked,
    Orphaned,
    DefaultPort,
    CustomPort,
}

impl Default for TrustedHostFilter {
    fn default() -> Self {
        Self::All
    }
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
    SyncDirection(PendingSyncDirectionState),
    SyncPullConfirm(PendingSyncPullConfirmState),
    LocalVaultDisableConfirm(PendingLocalVaultDisableConfirmState),
    LocalDataResetConfirm(PendingLocalDataResetConfirmState),
    LocalDataResetConfirmationPopup(PendingLocalDataResetConfirmationPopupState),
    SyncPassphraseClearConfirmPopup(PendingSyncPassphraseClearConfirmPopupState),
    SyncPassphrasePopup(PendingSyncPassphrasePopupState),
    AiProviderPopup(PendingAiProviderPopupState),
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
            Self::SyncDirection(_) => "sync-direction".to_string(),
            Self::SyncPullConfirm(_) => "sync-pull-confirm".to_string(),
            Self::LocalVaultDisableConfirm(_) => "local-vault-disable-confirm".to_string(),
            Self::LocalDataResetConfirm(_) => "local-data-reset-confirm".to_string(),
            Self::LocalDataResetConfirmationPopup(_) => "local-data-reset-confirmation".to_string(),
            Self::SyncPassphraseClearConfirmPopup(_) => "sync-passphrase-clear-confirm".to_string(),
            Self::SyncPassphrasePopup(_) => "sync-passphrase".to_string(),
            Self::AiProviderPopup(_) => "ai-provider".to_string(),
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
}

impl SftpDragSelectionState {
    pub(in crate::ui::shell) fn new(start: Point<Pixels>) -> Self {
        Self {
            start,
            current: start,
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

    pub(in crate::ui::shell) fn window_bounds(&self, origin: Point<Pixels>) -> Bounds<Pixels> {
        let bounds = self.bounds();
        Bounds::from_corners(
            Point::new(origin.x + bounds.origin.x, origin.y + bounds.origin.y),
            Point::new(
                origin.x + bounds.origin.x + bounds.size.width,
                origin.y + bounds.origin.y + bounds.size.height,
            ),
        )
    }

    pub(in crate::ui::shell) fn exceeds_threshold(&self, threshold: Pixels) -> bool {
        let bounds = self.bounds();
        bounds.size.width >= threshold || bounds.size.height >= threshold
    }
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
    pub(in crate::ui::shell) browser_container_width: Pixels,
    pub(in crate::ui::shell) page_container_height: Pixels,
    pub(in crate::ui::shell) drag: Option<SftpSplitDragState>,
}

impl Default for SftpLayoutState {
    fn default() -> Self {
        Self {
            local_panel_flex: None,
            browser_area_flex: None,
            browser_container_width: px(0.0),
            page_container_height: px(0.0),
            drag: None,
        }
    }
}

pub(in crate::ui::shell) struct SftpTabState {
    pub(in crate::ui::shell) profile_id: String,
    pub(in crate::ui::shell) commands: Option<SftpCommandSender>,
    pub(in crate::ui::shell) local_path: PathBuf,
    pub(in crate::ui::shell) local_entries: Vec<LocalSftpEntry>,
    pub(in crate::ui::shell) selected_local_path: Option<PathBuf>,
    pub(in crate::ui::shell) selected_local_paths: Vec<PathBuf>,
    pub(in crate::ui::shell) remote_path: String,
    pub(in crate::ui::shell) remote_entries: Vec<SftpEntry>,
    pub(in crate::ui::shell) selected_remote_path: Option<String>,
    pub(in crate::ui::shell) selected_remote_paths: Vec<String>,
    pub(in crate::ui::shell) transfers: Vec<SftpTransferRow>,
    pub(in crate::ui::shell) last_status: String,
    pub(in crate::ui::shell) last_error: Option<String>,
    pub(in crate::ui::shell) loading_remote: bool,
    pub(in crate::ui::shell) prompt: Option<SftpPromptState>,
    pub(in crate::ui::shell) local_drag_candidate: Option<Point<Pixels>>,
    pub(in crate::ui::shell) remote_drag_candidate: Option<Point<Pixels>>,
    pub(in crate::ui::shell) local_drag_selection: Option<SftpDragSelectionState>,
    pub(in crate::ui::shell) remote_drag_selection: Option<SftpDragSelectionState>,
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
                commands: None,
                local_path,
                local_entries: Vec::new(),
                selected_local_path: None,
                selected_local_paths: Vec::new(),
                remote_path: ".".into(),
                remote_entries: Vec::new(),
                selected_remote_path: None,
                selected_remote_paths: Vec::new(),
                transfers: Vec::new(),
                last_status: i18n::string("tabs.initial.sftp_starting_worker"),
                last_error: None,
                loading_remote: true,
                prompt: None,
                local_drag_candidate: None,
                remote_drag_candidate: None,
                local_drag_selection: None,
                remote_drag_selection: None,
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
pub(in crate::ui::shell) enum SyncSecretSaveOperation {
    GithubToken,
    WebdavPassword,
}

pub(in crate::ui::shell) struct SyncUiState {
    pub(in crate::ui::shell) sync_engine: SyncEngine,
    pub(in crate::ui::shell) sync_status: SyncStatus,
    pub(in crate::ui::shell) active_sync_task: Option<gpui::Task<()>>,
    pub(in crate::ui::shell) sync_secret_save_operation: Option<SyncSecretSaveOperation>,
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
}

#[derive(Default)]
pub(in crate::ui::shell) struct PanelState {
    pub(in crate::ui::shell) session_side_panel_open: bool,
    pub(in crate::ui::shell) session_side_panel_view: SessionSidePanelView,
    pub(in crate::ui::shell) session_agent_panel_open: bool,
    pub(in crate::ui::shell) visible_session_side_panel: bool,
    pub(in crate::ui::shell) visible_session_agent_panel: bool,
    pub(in crate::ui::shell) session_side_panel_transition: Option<WorkspaceSidePanelTransition>,
    pub(in crate::ui::shell) session_agent_panel_transition: Option<WorkspaceSidePanelTransition>,
    pub(in crate::ui::shell) selected_known_host: Option<(String, u16, String)>,
}

pub(in crate::ui::shell) struct WorkspaceState {
    pub(in crate::ui::shell) tabs: Vec<TabState>,
    pub(in crate::ui::shell) shared_profile_monitoring: HashMap<String, SessionMonitoringState>,
    pub(in crate::ui::shell) monitor_source_tabs: HashMap<String, usize>,
    pub(in crate::ui::shell) active_topbar_tab: Option<usize>,
    pub(in crate::ui::shell) topbar_tab_scroll_handle: ScrollHandle,
    pub(in crate::ui::shell) session_monitor_scroll_handle: ScrollHandle,
    pub(in crate::ui::shell) session_agent_scroll_handle: ScrollHandle,
    pub(in crate::ui::shell) session_agent_history_scroll_handle: ScrollHandle,
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
    pub(in crate::ui::shell) hosts_to_terminal_transition: Option<HostsToTerminalTransition>,
    pub(in crate::ui::shell) terminal_view_transition: Option<TerminalViewTransition>,
    pub(in crate::ui::shell) visible_terminal_view_tab_id: Option<usize>,
    pub(in crate::ui::shell) session_agent_panel_width: f32,
    pub(in crate::ui::shell) session_agent_panel_drag: Option<SessionAgentPanelDragState>,
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
pub(in crate::ui::shell) struct SessionAgentAutoScrollState {
    pub(in crate::ui::shell) anchor_y: f32,
    pub(in crate::ui::shell) pointer_y: f32,
    pub(in crate::ui::shell) generation: u64,
}

#[derive(Default)]
pub(in crate::ui::shell) struct ShellState {
    pub(in crate::ui::shell) suppressed_page_container_animation_section: Option<SidebarSection>,
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
}
