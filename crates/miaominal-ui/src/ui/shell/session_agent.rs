use super::*;
use crate::ui::i18n;
use crate::ui::shell::session_agent_stream_batch::{
    SESSION_AGENT_STREAM_UI_FLUSH_INTERVAL, SessionAgentStreamBatch,
    session_agent_event_is_finished, session_agent_event_requires_immediate_flush,
};
use gpui_component::WindowExt as _;
use miaominal_agent::{
    AgentChatEvent, AgentChatMessage, AgentChatProvider, AgentChatProviderKind, AgentChatRequest,
    AgentChatRole, AgentChatToolEvent, AgentError, AgentExecChannel, AgentMode, AgentResult,
    AgentToolResultContinuationRequest, AgentToolSet, TERMINAL_INTERRUPT_SETTLE_TIMEOUT,
    TerminalExecHandle, TerminalExecLeaseState,
};
use miaominal_secrets::SecretKind;
use miaominal_settings::{AiProviderConfig, AiProviderKind};
#[cfg(test)]
use miaominal_storage::chat_store::{ChatMessageRecord, ChatMessageRole};
use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::watch;

const SESSION_AGENT_CONTEXT_MAX_MESSAGES: usize = 40;
const SESSION_AGENT_CONTEXT_MAX_CHARS: usize = 80_000;
const SESSION_AGENT_PRODUCER_STOP_ACK_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(2);

pub(in crate::ui::shell) type SessionAgentPtyTap = TerminalLease;

pub(in crate::ui::shell) fn retire_session_agent_pty_leases(leases: &[TerminalLease]) {
    for lease in leases {
        lease.retire();
    }
}

#[derive(Clone)]
pub(in crate::ui::shell) struct SessionAgentPtyInterrupt {
    pub(in crate::ui::shell) handle: TerminalExecHandle,
}

impl SessionAgentPtyInterrupt {
    pub(in crate::ui::shell) fn cancel(&self) {
        if let Err(error) = self.handle.cancel() {
            log::debug!("failed to cancel stopped agent PTY command: {error:?}");
        }
    }

    pub(in crate::ui::shell) async fn cancel_and_wait(&self) -> TerminalExecLeaseState {
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
pub(in crate::ui::shell) struct SessionAgentPtyContext {
    pub(in crate::ui::shell) tap: SessionAgentPtyTap,
    pub(in crate::ui::shell) interrupt: SessionAgentPtyInterrupt,
}

#[derive(Clone, Default)]
pub(in crate::ui::shell) struct SessionAgentPtyInterrupts {
    pub(in crate::ui::shell) active: Option<SessionAgentPtyInterrupt>,
    pub(in crate::ui::shell) targets: HashMap<String, SessionAgentPtyInterrupt>,
}

impl SessionAgentPtyInterrupts {
    pub(in crate::ui::shell) fn cancel_commands(&self) {
        if let Some(interrupt) = self.active.as_ref() {
            interrupt.cancel();
        }
        for interrupt in self.targets.values() {
            interrupt.cancel();
        }
    }

    pub(in crate::ui::shell) async fn cancel_commands_and_wait(&self) -> bool {
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

pub(in crate::ui::shell) struct SessionAgentReceivedBatch {
    pub(in crate::ui::shell) events: Vec<AgentChatEvent>,
    pub(in crate::ui::shell) error: Option<AgentError>,
    pub(in crate::ui::shell) finished: bool,
    pub(in crate::ui::shell) stream_closed: bool,
    pub(in crate::ui::shell) stopped: bool,
    pub(in crate::ui::shell) producer_stop_ack_timed_out: bool,
}

pub(in crate::ui::shell) enum SessionAgentApprovedToolOutcome {
    Finished(anyhow::Result<String>),
    Stopped,
}

pub(in crate::ui::shell) fn approved_tool_was_stopped(
    outcome: &SessionAgentApprovedToolOutcome,
) -> bool {
    matches!(outcome, SessionAgentApprovedToolOutcome::Stopped)
}

pub(in crate::ui::shell) fn approved_tool_should_stop_after_finished(
    outcome: &SessionAgentApprovedToolOutcome,
    stop: &watch::Receiver<bool>,
) -> bool {
    *stop.borrow() && matches!(outcome, SessionAgentApprovedToolOutcome::Finished(_))
}

pub(in crate::ui::shell) fn session_agent_event_releases_pty_lease(event: &AgentChatEvent) -> bool {
    matches!(
        event,
        AgentChatEvent::ToolCallAutoExecuteRequired { .. }
            | AgentChatEvent::ToolCallApprovalRequired { .. }
            | AgentChatEvent::ToolCallUserInputRequired { .. }
    )
}

pub(in crate::ui::shell) async fn wait_for_session_agent_stop(stop: &mut watch::Receiver<bool>) {
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

pub(in crate::ui::shell) async fn wait_for_session_agent_tool_or_stop<T>(
    tool: impl std::future::Future<Output = T>,
    stop: &mut watch::Receiver<bool>,
) -> Option<T> {
    tokio::select! {
        biased;
        _ = wait_for_session_agent_stop(stop) => None,
        result = tool => Some(result),
    }
}

pub(in crate::ui::shell) async fn abort_and_wait_for_session_agent_tool_worker<T>(
    handle: &mut tokio::task::JoinHandle<T>,
) {
    // `abort` prevents a queued spawn_blocking job from starting, but cannot stop one that is
    // already running. Await it as well so its stop-aware runtime has dropped the tool future and
    // terminal guard before the UI releases this request's PTY lease to another session.
    handle.abort();
    let _ = handle.await;
}

pub(in crate::ui::shell) async fn receive_session_agent_event_batch(
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

pub(super) fn agent_provider_kind(kind: AiProviderKind) -> AgentChatProviderKind {
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

pub(in crate::ui::shell) fn build_session_agent_history(
    messages: &[SessionAgentMessage],
) -> Vec<AgentChatMessage> {
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

impl AgentController {
    fn set_session_agent_execution_context_error(
        &mut self,
        message: String,
        cx: &mut Context<Self>,
    ) {
        self.set_execution_context_error(message, cx);
    }

    fn set_session_agent_execution_context_error_for_session(
        &mut self,
        session_id: &str,
        message: String,
        cx: &mut Context<Self>,
    ) {
        self.set_execution_context_error_for_session(session_id, message, cx);
    }

    fn fail_session_agent_tool_start_for_session(
        &mut self,
        session_id: &str,
        tool_id: &str,
        message: String,
        cx: &mut Context<Self>,
    ) {
        self.fail_tool_start_for_session(session_id, tool_id, message, cx);
    }

    pub(in crate::ui::shell) fn submit_session_agent_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let draft = match self.prompt_submission_draft(cx) {
            AgentPromptDraftOutcome::Busy => return,
            AgentPromptDraftOutcome::Empty => {
                cx.emit(AppCommand::Feedback(i18n::string(
                    "workspace.panel.agent.empty_prompt",
                )));
                cx.notify();
                return;
            }
            AgentPromptDraftOutcome::Ready(draft) => draft,
        };
        let AgentPromptDraft {
            prompt,
            has_pending_attachments,
            target_names,
        } = draft;

        let Some(provider_id) = self.selected_ai_provider_id() else {
            let message = i18n::string("workspace.panel.agent.no_provider_configured");
            self.record_prompt_submission_error(message, false, cx);
            return;
        };

        if self.session_agent_requires_local_vault_unlock(&provider_id) {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Agent(
                AgentDeferredCommand::ResumeRequest,
            )));
            return;
        }

        let provider = match self.build_session_agent_provider(&provider_id) {
            Ok(provider) => provider,
            Err(error) => {
                let message = error.to_string();
                self.record_prompt_submission_error(message, false, cx);
                return;
            }
        };

        self.capture_prompt_execution_context();
        let mentions = self.resolve_session_agent_mentions(&target_names);
        if mentions.pty_busy {
            let message = i18n::string("workspace.panel.agent.messages.pty_terminal_busy");
            self.record_prompt_submission_error(message, true, cx);
            return;
        }
        if !mentions.unresolved.is_empty() {
            retire_session_agent_pty_leases(&mentions.pty_taps);
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
            self.record_prompt_submission_error(message, true, cx);
            return;
        }

        let pty_target_taps = mentions.pty_taps.clone();
        let pty_target_interrupts = mentions.pty_interrupts.clone();
        let Some((tools, active_pty_context)) =
            self.build_session_agent_tools(mentions.aux_channels, cx)
        else {
            retire_session_agent_pty_leases(&pty_target_taps);
            self.clear_prompt_execution_context();
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
        let AgentPromptRequestPreparation {
            session_id: stream_session_id,
            request_id,
            attachments,
            history_messages,
        } = self.prepare_prompt_request(target_names, cx);
        let history = build_session_agent_history(&history_messages);
        let prompt_images: Vec<miaominal_core::chat_attachment::ChatImage> = attachments
            .iter()
            .filter_map(|attachment| attachment.as_image().cloned())
            .collect();
        let llm_prompt = content_with_text_attachments(&model_prompt, &attachments);
        let images_as_text_fallback =
            !prompt_images.is_empty() && !agent_provider_supports_vision(provider.kind);

        self.commit_prompt_request(&prompt, model_prompt, attachments, request_id, window, cx);
        let status = if images_as_text_fallback {
            i18n::string("workspace.panel.agent.messages.image_attachments_text_fallback")
        } else {
            i18n::string("workspace.panel.agent.send_pending")
        };
        cx.emit(AppCommand::Feedback(status));

        let agent_cancellation = tools.as_ref().map(AgentToolSet::cancellation);
        let wait_for_producer_close = agent_cancellation.is_some();
        self.spawn_stream_task(
            AgentStreamTask {
                session_id: stream_session_id,
                request_id,
                request: AgentStreamTaskRequest::Chat(AgentChatRequest {
                    provider,
                    messages: history,
                    prompt: llm_prompt,
                    prompt_images,
                    tools,
                    target_guidance,
                }),
                active_pty_lease: active_pty_tap,
                target_pty_leases: pty_target_taps,
                pty_interrupts,
                agent_cancellation,
                wait_for_producer_close,
                error_context: "session agent stream",
            },
            cx,
        );
        cx.notify();
    }

    fn build_session_agent_tools(
        &mut self,
        aux_channels: HashMap<String, AgentExecChannel>,
        cx: &mut Context<Self>,
    ) -> SessionAgentTools {
        self.build_session_agent_tools_for_session(None, aux_channels, cx)
    }

    fn build_session_agent_tools_for_session(
        &mut self,
        session_id: Option<&str>,
        mut aux_channels: HashMap<String, AgentExecChannel>,
        cx: &mut Context<Self>,
    ) -> SessionAgentTools {
        let context = match session_id {
            Some(session_id) => self.active_execution_context_for_session(session_id),
            None => self.prompt_execution_context(),
        };
        let Some(context) = context else {
            return Some((None, None));
        };
        let active_target_marker = self.terminal_target_marker_for_execution_context(&context);

        let Some(profile) = self.profile_for_execution_context(&context) else {
            let message =
                i18n::string("workspace.panel.agent.messages.no_active_session_for_approval");
            if let Some(session_id) = session_id {
                self.set_session_agent_execution_context_error_for_session(session_id, message, cx);
            } else {
                self.set_session_agent_execution_context_error(message, cx);
            }
            return None;
        };

        let terminal_lease = match self.acquire_terminal_lease_for_execution_context(&context) {
            Ok(lease) => lease,
            Err(message) => {
                if let Some(session_id) = session_id {
                    self.set_session_agent_execution_context_error_for_session(
                        session_id, message, cx,
                    );
                } else {
                    self.set_session_agent_execution_context_error(message, cx);
                }
                return None;
            }
        };

        let mut active_pty_tap = None;
        let mut channel = self.agent_exec_channel_for_profile(profile);
        if let Some(TerminalLeaseGrant {
            commands,
            output,
            lease,
        }) = terminal_lease
        {
            let terminal_exec = TerminalExecHandle::new(commands, output);
            active_pty_tap = Some(SessionAgentPtyContext {
                tap: lease,
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
        let mode = match session_id {
            Some(session_id) => self.agent_mode_for_session(session_id)?,
            None => self.agent_mode(),
        };
        let tools = Some(AgentToolSet::for_channel(channel, mode));

        Some((tools, active_pty_tap))
    }

    pub(in crate::ui::shell) fn stop_session_agent_stream(&mut self, cx: &mut Context<Self>) {
        let has_pending_task = self.has_pending_task();
        if has_pending_task && self.request_stream_stop() {
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
        if let Some(session_id) = self.foreground_session_id() {
            return self.finalize_session_agent_stopped_for_session(
                &session_id,
                expected_request_id,
                pty_interrupts,
                cx,
            );
        }
        if !self.request_matches(expected_request_id) {
            return false;
        }

        if let Some(pty_interrupts) = pty_interrupts {
            pty_interrupts.cancel_commands();
        }
        self.finalize_stopped(cx)
    }

    fn finalize_session_agent_stopped_for_session(
        &mut self,
        session_id: &str,
        expected_request_id: Option<u64>,
        pty_interrupts: Option<&SessionAgentPtyInterrupts>,
        cx: &mut Context<Self>,
    ) -> bool {
        let is_loaded_session = self.session_is_foreground(session_id);
        if !self.session_exists(session_id) {
            return false;
        }
        if let Some(pty_interrupts) = pty_interrupts {
            pty_interrupts.cancel_commands();
        }
        let stopped = self.finalize_stopped_for_session(session_id, expected_request_id, cx);
        if stopped && !is_loaded_session {
            self.refresh_chat_sessions(cx);
            cx.notify();
        }
        stopped
    }

    pub(in crate::ui::shell) fn approve_session_agent_tool_call(
        &mut self,
        tool_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(session_id) = self.foreground_session_id() else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "workspace.panel.agent.messages.tool_not_found",
            )));
            cx.notify();
            return;
        };
        self.approve_session_agent_tool_call_for_session(&session_id, tool_id, cx);
    }

    pub(in crate::ui::shell) fn approve_session_agent_tool_call_for_session(
        &mut self,
        session_id: &str,
        tool_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(tool_call) = self.tool_call_for_approval_in_session(session_id, &tool_id) else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "workspace.panel.agent.messages.tool_not_found",
            )));
            cx.notify();
            return;
        };
        let Some(context) = self.active_execution_context_for_session(session_id) else {
            self.fail_session_agent_tool_start_for_session(
                session_id,
                &tool_id,
                i18n::string("workspace.panel.agent.messages.no_active_session_for_approval"),
                cx,
            );
            return;
        };
        let active_target_marker = self.terminal_target_marker_for_execution_context(&context);
        let Some(profile) = self.profile_for_execution_context(&context) else {
            self.fail_session_agent_tool_start_for_session(
                session_id,
                &tool_id,
                i18n::string("workspace.panel.agent.messages.no_active_session_for_approval"),
                cx,
            );
            return;
        };

        let arguments = parse_tool_arguments(&tool_call.arguments);
        let Some(AgentToolApprovalCommit {
            session_id: approval_session_id,
            reasoning,
            agent_mode,
        }) = self.approve_tool_for_execution_in_session(session_id, &tool_id, cx)
        else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "workspace.panel.agent.messages.tool_not_found",
            )));
            cx.notify();
            return;
        };

        let terminal_lease = match self.acquire_terminal_lease_for_execution_context(&context) {
            Ok(lease) => lease,
            Err(message) => {
                self.fail_session_agent_tool_start_for_session(session_id, &tool_id, message, cx);
                return;
            }
        };
        let mut active_pty_context = None;
        let pty_handle = if let Some(TerminalLeaseGrant {
            commands,
            output,
            lease,
        }) = terminal_lease
        {
            let terminal_exec = TerminalExecHandle::new(commands, output);
            active_pty_context = Some(SessionAgentPtyContext {
                tap: lease,
                interrupt: SessionAgentPtyInterrupt {
                    handle: terminal_exec.clone(),
                },
            });
            Some(terminal_exec)
        } else {
            None
        };
        let approval_mentions =
            self.resolve_mentions_from_tool_arguments_for_session(Some(session_id), &arguments);
        if approval_mentions.pty_busy {
            if let Some(context) = active_pty_context.as_ref() {
                retire_session_agent_pty_leases(std::slice::from_ref(&context.tap));
            }
            self.fail_session_agent_tool_start_for_session(
                session_id,
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
        let sessions = self.session_profiles();
        let agent_service = self.agent_service();
        let secrets = self.secrets();
        let known_hosts = self.known_hosts();
        let web_search_config = miaominal_settings::current_settings().web_search;
        let skip_policy = matches!(agent_mode, AgentMode::NonBlocking | AgentMode::FullAuto);
        let tool_name = tool_call.name.clone();
        let tool_arguments = tool_call.arguments.clone();
        let Some(execution_request_id) = self.begin_tool_execution_for_session(session_id) else {
            return;
        };
        self.spawn_approved_tool_task(
            AgentApprovedToolTask {
                session_id: approval_session_id,
                tool_id,
                execution_request_id,
                reasoning,
                tool_name,
                tool_arguments,
                arguments,
                approved: true,
                skip_policy,
                agent_service,
                profile,
                profiles: sessions,
                secrets,
                known_hosts,
                web_search_config,
                terminal_exec: pty_handle,
                aux_channels: approval_mentions.aux_channels,
                active_target_marker,
                active_pty_lease: active_pty_tap,
                target_pty_leases: approval_pty_target_taps,
                pty_interrupts,
            },
            cx,
        );
        cx.notify();
    }

    pub(in crate::ui::shell) fn submit_active_session_agent_user_answer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let continuation: Option<AgentToolContinuation> =
            self.prepare_active_user_answer(window, cx);
        if let Some(continuation) = continuation {
            self.continue_session_agent_after_tool_result(
                continuation.tool_call,
                continuation.reasoning,
                continuation.result,
                continuation.failed,
                cx,
            );
        }
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
        let continuation: Option<AgentToolContinuation> =
            self.prepare_user_answer(tool_id, answer, selected_index, custom, window, cx);
        if let Some(continuation) = continuation {
            self.continue_session_agent_after_tool_result(
                continuation.tool_call,
                continuation.reasoning,
                continuation.result,
                continuation.failed,
                cx,
            );
        }
    }

    fn continue_session_agent_after_tool_result(
        &mut self,
        tool_call: AgentChatToolEvent,
        reasoning: Option<String>,
        result: String,
        failed: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(session_id) = self.foreground_session_id() else {
            return;
        };
        self.continue_session_agent_after_tool_result_for_session(
            &session_id,
            tool_call,
            reasoning,
            result,
            failed,
            cx,
        );
    }

    pub(in crate::ui::shell) fn continue_session_agent_after_tool_result_for_session(
        &mut self,
        session_id: &str,
        tool_call: AgentChatToolEvent,
        reasoning: Option<String>,
        result: String,
        failed: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.ensure_continuation_idle_for_session(session_id, cx) {
            return;
        }

        let Some(provider_id) = self.selected_ai_provider_id() else {
            let message = i18n::string("workspace.panel.agent.no_provider_configured");
            self.record_continuation_setup_error_for_session(session_id, message, cx);
            return;
        };

        let provider = match self.build_session_agent_provider(&provider_id) {
            Ok(provider) => provider,
            Err(error) => {
                let message = error.to_string();
                self.record_continuation_setup_error_for_session(session_id, message, cx);
                return;
            }
        };

        let Some(active_targets) = self.active_target_names_for_session(session_id) else {
            return;
        };
        let mentions =
            self.resolve_session_agent_mentions_for_session(Some(session_id), &active_targets);
        if mentions.pty_busy {
            self.set_session_agent_execution_context_error_for_session(
                session_id,
                i18n::string("workspace.panel.agent.messages.pty_terminal_busy"),
                cx,
            );
            return;
        }
        let pty_target_taps = mentions.pty_taps.clone();
        let pty_target_interrupts = mentions.pty_interrupts.clone();
        let target_guidance = mentions.guidance;
        let Some((tools, active_pty_context)) =
            self.build_session_agent_tools_for_session(Some(session_id), mentions.aux_channels, cx)
        else {
            retire_session_agent_pty_leases(&pty_target_taps);
            return;
        };
        let active_pty_tap = active_pty_context
            .as_ref()
            .map(|context| context.tap.clone());
        let pty_interrupts = SessionAgentPtyInterrupts {
            active: active_pty_context.map(|context| context.interrupt),
            targets: pty_target_interrupts,
        };
        let Some(AgentContinuationPreparation {
            session_id: stream_session_id,
            request_id,
            history_messages,
        }) = self.prepare_continuation_request_for_session(session_id, cx)
        else {
            return;
        };
        let history = build_session_agent_history(&history_messages);
        cx.emit(AppCommand::Feedback(i18n::string(
            "workspace.panel.agent.thinking",
        )));

        let agent_cancellation = tools.as_ref().map(AgentToolSet::cancellation);
        let wait_for_producer_close = agent_cancellation.is_some();
        let result = if failed {
            format!("ERROR: {result}")
        } else {
            result
        };
        self.spawn_stream_task(
            AgentStreamTask {
                session_id: stream_session_id,
                request_id,
                request: AgentStreamTaskRequest::Continuation(AgentToolResultContinuationRequest {
                    provider,
                    messages: history,
                    tool_call,
                    reasoning,
                    result,
                    tools,
                    target_guidance,
                }),
                active_pty_lease: active_pty_tap,
                target_pty_leases: pty_target_taps,
                pty_interrupts,
                agent_cancellation,
                wait_for_producer_close,
                error_context: "session agent continuation",
            },
            cx,
        );
    }

    pub(in crate::ui::shell) fn notify_background_session_agent(
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
                warning_notification(title, message)
            }
            SessionAgentBackgroundNotificationKind::UserInputRequired { tool_name } => {
                let title = i18n::string("workspace.panel.agent.notifications.user_input_title");
                let message = i18n::string_args(
                    "workspace.panel.agent.notifications.user_input",
                    &[("chat", &chat_label), ("tool", &tool_name)],
                );
                warning_notification(title, message)
            }
            SessionAgentBackgroundNotificationKind::ReplyReady => {
                let title = i18n::string("workspace.panel.agent.notifications.reply_ready_title");
                let message = i18n::string_args(
                    "workspace.panel.agent.notifications.reply_ready",
                    &[("chat", &chat_label)],
                );
                success_notification(title, message)
            }
            SessionAgentBackgroundNotificationKind::StreamFailed { error } => {
                let title = i18n::string("workspace.panel.agent.notifications.stream_failed_title");
                let message = i18n::string_args(
                    "workspace.panel.agent.notifications.stream_failed",
                    &[("chat", &chat_label), ("error", &error)],
                );
                error_notification(title, message)
            }
        };

        let Some(window_handle) = cx.active_window() else {
            return;
        };
        let _ = window_handle.update(cx, move |_, window, cx| {
            window.push_notification(notification, cx);
        });
    }

    pub(in crate::ui::shell) fn generate_session_agent_title_for_session(
        &mut self,
        session_id: &str,
        user_message: String,
        assistant_message: String,
        cx: &mut Context<Self>,
    ) {
        let Some(provider_id) = self.selected_ai_provider_id() else {
            return;
        };
        let provider = match self.build_session_agent_provider(&provider_id) {
            Ok(provider) => provider,
            Err(_error) => {
                #[cfg(debug_assertions)]
                log::info!("skip title generation: {_error:?}");
                return;
            }
        };
        self.spawn_title_task(
            session_id.to_string(),
            provider,
            user_message,
            assistant_message,
            cx,
        );
    }

    pub(in crate::ui::shell) fn recover_session_agent_prompt_for_session(
        &mut self,
        session_id: &str,
        request_id: u64,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if !self.begin_prompt_recovery_for_session(session_id, request_id, &message, cx) {
            return;
        }

        let provider = self
            .selected_ai_provider_id()
            .ok_or_else(|| anyhow::anyhow!(message.clone()))
            .and_then(|provider_id| self.build_session_agent_provider(&provider_id));
        match provider {
            Ok(provider) => {
                self.continue_prompt_recovery_with_provider(session_id, message, provider, cx);
            }
            Err(error) => {
                self.fail_prompt_recovery_setup(session_id, error.to_string(), cx);
            }
        }
    }

    fn build_session_agent_provider(&self, provider_id: &str) -> anyhow::Result<AgentChatProvider> {
        let provider = self
            .settings()
            .ai_providers
            .into_iter()
            .find(|provider| provider.id == provider_id && provider.enabled)
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
            reasoning_effort: provider.reasoning_effort,
        })
    }

    fn settings(&self) -> miaominal_settings::AppSettings {
        miaominal_settings::current_settings()
    }

    fn selected_ai_provider_id(&self) -> Option<String> {
        let settings = self.settings();
        settings
            .selected_ai_provider_id
            .filter(|selected_id| {
                settings
                    .ai_providers
                    .iter()
                    .any(|provider| provider.id == *selected_id && provider.enabled)
            })
            .or_else(|| {
                settings
                    .ai_providers
                    .iter()
                    .find(|provider| provider.enabled)
                    .map(|provider| provider.id.clone())
            })
    }

    fn session_agent_requires_local_vault_unlock(&self, provider_id: &str) -> bool {
        if self.local_vault_status() != LocalVaultStatus::Locked {
            return false;
        }

        let settings = self.settings();
        let provider_requires_saved_key = settings
            .ai_providers
            .iter()
            .find(|provider| provider.id == provider_id && provider.enabled)
            .is_some_and(|provider| {
                provider.has_api_key
                    && (provider.api_key_env.trim().is_empty()
                        || std::env::var(provider.api_key_env.trim())
                            .map(|value| value.trim().is_empty())
                            .unwrap_or(true))
            });
        let web_search_requires_saved_key = settings.web_search.enabled
            && settings.web_search.has_api_key
            && (settings.web_search.api_key_env.trim().is_empty()
                || std::env::var(settings.web_search.api_key_env.trim())
                    .map(|value| value.trim().is_empty())
                    .unwrap_or(true));

        provider_requires_saved_key || web_search_requires_saved_key
    }

    fn agent_exec_channel_for_profile(&self, profile: SessionProfile) -> AgentExecChannel {
        let profiles = self.session_profiles();
        let agent_service = self.agent_service();
        let secrets = self.secrets();
        let known_hosts = self.known_hosts();
        let mut channel = agent_service.channel_for_profile_snapshot_with_stores(
            profile,
            &profiles,
            secrets.clone(),
            known_hosts,
        );
        let web_search_config = self.settings().web_search;
        if web_search_config.enabled {
            let web_search_api_key = secrets
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
        self.resolve_session_agent_mentions_for_session(None, targets)
    }

    fn resolve_session_agent_mentions_for_session(
        &mut self,
        session_id: Option<&str>,
        targets: &[String],
    ) -> ResolvedSessionAgentMentions {
        let mut aux_channels = HashMap::new();
        let mut guidance_lines = Vec::new();
        let mut resolved_names = Vec::new();
        let mut pending_pty_taps = Vec::new();
        let mut pty_interrupts = HashMap::new();
        let profiles = self.session_profiles();
        let terminal_targets = self.terminal_targets();
        let active_exec_context = match session_id {
            Some(session_id) => self.active_execution_context_for_session(session_id),
            None => self.prompt_execution_context(),
        };
        let active_terminal_tab_id = active_exec_context
            .as_ref()
            .filter(|context| context.exec_mode == AgentExecMode::Pty)
            .and_then(|context| context.terminal_tab_id);
        let execution_mode = match session_id {
            Some(session_id) => self
                .running_execution_mode_for_session(session_id)
                .unwrap_or(AgentExecMode::ExecChannel),
            None => self.running_execution_mode(),
        };

        match execution_mode {
            AgentExecMode::ExecChannel => {
                for profile in profiles {
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
                for target in terminal_targets {
                    let marker = format!("@{}", target.title);
                    if !targets.iter().any(|name| name == &target.title)
                        || resolved_names.contains(&target.title)
                    {
                        continue;
                    }
                    if !target.command_available {
                        continue;
                    }
                    let Some(profile) = target.profile.clone() else {
                        continue;
                    };
                    if active_terminal_tab_id == Some(target.tab_id) {
                        // The default active channel will own this tab's single output tap. An
                        // explicit `target: "@current-tab"` can safely fall through to that
                        // channel instead of acquiring a second lease for the same request.
                        guidance_lines.push(format!(
                            "- {marker}: terminal session \"{}\" (profile: {}, host: {}, user: {})",
                            target.title, profile.name, profile.host, profile.username
                        ));
                        resolved_names.push(target.title.clone());
                        continue;
                    }
                    let TerminalLeaseGrant {
                        commands,
                        output,
                        lease,
                    } = match self.acquire_terminal(target.tab_id) {
                        Ok(grant) => grant,
                        Err(TerminalLeaseError::Busy) => {
                            retire_session_agent_pty_leases(&pending_pty_taps);
                            return ResolvedSessionAgentMentions {
                                pty_busy: true,
                                ..Default::default()
                            };
                        }
                        Err(TerminalLeaseError::Missing | TerminalLeaseError::Disconnected) => {
                            continue;
                        }
                    };
                    let terminal_exec = TerminalExecHandle::new(commands, output);
                    let channel = self
                        .agent_exec_channel_for_profile(profile.clone())
                        .with_terminal_exec(terminal_exec.clone());
                    aux_channels.insert(marker.clone(), channel);
                    pending_pty_taps.push(lease);
                    pty_interrupts.insert(
                        marker.clone(),
                        SessionAgentPtyInterrupt {
                            handle: terminal_exec,
                        },
                    );
                    guidance_lines.push(format!(
                        "- {marker}: terminal session \"{}\" (profile: {}, host: {}, user: {})",
                        target.title, profile.name, profile.host, profile.username
                    ));
                    resolved_names.push(target.title);
                }
            }
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
            pty_taps: pending_pty_taps,
            pty_interrupts,
            pty_busy: false,
        }
    }

    fn resolve_mentions_from_tool_arguments_for_session(
        &mut self,
        session_id: Option<&str>,
        arguments: &Value,
    ) -> ResolvedSessionAgentMentions {
        let Some(target) = arguments.get("target").and_then(Value::as_str) else {
            return ResolvedSessionAgentMentions::default();
        };
        let target = target.trim().trim_start_matches('@').trim().to_string();
        self.resolve_session_agent_mentions_for_session(session_id, &[target])
    }

    fn resolve_ai_provider_api_key(&self, provider: &AiProviderConfig) -> anyhow::Result<String> {
        if !provider.api_key_env.trim().is_empty()
            && let Ok(value) = std::env::var(provider.api_key_env.trim())
            && !value.trim().is_empty()
        {
            return Ok(value);
        }

        let api_key = self
            .secrets()
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

#[derive(Default)]
struct ResolvedSessionAgentMentions {
    aux_channels: HashMap<String, AgentExecChannel>,
    guidance: Option<String>,
    unresolved: Vec<String>,
    pty_taps: Vec<TerminalLease>,
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

pub(in crate::ui::shell) fn is_recoverable_session_agent_prompt_error(message: &str) -> bool {
    message.contains("PromptError")
        || message.contains("UnknownToolCall")
        || message.contains("ToolCallError")
        || message.contains("ToolServerError")
        || message.contains("MaxTurnError")
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

    #[test]
    fn session_agent_history_excludes_persisted_error_messages() {
        let messages = vec![
            SessionAgentMessage::user("question"),
            SessionAgentMessage::assistant_raw("partial answer"),
            SessionAgentMessage::error("provider failed"),
        ];

        let history = build_session_agent_history(&messages);

        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content, "question");
        assert_eq!(history[1].content, "partial answer");
    }

    #[test]
    fn complete_conversation_with_error_round_trips_through_persistence_records() {
        let messages = vec![
            SessionAgentMessage::user("earlier question"),
            SessionAgentMessage::assistant_raw("earlier answer"),
            SessionAgentMessage::user("current question"),
            SessionAgentMessage::assistant_raw("partial answer"),
            SessionAgentMessage::error("provider failed"),
        ];

        let restored = messages
            .iter()
            .enumerate()
            .map(|(index, message)| {
                let record =
                    chat_record_from_session_agent_message("session-1", index, 100, message)
                        .expect("message should produce a persistence record");
                session_agent_message_from_record(record)
            })
            .collect::<Vec<_>>();

        assert_eq!(restored.len(), messages.len());
        assert_eq!(
            restored
                .iter()
                .map(|message| (message.role, message.content.as_str()))
                .collect::<Vec<_>>(),
            messages
                .iter()
                .map(|message| (message.role, message.content.as_str()))
                .collect::<Vec<_>>()
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
