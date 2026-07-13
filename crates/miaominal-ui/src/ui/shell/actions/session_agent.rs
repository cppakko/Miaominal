use super::super::*;
use crate::ui::i18n;
use crate::ui::shell::session_agent_stream_batch::{
    SESSION_AGENT_STREAM_UI_FLUSH_INTERVAL, SessionAgentStreamBatch,
    session_agent_event_is_finished, session_agent_event_requires_immediate_flush,
};
use crate::ui::shell::session_agent_view::SessionAgentConversationView;
use crate::ui::shell::state::{SessionAgentConversationViewport, SessionAgentExecutionContext};
use gpui_component::WindowExt as _;
use miaominal_agent::{
    AgentChatEvent, AgentChatMessage, AgentChatProvider, AgentChatProviderKind, AgentChatRequest,
    AgentChatRole, AgentChatToolEvent, AgentError, AgentExecChannel, AgentMode, AgentResult,
    AgentToolCallRequest, AgentToolCallResponse, AgentToolCancellation,
    AgentToolResultContinuationRequest, AgentToolSet, BackendRoute,
    TERMINAL_INTERRUPT_SETTLE_TIMEOUT, TerminalExecHandle, TerminalExecLeaseState,
    TerminalOutputTap, ToolOutput,
};
use miaominal_secrets::SecretKind;
use miaominal_settings::{AiProviderConfig, AiProviderKind};
use miaominal_storage::chat_store::{ChatMessageRecord, ChatMessageRole};
use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::watch;

const SESSION_AGENT_CONTEXT_MAX_MESSAGES: usize = 40;
const SESSION_AGENT_CONTEXT_MAX_CHARS: usize = 80_000;
const SESSION_AGENT_PRODUCER_STOP_ACK_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(2);

type SessionAgentPtyTap = (usize, TerminalOutputTap);

#[derive(Clone)]
struct SessionAgentPtyInterrupt {
    handle: TerminalExecHandle,
}

impl SessionAgentPtyInterrupt {
    fn cancel(&self) {
        if let Err(error) = self.handle.cancel() {
            log::debug!("failed to cancel stopped agent PTY command: {error:?}");
        }
    }

    async fn cancel_and_wait(&self) -> TerminalExecLeaseState {
        match self
            .handle
            .cancel_and_wait(TERMINAL_INTERRUPT_SETTLE_TIMEOUT)
            .await
        {
            Ok(state) => state,
            Err(error) => {
                log::debug!("failed to settle stopped agent PTY command: {error:?}");
                self.handle.lease_state()
            }
        }
    }
}

#[derive(Clone)]
struct SessionAgentPtyContext {
    tap: SessionAgentPtyTap,
    interrupt: SessionAgentPtyInterrupt,
}

#[derive(Clone, Default)]
struct SessionAgentPtyInterrupts {
    active: Option<SessionAgentPtyInterrupt>,
    targets: HashMap<String, SessionAgentPtyInterrupt>,
}

impl SessionAgentPtyInterrupts {
    fn cancel_commands(&self) {
        if let Some(interrupt) = self.active.as_ref() {
            interrupt.cancel();
        }
        for interrupt in self.targets.values() {
            interrupt.cancel();
        }
    }

    async fn cancel_commands_and_wait(&self) -> bool {
        let states = futures::future::join_all(
            self.active
                .iter()
                .chain(self.targets.values())
                .map(SessionAgentPtyInterrupt::cancel_and_wait),
        )
        .await;
        states.into_iter().all(TerminalExecLeaseState::is_settled)
    }
}

type SessionAgentTools = Option<(Option<AgentToolSet>, Option<SessionAgentPtyContext>)>;

struct SessionAgentReceivedBatch {
    events: Vec<AgentChatEvent>,
    error: Option<AgentError>,
    finished: bool,
    stream_closed: bool,
    stopped: bool,
    producer_stop_ack_timed_out: bool,
}

enum SessionAgentApprovedToolOutcome {
    Finished(anyhow::Result<String>),
    Stopped,
}

fn approved_tool_was_stopped(outcome: &SessionAgentApprovedToolOutcome) -> bool {
    matches!(outcome, SessionAgentApprovedToolOutcome::Stopped)
}

fn approved_tool_should_stop_after_finished(
    outcome: &SessionAgentApprovedToolOutcome,
    stop: &watch::Receiver<bool>,
) -> bool {
    *stop.borrow() && matches!(outcome, SessionAgentApprovedToolOutcome::Finished(_))
}

fn session_agent_event_releases_pty_lease(event: &AgentChatEvent) -> bool {
    matches!(
        event,
        AgentChatEvent::ToolCallAutoExecuteRequired { .. }
            | AgentChatEvent::ToolCallApprovalRequired { .. }
            | AgentChatEvent::ToolCallUserInputRequired { .. }
    )
}

async fn wait_for_session_agent_stop(stop: &mut watch::Receiver<bool>) {
    loop {
        if *stop.borrow_and_update() {
            return;
        }
        if stop.changed().await.is_err() {
            // Dropping the sender is part of every normal Finished/error/tool handoff cleanup.
            // Only an explicit `true` value represents a user stop request.
            std::future::pending::<()>().await;
        }
    }
}

async fn wait_for_session_agent_tool_or_stop<T>(
    tool: impl std::future::Future<Output = T>,
    stop: &mut watch::Receiver<bool>,
) -> Option<T> {
    tokio::select! {
        biased;
        _ = wait_for_session_agent_stop(stop) => None,
        result = tool => Some(result),
    }
}

async fn abort_and_wait_for_session_agent_tool_worker<T>(handle: &mut tokio::task::JoinHandle<T>) {
    // `abort` prevents a queued spawn_blocking job from starting, but cannot stop one that is
    // already running. Await it as well so its stop-aware runtime has dropped the tool future and
    // terminal guard before the UI releases this request's PTY lease to another session.
    handle.abort();
    let _ = handle.await;
}

async fn receive_session_agent_event_batch(
    receiver: &mut tokio::sync::mpsc::Receiver<AgentResult<AgentChatEvent>>,
    stop: &mut watch::Receiver<bool>,
    wait_for_producer_close: bool,
    background_executor: &gpui::BackgroundExecutor,
) -> SessionAgentReceivedBatch {
    receive_session_agent_event_batch_with_deadlines(
        receiver,
        stop,
        wait_for_producer_close,
        || background_executor.timer(SESSION_AGENT_STREAM_UI_FLUSH_INTERVAL),
        || background_executor.timer(SESSION_AGENT_PRODUCER_STOP_ACK_TIMEOUT),
    )
    .await
}

#[cfg(test)]
async fn receive_session_agent_event_batch_with_deadline<Deadline>(
    receiver: &mut tokio::sync::mpsc::Receiver<AgentResult<AgentChatEvent>>,
    stop: &mut watch::Receiver<bool>,
    deadline: impl FnOnce() -> Deadline,
) -> SessionAgentReceivedBatch
where
    Deadline: std::future::Future<Output = ()>,
{
    receive_session_agent_event_batch_with_deadlines(
        receiver,
        stop,
        false,
        deadline,
        std::future::pending::<()>,
    )
    .await
}

async fn receive_session_agent_event_batch_with_deadlines<Deadline, StopAckDeadline>(
    receiver: &mut tokio::sync::mpsc::Receiver<AgentResult<AgentChatEvent>>,
    stop: &mut watch::Receiver<bool>,
    wait_for_producer_close: bool,
    deadline: impl FnOnce() -> Deadline,
    stop_ack_deadline: impl FnOnce() -> StopAckDeadline,
) -> SessionAgentReceivedBatch
where
    Deadline: std::future::Future<Output = ()>,
    StopAckDeadline: std::future::Future<Output = ()>,
{
    let mut stop_ack_deadline = Some(stop_ack_deadline);
    let mut batch = SessionAgentStreamBatch::new();
    let mut error = None;
    let mut finished = false;
    let mut stream_closed = false;
    let mut stopped = false;
    let mut producer_stop_ack_timed_out = false;
    let first = tokio::select! {
        biased;
        _ = wait_for_session_agent_stop(stop) => {
            stopped = true;
            if wait_for_producer_close {
                let deadline = stop_ack_deadline
                    .take()
                    .expect("producer stop deadline should only be created once")();
                tokio::pin!(deadline);
                producer_stop_ack_timed_out = !drain_session_agent_events_until_producer_close(
                    receiver,
                    &mut batch,
                    &mut error,
                    &mut finished,
                    &mut stream_closed,
                    deadline.as_mut(),
                ).await;
            } else {
                drain_ready_session_agent_events_before_stop(
                    receiver,
                    &mut batch,
                    &mut error,
                    &mut finished,
                    &mut stream_closed,
                );
            }
            if finished {
                stopped = false;
            }
            return SessionAgentReceivedBatch {
                events: batch.take(),
                error,
                finished,
                stream_closed,
                stopped,
                producer_stop_ack_timed_out,
            };
        }
        first = receiver.recv() => first,
    };
    let Some(first) = first else {
        return SessionAgentReceivedBatch {
            events: Vec::new(),
            error: None,
            finished: false,
            stream_closed: true,
            stopped: false,
            producer_stop_ack_timed_out: false,
        };
    };
    let immediate =
        collect_session_agent_received_item(first, &mut batch, &mut error, &mut finished);

    if error.is_none() && !immediate {
        let deadline = deadline();
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                biased;
                _ = wait_for_session_agent_stop(stop) => {
                    stopped = true;
                    if wait_for_producer_close {
                        let deadline = stop_ack_deadline
                            .take()
                            .expect("producer stop deadline should only be created once")();
                        tokio::pin!(deadline);
                        producer_stop_ack_timed_out = !drain_session_agent_events_until_producer_close(
                            receiver,
                            &mut batch,
                            &mut error,
                            &mut finished,
                            &mut stream_closed,
                            deadline.as_mut(),
                        ).await;
                    } else {
                        drain_ready_session_agent_events_before_stop(
                            receiver,
                            &mut batch,
                            &mut error,
                            &mut finished,
                            &mut stream_closed,
                        );
                    }
                    if finished {
                        stopped = false;
                    }
                    break;
                }
                _ = &mut deadline => break,
                next = receiver.recv() => {
                    let Some(next) = next else {
                        stream_closed = true;
                        break;
                    };
                    if collect_session_agent_received_item(
                        next,
                        &mut batch,
                        &mut error,
                        &mut finished,
                    ) {
                        break;
                    }
                }
            }
        }
    }

    SessionAgentReceivedBatch {
        events: batch.take(),
        error,
        finished,
        stream_closed,
        stopped,
        producer_stop_ack_timed_out,
    }
}

fn collect_session_agent_received_item(
    item: AgentResult<AgentChatEvent>,
    batch: &mut SessionAgentStreamBatch,
    error: &mut Option<AgentError>,
    finished: &mut bool,
) -> bool {
    match item {
        Ok(event) => {
            let immediate = session_agent_event_requires_immediate_flush(&event);
            *finished |= session_agent_event_is_finished(&event);
            batch.push(event);
            immediate
        }
        Err(stream_error) => {
            *error = Some(stream_error);
            true
        }
    }
}

fn collect_session_agent_stopped_item(
    item: AgentResult<AgentChatEvent>,
    batch: &mut SessionAgentStreamBatch,
    error: &mut Option<AgentError>,
    finished: &mut bool,
) {
    match item {
        Ok(
            AgentChatEvent::ToolCallAutoExecuteRequired { .. }
            | AgentChatEvent::ToolCallApprovalRequired { .. }
            | AgentChatEvent::ToolCallUserInputRequired { .. },
        ) => {
            // Applying a handoff after Stop could start a new side effect.
        }
        Ok(event) => {
            *finished |= session_agent_event_is_finished(&event);
            batch.push(event);
        }
        Err(stream_error) => {
            if error.is_none() {
                *error = Some(stream_error);
            }
        }
    }
}

fn drain_ready_session_agent_events_before_stop(
    receiver: &mut tokio::sync::mpsc::Receiver<AgentResult<AgentChatEvent>>,
    batch: &mut SessionAgentStreamBatch,
    error: &mut Option<AgentError>,
    finished: &mut bool,
    stream_closed: &mut bool,
) {
    let ready_count = receiver.len();
    for _ in 0..ready_count {
        let item = match receiver.try_recv() {
            Ok(item) => item,
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                *stream_closed = true;
                break;
            }
        };
        collect_session_agent_stopped_item(item, batch, error, finished);
    }
    if receiver.is_closed() && receiver.is_empty() {
        *stream_closed = true;
    }
}

async fn drain_session_agent_events_until_producer_close<StopAckDeadline>(
    receiver: &mut tokio::sync::mpsc::Receiver<AgentResult<AgentChatEvent>>,
    batch: &mut SessionAgentStreamBatch,
    error: &mut Option<AgentError>,
    finished: &mut bool,
    stream_closed: &mut bool,
    mut stop_ack_deadline: std::pin::Pin<&mut StopAckDeadline>,
) -> bool
where
    StopAckDeadline: std::future::Future<Output = ()>,
{
    loop {
        tokio::select! {
            biased;
            _ = stop_ack_deadline.as_mut() => {
                // Preserve everything that was already published before the timeout fallback.
                drain_ready_session_agent_events_before_stop(
                    receiver,
                    batch,
                    error,
                    finished,
                    stream_closed,
                );
                return false;
            }
            next = receiver.recv() => {
                let Some(item) = next else {
                    *stream_closed = true;
                    return true;
                };
                collect_session_agent_stopped_item(item, batch, error, finished);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum PromptHistoryDirection {
    Previous,
    Next,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SessionAgentBackgroundNotificationKind {
    ToolApprovalRequired { tool_name: String },
    UserInputRequired { tool_name: String },
    ReplyReady,
    StreamFailed { error: String },
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
        let content = content_with_text_attachments(&message.content, &message.attachments);
        let images: Vec<miaominal_core::chat_attachment::ChatImage> = message
            .attachments
            .iter()
            .filter_map(|attachment| attachment.as_image().cloned())
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
fn append_text_attachments_to_content(
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

fn content_with_text_attachments(
    content: &str,
    attachments: &[miaominal_core::chat_attachment::ChatAttachment],
) -> String {
    let mut content = content.to_string();
    append_text_attachments_to_content(&mut content, attachments);
    content
}

fn agent_provider_supports_vision(kind: AgentChatProviderKind) -> bool {
    matches!(
        kind,
        AgentChatProviderKind::OpenAi
            | AgentChatProviderKind::Anthropic
            | AgentChatProviderKind::Gemini
            | AgentChatProviderKind::OpenRouter
            | AgentChatProviderKind::Xai
    )
}

fn session_agent_history_cost(message: &AgentChatMessage) -> usize {
    message.content.len()
        + message
            .images
            .iter()
            .map(|image| image.data_base64.len())
            .sum::<usize>()
}

fn build_session_agent_history(messages: &[SessionAgentMessage]) -> Vec<AgentChatMessage> {
    let mut selected = Vec::new();
    let mut total_chars = 0usize;

    for message in messages.iter().rev().filter(|message| {
        matches!(
            message.role,
            SessionAgentMessageRole::User | SessionAgentMessageRole::Assistant
        )
    }) {
        if selected.len() >= SESSION_AGENT_CONTEXT_MAX_MESSAGES {
            break;
        }

        let chat_message = AgentChatMessage::from(message);
        if chat_message.content.trim().is_empty() && chat_message.images.is_empty() {
            continue;
        }

        let cost = session_agent_history_cost(&chat_message);
        if !selected.is_empty()
            && total_chars.saturating_add(cost) > SESSION_AGENT_CONTEXT_MAX_CHARS
        {
            break;
        }

        total_chars = total_chars.saturating_add(cost);
        selected.push(chat_message);
    }

    selected.reverse();
    selected
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

    pub(in crate::ui::shell) fn ensure_session_agent_conversation_view(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Entity<SessionAgentConversationView> {
        let search_layout_active = self
            .session_agent
            .search_query
            .as_ref()
            .is_some_and(|query| !query.trim().is_empty());
        let view = if let Some(view) = self.session_agent.conversation_view.as_ref() {
            view.clone()
        } else {
            let messages = self.session_agent.messages.clone();
            let generating = self.session_agent.has_pending_task();
            let viewport = self.session_agent.conversation_viewport.take();
            let view = cx.new(move |cx| {
                SessionAgentConversationView::from_messages(messages, generating, cx)
            });
            if let Some(viewport) = viewport
                && !viewport.following_tail
            {
                let offset = viewport.offset_for_search_layout(search_layout_active);
                view.read(cx)
                    .scroll_to(offset.item_ix, offset.offset_in_item);
            }
            self.session_agent.conversation_view = Some(view.clone());
            view
        };

        if self.session_agent.conversation_view_observation.is_none() {
            self.session_agent.conversation_view_observation =
                Some(cx.observe(&view, |this, observed_view, cx| {
                    if !this
                        .workspace_state
                        .session_agent_background_projection_active
                        && this.session_agent.conversation_view.as_ref().is_some_and(
                            |active_view| active_view.entity_id() == observed_view.entity_id(),
                        )
                    {
                        cx.notify();
                    }
                }));
        }
        let generating_label = i18n::string("workspace.panel.agent.thinking");
        view.update(cx, |view, cx| {
            view.set_generating_label(generating_label, cx);
        });
        if self.panels.session_agent_panel_open
            && self.active_terminal_session_index().is_some()
            && self.session_agent.panel_view == ChatPanelView::Conversation
            && self
                .workspace_state
                .session_agent_text_drag_conversation
                .is_none()
            && let Some((message_index, _)) = self.session_agent.search_scroll_target
        {
            // Replay search's first-phase virtual-list positioning after a hidden panel is
            // reopened or a text drag ends. The block prepaint callback performs the precise
            // second-phase offset and clears the pending target.
            view.read(cx).scroll_to(message_index, px(0.0));
        }
        view
    }

    fn push_session_agent_message_view(
        &mut self,
        message: SessionAgentMessage,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.session_agent.conversation_view.as_ref().cloned() {
            view.update(cx, |view, cx| {
                view.push_message(message, cx);
            });
        }
    }

    fn sync_session_agent_message_view(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(message) = self.session_agent.messages.get(index).cloned() else {
            return;
        };
        if let Some(view) = self.session_agent.conversation_view.as_ref().cloned() {
            view.update(cx, |view, cx| {
                view.set_message_snapshot(index, message, cx);
            });
        }
    }

    fn append_session_agent_message_view_delta(
        &mut self,
        index: usize,
        delta: &str,
        cx: &mut Context<Self>,
    ) {
        if delta.is_empty() {
            return;
        }
        if let Some(view) = self.session_agent.conversation_view.as_ref().cloned() {
            view.update(cx, |view, cx| {
                view.append_to_message(index, delta, cx);
            });
        }
    }

    fn clear_session_agent_conversation_view(&mut self, cx: &mut Context<Self>) {
        self.finish_session_agent_text_drag(cx);
        if let Some(view) = self.session_agent.conversation_view.as_ref().cloned() {
            view.update(cx, |view, cx| view.clear(cx));
        }
    }

    /// Releases the expensive message/Markdown projection while retaining the authoritative
    /// session state for background streaming and a lightweight viewport anchor for restoration.
    pub(in crate::ui::shell) fn release_session_agent_conversation_view(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        self.finish_session_agent_text_drag(cx);
        if let Some(view) = self.session_agent.conversation_view.as_ref() {
            let view = view.read(cx);
            let motion_keys = view.enter_motion_keys_for_rebuild(cx);
            for (message, motion_key) in self.session_agent.messages.iter_mut().zip(motion_keys) {
                message.motion.enter_key = motion_key;
            }
            let search_layout_active = self
                .session_agent
                .search_query
                .as_ref()
                .is_some_and(|query| !query.trim().is_empty());
            self.session_agent.conversation_viewport = Some(SessionAgentConversationViewport {
                offset: view.list_state().logical_scroll_top(),
                following_tail: view.is_following_tail(),
                search_layout_active,
            });
        }
        self.session_agent.conversation_view_observation = None;
        self.session_agent.conversation_view = None;
    }

    fn sync_session_agent_generating_view(&mut self, cx: &mut Context<Self>) {
        let generating = self.session_agent.has_pending_task();
        if let Some(view) = self.session_agent.conversation_view.as_ref().cloned() {
            view.update(cx, |view, cx| view.set_generating(generating, cx));
        }
    }

    fn install_session_agent_pending_task(
        &mut self,
        task: gpui::Task<()>,
        stop: watch::Sender<bool>,
        agent_cancellation: Option<AgentToolCancellation>,
        cx: &mut Context<Self>,
    ) {
        self.session_agent.pending_stream_stop = Some(stop);
        self.session_agent.pending_agent_cancellation = agent_cancellation;
        self.session_agent.pending_task = Some(task);
        self.sync_session_agent_generating_view(cx);
    }

    fn take_session_agent_pending_task(&mut self, cx: &mut Context<Self>) -> bool {
        self.session_agent.pending_stream_stop = None;
        self.session_agent.pending_agent_cancellation = None;
        let had_pending_task = self.session_agent.pending_task.take().is_some();
        self.sync_session_agent_generating_view(cx);
        had_pending_task
    }

    fn request_session_agent_stream_stop(&self) -> bool {
        let Some(stop) = self.session_agent.pending_stream_stop.as_ref() else {
            return false;
        };
        if let Some(cancellation) = self.session_agent.pending_agent_cancellation.as_ref() {
            cancellation.cancel();
        }
        stop.send(true).is_ok()
    }

    fn session_agent_active_thinking_index(&self) -> Option<usize> {
        self.session_agent
            .messages
            .len()
            .checked_sub(1)
            .filter(|&index| {
                self.session_agent.messages[index].role == SessionAgentMessageRole::Thinking
                    && self.session_agent.messages[index]
                        .thinking
                        .as_ref()
                        .is_some_and(|thinking| thinking.elapsed_ms.is_none())
            })
    }

    fn session_agent_tool_message_index(&self, tool_id: &str) -> Option<usize> {
        self.session_agent.messages.iter().rposition(|message| {
            message
                .tool_call
                .as_ref()
                .is_some_and(|tool_call| tool_call.id == tool_id)
        })
    }

    fn push_session_agent_message_views_from(&mut self, start: usize, cx: &mut Context<Self>) {
        let start = start.min(self.session_agent.messages.len());
        let messages = self.session_agent.messages[start..].to_vec();
        for (offset, message) in messages.into_iter().enumerate() {
            let index = start + offset;
            self.push_session_agent_message_view(message, cx);
            self.refresh_conversation_search_message(index, cx);
        }
    }

    fn start_session_agent_reply(&mut self, cx: &mut Context<Self>) {
        let previous_message_count = self.session_agent.messages.len();
        let thinking_index = self.session_agent_active_thinking_index();
        self.session_agent.start_assistant_reply();
        if let Some(index) = thinking_index {
            self.sync_session_agent_message_view(index, cx);
        }
        self.push_session_agent_message_views_from(previous_message_count, cx);
    }

    fn push_session_agent_message(&mut self, message: SessionAgentMessage, cx: &mut Context<Self>) {
        let previous_message_count = self.session_agent.messages.len();
        self.session_agent.push_message_with_enter_motion(message);
        self.push_session_agent_message_views_from(previous_message_count, cx);
    }

    pub(in crate::ui::shell) fn reset_session_agent_chat(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_conversation_search_state(cx);
        self.clear_session_agent_conversation_view(cx);
        self.session_agent.messages.clear();
        self.session_agent.conversation_view = None;
        self.session_agent.conversation_view_observation = None;
        self.session_agent.conversation_viewport = None;
        self.session_agent.session_id = None;
        self.session_agent.last_error = None;
        self.session_agent.active_request_id = self.session_agent.active_request_id.wrapping_add(1);
        self.session_agent.pending_stream_stop = None;
        self.session_agent.pending_agent_cancellation = None;
        self.session_agent.pending_task = None;
        self.session_agent.selected_at_targets.clear();
        self.session_agent.active_at_targets.clear();
        self.session_agent.active_exec_context = None;
        self.session_agent.title = None;
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
        self.finish_session_agent_text_drag(cx);
        self.release_session_agent_conversation_view(cx);
        self.clear_conversation_search_state(cx);
        self.stash_current_session_agent(cx);
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
        self.workspace_forms.agent.editing_title = false;
        cx.notify();
    }

    pub(in crate::ui::shell) fn start_session_agent_conversation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_session_agent_text_drag(cx);
        self.release_session_agent_conversation_view(cx);
        // Clear any active search state
        self.clear_conversation_search_state(cx);
        let chat_search = &mut self.workspace_forms.chat_search;
        chat_search.conversation_search_open = false;
        chat_search.conversation_search_visible = false;
        chat_search.conversation_search_visibility = 0.0;
        chat_search.conversation_search_animation = None;
        chat_search.match_count = 0;
        chat_search.current_match = None;
        chat_search.status = None;

        self.stash_current_session_agent(cx);
        self.reset_session_agent_chat(window, cx);
        self.session_agent.panel_view = ChatPanelView::Conversation;
        cx.notify();
    }

    pub(in crate::ui::shell) fn load_session_agent_chat(
        &mut self,
        session_id: String,
        cx: &mut Context<Self>,
    ) {
        self.finish_session_agent_text_drag(cx);
        self.release_session_agent_conversation_view(cx);
        // Clear any active search state when loading a session
        self.clear_conversation_search_state(cx);
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
            cx.notify();
            return;
        }

        self.stash_current_session_agent(cx);
        if let Some(mut state) = self.session_agent_sessions.remove(&session_id) {
            state.panel_view = ChatPanelView::Conversation;
            self.session_agent = state;
            self.clear_conversation_search_state(cx);
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
        self.session_agent.active_exec_context = None;
        self.session_agent.panel_view = ChatPanelView::Conversation;
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
            self.clear_session_agent_conversation_view(cx);
            self.session_agent.session_id = None;
            self.session_agent.messages.clear();
            self.session_agent.conversation_view = None;
            self.session_agent.conversation_view_observation = None;
            self.session_agent.conversation_viewport = None;
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

    fn stash_current_session_agent(&mut self, cx: &mut Context<Self>) {
        let Some(session_id) = self.session_agent.session_id.clone() else {
            return;
        };
        if self.session_agent.messages.is_empty() && !self.session_agent.is_busy() {
            return;
        }
        self.release_session_agent_conversation_view(cx);
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

    pub(in crate::ui::shell) fn with_session_agent_state(
        &mut self,
        session_id: &str,
        f: impl FnOnce(&mut Self),
    ) -> bool {
        if self.session_agent.session_id.as_deref() == Some(session_id) {
            f(self);
            return true;
        }

        let Some(mut target) = self.session_agent_sessions.remove(session_id) else {
            return false;
        };
        let foreground_status = self.status_message.clone();
        let previous_projection_state = self
            .workspace_state
            .session_agent_background_projection_active;
        self.workspace_state
            .session_agent_background_projection_active = true;
        std::mem::swap(&mut self.session_agent, &mut target);
        f(self);
        std::mem::swap(&mut self.session_agent, &mut target);
        self.workspace_state
            .session_agent_background_projection_active = previous_projection_state;
        self.status_message = foreground_status;
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

        if let Err(error) = chat_service.replace_session_messages(&session_id, &records) {
            log::warn!("failed to persist chat messages: {error:?}");
            return;
        }

        if let Some(title) = self.session_agent.title.as_deref()
            && let Err(error) = chat_service.update_session_title(&session_id, title)
        {
            log::warn!("failed to persist chat title: {error:?}");
        }
        self.refresh_chat_sessions();
    }

    fn capture_session_agent_execution_context(&self) -> Option<SessionAgentExecutionContext> {
        match self.session_agent.exec_mode {
            AgentExecMode::ExecChannel => {
                self.active_profile()
                    .map(|profile| SessionAgentExecutionContext {
                        profile_id: profile.id.clone(),
                        exec_mode: AgentExecMode::ExecChannel,
                        terminal_tab_id: None,
                    })
            }
            AgentExecMode::Pty => {
                let index = self.active_terminal_session_index()?;
                let tab = self.workspace_state.tabs.get(index)?;
                let session = tab.as_session()?;
                Some(SessionAgentExecutionContext {
                    profile_id: session.profile_id.clone(),
                    exec_mode: AgentExecMode::Pty,
                    terminal_tab_id: Some(tab.id),
                })
            }
        }
    }

    fn session_agent_profile_for_context(
        &self,
        context: &SessionAgentExecutionContext,
    ) -> Option<SessionProfile> {
        self.data
            .sessions
            .iter()
            .find(|profile| profile.id == context.profile_id)
            .cloned()
    }

    fn session_agent_terminal_target_marker_for_context(
        &self,
        context: &SessionAgentExecutionContext,
    ) -> Option<String> {
        (context.exec_mode == AgentExecMode::Pty)
            .then_some(context.terminal_tab_id)
            .flatten()
            .and_then(|tab_id| {
                self.workspace_state
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id)
            })
            .and_then(|tab| {
                tab.as_session()
                    .filter(|session| session.purpose == SessionPurpose::Terminal)
                    .map(|_| format!("@{}", tab.title))
            })
    }

    fn session_agent_pty_commands_for_context(
        &self,
        context: &SessionAgentExecutionContext,
    ) -> Result<Option<(usize, miaominal_ssh::SessionCommandSender)>, String> {
        if context.exec_mode != AgentExecMode::Pty {
            return Ok(None);
        }

        let Some(tab_id) = context.terminal_tab_id else {
            return Err(i18n::string(
                "workspace.panel.agent.messages.pty_requires_active_session",
            ));
        };
        let Some(session) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_session)
            .filter(|session| session.purpose == SessionPurpose::Terminal)
        else {
            return Err(i18n::string(
                "workspace.panel.agent.messages.pty_requires_active_session",
            ));
        };
        let Some(commands) = session.commands.clone() else {
            return Err(i18n::string(
                "workspace.panel.agent.messages.pty_requires_connected_session",
            ));
        };

        Ok(Some((tab_id, commands)))
    }

    fn set_session_agent_execution_context_error(
        &mut self,
        message: String,
        cx: &mut Context<Self>,
    ) {
        self.session_agent.last_error = Some(message.clone());
        self.status_message = message;
        cx.notify();
    }

    fn fail_session_agent_tool_start(
        &mut self,
        tool_id: &str,
        message: String,
        cx: &mut Context<Self>,
    ) {
        self.session_agent.fail_tool_call(tool_id, message.clone());
        if let Some(index) = self.session_agent_tool_message_index(tool_id) {
            self.sync_session_agent_message_view(index, cx);
        }
        self.session_agent.last_error = Some(message.clone());
        self.status_message = message;
        self.persist_session_agent_chat();
        cx.notify();
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
        let has_pending_attachments = !self.session_agent.pending_attachments.is_empty();
        if prompt.is_empty() && !has_pending_attachments {
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

        let exec_context = self.capture_session_agent_execution_context();
        self.session_agent.active_exec_context = exec_context;

        let target_names = self.session_agent.selected_at_targets.clone();
        let mentions = self.resolve_session_agent_mentions(&target_names);
        if mentions.pty_busy {
            self.session_agent.active_exec_context = None;
            self.set_session_agent_execution_context_error(
                i18n::string("workspace.panel.agent.messages.pty_terminal_busy"),
                cx,
            );
            return;
        }
        if !mentions.unresolved.is_empty() {
            self.clear_session_pty_taps_if_same(&mentions.pty_taps);
            self.session_agent.active_exec_context = None;
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

        let pty_target_taps = mentions.pty_taps.clone();
        let pty_target_interrupts = mentions.pty_interrupts.clone();
        let Some((tools, active_pty_context)) =
            self.build_session_agent_tools(mentions.aux_channels, cx)
        else {
            self.clear_session_pty_taps_if_same(&pty_target_taps);
            self.session_agent.active_exec_context = None;
            return;
        };
        let active_pty_tap = active_pty_context
            .as_ref()
            .map(|context| context.tap.clone());
        let pty_interrupts = SessionAgentPtyInterrupts {
            active: active_pty_context.map(|context| context.interrupt),
            targets: pty_target_interrupts,
        };
        let target_guidance = mentions.guidance;
        self.session_agent.panel_view = ChatPanelView::Conversation;
        self.session_agent.active_at_targets = target_names.clone();
        let prompt_for_message = if prompt.is_empty() && has_pending_attachments {
            i18n::string("workspace.panel.agent.attachment_only_prompt")
        } else {
            prompt.clone()
        };
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
        let model_prompt = format!("{target_prefix}{prompt_for_message}");
        let stream_session_id = self.ensure_session_agent_session();
        let request_id = self.session_agent.next_request_id();
        let attachments = std::mem::take(&mut self.session_agent.pending_attachments);
        let prompt_images: Vec<miaominal_core::chat_attachment::ChatImage> = attachments
            .iter()
            .filter_map(|attachment| attachment.as_image().cloned())
            .collect();
        let llm_prompt = content_with_text_attachments(&model_prompt, &attachments);
        let images_as_text_fallback =
            !prompt_images.is_empty() && !agent_provider_supports_vision(provider.kind);
        let history = build_session_agent_history(&self.session_agent.messages);

        self.push_session_agent_message(
            SessionAgentMessage::user_with_attachments(model_prompt.clone(), attachments),
            cx,
        );
        if !prompt.is_empty() {
            self.record_session_agent_prompt_history(&prompt);
        }
        self.persist_session_agent_chat();
        self.session_agent.active_request_id = request_id;
        self.session_agent.last_error = None;
        self.status_message = if images_as_text_fallback {
            i18n::string("workspace.panel.agent.messages.image_attachments_text_fallback")
        } else {
            i18n::string("workspace.panel.agent.send_pending")
        };
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
        let agent_cancellation = tools.as_ref().map(AgentToolSet::cancellation);
        let wait_for_producer_close = agent_cancellation.is_some();
        let (stream_stop, mut stream_stop_receiver) = watch::channel(false);
        let task = cx.spawn(async move |this, cx| {
            let stream_session_id_for_error = stream_session_id.clone();
            let mut stream_task = runtime.spawn(async move {
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
            });
            let stream_result = tokio::select! {
                biased;
                _ = wait_for_session_agent_stop(&mut stream_stop_receiver) => {
                    pty_interrupts.cancel_commands();
                    stream_task.abort();
                    let stopped_session_id = stream_session_id.clone();
                    let cleanup_active_pty_tap = active_pty_tap.clone();
                    let cleanup_pty_target_taps = pty_target_taps.clone();
                    let stop_pty_interrupts = pty_interrupts.clone();
                    let _ = this.update(cx, move |this, cx| {
                        this.finalize_session_agent_stopped_for_session(
                            &stopped_session_id,
                            request_id,
                            Some(&stop_pty_interrupts),
                            cx,
                        );
                        if let Some(tap) = cleanup_active_pty_tap.as_ref() {
                            this.clear_session_pty_taps_if_same(std::slice::from_ref(tap));
                        }
                        this.clear_session_pty_taps_if_same(&cleanup_pty_target_taps);
                    });
                    return;
                }
                result = &mut stream_task => result.unwrap_or_else(|error| {
                    Err(anyhow::anyhow!(
                        "session agent stream task cancelled: {error}"
                    ))
                }),
            };

            let mut receiver = match stream_result {
                Ok(receiver) => receiver,
                Err(error) => {
                    pty_interrupts.cancel_commands();
                    let cleanup_active_pty_tap = active_pty_tap.clone();
                    let cleanup_pty_target_taps = pty_target_taps.clone();
                    let _ = this.update(cx, move |this, cx| {
                        this.handle_session_agent_stream_error_for_session(
                            &stream_session_id_for_error,
                            request_id,
                            error,
                            cx,
                        );
                        if let Some(tap) = cleanup_active_pty_tap.as_ref() {
                            this.clear_session_pty_taps_if_same(std::slice::from_ref(tap));
                        }
                        this.clear_session_pty_taps_if_same(&cleanup_pty_target_taps);
                    });
                    return;
                }
            };

            loop {
                let received = receive_session_agent_event_batch(
                    &mut receiver,
                    &mut stream_stop_receiver,
                    wait_for_producer_close,
                    cx.background_executor(),
                )
                .await;
                if !received.events.is_empty() {
                    let releases_pty_lease = received
                        .events
                        .iter()
                        .any(session_agent_event_releases_pty_lease);
                    if releases_pty_lease {
                        pty_interrupts.cancel_commands();
                    }
                    let release_active_pty_tap =
                        releases_pty_lease.then(|| active_pty_tap.clone()).flatten();
                    let release_pty_target_taps = releases_pty_lease
                        .then(|| pty_target_taps.clone())
                        .unwrap_or_default();
                    let event_session_id = stream_session_id.clone();
                    if let Err(error) = this.update(cx, move |this, cx| {
                        if let Some(tap) = release_active_pty_tap.as_ref() {
                            this.clear_session_pty_taps_if_same(std::slice::from_ref(tap));
                        }
                        this.clear_session_pty_taps_if_same(&release_pty_target_taps);
                        this.apply_session_agent_events_for_session(
                            &event_session_id,
                            request_id,
                            received.events,
                            cx,
                        );
                    }) {
                        log::debug!("failed to apply session agent chat events: {error:?}");
                        break;
                    }
                }

                if received.stopped {
                    // Wake the producer's `sender.closed()` branch before doing UI cleanup so an
                    // in-flight auto-approved rig tool is dropped immediately.
                    drop(receiver);
                    pty_interrupts.cancel_commands();
                    let stopped_session_id = stream_session_id.clone();
                    let cleanup_active_pty_tap = active_pty_tap.clone();
                    let cleanup_pty_target_taps = pty_target_taps.clone();
                    let stop_pty_interrupts = pty_interrupts.clone();
                    let producer_stop_ack_timed_out = received.producer_stop_ack_timed_out;
                    let _ = this.update(cx, move |this, cx| {
                        if producer_stop_ack_timed_out {
                            this.mark_session_agent_tools_unconfirmed_for_session(
                                &stopped_session_id,
                                request_id,
                                cx,
                            );
                        }
                        this.finalize_session_agent_stopped_for_session(
                            &stopped_session_id,
                            request_id,
                            Some(&stop_pty_interrupts),
                            cx,
                        );
                        if let Some(tap) = cleanup_active_pty_tap.as_ref() {
                            this.clear_session_pty_taps_if_same(std::slice::from_ref(tap));
                        }
                        this.clear_session_pty_taps_if_same(&cleanup_pty_target_taps);
                    });
                    return;
                }

                if let Some(error) = received.error {
                    pty_interrupts.cancel_commands();
                    let error_session_id = stream_session_id.clone();
                    let cleanup_active_pty_tap = active_pty_tap.clone();
                    let cleanup_pty_target_taps = pty_target_taps.clone();
                    let _ = this
                        .update(cx, move |this, cx| {
                            this.handle_session_agent_stream_error_for_session(
                                &error_session_id,
                                request_id,
                                anyhow::Error::from(error),
                                cx,
                            );
                            if let Some(tap) = cleanup_active_pty_tap.as_ref() {
                                this.clear_session_pty_taps_if_same(std::slice::from_ref(tap));
                            }
                            this.clear_session_pty_taps_if_same(&cleanup_pty_target_taps);
                        })
                        .map_err(|error| {
                            log::debug!("failed to apply session agent chat error: {error:?}");
                        });
                    return;
                }

                if received.finished || received.stream_closed {
                    break;
                }
            }

            pty_interrupts.cancel_commands();
            let finish_session_id = stream_session_id.clone();
            let cleanup_active_pty_tap = active_pty_tap.clone();
            let cleanup_pty_target_taps = pty_target_taps.clone();
            let _ = this.update(cx, move |this, cx| {
                this.finish_session_agent_stream_for_session(&finish_session_id, request_id, cx);
                if let Some(tap) = cleanup_active_pty_tap.as_ref() {
                    this.clear_session_pty_taps_if_same(std::slice::from_ref(tap));
                }
                this.clear_session_pty_taps_if_same(&cleanup_pty_target_taps);
            });
        });
        self.install_session_agent_pending_task(task, stream_stop, agent_cancellation, cx);
        cx.notify();
    }

    fn build_session_agent_tools(
        &mut self,
        mut aux_channels: HashMap<String, AgentExecChannel>,
        cx: &mut Context<Self>,
    ) -> SessionAgentTools {
        let Some(context) = self.session_agent.active_exec_context.clone() else {
            return Some((None, None));
        };
        let active_target_marker = self.session_agent_terminal_target_marker_for_context(&context);

        let Some(profile) = self.session_agent_profile_for_context(&context) else {
            let message =
                i18n::string("workspace.panel.agent.messages.no_active_session_for_approval");
            self.set_session_agent_execution_context_error(message, cx);
            return None;
        };

        let pty_commands = match self.session_agent_pty_commands_for_context(&context) {
            Ok(commands) => commands,
            Err(message) => {
                self.set_session_agent_execution_context_error(message, cx);
                return None;
            }
        };

        let mut active_pty_tap = None;
        let mut channel = self.agent_exec_channel_for_profile(profile);
        if let Some((tab_id, command_sender)) = pty_commands {
            let (sender, receiver) = TerminalOutputTap::channel();
            let terminal_exec = TerminalExecHandle::new(command_sender, receiver);
            if !self.try_set_session_pty_tap_by_tab_id(tab_id, sender.clone()) {
                self.set_session_agent_execution_context_error(
                    i18n::string("workspace.panel.agent.messages.pty_terminal_busy"),
                    cx,
                );
                return None;
            }
            active_pty_tap = Some(SessionAgentPtyContext {
                tap: (tab_id, sender),
                interrupt: SessionAgentPtyInterrupt {
                    handle: terminal_exec.clone(),
                },
            });
            channel = channel.with_terminal_exec(terminal_exec);
        }
        if let Some(marker) = active_target_marker {
            aux_channels
                .entry(marker)
                .or_insert_with(|| channel.clone());
        }
        channel = channel.with_aux_channels(aux_channels);
        let mode = self.session_agent.agent_mode;
        let tools = Some(AgentToolSet::for_channel(channel, mode));

        Some((tools, active_pty_tap))
    }

    pub(in crate::ui::shell) fn stop_session_agent_stream(&mut self, cx: &mut Context<Self>) {
        if self.session_agent.has_pending_task() && self.request_session_agent_stream_stop() {
            // Keep the task alive long enough to apply any deltas it has already removed from the
            // provider receiver. The task will call `finalize_session_agent_stopped` immediately
            // after that batch is visible.
            cx.notify();
            return;
        }

        self.finalize_session_agent_stopped(None, None, cx);
    }

    fn finalize_session_agent_stopped(
        &mut self,
        expected_request_id: Option<u64>,
        pty_interrupts: Option<&SessionAgentPtyInterrupts>,
        cx: &mut Context<Self>,
    ) -> bool {
        if expected_request_id
            .is_some_and(|request_id| self.session_agent.active_request_id != request_id)
        {
            return false;
        }

        if let Some(pty_interrupts) = pty_interrupts {
            pty_interrupts.cancel_commands();
        }

        let previous_message_count = self.session_agent.messages.len();
        let thinking_index = self.session_agent_active_thinking_index();
        let active_tool_indices: Vec<usize> = self
            .session_agent
            .messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| {
                message.tool_call.as_ref().and_then(|tool_call| {
                    matches!(
                        tool_call.status,
                        SessionAgentToolStatus::Pending
                            | SessionAgentToolStatus::WaitingForConfirmation
                            | SessionAgentToolStatus::InProgress
                    )
                    .then_some(index)
                })
            })
            .collect();
        let had_pending_task = self.take_session_agent_pending_task(cx);
        let had_active_tool = self.session_agent.reject_active_tool_calls(&i18n::string(
            "workspace.panel.agent.messages.stopped_by_user",
        ));
        if !had_pending_task && !had_active_tool {
            return false;
        }

        self.session_agent.active_request_id = self.session_agent.active_request_id.wrapping_add(1);
        self.session_agent.finish_stopped_turn();
        if let Some(index) = thinking_index {
            self.sync_session_agent_message_view(index, cx);
        }
        for index in active_tool_indices {
            self.sync_session_agent_message_view(index, cx);
        }
        if self.session_agent.messages.len() > previous_message_count {
            self.push_session_agent_message_views_from(previous_message_count, cx);
        } else if let Some(index) = self.session_agent.messages.len().checked_sub(1)
            && self.session_agent.messages[index].role == SessionAgentMessageRole::Assistant
        {
            self.sync_session_agent_message_view(index, cx);
            self.refresh_conversation_search_message(index, cx);
        }
        self.session_agent.active_exec_context = None;
        self.status_message = i18n::string("workspace.panel.agent.messages.stopped");
        self.session_agent.last_error = None;
        self.persist_session_agent_chat();
        cx.notify();
        true
    }

    fn finalize_session_agent_stopped_for_session(
        &mut self,
        session_id: &str,
        request_id: u64,
        pty_interrupts: Option<&SessionAgentPtyInterrupts>,
        cx: &mut Context<Self>,
    ) {
        let is_loaded_session = self.session_agent.session_id.as_deref() == Some(session_id);
        let mut stopped = false;
        let updated = self.with_session_agent_state(session_id, |this| {
            stopped = this.finalize_session_agent_stopped(Some(request_id), pty_interrupts, cx);
        });
        if updated && stopped && !is_loaded_session {
            self.refresh_chat_sessions();
            cx.notify();
        }
    }

    fn mark_session_agent_tools_unconfirmed_for_session(
        &mut self,
        session_id: &str,
        request_id: u64,
        cx: &mut Context<Self>,
    ) {
        let message = i18n::string("workspace.panel.agent.messages.tool_stop_unconfirmed");
        self.with_session_agent_state(session_id, |this| {
            if this.session_agent.active_request_id != request_id {
                return;
            }
            let active_tool_indices = this
                .session_agent
                .messages
                .iter()
                .enumerate()
                .filter_map(|(index, message)| {
                    message.tool_call.as_ref().and_then(|tool_call| {
                        matches!(
                            tool_call.status,
                            SessionAgentToolStatus::Pending
                                | SessionAgentToolStatus::WaitingForConfirmation
                                | SessionAgentToolStatus::InProgress
                        )
                        .then_some(index)
                    })
                })
                .collect::<Vec<_>>();
            if this.session_agent.fail_active_tool_calls(&message) {
                for index in active_tool_indices {
                    this.sync_session_agent_message_view(index, cx);
                }
            }
        });
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
        let Some(context) = self.session_agent.active_exec_context.clone() else {
            self.fail_session_agent_tool_start(
                &tool_id,
                i18n::string("workspace.panel.agent.messages.no_active_session_for_approval"),
                cx,
            );
            return;
        };
        let active_target_marker = self.session_agent_terminal_target_marker_for_context(&context);
        let Some(profile) = self.session_agent_profile_for_context(&context) else {
            self.fail_session_agent_tool_start(
                &tool_id,
                i18n::string("workspace.panel.agent.messages.no_active_session_for_approval"),
                cx,
            );
            return;
        };

        let arguments = parse_tool_arguments(&tool_call.arguments);
        let reasoning = self.session_agent.reasoning_before_tool_call(&tool_id);
        self.session_agent.approve_tool_call(&tool_id);
        if let Some(index) = self.session_agent_tool_message_index(&tool_id) {
            self.sync_session_agent_message_view(index, cx);
        }
        self.status_message = i18n::string("workspace.panel.agent.messages.tool_approved_running");
        let approval_session_id = self.ensure_session_agent_session();

        let pty_commands = match self.session_agent_pty_commands_for_context(&context) {
            Ok(commands) => commands,
            Err(message) => {
                self.fail_session_agent_tool_start(&tool_id, message, cx);
                return;
            }
        };
        let mut active_pty_context = None;
        let pty_handle = if let Some((tab_id, command_sender)) = pty_commands {
            let (sender, receiver) = TerminalOutputTap::channel();
            let terminal_exec = TerminalExecHandle::new(command_sender, receiver);
            if !self.try_set_session_pty_tap_by_tab_id(tab_id, sender.clone()) {
                self.fail_session_agent_tool_start(
                    &tool_id,
                    i18n::string("workspace.panel.agent.messages.pty_terminal_busy"),
                    cx,
                );
                return;
            }
            active_pty_context = Some(SessionAgentPtyContext {
                tap: (tab_id, sender),
                interrupt: SessionAgentPtyInterrupt {
                    handle: terminal_exec.clone(),
                },
            });
            Some(terminal_exec)
        } else {
            None
        };
        let approval_mentions = self.resolve_mentions_from_tool_arguments(&arguments);
        if approval_mentions.pty_busy {
            if let Some(context) = active_pty_context.as_ref() {
                self.clear_session_pty_taps_if_same(std::slice::from_ref(&context.tap));
            }
            self.fail_session_agent_tool_start(
                &tool_id,
                i18n::string("workspace.panel.agent.messages.pty_terminal_busy"),
                cx,
            );
            return;
        }
        let approval_pty_target_taps = approval_mentions.pty_taps.clone();
        let active_pty_tap = active_pty_context
            .as_ref()
            .map(|context| context.tap.clone());
        let pty_interrupts = SessionAgentPtyInterrupts {
            active: active_pty_context.map(|context| context.interrupt),
            targets: approval_mentions.pty_interrupts.clone(),
        };
        let sessions = self.data.sessions.clone();
        let agent_service = self.services.agent_service.clone();
        let secrets = self.services.secrets.clone();
        let known_hosts = self.services.known_hosts.clone();
        let web_search_config = self.settings_store.settings().web_search.clone();
        let skip_policy = matches!(
            self.session_agent.agent_mode,
            AgentMode::NonBlocking | AgentMode::FullAuto
        );
        let tool_name = tool_call.name.clone();
        let tool_arguments = tool_call.arguments.clone();
        let execution_request_id = self.session_agent.next_request_id();
        self.session_agent.active_request_id = execution_request_id;
        let (tool_stop, mut tool_stop_receiver) = watch::channel(false);
        let mut worker_stop_receiver = tool_stop.subscribe();
        let task = cx.spawn(async move |this, cx| {
            let worker_tool_name = tool_name.clone();
            let mut handle = miaominal_agent::agent_runtime().spawn_blocking(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        return SessionAgentApprovedToolOutcome::Finished(Err(anyhow::anyhow!(
                            error
                        )));
                    }
                };
                runtime.block_on(async move {
                    let tool = async move {
                        let mut channel = agent_service.channel_for_profile_snapshot_with_stores(
                            profile,
                            &sessions,
                            secrets.clone(),
                            known_hosts,
                        );
                        if web_search_config.enabled {
                            let web_search_api_key =
                                secrets.get("web_search", SecretKind::WebSearchApiKey)?;
                            channel = channel
                                .with_web_search_config(web_search_config, web_search_api_key);
                        }
                        if let Some(ref pty_handle) = pty_handle {
                            channel = channel.with_terminal_exec(pty_handle.clone());
                        }
                        let mut aux_channels = approval_mentions.aux_channels;
                        if let Some(marker) = active_target_marker {
                            aux_channels
                                .entry(marker)
                                .or_insert_with(|| channel.clone());
                        }
                        channel = channel.with_aux_channels(aux_channels);
                        channel
                            .call_tool(AgentToolCallRequest {
                                tool_name: worker_tool_name,
                                arguments,
                                approved: true,
                                route: None,
                                skip_policy,
                            })
                            .await
                            .map_err(anyhow::Error::from)
                            .and_then(|response| {
                                serde_json::to_string(&response)
                                    .map_err(|error| anyhow::anyhow!(error))
                            })
                    };

                    match wait_for_session_agent_tool_or_stop(tool, &mut worker_stop_receiver).await
                    {
                        Some(result) => SessionAgentApprovedToolOutcome::Finished(result),
                        None => SessionAgentApprovedToolOutcome::Stopped,
                    }
                })
            });

            let outcome = tokio::select! {
                biased;
                result = &mut handle => match result {
                    Ok(outcome) => outcome,
                    Err(error) => SessionAgentApprovedToolOutcome::Finished(Err(anyhow::anyhow!(
                        "agent tool task failed: {error}"
                    ))),
                },
                _ = wait_for_session_agent_stop(&mut tool_stop_receiver) => {
                    pty_interrupts.cancel_commands();
                    abort_and_wait_for_session_agent_tool_worker(&mut handle).await;
                    let _ = pty_interrupts.cancel_commands_and_wait().await;
                    SessionAgentApprovedToolOutcome::Stopped
                }
            };

            let _ = this.update(cx, move |this, cx| {
                if approved_tool_was_stopped(&outcome) {
                    pty_interrupts.cancel_commands();
                    this.finalize_session_agent_stopped_for_session(
                        &approval_session_id,
                        execution_request_id,
                        Some(&pty_interrupts),
                        cx,
                    );
                    if let Some(tap) = active_pty_tap.as_ref() {
                        this.clear_session_pty_taps_if_same(std::slice::from_ref(tap));
                    }
                    this.clear_session_pty_taps_if_same(&approval_pty_target_taps);
                    return;
                }

                let stop_after_finished =
                    approved_tool_should_stop_after_finished(&outcome, &tool_stop_receiver);
                pty_interrupts.cancel_commands();
                if let Some(tap) = active_pty_tap.as_ref() {
                    this.clear_session_pty_taps_if_same(std::slice::from_ref(tap));
                }
                this.clear_session_pty_taps_if_same(&approval_pty_target_taps);
                let state_session_id = approval_session_id.clone();
                let mut finished_was_committed = false;
                this.with_session_agent_state(&state_session_id, |this| {
                    if this.session_agent.active_request_id != execution_request_id {
                        return;
                    }
                    let SessionAgentApprovedToolOutcome::Finished(result) = outcome else {
                        unreachable!("stopped tool execution returned past stop handling");
                    };
                    if !matches!(
                        this.session_agent
                            .tool_call(&tool_id)
                            .map(|tool_call| tool_call.status),
                        Some(SessionAgentToolStatus::InProgress)
                    ) {
                        this.take_session_agent_pending_task(cx);
                        this.session_agent.active_request_id = 0;
                        this.status_message =
                            i18n::string("workspace.panel.agent.messages.stopped");
                        cx.notify();
                        return;
                    }
                    let (tool_result, failed) = match result {
                        Ok(result) => {
                            this.session_agent
                                .complete_tool_call(&tool_id, result.clone());
                            if let Some(index) = this.session_agent_tool_message_index(&tool_id) {
                                this.sync_session_agent_message_view(index, cx);
                            }
                            this.status_message = i18n::string(
                                "workspace.panel.agent.messages.tool_finished_continuing",
                            );
                            (result, false)
                        }
                        Err(error) => {
                            let result = format!("tool failed after approval: {error}");
                            this.session_agent.fail_tool_call(&tool_id, result.clone());
                            if let Some(index) = this.session_agent_tool_message_index(&tool_id) {
                                this.sync_session_agent_message_view(index, cx);
                            }
                            this.status_message = i18n::string(
                                "workspace.panel.agent.messages.tool_failed_continuing",
                            );
                            (result, true)
                        }
                    };
                    finished_was_committed = true;
                    if stop_after_finished {
                        return;
                    }
                    this.take_session_agent_pending_task(cx);
                    this.session_agent.active_request_id = 0;
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
                if stop_after_finished && finished_was_committed {
                    this.finalize_session_agent_stopped_for_session(
                        &approval_session_id,
                        execution_request_id,
                        Some(&pty_interrupts),
                        cx,
                    );
                }
                cx.notify();
            });
        });
        self.install_session_agent_pending_task(task, tool_stop, None, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn submit_active_session_agent_user_answer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = self
            .workspace_forms
            .agent
            .ask_user_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let Some(tool_call) = self.session_agent.active_ask_user_tool_call() else {
            self.status_message = i18n::string("workspace.panel.agent.messages.tool_not_found");
            cx.notify();
            return;
        };
        self.submit_session_agent_user_answer(tool_call.id, answer, None, true, window, cx);
    }

    pub(in crate::ui::shell) fn submit_session_agent_user_answer(
        &mut self,
        tool_id: String,
        answer: String,
        selected_index: Option<usize>,
        custom: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = answer.trim().to_string();
        if answer.is_empty() {
            self.status_message = i18n::string("workspace.panel.agent.messages.answer_required");
            cx.notify();
            return;
        }

        let Some(tool_call) = self.session_agent.tool_call(&tool_id) else {
            self.status_message = i18n::string("workspace.panel.agent.messages.tool_not_found");
            cx.notify();
            return;
        };
        if tool_call.name != "ask_user"
            || tool_call.status != SessionAgentToolStatus::WaitingForConfirmation
        {
            self.status_message = i18n::string("workspace.panel.agent.messages.tool_not_found");
            cx.notify();
            return;
        }

        let arguments = parse_tool_arguments(&tool_call.arguments);
        let operation_hash = arguments
            .get("operation_hash")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let reasoning = self.session_agent.reasoning_before_tool_call(&tool_id);
        let tool_name = tool_call.name.clone();
        let tool_arguments = tool_call.arguments.clone();
        let response = AgentToolCallResponse {
            tool_name: tool_name.clone(),
            route: BackendRoute::SshExec,
            output: ToolOutput::UserResponse {
                answer,
                selected_index,
                custom,
                operation_hash,
            },
        };
        let tool_result = match serde_json::to_string(&response) {
            Ok(result) => result,
            Err(error) => {
                self.status_message = error.to_string();
                cx.notify();
                return;
            }
        };

        self.session_agent
            .complete_tool_call(&tool_id, tool_result.clone());
        if let Some(index) = self.session_agent_tool_message_index(&tool_id) {
            self.sync_session_agent_message_view(index, cx);
        }
        set_input_value(
            &self.workspace_forms.agent.ask_user_input,
            String::new(),
            window,
            cx,
        );
        self.status_message = i18n::string("workspace.panel.agent.messages.user_answer_sent");
        self.continue_session_agent_after_tool_result(
            AgentChatToolEvent {
                id: tool_id,
                name: tool_name,
                arguments: tool_arguments,
            },
            reasoning,
            tool_result,
            false,
            cx,
        );
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
        if mentions.pty_busy {
            self.set_session_agent_execution_context_error(
                i18n::string("workspace.panel.agent.messages.pty_terminal_busy"),
                cx,
            );
            return;
        }
        let pty_target_taps = mentions.pty_taps.clone();
        let pty_target_interrupts = mentions.pty_interrupts.clone();
        let target_guidance = mentions.guidance;
        let Some((tools, active_pty_context)) =
            self.build_session_agent_tools(mentions.aux_channels, cx)
        else {
            self.clear_session_pty_taps_if_same(&pty_target_taps);
            return;
        };
        let active_pty_tap = active_pty_context
            .as_ref()
            .map(|context| context.tap.clone());
        let pty_interrupts = SessionAgentPtyInterrupts {
            active: active_pty_context.map(|context| context.interrupt),
            targets: pty_target_interrupts,
        };
        let stream_session_id = self.ensure_session_agent_session();
        let request_id = self.session_agent.next_request_id();
        let history = build_session_agent_history(&self.session_agent.messages);

        self.session_agent.active_request_id = request_id;
        self.session_agent.last_error = None;
        self.start_session_agent_reply(cx);
        self.status_message = i18n::string("workspace.panel.agent.thinking");

        let runtime = self.services.runtime.clone();
        let agent_cancellation = tools.as_ref().map(AgentToolSet::cancellation);
        let wait_for_producer_close = agent_cancellation.is_some();
        let (stream_stop, mut stream_stop_receiver) = watch::channel(false);
        let task = cx.spawn(async move |this, cx| {
            let stream_session_id_for_error = stream_session_id.clone();
            let mut stream_task = runtime.spawn(async move {
                let result = if failed {
                    format!("ERROR: {result}")
                } else {
                    result
                };
                miaominal_agent::stream_chat_after_tool_result(AgentToolResultContinuationRequest {
                    provider,
                    messages: history,
                    tool_call,
                    reasoning,
                    result,
                    tools,
                    target_guidance,
                })
                .await
                .map_err(anyhow::Error::from)
            });
            let stream_result = tokio::select! {
                biased;
                _ = wait_for_session_agent_stop(&mut stream_stop_receiver) => {
                    pty_interrupts.cancel_commands();
                    stream_task.abort();
                    let stopped_session_id = stream_session_id.clone();
                    let cleanup_active_pty_tap = active_pty_tap.clone();
                    let cleanup_pty_target_taps = pty_target_taps.clone();
                    let stop_pty_interrupts = pty_interrupts.clone();
                    let _ = this.update(cx, move |this, cx| {
                        this.finalize_session_agent_stopped_for_session(
                            &stopped_session_id,
                            request_id,
                            Some(&stop_pty_interrupts),
                            cx,
                        );
                        if let Some(tap) = cleanup_active_pty_tap.as_ref() {
                            this.clear_session_pty_taps_if_same(std::slice::from_ref(tap));
                        }
                        this.clear_session_pty_taps_if_same(&cleanup_pty_target_taps);
                    });
                    return;
                }
                result = &mut stream_task => result.unwrap_or_else(|error| {
                    Err(anyhow::anyhow!(
                        "session agent continuation task cancelled: {error}"
                    ))
                }),
            };

            let mut receiver = match stream_result {
                Ok(receiver) => receiver,
                Err(error) => {
                    pty_interrupts.cancel_commands();
                    let cleanup_active_pty_tap = active_pty_tap.clone();
                    let cleanup_pty_target_taps = pty_target_taps.clone();
                    let _ = this.update(cx, move |this, cx| {
                        this.handle_session_agent_stream_error_for_session(
                            &stream_session_id_for_error,
                            request_id,
                            error,
                            cx,
                        );
                        if let Some(tap) = cleanup_active_pty_tap.as_ref() {
                            this.clear_session_pty_taps_if_same(std::slice::from_ref(tap));
                        }
                        this.clear_session_pty_taps_if_same(&cleanup_pty_target_taps);
                    });
                    return;
                }
            };

            loop {
                let received = receive_session_agent_event_batch(
                    &mut receiver,
                    &mut stream_stop_receiver,
                    wait_for_producer_close,
                    cx.background_executor(),
                )
                .await;
                if !received.events.is_empty() {
                    let releases_pty_lease = received
                        .events
                        .iter()
                        .any(session_agent_event_releases_pty_lease);
                    if releases_pty_lease {
                        pty_interrupts.cancel_commands();
                    }
                    let release_active_pty_tap =
                        releases_pty_lease.then(|| active_pty_tap.clone()).flatten();
                    let release_pty_target_taps = releases_pty_lease
                        .then(|| pty_target_taps.clone())
                        .unwrap_or_default();
                    let event_session_id = stream_session_id.clone();
                    if let Err(error) = this.update(cx, move |this, cx| {
                        if let Some(tap) = release_active_pty_tap.as_ref() {
                            this.clear_session_pty_taps_if_same(std::slice::from_ref(tap));
                        }
                        this.clear_session_pty_taps_if_same(&release_pty_target_taps);
                        this.apply_session_agent_events_for_session(
                            &event_session_id,
                            request_id,
                            received.events,
                            cx,
                        );
                    }) {
                        log::debug!("failed to apply session agent continuation events: {error:?}");
                        break;
                    }
                }

                if received.stopped {
                    drop(receiver);
                    pty_interrupts.cancel_commands();
                    let stopped_session_id = stream_session_id.clone();
                    let cleanup_active_pty_tap = active_pty_tap.clone();
                    let cleanup_pty_target_taps = pty_target_taps.clone();
                    let stop_pty_interrupts = pty_interrupts.clone();
                    let producer_stop_ack_timed_out = received.producer_stop_ack_timed_out;
                    let _ = this.update(cx, move |this, cx| {
                        if producer_stop_ack_timed_out {
                            this.mark_session_agent_tools_unconfirmed_for_session(
                                &stopped_session_id,
                                request_id,
                                cx,
                            );
                        }
                        this.finalize_session_agent_stopped_for_session(
                            &stopped_session_id,
                            request_id,
                            Some(&stop_pty_interrupts),
                            cx,
                        );
                        if let Some(tap) = cleanup_active_pty_tap.as_ref() {
                            this.clear_session_pty_taps_if_same(std::slice::from_ref(tap));
                        }
                        this.clear_session_pty_taps_if_same(&cleanup_pty_target_taps);
                    });
                    return;
                }

                if let Some(error) = received.error {
                    pty_interrupts.cancel_commands();
                    let error_session_id = stream_session_id.clone();
                    let cleanup_active_pty_tap = active_pty_tap.clone();
                    let cleanup_pty_target_taps = pty_target_taps.clone();
                    let _ = this
                        .update(cx, move |this, cx| {
                            this.handle_session_agent_stream_error_for_session(
                                &error_session_id,
                                request_id,
                                anyhow::Error::from(error),
                                cx,
                            );
                            if let Some(tap) = cleanup_active_pty_tap.as_ref() {
                                this.clear_session_pty_taps_if_same(std::slice::from_ref(tap));
                            }
                            this.clear_session_pty_taps_if_same(&cleanup_pty_target_taps);
                        })
                        .map_err(|error| {
                            log::debug!(
                                "failed to apply session agent continuation error: {error:?}"
                            );
                        });
                    return;
                }

                if received.finished || received.stream_closed {
                    break;
                }
            }

            pty_interrupts.cancel_commands();
            let finish_session_id = stream_session_id.clone();
            let cleanup_active_pty_tap = active_pty_tap.clone();
            let cleanup_pty_target_taps = pty_target_taps.clone();
            let _ = this.update(cx, move |this, cx| {
                this.finish_session_agent_stream_for_session(&finish_session_id, request_id, cx);
                if let Some(tap) = cleanup_active_pty_tap.as_ref() {
                    this.clear_session_pty_taps_if_same(std::slice::from_ref(tap));
                }
                this.clear_session_pty_taps_if_same(&cleanup_pty_target_taps);
            });
        });
        self.install_session_agent_pending_task(task, stream_stop, agent_cancellation, cx);
    }

    pub(in crate::ui::shell) fn deny_session_agent_tool_call(
        &mut self,
        tool_id: String,
        cx: &mut Context<Self>,
    ) {
        self.session_agent.reject_tool_call(&tool_id);
        if let Some(index) = self.session_agent_tool_message_index(&tool_id) {
            self.sync_session_agent_message_view(index, cx);
        }
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
        let notification = match kind {
            SessionAgentBackgroundNotificationKind::ToolApprovalRequired { tool_name } => {
                let title = i18n::string("workspace.panel.agent.notifications.tool_approval_title");
                let message = i18n::string_args(
                    "workspace.panel.agent.notifications.tool_approval",
                    &[("chat", &chat_label), ("tool", &tool_name)],
                );
                Self::warning_notification(title, message)
            }
            SessionAgentBackgroundNotificationKind::UserInputRequired { tool_name } => {
                let title = i18n::string("workspace.panel.agent.notifications.user_input_title");
                let message = i18n::string_args(
                    "workspace.panel.agent.notifications.user_input",
                    &[("chat", &chat_label), ("tool", &tool_name)],
                );
                Self::warning_notification(title, message)
            }
            SessionAgentBackgroundNotificationKind::ReplyReady => {
                let title = i18n::string("workspace.panel.agent.notifications.reply_ready_title");
                let message = i18n::string_args(
                    "workspace.panel.agent.notifications.reply_ready",
                    &[("chat", &chat_label)],
                );
                Self::success_notification(title, message)
            }
            SessionAgentBackgroundNotificationKind::StreamFailed { error } => {
                let title = i18n::string("workspace.panel.agent.notifications.stream_failed_title");
                let message = i18n::string_args(
                    "workspace.panel.agent.notifications.stream_failed",
                    &[("chat", &chat_label), ("error", &error)],
                );
                Self::error_notification(title, message)
            }
        };

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

        let notification_kind = match event {
            AgentChatEvent::TextDelta(delta) => {
                let previous_message_count = self.session_agent.messages.len();
                let thinking_index = self.session_agent_active_thinking_index();
                self.session_agent.append_assistant_delta(&delta);
                if let Some(index) = thinking_index {
                    self.sync_session_agent_message_view(index, cx);
                }
                if self.session_agent.messages.len() > previous_message_count {
                    self.push_session_agent_message_views_from(previous_message_count, cx);
                } else if !delta.is_empty()
                    && self
                        .session_agent
                        .messages
                        .last()
                        .is_some_and(|message| message.role == SessionAgentMessageRole::Assistant)
                {
                    let index = self.session_agent.messages.len().saturating_sub(1);
                    self.append_session_agent_message_view_delta(index, &delta, cx);
                }
                if !delta.is_empty()
                    && let Some(index) = self.session_agent.messages.len().checked_sub(1)
                    && self.session_agent.messages[index].role == SessionAgentMessageRole::Assistant
                {
                    self.schedule_conversation_search_message_refresh(index, cx);
                }
                self.session_agent.last_error = None;
                None
            }
            AgentChatEvent::ThinkingDelta(delta) => {
                let previous_message_count = self.session_agent.messages.len();
                let changed = !delta.trim().is_empty();
                self.session_agent.append_thinking_delta(&delta);
                if self.session_agent.messages.len() > previous_message_count {
                    self.push_session_agent_message_views_from(previous_message_count, cx);
                } else if changed {
                    let index = self.session_agent.messages.len().saturating_sub(1);
                    self.append_session_agent_message_view_delta(index, &delta, cx);
                }
                self.status_message = i18n::string("workspace.panel.agent.thinking");
                None
            }
            AgentChatEvent::ToolCallStarted(tool) => {
                let previous_message_count = self.session_agent.messages.len();
                let thinking_index = self.session_agent_active_thinking_index();
                self.session_agent.push_tool_call(
                    tool.id,
                    tool.name,
                    tool.arguments,
                    SessionAgentToolStatus::InProgress,
                );
                if let Some(index) = thinking_index {
                    self.sync_session_agent_message_view(index, cx);
                }
                self.push_session_agent_message_views_from(previous_message_count, cx);
                None
            }
            AgentChatEvent::ToolCallDelta { id, delta } => {
                let index = self.session_agent_tool_message_index(&id);
                let changed = !delta.trim().is_empty();
                self.session_agent.append_tool_call_delta(&id, delta);
                if changed && let Some(index) = index {
                    self.sync_session_agent_message_view(index, cx);
                }
                None
            }
            AgentChatEvent::ToolCallCompleted { id, result } => {
                let index = self.session_agent_tool_message_index(&id);
                self.session_agent.complete_tool_call(&id, result);
                if let Some(index) = index {
                    self.sync_session_agent_message_view(index, cx);
                }
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
            AgentChatEvent::ToolCallCancelled { id } => {
                let index = self.session_agent_tool_message_index(&id);
                self.session_agent.reject_tool_call_with_message(
                    &id,
                    i18n::string("workspace.panel.agent.messages.stopped_by_user"),
                );
                if let Some(index) = index {
                    self.sync_session_agent_message_view(index, cx);
                }
                None
            }
            AgentChatEvent::ToolCallAutoExecuteRequired { id } => {
                self.take_session_agent_pending_task(cx);
                self.session_agent.active_request_id = 0;
                self.approve_session_agent_tool_call(id, cx);
                None
            }
            AgentChatEvent::ToolCallApprovalRequired { id, message } => {
                if matches!(self.session_agent.agent_mode, AgentMode::FullAuto) {
                    self.take_session_agent_pending_task(cx);
                    self.session_agent.active_request_id = 0;
                    self.approve_session_agent_tool_call(id, cx);
                    None
                } else {
                    let index = self.session_agent_tool_message_index(&id);
                    let tool_name = self
                        .session_agent
                        .tool_call(&id)
                        .map(|tool_call| tool_call.name)
                        .unwrap_or_else(|| i18n::string("workspace.panel.agent.tool"));
                    self.session_agent
                        .require_tool_call_confirmation(&id, message);
                    if let Some(index) = index {
                        self.sync_session_agent_message_view(index, cx);
                    }
                    self.finish_session_agent_stream(request_id, cx);
                    Some(SessionAgentBackgroundNotificationKind::ToolApprovalRequired { tool_name })
                }
            }
            AgentChatEvent::ToolCallUserInputRequired { id, message } => {
                let index = self.session_agent_tool_message_index(&id);
                let tool_name = self
                    .session_agent
                    .tool_call(&id)
                    .map(|tool_call| tool_call.name)
                    .unwrap_or_else(|| i18n::string("workspace.panel.agent.tool"));
                self.session_agent
                    .require_tool_call_confirmation(&id, message);
                if let Some(index) = index {
                    self.sync_session_agent_message_view(index, cx);
                }
                self.finish_session_agent_stream(request_id, cx);
                Some(SessionAgentBackgroundNotificationKind::UserInputRequired { tool_name })
            }
            AgentChatEvent::Finished(reply) => {
                let previous_message_count = self.session_agent.messages.len();
                let thinking_index = self.session_agent_active_thinking_index();
                self.session_agent.finish_assistant_reply(reply);
                if let Some(index) = thinking_index {
                    self.sync_session_agent_message_view(index, cx);
                }
                if self.session_agent.messages.len() > previous_message_count {
                    self.push_session_agent_message_views_from(previous_message_count, cx);
                } else if let Some(index) = self.session_agent.messages.len().checked_sub(1)
                    && self.session_agent.messages[index].role == SessionAgentMessageRole::Assistant
                {
                    self.sync_session_agent_message_view(index, cx);
                }
                if let Some(index) = self.session_agent.messages.len().checked_sub(1)
                    && self.session_agent.messages[index].role == SessionAgentMessageRole::Assistant
                {
                    self.refresh_conversation_search_message(index, cx);
                }
                if self.finish_session_agent_stream(request_id, cx)
                    && !self.session_agent.has_active_tool_call()
                {
                    Some(SessionAgentBackgroundNotificationKind::ReplyReady)
                } else {
                    None
                }
            }
            AgentChatEvent::TokenUsage { .. } => None,
        };

        notification_kind
    }

    fn apply_session_agent_events_for_session(
        &mut self,
        session_id: &str,
        request_id: u64,
        events: Vec<AgentChatEvent>,
        cx: &mut Context<Self>,
    ) {
        if events.is_empty() {
            return;
        }

        let requires_root_notify = events.iter().any(|event| {
            !matches!(
                event,
                AgentChatEvent::TextDelta(_)
                    | AgentChatEvent::ThinkingDelta(_)
                    | AgentChatEvent::ToolCallDelta { .. }
                    | AgentChatEvent::TokenUsage { .. }
            )
        });
        let is_loaded_session = self.session_agent.session_id.as_deref() == Some(session_id);
        let should_background_notify = !self.session_agent_session_is_foreground(session_id);
        let mut notifications = Vec::new();
        let updated = self.with_session_agent_state(session_id, |this| {
            for event in events {
                let kind = this.apply_session_agent_event(request_id, event, cx);
                if should_background_notify && let Some(kind) = kind {
                    notifications.push((this.session_agent_notification_chat_label(), kind));
                }
            }
        });
        if !updated {
            return;
        }

        for (chat_label, kind) in notifications {
            self.notify_background_session_agent(chat_label, kind, cx);
        }

        if !is_loaded_session && requires_root_notify {
            self.refresh_chat_sessions();
        }
        if requires_root_notify {
            cx.notify();
        }
    }

    fn finish_session_agent_stream(&mut self, request_id: u64, cx: &mut Context<Self>) -> bool {
        if self.session_agent.active_request_id != request_id {
            return false;
        }

        let thinking_index = self.session_agent_active_thinking_index();
        self.session_agent.finish_active_thinking();
        if let Some(index) = thinking_index {
            self.sync_session_agent_message_view(index, cx);
        }
        self.take_session_agent_pending_task(cx);
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
                                #[cfg(debug_assertions)]
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
                                    #[cfg(debug_assertions)]
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

        let thinking_index = self.session_agent_active_thinking_index();
        self.session_agent.finish_active_thinking();
        if let Some(index) = thinking_index {
            self.sync_session_agent_message_view(index, cx);
        }
        self.take_session_agent_pending_task(cx);
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
    ) -> Option<String> {
        let message = error.to_string();
        if is_recoverable_session_agent_prompt_error(&message) {
            self.recover_session_agent_prompt_error(request_id, message, cx)
        } else if self.fail_session_agent_stream(request_id, error, cx) {
            Some(message)
        } else {
            None
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
        let should_background_notify = !self.session_agent_session_is_foreground(session_id);
        let foreground_status = should_background_notify.then(|| self.status_message.clone());
        let mut notification = None;
        let updated = self.with_session_agent_state(session_id, |this| {
            if let Some(error) = this.handle_session_agent_stream_error(request_id, error, cx)
                && should_background_notify
            {
                notification = Some((
                    this.session_agent_notification_chat_label(),
                    SessionAgentBackgroundNotificationKind::StreamFailed {
                        error: truncate_with_ellipsis(&error, 160),
                    },
                ));
            }
        });
        if !updated {
            return;
        }

        if let Some(foreground_status) = foreground_status {
            self.status_message = foreground_status;
        }

        if let Some((chat_label, kind)) = notification {
            self.notify_background_session_agent(chat_label, kind, cx);
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
    ) -> Option<String> {
        if self.session_agent.active_request_id != request_id {
            return None;
        }

        let thinking_index = self.session_agent_active_thinking_index();
        self.session_agent.finish_active_thinking();
        if let Some(index) = thinking_index {
            self.sync_session_agent_message_view(index, cx);
        }
        self.take_session_agent_pending_task(cx);
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
            self.status_message = message.clone();
            self.persist_session_agent_chat();
            cx.notify();
            return Some(message);
        };

        let provider = match self.build_session_agent_provider(&provider_id) {
            Ok(provider) => provider,
            Err(error) => {
                let message = error.to_string();
                self.session_agent.last_error = Some(message.clone());
                self.status_message = message.clone();
                self.persist_session_agent_chat();
                cx.notify();
                return Some(message);
            }
        };

        let history = build_session_agent_history(&self.session_agent.messages);
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
        self.start_session_agent_reply(cx);
        self.status_message = i18n::string("workspace.panel.agent.thinking");

        let runtime = self.services.runtime.clone();
        let recovery_session_id = self.ensure_session_agent_session();
        let (stream_stop, mut stream_stop_receiver) = watch::channel(false);
        let task = cx.spawn(async move |this, cx| {
            let recovery_session_id_for_error = recovery_session_id.clone();
            let mut stream_task = runtime.spawn(async move {
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
            });
            let stream_result = tokio::select! {
                biased;
                _ = wait_for_session_agent_stop(&mut stream_stop_receiver) => {
                    stream_task.abort();
                    let stopped_session_id = recovery_session_id.clone();
                    let _ = this.update(cx, move |this, cx| {
                        this.finalize_session_agent_stopped_for_session(
                            &stopped_session_id,
                            request_id,
                            None,
                            cx,
                        );
                    });
                    return;
                }
                result = &mut stream_task => result.unwrap_or_else(|error| {
                    Err(anyhow::anyhow!(
                        "session agent recovery task cancelled: {error}"
                    ))
                }),
            };

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

            loop {
                let received = receive_session_agent_event_batch(
                    &mut receiver,
                    &mut stream_stop_receiver,
                    false,
                    cx.background_executor(),
                )
                .await;
                if !received.events.is_empty() {
                    let event_session_id = recovery_session_id.clone();
                    if let Err(error) = this.update(cx, move |this, cx| {
                        this.apply_session_agent_events_for_session(
                            &event_session_id,
                            request_id,
                            received.events,
                            cx,
                        );
                    }) {
                        log::debug!("failed to apply session agent recovery events: {error:?}");
                        break;
                    }
                }

                if received.stopped {
                    drop(receiver);
                    let stopped_session_id = recovery_session_id.clone();
                    let _ = this.update(cx, move |this, cx| {
                        this.finalize_session_agent_stopped_for_session(
                            &stopped_session_id,
                            request_id,
                            None,
                            cx,
                        );
                    });
                    return;
                }

                if let Some(error) = received.error {
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
                            log::debug!("failed to apply session agent recovery error: {error:?}");
                        });
                    return;
                }

                if received.finished || received.stream_closed {
                    break;
                }
            }

            let finish_session_id = recovery_session_id.clone();
            let _ = this.update(cx, move |this, cx| {
                this.finish_session_agent_stream_for_session(&finish_session_id, request_id, cx);
            });
        });
        self.install_session_agent_pending_task(task, stream_stop, None, cx);
        cx.notify();
        None
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
        let mut channel = self
            .services
            .agent_service
            .channel_for_profile_snapshot_with_stores(
                profile,
                &self.data.sessions,
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
        let mut pending_pty_taps = Vec::new();
        let mut pty_interrupts = HashMap::new();
        let active_terminal_tab_id = self
            .session_agent
            .active_exec_context
            .as_ref()
            .filter(|context| context.exec_mode == AgentExecMode::Pty)
            .and_then(|context| context.terminal_tab_id);

        match self.session_agent.execution_mode_for_running_tools() {
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
                    if active_terminal_tab_id == Some(tab.id) {
                        // The default active channel will own this tab's single output tap. An
                        // explicit `target: "@current-tab"` can safely fall through to that
                        // channel instead of acquiring a second lease for the same request.
                        guidance_lines.push(format!(
                            "- {marker}: terminal session \"{}\" (profile: {}, host: {}, user: {})",
                            tab.title, profile.name, profile.host, profile.username
                        ));
                        resolved_names.push(tab.title.clone());
                        continue;
                    }
                    let (sender, receiver) = TerminalOutputTap::channel();
                    let terminal_exec = TerminalExecHandle::new(command_sender, receiver);
                    let channel = self
                        .agent_exec_channel_for_profile(profile.clone())
                        .with_terminal_exec(terminal_exec.clone());
                    aux_channels.insert(marker.clone(), channel);
                    pending_pty_taps.push((tab.id, sender));
                    pty_interrupts.insert(
                        marker.clone(),
                        SessionAgentPtyInterrupt {
                            handle: terminal_exec,
                        },
                    );
                    guidance_lines.push(format!(
                        "- {marker}: terminal session \"{}\" (profile: {}, host: {}, user: {})",
                        tab.title, profile.name, profile.host, profile.username
                    ));
                    resolved_names.push(tab.title.clone());
                }
            }
        }

        let mut acquired_pty_taps = Vec::new();
        let mut pty_busy = false;
        for (tab_id, sender) in &pending_pty_taps {
            if self.try_set_session_pty_tap_by_tab_id(*tab_id, sender.clone()) {
                acquired_pty_taps.push((*tab_id, sender.clone()));
            } else {
                pty_busy = true;
                break;
            }
        }
        if pty_busy {
            self.clear_session_pty_taps_if_same(&acquired_pty_taps);
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
            pty_taps: (!pty_busy).then_some(pending_pty_taps).unwrap_or_default(),
            pty_interrupts,
            pty_busy,
        }
    }

    fn resolve_mentions_from_tool_arguments(
        &mut self,
        arguments: &Value,
    ) -> ResolvedSessionAgentMentions {
        let Some(target) = arguments.get("target").and_then(Value::as_str) else {
            return ResolvedSessionAgentMentions::default();
        };
        let target = target.trim().trim_start_matches('@').trim().to_string();
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
    pty_taps: Vec<(usize, TerminalOutputTap)>,
    pty_interrupts: HashMap<String, SessionAgentPtyInterrupt>,
    pty_busy: bool,
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

    #[test]
    fn text_attachments_are_embedded_for_model_content() {
        let attachment = miaominal_core::chat_attachment::ChatAttachment {
            id: "att-1".to_string(),
            filename: "main.rs".to_string(),
            mime_type: "text/plain".to_string(),
            size_bytes: 12,
            content: miaominal_core::chat_attachment::ChatAttachmentContent::TextFile(
                miaominal_core::chat_attachment::ChatTextFile {
                    text: "fn main() {}".to_string(),
                    language: Some("rust".to_string()),
                },
            ),
        };

        let content = content_with_text_attachments("review this", &[attachment]);

        assert!(content.contains("[Attached file: main.rs]"));
        assert!(content.contains("```rust"));
        assert!(content.contains("fn main() {}"));
    }

    #[test]
    fn session_agent_history_is_limited_to_recent_context() {
        let messages = (0..50)
            .flat_map(|index| {
                [
                    SessionAgentMessage::user(format!("user {index}")),
                    SessionAgentMessage::assistant_raw(format!("assistant {index}")),
                ]
            })
            .collect::<Vec<_>>();

        let history = build_session_agent_history(&messages);

        assert_eq!(history.len(), SESSION_AGENT_CONTEXT_MAX_MESSAGES);
        assert_eq!(
            history.first().map(|message| message.content.as_str()),
            Some("user 30")
        );
        assert_eq!(
            history.last().map(|message| message.content.as_str()),
            Some("assistant 49")
        );
    }

    #[tokio::test]
    async fn stream_stop_flushes_a_delta_already_collected_for_the_ui_batch() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<AgentResult<AgentChatEvent>>(4);
        sender
            .send(Ok(AgentChatEvent::TextDelta("partial reply".into())))
            .await
            .expect("stream receiver should remain open");
        let (stop_sender, mut stop_receiver) = watch::channel(false);

        let receive = receive_session_agent_event_batch_with_deadline(
            &mut receiver,
            &mut stop_receiver,
            std::future::pending::<()>,
        );
        let request_stop = async move {
            // Let the receiver consume the first delta and enter its pending deadline before the
            // user stop signal arrives.
            tokio::task::yield_now().await;
            stop_sender
                .send(true)
                .expect("stop receiver should remain open");
        };
        let (batch, ()) = tokio::join!(receive, request_stop);

        assert!(batch.stopped);
        assert!(!batch.finished);
        assert!(!batch.stream_closed);
        assert_eq!(
            batch.events,
            vec![AgentChatEvent::TextDelta("partial reply".into())]
        );
    }

    #[tokio::test]
    async fn queued_full_auto_tool_completion_wins_over_stop() {
        let completion = AgentChatEvent::ToolCallCompleted {
            id: "tool-1".into(),
            result: "done".into(),
        };
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<AgentResult<AgentChatEvent>>(1);
        sender
            .send(Ok(completion.clone()))
            .await
            .expect("stream receiver should remain open");
        let (stop_sender, mut stop_receiver) = watch::channel(false);
        stop_sender
            .send(true)
            .expect("stop receiver should remain open");

        let completed = receive_session_agent_event_batch_with_deadline(
            &mut receiver,
            &mut stop_receiver,
            std::future::pending::<()>,
        )
        .await;

        assert_eq!(completed.events, vec![completion]);
        assert!(completed.stopped);
    }

    #[tokio::test]
    async fn stop_bounded_drain_preserves_a_queued_completion_behind_deltas() {
        let completion = AgentChatEvent::ToolCallCompleted {
            id: "tool-1".into(),
            result: "done".into(),
        };
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<AgentResult<AgentChatEvent>>(4);
        sender
            .send(Ok(AgentChatEvent::TextDelta("before".into())))
            .await
            .expect("stream receiver should remain open");
        sender
            .send(Ok(completion.clone()))
            .await
            .expect("stream receiver should remain open");
        let (stop_sender, mut stop_receiver) = watch::channel(false);
        stop_sender
            .send(true)
            .expect("stop receiver should remain open");

        let batch = receive_session_agent_event_batch_with_deadline(
            &mut receiver,
            &mut stop_receiver,
            std::future::pending::<()>,
        )
        .await;

        assert!(batch.stopped);
        assert_eq!(
            batch.events,
            vec![AgentChatEvent::TextDelta("before".into()), completion,]
        );
    }

    #[tokio::test]
    async fn stop_drains_the_entire_fixed_tool_lifecycle_snapshot_in_order() {
        let events = vec![
            AgentChatEvent::ToolCallStarted(AgentChatToolEvent {
                id: "tool-1".into(),
                name: "run_shell".into(),
                arguments: "{}".into(),
            }),
            AgentChatEvent::ToolCallCompleted {
                id: "tool-1".into(),
                result: "first done".into(),
            },
            AgentChatEvent::ToolCallStarted(AgentChatToolEvent {
                id: "tool-2".into(),
                name: "read".into(),
                arguments: "{}".into(),
            }),
            AgentChatEvent::ToolCallCompleted {
                id: "tool-2".into(),
                result: "second done".into(),
            },
        ];
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<AgentResult<AgentChatEvent>>(4);
        for event in events.iter().cloned() {
            sender
                .send(Ok(event))
                .await
                .expect("stream receiver should remain open");
        }
        let (stop_sender, mut stop_receiver) = watch::channel(false);
        stop_sender
            .send(true)
            .expect("stop receiver should remain open");

        let batch = receive_session_agent_event_batch_with_deadline(
            &mut receiver,
            &mut stop_receiver,
            std::future::pending::<()>,
        )
        .await;

        assert_eq!(batch.events, events);
        assert!(batch.stopped);
    }

    #[tokio::test]
    async fn stop_waits_for_the_producer_close_ack_before_returning_completion() {
        let started = AgentChatEvent::ToolCallStarted(AgentChatToolEvent {
            id: "tool-1".into(),
            name: "run_shell".into(),
            arguments: "{}".into(),
        });
        let completion = AgentChatEvent::ToolCallCompleted {
            id: "tool-1".into(),
            result: "done".into(),
        };
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<AgentResult<AgentChatEvent>>(2);
        let (stop_sender, mut stop_receiver) = watch::channel(false);
        stop_sender
            .send(true)
            .expect("stop receiver should remain open");

        let receive = receive_session_agent_event_batch_with_deadlines(
            &mut receiver,
            &mut stop_receiver,
            true,
            std::future::pending::<()>,
            std::future::pending::<()>,
        );
        let delayed_producer = async move {
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
            sender
                .send(Ok(started.clone()))
                .await
                .expect("stop collector should keep the receiver open");
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
            sender
                .send(Ok(completion.clone()))
                .await
                .expect("completion should publish before producer close");
            (started, completion)
        };

        let (batch, (started, completion)) = tokio::join!(receive, delayed_producer);

        assert_eq!(batch.events, vec![started, completion]);
        assert!(batch.stopped);
        assert!(batch.stream_closed);
    }

    #[tokio::test]
    async fn stop_collector_preserves_structured_tool_cancellation() {
        let events = vec![
            AgentChatEvent::ToolCallStarted(AgentChatToolEvent {
                id: "tool-1".into(),
                name: "run_shell".into(),
                arguments: "{}".into(),
            }),
            AgentChatEvent::ToolCallCancelled {
                id: "tool-1".into(),
            },
        ];
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<AgentResult<AgentChatEvent>>(2);
        for event in events.iter().cloned() {
            sender
                .send(Ok(event))
                .await
                .expect("stop collector should remain open");
        }
        drop(sender);
        let (stop_sender, mut stop_receiver) = watch::channel(false);
        stop_sender
            .send(true)
            .expect("stop receiver should remain open");

        let batch = receive_session_agent_event_batch_with_deadlines(
            &mut receiver,
            &mut stop_receiver,
            true,
            std::future::pending::<()>,
            std::future::pending::<()>,
        )
        .await;

        assert_eq!(batch.events, events);
        assert!(batch.stopped);
        assert!(batch.stream_closed);
    }

    #[tokio::test]
    async fn producer_error_during_stop_keeps_user_stop_semantics() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<AgentResult<AgentChatEvent>>(1);
        let (stop_sender, mut stop_receiver) = watch::channel(false);
        stop_sender
            .send(true)
            .expect("stop receiver should remain open");

        let receive = receive_session_agent_event_batch_with_deadlines(
            &mut receiver,
            &mut stop_receiver,
            true,
            std::future::pending::<()>,
            std::future::pending::<()>,
        );
        let fail_producer = async move {
            tokio::task::yield_now().await;
            sender
                .send(Err(AgentError::Backend(anyhow::anyhow!(
                    "tool execution cancelled"
                ))))
                .await
                .expect("stop collector should remain open until producer close");
        };

        let (batch, ()) = tokio::join!(receive, fail_producer);

        assert!(batch.stopped);
        assert!(batch.error.is_some());
        assert!(batch.stream_closed);
    }

    #[tokio::test]
    async fn producer_stop_ack_timeout_is_reported_without_claiming_channel_close() {
        let (_sender, mut receiver) = tokio::sync::mpsc::channel::<AgentResult<AgentChatEvent>>(1);
        let (stop_sender, mut stop_receiver) = watch::channel(false);
        stop_sender
            .send(true)
            .expect("stop receiver should remain open");

        let batch = receive_session_agent_event_batch_with_deadlines(
            &mut receiver,
            &mut stop_receiver,
            true,
            std::future::pending::<()>,
            || std::future::ready(()),
        )
        .await;

        assert!(batch.stopped);
        assert!(batch.producer_stop_ack_timed_out);
        assert!(!batch.stream_closed);
    }

    #[tokio::test]
    async fn stop_discards_a_queued_full_auto_handoff() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<AgentResult<AgentChatEvent>>(1);
        sender
            .send(Ok(AgentChatEvent::ToolCallAutoExecuteRequired {
                id: "tool-1".into(),
            }))
            .await
            .expect("stream receiver should remain open");
        drop(sender);
        let (stop_sender, mut stop_receiver) = watch::channel(false);
        stop_sender
            .send(true)
            .expect("stop receiver should remain open");

        let batch = receive_session_agent_event_batch_with_deadlines(
            &mut receiver,
            &mut stop_receiver,
            true,
            std::future::pending::<()>,
            std::future::pending::<()>,
        )
        .await;

        assert!(batch.events.is_empty());
        assert!(batch.stopped);
        assert!(batch.stream_closed);
    }

    #[tokio::test]
    async fn ready_deadline_is_not_starved_by_a_queued_delta_stream() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<AgentResult<AgentChatEvent>>(4);
        for delta in ["first", "second", "third"] {
            sender
                .send(Ok(AgentChatEvent::TextDelta(delta.into())))
                .await
                .expect("stream receiver should remain open");
        }
        let (_stop_sender, mut stop_receiver) = watch::channel(false);

        let batch = receive_session_agent_event_batch_with_deadline(
            &mut receiver,
            &mut stop_receiver,
            || std::future::ready(()),
        )
        .await;

        assert_eq!(
            batch.events,
            vec![AgentChatEvent::TextDelta("first".into())]
        );
        assert_eq!(receiver.len(), 2);
        assert!(!batch.stopped);
    }

    #[tokio::test]
    async fn dropping_stream_stop_sender_is_not_a_user_stop() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<AgentResult<AgentChatEvent>>(4);
        sender
            .send(Ok(AgentChatEvent::TextDelta("normal reply".into())))
            .await
            .expect("stream receiver should remain open");
        let (stop_sender, mut stop_receiver) = watch::channel(false);
        drop(stop_sender);

        let batch = receive_session_agent_event_batch_with_deadline(
            &mut receiver,
            &mut stop_receiver,
            || async { tokio::task::yield_now().await },
        )
        .await;

        assert!(!batch.stopped);
        assert_eq!(
            batch.events,
            vec![AgentChatEvent::TextDelta("normal reply".into())]
        );
    }

    #[tokio::test]
    async fn approved_tool_stop_drops_the_running_tool_future() {
        struct DropFlag(std::sync::Arc<std::sync::atomic::AtomicBool>);

        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::Release);
            }
        }

        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let started = std::sync::Arc::new(tokio::sync::Notify::new());
        let (stop_sender, mut stop_receiver) = watch::channel(false);
        let tool_started = started.clone();
        let tool_dropped = dropped.clone();
        let tool = async move {
            let _drop_flag = DropFlag(tool_dropped);
            tool_started.notify_one();
            std::future::pending::<()>().await;
        };
        let stop = async move {
            started.notified().await;
            stop_sender
                .send(true)
                .expect("tool stop receiver should remain open");
        };

        let (result, ()) = tokio::join!(
            wait_for_session_agent_tool_or_stop(tool, &mut stop_receiver),
            stop
        );

        assert!(result.is_none());
        assert!(dropped.load(std::sync::atomic::Ordering::Acquire));
    }

    #[tokio::test]
    async fn approved_tool_stop_waits_for_blocking_worker_cleanup() {
        let started = std::sync::Arc::new(tokio::sync::Notify::new());
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let worker_started = started.clone();
        let worker_finished = finished.clone();
        let mut handle = tokio::task::spawn_blocking(move || {
            worker_started.notify_one();
            release_receiver
                .recv()
                .expect("cleanup test should release the worker");
            worker_finished.store(true, std::sync::atomic::Ordering::Release);
        });
        started.notified().await;

        let finished_before_release = finished.clone();
        let release = async move {
            tokio::task::yield_now().await;
            assert!(!finished_before_release.load(std::sync::atomic::Ordering::Acquire));
            release_sender
                .send(())
                .expect("blocking worker should remain alive until cleanup");
        };
        let ((), ()) = tokio::join!(
            abort_and_wait_for_session_agent_tool_worker(&mut handle),
            release,
        );

        assert!(finished.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn approved_tool_completion_is_committed_before_stopping_the_continuation() {
        let (stop_sender, stop_receiver) = watch::channel(false);
        let outcome = SessionAgentApprovedToolOutcome::Finished(Ok("done".to_string()));

        assert!(!approved_tool_was_stopped(&outcome));
        assert!(!approved_tool_should_stop_after_finished(
            &outcome,
            &stop_receiver,
        ));
        stop_sender
            .send(true)
            .expect("tool stop receiver should remain open");
        assert!(!approved_tool_was_stopped(&outcome));
        assert!(approved_tool_should_stop_after_finished(
            &outcome,
            &stop_receiver,
        ));

        let stopped = SessionAgentApprovedToolOutcome::Stopped;
        assert!(approved_tool_was_stopped(&stopped));
        assert!(!approved_tool_should_stop_after_finished(
            &stopped,
            &stop_receiver,
        ));
    }

    #[test]
    fn tool_handoff_events_release_the_previous_pty_lease() {
        for event in [
            AgentChatEvent::ToolCallAutoExecuteRequired {
                id: "auto".to_string(),
            },
            AgentChatEvent::ToolCallApprovalRequired {
                id: "approval".to_string(),
                message: "approve".to_string(),
            },
            AgentChatEvent::ToolCallUserInputRequired {
                id: "input".to_string(),
                message: "answer".to_string(),
            },
        ] {
            assert!(session_agent_event_releases_pty_lease(&event));
        }
        assert!(!session_agent_event_releases_pty_lease(
            &AgentChatEvent::TextDelta("still streaming".to_string())
        ));
    }

    #[tokio::test]
    async fn normal_terminal_events_ignore_a_dropped_stop_sender() {
        for event in [
            AgentChatEvent::Finished("done".into()),
            AgentChatEvent::ToolCallAutoExecuteRequired {
                id: "tool-1".into(),
            },
        ] {
            let (sender, mut receiver) =
                tokio::sync::mpsc::channel::<AgentResult<AgentChatEvent>>(1);
            sender
                .send(Ok(event.clone()))
                .await
                .expect("stream receiver should remain open");
            let (stop_sender, mut stop_receiver) = watch::channel(false);
            drop(stop_sender);

            let batch = receive_session_agent_event_batch_with_deadline(
                &mut receiver,
                &mut stop_receiver,
                std::future::pending::<()>,
            )
            .await;

            assert!(!batch.stopped);
            assert_eq!(batch.events, vec![event]);
        }

        let (sender, mut receiver) = tokio::sync::mpsc::channel::<AgentResult<AgentChatEvent>>(1);
        sender
            .send(Err(AgentError::InvalidArguments("bad input".into())))
            .await
            .expect("stream receiver should remain open");
        let (stop_sender, mut stop_receiver) = watch::channel(false);
        drop(stop_sender);

        let batch = receive_session_agent_event_batch_with_deadline(
            &mut receiver,
            &mut stop_receiver,
            std::future::pending::<()>,
        )
        .await;

        assert!(!batch.stopped);
        assert!(batch.error.is_some());
    }
}
