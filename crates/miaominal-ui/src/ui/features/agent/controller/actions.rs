//! Panel actions on top of the agent engine: submit, steer, stop, approve,
//! answer, and applying engine events to the views.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use super::engine::{EngineSignal, MessageChange, SessionEngine};
use super::*;
use crate::ui::i18n;
use miaominal_agent::context::TargetInfo;
use miaominal_agent::engine::{
    AgentCommand, AgentConfigPatch, AgentDeps, AgentEvent, AgentSessionConfig, ApprovalDecision,
    NullSink, TranscriptSink, TurnEndReason, TurnLimits, spawn_session,
};
use miaominal_agent::model::{ModelClient, ModelProfile, client_for};
use miaominal_agent::policy::ApprovalSettings;
use miaominal_agent::toolkit::{
    ChannelToolRuntime, TerminalContextSource, TerminalReadRequest, TerminalReadResult,
    TerminalSnapshot, ToolRuntime, refers_to_terminal, render_context,
};
use miaominal_agent::{AgentExecChannel, TerminalExecHandle};
use miaominal_secrets::SecretKind;
use miaominal_settings::AiProviderConfig;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) enum SessionAgentBackgroundNotificationKind {
    ToolApprovalRequired { tool_name: String },
    UserInputRequired { tool_name: String },
    ReplyReady,
    StreamFailed { error: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum ApprovalInputMode {
    /// Edit the command/patch, then approve the edited version.
    Edit,
    /// Deny with a reason the model can act on.
    DenyReason,
}

pub(in crate::ui) fn retire_session_agent_pty_leases(leases: &[TerminalLease]) {
    for lease in leases {
        lease.retire();
    }
}

/// Everything a turn needs that depends on current settings and sessions.
struct PreparedTurn {
    model: Arc<dyn ModelClient>,
    tools: Arc<dyn ToolRuntime>,
    targets: Vec<TargetInfo>,
    leases: Vec<TerminalLease>,
    web_search: bool,
    terminal_available: bool,
}

/// Lines of terminal text cached for `terminal_read`.
const TERMINAL_CACHE_LINES: usize = 2000;
/// Tail attached to a user message that refers to the terminal.
const CONTEXT_TAIL_LINES: usize = 80;
const CONTEXT_TAIL_BYTES: usize = 8 * 1024;

/// Terminal text captured on the UI thread and read by the engine's tools
/// (terminal state itself is not `Send`).
#[derive(Clone, Default)]
pub(in crate::ui) struct SharedTerminalContext(Arc<std::sync::Mutex<Option<TerminalSnapshot>>>);

impl SharedTerminalContext {
    fn set(&self, snapshot: Option<TerminalSnapshot>) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = snapshot;
    }

    fn get(&self) -> Option<TerminalSnapshot> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// The last `count` lines of `text`; true when lines were dropped.
fn last_lines(text: &str, count: usize) -> (String, bool) {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(count);
    (lines[start..].join("\n"), start > 0)
}

impl TerminalContextSource for SharedTerminalContext {
    fn snapshot(&self, _target: Option<&str>, tail_lines: usize) -> Option<TerminalSnapshot> {
        let mut snapshot = self.get()?;
        snapshot.tail = last_lines(&snapshot.tail, tail_lines).0;
        Some(snapshot)
    }

    fn read(&self, request: TerminalReadRequest) -> Option<TerminalReadResult> {
        let snapshot = self.get()?;
        let (text, truncated) = last_lines(&snapshot.tail, request.lines.max(1));
        Some(TerminalReadResult {
            target: snapshot.target,
            text,
            truncated,
        })
    }
}

#[derive(Default)]
struct ResolvedMentions {
    aux_channels: HashMap<String, AgentExecChannel>,
    unresolved: Vec<String>,
    leases: Vec<TerminalLease>,
    pty_busy: bool,
}

impl AgentController {
    pub(in crate::ui) fn submit_session_agent_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // While a turn runs, new input steers it instead of starting another.
        if self.session_agent().is_busy() {
            let prompt = self.forms.prompt_input.read(cx).value().trim().to_string();
            if prompt.is_empty() {
                return;
            }
            let sent = self.session_agent().engine.as_ref().is_some_and(|engine| {
                engine.send(AgentCommand::Steer {
                    text: prompt.clone(),
                })
            });
            if sent {
                self.record_prompt_history(&prompt);
                self.clear_prompt_after_submission(window, cx);
                cx.emit(AppCommand::Feedback(i18n::string(
                    "workspace.panel.agent.messages.steer_queued",
                )));
            }
            cx.notify();
            return;
        }

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

        self.capture_prompt_execution_context();
        let terminal_snapshot = self.capture_terminal_snapshot(cx);
        let terminal_context = self.session_agent().terminal_context.clone();
        terminal_context.set(terminal_snapshot.clone());
        let terminal_source: Option<Arc<dyn TerminalContextSource>> = terminal_snapshot
            .is_some()
            .then(|| Arc::new(terminal_context) as Arc<dyn TerminalContextSource>);
        let context = terminal_snapshot.map(|mut snapshot| {
            let (tail, _) = last_lines(&snapshot.tail, CONTEXT_TAIL_LINES);
            let start = tail.len().saturating_sub(CONTEXT_TAIL_BYTES);
            let start = (start..tail.len())
                .find(|index| tail.is_char_boundary(*index))
                .unwrap_or(tail.len());
            snapshot.tail = tail[start..].to_string();
            render_context(&snapshot, refers_to_terminal(&prompt))
        });
        let prepared = match self.prepare_turn(&provider_id, &target_names, terminal_source) {
            Ok(prepared) => prepared,
            Err(message) => {
                self.record_prompt_submission_error(message, true, cx);
                return;
            }
        };

        let session_id = self.ensure_foreground_session(cx);
        let attachments = {
            let state = &mut self.runtime.get_mut().foreground;
            state.panel_view = ChatPanelView::Conversation;
            state.active_at_targets = target_names.clone();
            std::mem::take(&mut state.pending_attachments)
        };
        let text = if prompt.is_empty() && has_pending_attachments {
            i18n::string("workspace.panel.agent.attachment_only_prompt")
        } else {
            prompt.clone()
        };
        if !self.dispatch_turn(&session_id, prepared, cx) {
            self.record_prompt_submission_error(
                i18n::string("workspace.panel.agent.messages.engine_unavailable"),
                true,
                cx,
            );
            return;
        }
        let sent = self.session_agent().engine.as_ref().is_some_and(|engine| {
            engine.send(AgentCommand::Submit {
                text,
                attachments,
                targets: target_names,
                context,
            })
        });
        if sent {
            let state = &mut self.runtime.get_mut().foreground;
            state.engine_busy = true;
            state.last_error = None;
        }
        self.sync_conversation_generating_view(cx);
        if !prompt.is_empty() {
            self.record_prompt_history(&prompt);
        }
        self.clear_prompt_after_submission(window, cx);
        cx.emit(AppCommand::Feedback(i18n::string(
            "workspace.panel.agent.send_pending",
        )));
        cx.notify();
    }

    /// Ensure the session engine exists and push the current model, tools
    /// and approval settings to it. Returns false when it cannot start.
    fn dispatch_turn(
        &mut self,
        session_id: &str,
        prepared: PreparedTurn,
        cx: &mut Context<Self>,
    ) -> bool {
        let approval = self.approval_settings();
        let PreparedTurn {
            model,
            tools,
            targets,
            leases,
            web_search,
            terminal_available,
        } = prepared;
        let needs_engine = self
            .runtime
            .borrow()
            .session(session_id)
            .is_some_and(|state| state.engine.is_none());
        if needs_engine {
            let transcript = self
                .runtime
                .get_mut()
                .session_mut(session_id)
                .and_then(|state| state.transcript_seed.take())
                .unwrap_or_default();
            let store: Arc<dyn TranscriptSink> = match self
                .chat_service
                .as_ref()
                .map(|service| service.transcript_sink(session_id))
            {
                Some(Ok(sink)) => Arc::new(sink),
                Some(Err(error)) => {
                    log::warn!("chat history unavailable for agent session: {error:?}");
                    Arc::new(NullSink)
                }
                None => Arc::new(NullSink),
            };
            let config = AgentSessionConfig {
                session_id: session_id.to_string(),
                approval,
                limits: TurnLimits::default(),
                targets: targets.clone(),
                web_search,
                terminal_available,
            };
            let (handle, events) = {
                let _runtime = self.task_runtime.enter();
                spawn_session(
                    config,
                    transcript,
                    AgentDeps {
                        model: model.clone(),
                        tools: tools.clone(),
                        store,
                    },
                )
            };
            let pump = self.spawn_engine_pump(session_id.to_string(), events, cx);
            if let Some(state) = self.runtime.get_mut().session_mut(session_id) {
                state.engine = Some(SessionEngine {
                    handle,
                    _pump: pump,
                });
            }
        }
        let Some(state) = self.runtime.get_mut().session_mut(session_id) else {
            retire_session_agent_pty_leases(&leases);
            return false;
        };
        retire_session_agent_pty_leases(&std::mem::take(&mut state.turn_leases));
        state.turn_leases = leases;
        state.engine.as_ref().is_some_and(|engine| {
            engine.send(AgentCommand::UpdateConfig(AgentConfigPatch {
                model: Some(model),
                tools: Some(tools),
                approval: Some(approval),
                targets: Some(targets),
                web_search: Some(web_search),
                terminal_available: Some(terminal_available),
            }))
        })
    }

    fn spawn_engine_pump(
        &mut self,
        session_id: String,
        mut events: tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Task<()> {
        cx.spawn(async move |this, cx| {
            while let Some(first) = events.recv().await {
                let mut batch = vec![first];
                while batch.len() < 512 {
                    match events.try_recv() {
                        Ok(event) => batch.push(event),
                        Err(_) => break,
                    }
                }
                let applied = this.update(cx, |controller, cx| {
                    controller.apply_engine_events(&session_id, batch, cx);
                });
                if applied.is_err() {
                    break;
                }
            }
        })
    }

    fn apply_engine_events(
        &mut self,
        session_id: &str,
        events: Vec<AgentEvent>,
        cx: &mut Context<Self>,
    ) {
        let is_foreground = self.session_is_foreground(session_id);
        // Refresh the terminal cache right before the model reads it.
        if events.iter().any(|event| {
            matches!(event, AgentEvent::ItemStarted { name: Some(name), .. } if name == "terminal_read")
        }) {
            let snapshot = self.capture_terminal_snapshot(cx);
            if let Some(state) = self.runtime.borrow().session(session_id) {
                state.terminal_context.set(snapshot);
            }
        }
        let mut changes = Vec::new();
        let mut signals = Vec::new();
        {
            let Some(state) = self.runtime.get_mut().session_mut(session_id) else {
                return;
            };
            for event in events {
                let (event_changes, event_signals) = state.apply_engine_event(event);
                changes.extend(event_changes);
                signals.extend(event_signals);
            }
        }
        if is_foreground {
            self.apply_message_changes(changes, cx);
        }
        let chat_label = self
            .session_chat_label(session_id)
            .unwrap_or_else(|| i18n::string("workspace.panel.agent.sidebar_title"));
        let visible = is_foreground
            && self.panel_open()
            && self.session_panel_view(session_id) == Some(ChatPanelView::Conversation);
        for signal in signals {
            match signal {
                EngineSignal::Feedback(text) => {
                    if is_foreground {
                        cx.emit(AppCommand::Feedback(text));
                    }
                }
                EngineSignal::ApprovalRequired { tool_name } => {
                    if visible {
                        cx.emit(AppCommand::Feedback(i18n::string(
                            "workspace.panel.agent.messages.waiting_for_tool_approval",
                        )));
                    } else {
                        self.notify_background_session_agent(
                            chat_label.clone(),
                            SessionAgentBackgroundNotificationKind::ToolApprovalRequired {
                                tool_name,
                            },
                            cx,
                        );
                    }
                }
                EngineSignal::UserInputRequired { tool_name } => {
                    if !visible {
                        self.notify_background_session_agent(
                            chat_label.clone(),
                            SessionAgentBackgroundNotificationKind::UserInputRequired { tool_name },
                            cx,
                        );
                    }
                }
                EngineSignal::TurnFinished(reason) => {
                    self.finish_engine_turn(session_id, reason, visible, &chat_label, cx);
                }
            }
        }
        cx.notify();
    }

    fn finish_engine_turn(
        &mut self,
        session_id: &str,
        reason: TurnEndReason,
        visible: bool,
        chat_label: &str,
        cx: &mut Context<Self>,
    ) {
        let (leases, title_seed, error) = {
            let Some(state) = self.runtime.get_mut().session_mut(session_id) else {
                return;
            };
            let leases = std::mem::take(&mut state.turn_leases);
            let title_seed = (state.title.is_none() && reason == TurnEndReason::Completed)
                .then(|| {
                    state
                        .messages
                        .iter()
                        .find(|message| message.role == SessionAgentMessageRole::User)
                        .map(|message| message.content.clone())
                })
                .flatten();
            let error = state
                .messages
                .iter()
                .rev()
                .take_while(|message| message.role != SessionAgentMessageRole::User)
                .find(|message| message.role == SessionAgentMessageRole::Error)
                .map(|message| message.content.clone());
            (leases, title_seed, error)
        };
        retire_session_agent_pty_leases(&leases);
        self.refresh_chat_sessions(cx);
        let feedback_key = match reason {
            TurnEndReason::Completed => "workspace.panel.agent.reply_ready",
            TurnEndReason::Interrupted => "workspace.panel.agent.messages.stopped",
            TurnEndReason::LimitReached | TurnEndReason::Failed => {
                "workspace.panel.agent.messages.turn_failed"
            }
        };
        if visible {
            cx.emit(AppCommand::Feedback(i18n::string(feedback_key)));
        } else if reason == TurnEndReason::Completed {
            self.notify_background_session_agent(
                chat_label.to_string(),
                SessionAgentBackgroundNotificationKind::ReplyReady,
                cx,
            );
        } else if reason == TurnEndReason::Failed {
            self.notify_background_session_agent(
                chat_label.to_string(),
                SessionAgentBackgroundNotificationKind::StreamFailed {
                    error: error.unwrap_or_default(),
                },
                cx,
            );
        }
        if let Some(seed) = title_seed {
            self.generate_session_title(session_id.to_string(), seed, cx);
        }
    }

    fn apply_message_changes(&mut self, changes: Vec<MessageChange>, cx: &mut Context<Self>) {
        if changes.is_empty() {
            return;
        }
        if changes.contains(&MessageChange::Rebuild) {
            self.reconcile_foreground_conversation_view(cx);
            self.sync_conversation_generating_view(cx);
            return;
        }
        // Collapse the batch per message: the state already contains the
        // final content, so a push or update supersedes any deltas.
        #[derive(PartialEq)]
        enum Strongest {
            Push,
            Update,
            Append(String),
        }
        let mut per_index: BTreeMap<usize, Strongest> = BTreeMap::new();
        let mut generating = false;
        for change in changes {
            match change {
                MessageChange::Pushed(index) => {
                    per_index.insert(index, Strongest::Push);
                }
                MessageChange::Updated(index) => {
                    let entry = per_index.entry(index).or_insert(Strongest::Update);
                    if *entry != Strongest::Push {
                        *entry = Strongest::Update;
                    }
                }
                MessageChange::Appended(index, delta) => match per_index.get_mut(&index) {
                    Some(Strongest::Append(existing)) => existing.push_str(&delta),
                    Some(_) => {}
                    None => {
                        per_index.insert(index, Strongest::Append(delta));
                    }
                },
                MessageChange::Generating => generating = true,
                MessageChange::Rebuild => {}
            }
        }
        let message_count = self.session_agent().messages.len();
        for (index, change) in per_index {
            if index >= message_count {
                continue;
            }
            match change {
                Strongest::Push => {
                    let message = self.session_agent().messages[index].clone();
                    self.push_conversation_message_view(message, cx);
                    self.refresh_conversation_search_message(index, cx);
                }
                Strongest::Update => {
                    self.sync_conversation_message_view(index, cx);
                    self.refresh_conversation_search_message(index, cx);
                }
                Strongest::Append(delta) => {
                    self.append_conversation_message_view_delta(index, &delta, cx);
                    self.schedule_conversation_search_message_refresh(index, cx);
                }
            }
        }
        if generating {
            self.sync_conversation_generating_view(cx);
        }
    }

    pub(in crate::ui) fn stop_session_agent_stream(&mut self, cx: &mut Context<Self>) {
        let sent = self
            .session_agent()
            .engine
            .as_ref()
            .is_some_and(|engine| engine.send(AgentCommand::Interrupt));
        if !sent {
            self.runtime.get_mut().foreground.engine_busy = false;
            self.sync_conversation_generating_view(cx);
        }
        cx.notify();
    }

    fn send_decision(
        &mut self,
        call_id: String,
        decision: ApprovalDecision,
        cx: &mut Context<Self>,
    ) {
        if self
            .approval_input
            .as_ref()
            .is_some_and(|(id, _)| *id == call_id)
        {
            self.approval_input = None;
        }
        let approved = !matches!(decision, ApprovalDecision::Deny { .. });
        let sent = self.session_agent().engine.as_ref().is_some_and(|engine| {
            engine.send(AgentCommand::Decide {
                call_id: call_id.clone(),
                decision,
            })
        });
        if !sent {
            cx.emit(AppCommand::Feedback(i18n::string(
                "workspace.panel.agent.messages.tool_not_found",
            )));
        } else {
            // Reflect the decision right away; the engine reports the result
            // when the call finishes.
            let index = self
                .runtime
                .get_mut()
                .foreground
                .mark_decision_sent(&call_id, approved);
            if let Some(index) = index {
                self.sync_conversation_message_view(index, cx);
            }
            if approved {
                cx.emit(AppCommand::Feedback(i18n::string(
                    "workspace.panel.agent.messages.tool_running",
                )));
            }
        }
        cx.notify();
    }

    pub(in crate::ui) fn approve_session_agent_tool_call(
        &mut self,
        tool_id: String,
        cx: &mut Context<Self>,
    ) {
        self.send_decision(tool_id, ApprovalDecision::Approve, cx);
    }

    /// Approve and auto-approve the same command prefix on the same target
    /// for the rest of this conversation.
    pub(in crate::ui) fn approve_session_agent_tool_call_for_session_scope(
        &mut self,
        tool_id: String,
        cx: &mut Context<Self>,
    ) {
        self.send_decision(tool_id, ApprovalDecision::ApproveAndGrant, cx);
    }

    /// Approve with the command/patch the user edited in the answer field.
    pub(in crate::ui) fn approve_session_agent_tool_call_edited(
        &mut self,
        tool_id: String,
        edited: String,
        cx: &mut Context<Self>,
    ) {
        self.send_decision(tool_id, ApprovalDecision::ApproveEdited { edited }, cx);
    }

    pub(in crate::ui) fn deny_tool_call_with_reason(
        &mut self,
        tool_id: String,
        reason: String,
        cx: &mut Context<Self>,
    ) {
        let reason = (!reason.trim().is_empty()).then_some(reason);
        self.send_decision(tool_id, ApprovalDecision::Deny { reason }, cx);
    }

    /// Which inline input (if any) is open under this approval card.
    pub(in crate::ui) fn approval_input_mode(&self, tool_id: &str) -> Option<ApprovalInputMode> {
        self.approval_input
            .as_ref()
            .filter(|(id, _)| id == tool_id)
            .map(|(_, mode)| *mode)
    }

    pub(in crate::ui) fn begin_approval_input(
        &mut self,
        tool_id: String,
        mode: ApprovalInputMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (value, placeholder) = match mode {
            ApprovalInputMode::Edit => (
                self.pending_approval_details(&tool_id)
                    .and_then(|(_, editable, _)| editable)
                    .unwrap_or_default(),
                "workspace.panel.agent.tool_actions.edit_placeholder",
            ),
            ApprovalInputMode::DenyReason => (
                String::new(),
                "workspace.panel.agent.tool_actions.reason_placeholder",
            ),
        };
        crate::ui::components::forms::set_input_value(
            &self.forms.ask_user_input,
            value,
            window,
            cx,
        );
        set_input_placeholder(
            &self.forms.ask_user_input,
            i18n::string(placeholder),
            window,
            cx,
        );
        self.approval_input = Some((tool_id, mode));
        cx.notify();
    }

    pub(in crate::ui) fn submit_approval_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((tool_id, mode)) = self.approval_input.take() else {
            return;
        };
        let value = self.forms.ask_user_input.read(cx).value().to_string();
        if mode == ApprovalInputMode::Edit && value.trim().is_empty() {
            self.approval_input = Some((tool_id, mode));
            return;
        }
        crate::ui::components::forms::set_input_value(
            &self.forms.ask_user_input,
            String::new(),
            window,
            cx,
        );
        set_input_placeholder(
            &self.forms.ask_user_input,
            i18n::string("workspace.panel.agent.tool_placeholders.custom_answer"),
            window,
            cx,
        );
        match mode {
            ApprovalInputMode::Edit => {
                self.approve_session_agent_tool_call_edited(tool_id, value, cx)
            }
            ApprovalInputMode::DenyReason => self.deny_tool_call_with_reason(tool_id, value, cx),
        }
    }

    /// Grant prefix and editable payload of a pending approval.
    pub(in crate::ui) fn pending_approval_details(
        &self,
        tool_id: &str,
    ) -> Option<(Option<String>, Option<String>, String)> {
        self.session_agent()
            .pending_approval(tool_id)
            .map(|request| {
                (
                    request.grant_prefix.clone(),
                    request.editable.clone(),
                    request.target.clone(),
                )
            })
    }

    pub(in crate::ui) fn submit_active_session_agent_user_answer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = self
            .forms
            .ask_user_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if answer.is_empty() {
            return;
        }
        let Some(tool) = self.session_agent().active_ask_user_tool_call() else {
            return;
        };
        self.submit_session_agent_user_answer(tool.id, answer, None, true, window, cx);
    }

    pub(in crate::ui) fn submit_session_agent_user_answer(
        &mut self,
        tool_id: String,
        answer: String,
        _selected_index: Option<usize>,
        _custom: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sent = self.session_agent().engine.as_ref().is_some_and(|engine| {
            engine.send(AgentCommand::Answer {
                call_id: tool_id,
                answer,
            })
        });
        if sent {
            crate::ui::components::forms::set_input_value(
                &self.forms.ask_user_input,
                String::new(),
                window,
                cx,
            );
        }
        cx.notify();
    }

    pub(in crate::ui) fn notify_background_session_agent(
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
            crate::ui::components::notifications::push_app_notification(window, notification, cx);
        });
    }

    fn generate_session_title(
        &mut self,
        session_id: String,
        first_message: String,
        cx: &mut Context<Self>,
    ) {
        let Some(provider_id) = self.selected_ai_provider_id() else {
            return;
        };
        let Ok(model) = self.model_client(&provider_id) else {
            return;
        };
        let runtime = self.task_runtime.clone();
        cx.spawn(async move |this, cx| {
            let title = runtime
                .spawn(async move {
                    miaominal_agent::engine::generate_title(model.as_ref(), &first_message).await
                })
                .await
                .ok()
                .and_then(Result::ok)
                .filter(|title| !title.trim().is_empty());
            if let Some(title) = title {
                let _ = this.update(cx, move |controller, cx| {
                    controller.update_session_title_for_session(&session_id, Some(title), cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn approval_settings(&self) -> ApprovalSettings {
        ApprovalSettings {
            mode: self.session_agent().agent_mode,
            auto_approve_dangerous: self.settings().agent_auto_approve_dangerous,
        }
    }

    fn model_client(&self, provider_id: &str) -> Result<Arc<dyn ModelClient>, String> {
        let provider = self
            .settings()
            .ai_providers
            .into_iter()
            .find(|provider| provider.id == provider_id && provider.enabled)
            .ok_or_else(|| i18n::string("workspace.panel.agent.provider_missing"))?;
        let api_key = self
            .resolve_ai_provider_api_key(&provider)
            .map_err(|error| error.to_string())?;
        let profile =
            ModelProfile::from_provider(&provider, api_key).map_err(|error| error.to_string())?;
        client_for(profile).map_err(|error| error.to_string())
    }

    fn prepare_turn(
        &mut self,
        provider_id: &str,
        target_names: &[String],
        terminal: Option<Arc<dyn TerminalContextSource>>,
    ) -> Result<PreparedTurn, String> {
        let terminal_available = terminal.is_some();
        let model = self.model_client(provider_id)?;
        let web_search = self.settings().web_search.enabled;
        let mentions = self.resolve_session_agent_mentions(target_names);
        if mentions.pty_busy {
            return Err(i18n::string(
                "workspace.panel.agent.messages.pty_terminal_busy",
            ));
        }
        if !mentions.unresolved.is_empty() {
            retire_session_agent_pty_leases(&mentions.leases);
            let targets = mentions
                .unresolved
                .iter()
                .map(|name| format!("@{name}"))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(i18n::string_args(
                "workspace.panel.agent.messages.unknown_target",
                &[("targets", &targets)],
            ));
        }
        let mut leases = mentions.leases;
        let mut aux_channels = mentions.aux_channels;

        let Some(context) = self.prompt_execution_context() else {
            // No session: the model can still answer and use web tools.
            let tools: Arc<dyn ToolRuntime> =
                Arc::new(ChannelToolRuntime::new(None, "local", terminal));
            retire_session_agent_pty_leases(&leases);
            return Ok(PreparedTurn {
                model,
                tools,
                targets: Vec::new(),
                leases: Vec::new(),
                web_search,
                terminal_available,
            });
        };
        let Some(profile) = self.profile_for_execution_context(&context) else {
            retire_session_agent_pty_leases(&leases);
            return Err(i18n::string(
                "workspace.panel.agent.messages.no_active_session_for_approval",
            ));
        };
        let terminal_lease = match self.acquire_terminal_lease_for_execution_context(&context) {
            Ok(lease) => lease,
            Err(message) => {
                retire_session_agent_pty_leases(&leases);
                return Err(message);
            }
        };
        let active_marker = self.terminal_target_marker_for_execution_context(&context);
        let default_target = active_marker
            .clone()
            .map(|marker| marker.trim_start_matches('@').to_string())
            .unwrap_or_else(|| profile.name.clone());
        let mut channel = self.agent_exec_channel_for_profile(profile);
        if let Some(TerminalLeaseGrant {
            commands,
            output,
            lease,
        }) = terminal_lease
        {
            channel = channel.with_terminal_exec(TerminalExecHandle::new(commands, output));
            leases.push(lease);
        }
        if let Some(marker) = active_marker {
            aux_channels.remove(&marker);
        }
        let channel = channel.with_aux_channels(aux_channels);
        let runtime = ChannelToolRuntime::new(Some(channel), default_target, terminal);
        let targets = runtime.target_infos();
        Ok(PreparedTurn {
            model,
            tools: Arc::new(runtime),
            targets,
            leases,
            web_search,
            terminal_available,
        })
    }

    fn capture_terminal_snapshot(&self, cx: &App) -> Option<TerminalSnapshot> {
        let target = self.session_terminal.active_target()?;
        let (tail, selection) =
            self.session_terminal
                .text_snapshot(target.tab_id, TERMINAL_CACHE_LINES, cx)?;
        Some(TerminalSnapshot {
            target: target.title.clone(),
            title: target.title,
            selection,
            tail,
            cwd: None,
            busy: false,
        })
    }

    pub(super) fn settings(&self) -> miaominal_settings::AppSettings {
        miaominal_settings::current_settings()
    }

    pub(super) fn selected_ai_provider_id(&self) -> Option<String> {
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
        let missing_env = |env: &str| {
            env.trim().is_empty()
                || std::env::var(env.trim())
                    .map(|value| value.trim().is_empty())
                    .unwrap_or(true)
        };
        let provider_requires_saved_key = settings
            .ai_providers
            .iter()
            .find(|provider| provider.id == provider_id && provider.enabled)
            .is_some_and(|provider| provider.has_api_key && missing_env(&provider.api_key_env));
        let web_search_requires_saved_key = settings.web_search.enabled
            && settings.web_search.has_api_key
            && missing_env(&settings.web_search.api_key_env);
        provider_requires_saved_key || web_search_requires_saved_key
    }

    fn agent_exec_channel_for_profile(&self, profile: SessionProfile) -> AgentExecChannel {
        let profiles = self.session_profiles();
        let proxies = self.session_proxies();
        let secrets = self.secrets();
        let mut channel = self
            .agent_service()
            .channel_for_profile_snapshot_with_stores_and_proxies(
                profile,
                &profiles,
                &proxies,
                secrets.clone(),
                self.known_hosts(),
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

    fn resolve_session_agent_mentions(&mut self, targets: &[String]) -> ResolvedMentions {
        let mut resolved = ResolvedMentions::default();
        if targets.is_empty() {
            return resolved;
        }
        let mut resolved_names: Vec<String> = Vec::new();
        let active_terminal_tab_id = self
            .prompt_execution_context()
            .filter(|context| context.exec_mode == AgentExecMode::Pty)
            .and_then(|context| context.terminal_tab_id);
        match self.session_agent().exec_mode {
            AgentExecMode::ExecChannel => {
                for profile in self.session_profiles() {
                    if !targets.iter().any(|target| target == &profile.name)
                        || resolved_names.contains(&profile.name)
                    {
                        continue;
                    }
                    resolved.aux_channels.insert(
                        format!("@{}", profile.name),
                        self.agent_exec_channel_for_profile(profile.clone()),
                    );
                    resolved_names.push(profile.name.clone());
                }
            }
            AgentExecMode::Pty => {
                for target in self.terminal_targets() {
                    if !targets.iter().any(|name| name == &target.title)
                        || resolved_names.contains(&target.title)
                        || !target.command_available
                    {
                        continue;
                    }
                    let Some(profile) = target.profile.clone() else {
                        continue;
                    };
                    if active_terminal_tab_id == Some(target.tab_id) {
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
                            retire_session_agent_pty_leases(&resolved.leases);
                            return ResolvedMentions {
                                pty_busy: true,
                                ..Default::default()
                            };
                        }
                        Err(TerminalLeaseError::Missing | TerminalLeaseError::Disconnected) => {
                            continue;
                        }
                    };
                    let channel = self
                        .agent_exec_channel_for_profile(profile)
                        .with_terminal_exec(TerminalExecHandle::new(commands, output));
                    resolved
                        .aux_channels
                        .insert(format!("@{}", target.title), channel);
                    resolved.leases.push(lease);
                    resolved_names.push(target.title);
                }
            }
        }
        resolved.unresolved = targets
            .iter()
            .filter(|name| {
                !resolved_names.iter().any(|resolved| {
                    resolved == *name
                        || resolved
                            .strip_prefix(name.as_str())
                            .is_some_and(|suffix| suffix.starts_with(' '))
                })
            })
            .cloned()
            .collect();
        resolved
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_terminal_context_serves_snapshots_and_reads() {
        let context = SharedTerminalContext::default();
        assert!(
            context
                .read(TerminalReadRequest {
                    target: None,
                    lines: 5
                })
                .is_none()
        );
        context.set(Some(TerminalSnapshot {
            target: "web".into(),
            title: "web".into(),
            selection: Some("err".into()),
            tail: "a\nb\nc\nd".into(),
            cwd: None,
            busy: false,
        }));
        let read = context
            .read(TerminalReadRequest {
                target: None,
                lines: 2,
            })
            .unwrap();
        assert_eq!(read.text, "c\nd");
        assert!(read.truncated);
        let snapshot = context.snapshot(None, 3).unwrap();
        assert_eq!(snapshot.tail, "b\nc\nd");
        assert_eq!(snapshot.selection.as_deref(), Some("err"));
        assert_eq!(last_lines("x", 10), ("x".to_string(), false));
    }
}
