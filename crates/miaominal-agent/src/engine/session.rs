//! Session task: command routing, the turn loop and tool batches.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use super::compact;
use super::{
    AgentCommand, AgentConfigPatch, AgentDeps, AgentEvent, AgentHandle, AgentSessionConfig,
    ApprovalDecision, ApprovalRequest, ItemHint, SessionStatus, TranscriptSink, TurnEndReason,
};
use crate::context::{PromptOptions, estimate_tokens, system_prompt};
use crate::error::AgentError;
use crate::model::{
    AssistantPart, FinishReason, ModelClient, ModelEvent, ModelRequest, TokenUsage, UserPart,
    WireMessage,
};
use crate::policy::{AgentRiskLevel, ApprovalMode, SessionGrants, Verdict};
use crate::toolkit::{
    ToolClass, ToolInvocation, ToolOutcome, ToolProgress, ToolRuntime, ToolSetOptions, tool_class,
    validate_arguments,
};
use crate::tools::AgentToolCancellation;
use crate::transcript::{
    Item, ItemKind, NoticeLevel, PlanItem, PlanStatus, ProjectionBudget, ToolDisplay,
    ToolResultStatus, new_id, project, repair_dangling_tool_calls,
};

enum Reply {
    Decision(ApprovalDecision),
    Answer(String),
}

enum Work {
    Submit {
        text: String,
        attachments: Vec<miaominal_core::chat_attachment::ChatAttachment>,
        targets: Vec<String>,
        context: Option<String>,
    },
    Compact,
}

#[derive(Default)]
struct Control {
    pending: Mutex<HashMap<String, oneshot::Sender<Reply>>>,
    steer: Mutex<Vec<String>>,
    patch: Mutex<Option<AgentConfigPatch>>,
    cancel: Mutex<Option<AgentToolCancellation>>,
    busy: AtomicBool,
}

impl Control {
    fn merge_patch(&self, patch: AgentConfigPatch) {
        let mut slot = self.patch.lock().unwrap_or_else(|p| p.into_inner());
        let merged = match slot.take() {
            None => patch,
            Some(old) => AgentConfigPatch {
                model: patch.model.or(old.model),
                tools: patch.tools.or(old.tools),
                approval: patch.approval.or(old.approval),
                targets: patch.targets.or(old.targets),
                web_search: patch.web_search.or(old.web_search),
                terminal_available: patch.terminal_available.or(old.terminal_available),
            },
        };
        *slot = Some(merged);
    }

    fn interrupt(&self) {
        if let Some(cancel) = self
            .cancel
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            cancel.cancel();
        }
        // Wake every waiter; dropping the senders resolves them as cancelled.
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }
}

/// Start a session task. `transcript` is the already persisted history.
pub fn spawn_session(
    config: AgentSessionConfig,
    transcript: Vec<Item>,
    deps: AgentDeps,
) -> (AgentHandle, mpsc::UnboundedReceiver<AgentEvent>) {
    let (command_tx, mut command_rx) = mpsc::unbounded_channel::<AgentCommand>();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (work_tx, mut work_rx) = mpsc::unbounded_channel::<Work>();
    let control = Arc::new(Control::default());

    let mut worker = Worker {
        config,
        model: deps.model,
        tools: deps.tools,
        store: deps.store,
        items: transcript,
        grants: SessionGrants::default(),
        control: control.clone(),
        events: event_tx,
        budget: ProjectionBudget::default(),
        invalid_streak: None,
    };

    let spawn = |future: futures::future::BoxFuture<'static, ()>| {
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(future);
            }
            Err(_) => {
                crate::runtime::agent_runtime().spawn(future);
            }
        }
    };

    let router_control = control.clone();
    spawn(Box::pin(async move {
        while let Some(command) = command_rx.recv().await {
            match command {
                AgentCommand::Submit {
                    text,
                    attachments,
                    targets,
                    context,
                } => {
                    let _ = work_tx.send(Work::Submit {
                        text,
                        attachments,
                        targets,
                        context,
                    });
                }
                AgentCommand::Compact => {
                    let _ = work_tx.send(Work::Compact);
                }
                AgentCommand::Decide { call_id, decision } => {
                    let sender = router_control
                        .pending
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&call_id);
                    if let Some(sender) = sender {
                        let _ = sender.send(Reply::Decision(decision));
                    }
                }
                AgentCommand::Answer { call_id, answer } => {
                    let sender = router_control
                        .pending
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&call_id);
                    if let Some(sender) = sender {
                        let _ = sender.send(Reply::Answer(answer));
                    }
                }
                AgentCommand::Interrupt => router_control.interrupt(),
                AgentCommand::Steer { text } => {
                    if router_control.busy.load(Ordering::SeqCst) {
                        router_control
                            .steer
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .push(text);
                    } else {
                        let _ = work_tx.send(Work::Submit {
                            text,
                            attachments: Vec::new(),
                            targets: Vec::new(),
                            context: None,
                        });
                    }
                }
                AgentCommand::UpdateConfig(patch) => router_control.merge_patch(patch),
                AgentCommand::Shutdown => {
                    router_control.interrupt();
                    break;
                }
            }
        }
        router_control.interrupt();
    }));

    spawn(Box::pin(async move {
        while let Some(work) = work_rx.recv().await {
            match work {
                Work::Submit {
                    text,
                    attachments,
                    targets,
                    context,
                } => {
                    worker.run_turn(text, attachments, targets, context).await;
                }
                Work::Compact => worker.manual_compact().await,
            }
        }
    }));

    (
        AgentHandle {
            commands: command_tx,
        },
        event_rx,
    )
}

pub(super) struct Worker {
    pub(super) config: AgentSessionConfig,
    pub(super) model: Arc<dyn ModelClient>,
    tools: Arc<dyn ToolRuntime>,
    store: Arc<dyn TranscriptSink>,
    pub(super) items: Vec<Item>,
    grants: SessionGrants,
    control: Arc<Control>,
    events: mpsc::UnboundedSender<AgentEvent>,
    pub(super) budget: ProjectionBudget,
    invalid_streak: Option<(String, usize)>,
}

/// One streamed part of a completion, kept in model output order.
enum Part {
    Text {
        id: String,
        text: String,
    },
    Reasoning {
        id: String,
        text: String,
        signature: Option<String>,
        redacted: Option<String>,
        started: std::time::Instant,
        duration_ms: Option<u64>,
    },
    Tool {
        id: String,
        index: usize,
        call_id: String,
        name: String,
        arguments: String,
    },
}

impl Part {
    fn id(&self) -> &str {
        match self {
            Part::Text { id, .. } | Part::Reasoning { id, .. } | Part::Tool { id, .. } => id,
        }
    }
}

enum StreamFailure {
    Interrupted,
    Retry(AgentError),
    Fatal(AgentError),
}

pub(super) struct ToolCallRef {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

impl Worker {
    pub(super) fn emit(&self, event: AgentEvent) {
        let _ = self.events.send(event);
    }

    fn status(&self, status: SessionStatus) {
        self.emit(AgentEvent::Status(status));
    }

    pub(super) fn commit(&mut self, items: Vec<Item>) {
        if items.is_empty() {
            return;
        }
        if let Err(error) = self.store.append(&items) {
            log::warn!("failed to persist agent items: {error}");
            self.emit(AgentEvent::Info {
                level: NoticeLevel::Error,
                text: format!("保存会话记录失败：{error}"),
            });
        }
        for item in items {
            self.emit(AgentEvent::ItemCommitted(item.clone()));
            self.items.push(item);
        }
    }

    fn notice(&mut self, turn_id: &str, level: NoticeLevel, text: impl Into<String>) {
        let item = Item::new(
            turn_id,
            ItemKind::Notice {
                level,
                text: text.into(),
            },
        );
        self.commit(vec![item]);
    }

    fn apply_pending_patch(&mut self) {
        let patch = self
            .control
            .patch
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        let Some(patch) = patch else {
            return;
        };
        if let Some(model) = patch.model {
            self.model = model;
        }
        if let Some(tools) = patch.tools {
            self.tools = tools;
        }
        if let Some(approval) = patch.approval {
            self.config.approval = approval;
        }
        if let Some(targets) = patch.targets {
            self.config.targets = targets;
        }
        if let Some(web_search) = patch.web_search {
            self.config.web_search = web_search;
        }
        if let Some(terminal) = patch.terminal_available {
            self.config.terminal_available = terminal;
        }
    }

    pub(super) fn tool_options(&self) -> ToolSetOptions {
        ToolSetOptions {
            mode: Some(self.config.approval.mode),
            has_targets: !self.config.targets.is_empty(),
            target_names: self
                .config
                .targets
                .iter()
                .map(|target| target.name.clone())
                .collect(),
            web_search: self.config.web_search,
            terminal: self.config.terminal_available,
        }
    }

    pub(super) fn system_prompt(&self) -> String {
        system_prompt(
            &self.config.targets,
            PromptOptions {
                mode: self.config.approval.mode,
                web_search: self.config.web_search,
                terminal_available: self.config.terminal_available,
            },
        )
    }

    fn build_request(&self) -> ModelRequest {
        let capabilities = self.model.capabilities();
        ModelRequest {
            system: self.system_prompt(),
            messages: project(&self.items, &capabilities, self.budget),
            tools: self.tools.specs(&self.tool_options()),
        }
    }

    fn begin_turn(&mut self) -> AgentToolCancellation {
        let cancel = AgentToolCancellation::new();
        *self
            .control
            .cancel
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(cancel.clone());
        self.control.busy.store(true, Ordering::SeqCst);
        cancel
    }

    fn end_turn(&mut self) {
        *self
            .control
            .cancel
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
        self.control.busy.store(false, Ordering::SeqCst);
        self.control
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }

    pub(super) async fn run_turn(
        &mut self,
        text: String,
        attachments: Vec<miaominal_core::chat_attachment::ChatAttachment>,
        targets: Vec<String>,
        context: Option<String>,
    ) {
        let turn_id = new_id();
        let cancel = self.begin_turn();
        self.invalid_streak = None;

        // A previous crash or interrupt may have left calls without results.
        let repairs = repair_dangling_tool_calls(&self.items);
        self.commit(repairs);

        let text = if text.trim().is_empty() && attachments.is_empty() {
            "Continue.".to_string()
        } else {
            text
        };
        self.commit(vec![Item::new(
            &turn_id,
            ItemKind::UserMessage {
                text,
                attachments,
                context,
                targets,
            },
        )]);

        let reason = self.turn_loop(&turn_id, &cancel).await;

        let repairs = repair_dangling_tool_calls(&self.items);
        let repairs = repairs
            .into_iter()
            .map(|mut item| {
                item.turn_id = turn_id.clone();
                item
            })
            .collect();
        self.commit(repairs);
        // Steers that arrived after the last model request become the next turn.
        let leftover: Vec<String> =
            std::mem::take(&mut *self.control.steer.lock().unwrap_or_else(|p| p.into_inner()));
        self.end_turn();
        self.status(SessionStatus::Idle);
        self.emit(AgentEvent::TurnFinished {
            turn_id: turn_id.clone(),
            reason,
        });
        if !leftover.is_empty() && reason == TurnEndReason::Completed {
            Box::pin(self.run_turn(leftover.join("\n\n"), Vec::new(), Vec::new(), None)).await;
        } else if !leftover.is_empty() {
            for text in leftover {
                self.commit(vec![Item::new(
                    &turn_id,
                    ItemKind::UserMessage {
                        text,
                        attachments: Vec::new(),
                        context: None,
                        targets: Vec::new(),
                    },
                )]);
            }
        }
    }

    async fn turn_loop(&mut self, turn_id: &str, cancel: &AgentToolCancellation) -> TurnEndReason {
        let max_steps = self.config.limits.max_steps.max(1);
        for step in 0..max_steps {
            self.apply_pending_patch();
            if step == 0 {
                // Facts about the targets go into the static system block,
                // so the model knows the shell syntax from the first step.
                let tools = self.tools.clone();
                let probed = tokio::select! {
                    probed = tools.environment() => probed,
                    _ = cancel.cancelled() => return TurnEndReason::Interrupted,
                };
                if let Some(targets) = probed {
                    self.config.targets = targets;
                }
            }
            let steers: Vec<String> =
                std::mem::take(&mut *self.control.steer.lock().unwrap_or_else(|p| p.into_inner()));
            for text in steers {
                self.commit(vec![Item::new(
                    turn_id,
                    ItemKind::UserMessage {
                        text,
                        attachments: Vec::new(),
                        context: None,
                        targets: Vec::new(),
                    },
                )]);
            }
            if cancel.is_cancelled_runtime() {
                return TurnEndReason::Interrupted;
            }

            self.ensure_budget(turn_id, cancel).await;
            if cancel.is_cancelled_runtime() {
                return TurnEndReason::Interrupted;
            }

            let calls = match self.completion_with_retries(turn_id, cancel).await {
                Ok(calls) => calls,
                Err(reason) => return reason,
            };
            if calls.is_empty() {
                return TurnEndReason::Completed;
            }
            if let Err(reason) = self.run_tools(turn_id, calls, cancel).await {
                return reason;
            }
        }
        self.notice(
            turn_id,
            NoticeLevel::Warning,
            format!("已达到单轮步数上限（{max_steps} 步）。发送“继续”可接着执行。"),
        );
        TurnEndReason::LimitReached
    }

    /// Run the context budget policy (design §7.4) before a request.
    async fn ensure_budget(&mut self, turn_id: &str, cancel: &AgentToolCancellation) {
        let capabilities = self.model.capabilities();
        let window = super::context_window(&capabilities) as f64;
        let limit = (window * self.config.limits.context_budget_ratio) as usize;
        let user_turns = self
            .items
            .iter()
            .filter(|item| matches!(item.kind, ItemKind::UserMessage { .. }))
            .count();

        self.budget = ProjectionBudget::default();
        if self.estimate_request_tokens() <= limit {
            return;
        }
        // 1. Elide tool output older than two turns; keep images of the
        //    latest turn only.
        self.budget = ProjectionBudget {
            elide_tool_results_before_turn: Some(user_turns.saturating_sub(2)),
            drop_images_before_turn: Some(user_turns.saturating_sub(1)),
        };
        if self.estimate_request_tokens() <= limit {
            return;
        }
        // 2. Summarize the oldest half.
        if compact::summarize_oldest(self, turn_id, cancel).await {
            return;
        }
        // 3. Elide everything but the current turn as a last resort.
        self.budget = ProjectionBudget {
            elide_tool_results_before_turn: Some(user_turns.saturating_sub(1)),
            drop_images_before_turn: Some(user_turns.saturating_sub(1)),
        };
    }

    pub(super) fn estimate_request_tokens(&self) -> usize {
        let request = self.build_request();
        let tools: usize = request
            .tools
            .iter()
            .map(|tool| {
                estimate_tokens(&tool.description) + estimate_tokens(&tool.parameters.to_string())
            })
            .sum();
        estimate_tokens(&request.system) + tools + estimate_messages(&request.messages)
    }

    pub(super) async fn manual_compact(&mut self) {
        let turn_id = new_id();
        let cancel = self.begin_turn();
        self.apply_pending_patch();
        if !compact::summarize_oldest(self, &turn_id, &cancel).await {
            self.emit(AgentEvent::Info {
                level: NoticeLevel::Info,
                text: "没有可以压缩的历史。".into(),
            });
        }
        self.end_turn();
        self.status(SessionStatus::Idle);
    }

    /// Stream one completion, retrying transient errors and compacting once
    /// on context overflow. Returns the committed tool calls.
    async fn completion_with_retries(
        &mut self,
        turn_id: &str,
        cancel: &AgentToolCancellation,
    ) -> Result<Vec<ToolCallRef>, TurnEndReason> {
        let max_retries = self.config.limits.max_retries;
        let mut attempt = 0usize;
        let mut compacted = false;
        loop {
            match self.stream_completion(turn_id, cancel).await {
                Ok(calls) => return Ok(calls),
                Err(StreamFailure::Interrupted) => return Err(TurnEndReason::Interrupted),
                Err(StreamFailure::Retry(error)) | Err(StreamFailure::Fatal(error))
                    if is_context_overflow(&error) && !compacted =>
                {
                    compacted = true;
                    self.emit(AgentEvent::Info {
                        level: NoticeLevel::Warning,
                        text: "上下文超出模型限制，正在压缩后重试…".into(),
                    });
                    if !compact::summarize_oldest(self, turn_id, cancel).await {
                        let turns = self
                            .items
                            .iter()
                            .filter(|item| matches!(item.kind, ItemKind::UserMessage { .. }))
                            .count();
                        self.budget = ProjectionBudget {
                            elide_tool_results_before_turn: Some(turns),
                            drop_images_before_turn: Some(turns.saturating_sub(1)),
                        };
                    }
                }
                Err(StreamFailure::Retry(error)) if attempt < max_retries => {
                    attempt += 1;
                    let delay = Duration::from_millis(1000 * (1 << (attempt - 1)));
                    self.emit(AgentEvent::Info {
                        level: NoticeLevel::Warning,
                        text: format!(
                            "模型请求失败（{}），{} 秒后重试（{attempt}/{max_retries}）…",
                            error_text(&error),
                            delay.as_secs()
                        ),
                    });
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = cancel.cancelled() => return Err(TurnEndReason::Interrupted),
                    }
                }
                Err(StreamFailure::Retry(error)) | Err(StreamFailure::Fatal(error)) => {
                    self.notice(
                        turn_id,
                        NoticeLevel::Error,
                        format!("模型请求失败：{}", error_text(&error)),
                    );
                    return Err(TurnEndReason::Failed);
                }
            }
        }
    }

    async fn stream_completion(
        &mut self,
        turn_id: &str,
        cancel: &AgentToolCancellation,
    ) -> Result<Vec<ToolCallRef>, StreamFailure> {
        let request = self.build_request();
        let capabilities = self.model.capabilities();
        self.status(SessionStatus::Streaming);
        let model = self.model.clone();
        let stream = tokio::select! {
            result = model.stream(request) => result,
            _ = cancel.cancelled() => return Err(StreamFailure::Interrupted),
        };
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(error) => return Err(classify_error(error)),
        };

        let completion_id = new_id();
        let mut parts: Vec<Part> = Vec::new();
        let mut usage: Option<TokenUsage> = None;
        let mut finish: Option<FinishReason> = None;

        let failure = loop {
            let event = tokio::select! {
                event = stream.next() => event,
                _ = cancel.cancelled() => break Some(StreamFailure::Interrupted),
            };
            let Some(event) = event else {
                break None;
            };
            let event = match event {
                Ok(event) => event,
                Err(error) => break Some(classify_error(error)),
            };
            match event {
                ModelEvent::TextDelta(delta) => {
                    if delta.is_empty() {
                        continue;
                    }
                    let id = match parts.last_mut() {
                        Some(Part::Text { id, text }) => {
                            text.push_str(&delta);
                            id.clone()
                        }
                        _ => {
                            let id = new_id();
                            parts.push(Part::Text {
                                id: id.clone(),
                                text: delta.clone(),
                            });
                            self.emit(AgentEvent::ItemStarted {
                                item_id: id.clone(),
                                hint: ItemHint::AssistantText,
                                name: None,
                            });
                            id
                        }
                    };
                    self.emit(AgentEvent::TextDelta { item_id: id, delta });
                }
                ModelEvent::ReasoningDelta(delta) => {
                    if delta.is_empty() {
                        continue;
                    }
                    let id = match parts.last_mut() {
                        Some(Part::Reasoning {
                            id,
                            text,
                            signature: None,
                            redacted: None,
                            ..
                        }) => {
                            text.push_str(&delta);
                            id.clone()
                        }
                        _ => {
                            close_reasoning(&mut parts);
                            let id = new_id();
                            parts.push(Part::Reasoning {
                                id: id.clone(),
                                text: delta.clone(),
                                signature: None,
                                redacted: None,
                                started: std::time::Instant::now(),
                                duration_ms: None,
                            });
                            self.emit(AgentEvent::ItemStarted {
                                item_id: id.clone(),
                                hint: ItemHint::Reasoning,
                                name: None,
                            });
                            id
                        }
                    };
                    self.emit(AgentEvent::ReasoningDelta { item_id: id, delta });
                }
                ModelEvent::ReasoningSignature(sig) => match parts.last_mut() {
                    Some(Part::Reasoning {
                        signature,
                        started,
                        duration_ms,
                        ..
                    }) if signature.is_none() => {
                        *signature = Some(sig);
                        *duration_ms = Some(started.elapsed().as_millis() as u64);
                    }
                    _ => {
                        let id = new_id();
                        parts.push(Part::Reasoning {
                            id: id.clone(),
                            text: String::new(),
                            signature: Some(sig),
                            redacted: None,
                            started: std::time::Instant::now(),
                            duration_ms: Some(0),
                        });
                    }
                },
                ModelEvent::RedactedReasoning(data) => {
                    close_reasoning(&mut parts);
                    parts.push(Part::Reasoning {
                        id: new_id(),
                        text: String::new(),
                        signature: None,
                        redacted: Some(data),
                        started: std::time::Instant::now(),
                        duration_ms: Some(0),
                    });
                }
                ModelEvent::ToolCallStart { index, id, name } => {
                    close_reasoning(&mut parts);
                    let item_id = new_id();
                    self.emit(AgentEvent::ItemStarted {
                        item_id: item_id.clone(),
                        hint: ItemHint::ToolCall,
                        name: Some(name.clone()),
                    });
                    parts.push(Part::Tool {
                        id: item_id,
                        index,
                        call_id: if id.is_empty() {
                            format!("call_{}", new_id().replace('-', ""))
                        } else {
                            id
                        },
                        name,
                        arguments: String::new(),
                    });
                }
                ModelEvent::ToolCallArgsDelta { index, delta } => {
                    let found = parts.iter_mut().rev().find_map(|part| match part {
                        Part::Tool {
                            id,
                            index: part_index,
                            arguments,
                            ..
                        } if *part_index == index => {
                            arguments.push_str(&delta);
                            Some(id.clone())
                        }
                        _ => None,
                    });
                    if let Some(item_id) = found {
                        self.emit(AgentEvent::ToolArgumentsDelta { item_id, delta });
                    }
                }
                ModelEvent::Usage(value) => usage = Some(value),
                ModelEvent::Finish(reason) => finish = Some(reason),
            }
        };
        close_reasoning(&mut parts);

        if let Some(failure) = failure {
            match failure {
                StreamFailure::Interrupted => {
                    // Keep what the user already saw; drop unfinished calls.
                    let items =
                        self.parts_to_items(turn_id, &completion_id, parts, false, &capabilities);
                    self.commit(items);
                    return Err(StreamFailure::Interrupted);
                }
                StreamFailure::Retry(error) => {
                    for part in &parts {
                        self.emit(AgentEvent::ItemDiscarded {
                            item_id: part.id().to_string(),
                        });
                    }
                    return Err(StreamFailure::Retry(error));
                }
                StreamFailure::Fatal(error) => {
                    let items =
                        self.parts_to_items(turn_id, &completion_id, parts, false, &capabilities);
                    self.commit(items);
                    return Err(StreamFailure::Fatal(error));
                }
            }
        }

        let truncated = matches!(finish, Some(FinishReason::Length));
        let mut items = self.parts_to_items(turn_id, &completion_id, parts, true, &capabilities);
        let calls: Vec<ToolCallRef> = items
            .iter()
            .filter_map(|item| match &item.kind {
                ItemKind::ToolCall {
                    call_id,
                    name,
                    arguments,
                    ..
                } => Some(ToolCallRef {
                    call_id: call_id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                }),
                _ => None,
            })
            .collect();
        if let Some(usage) = usage {
            let context_used = usage.input_tokens + usage.output_tokens;
            items.push(Item::new(
                turn_id,
                ItemKind::Usage {
                    usage,
                    context_used,
                },
            ));
            self.emit(AgentEvent::Usage {
                input: usage.input_tokens,
                output: usage.output_tokens,
                cached: usage.cached_input_tokens,
                context_used,
                context_window: super::context_window(&capabilities),
            });
        }
        self.commit(items);
        if truncated && calls.is_empty() {
            self.notice(
                turn_id,
                NoticeLevel::Warning,
                "模型输出达到长度上限被截断。发送“继续”可让它接着写。",
            );
        }
        Ok(calls)
    }

    fn parts_to_items(
        &self,
        turn_id: &str,
        completion_id: &str,
        parts: Vec<Part>,
        include_calls: bool,
        capabilities: &crate::model::ModelCapabilities,
    ) -> Vec<Item> {
        let mut items = Vec::new();
        for part in parts {
            let (id, kind) = match part {
                Part::Text { id, text } => (
                    id,
                    ItemKind::AssistantText {
                        text,
                        completion_id: completion_id.to_string(),
                    },
                ),
                Part::Reasoning {
                    id,
                    text,
                    signature,
                    redacted,
                    duration_ms,
                    ..
                } => (
                    id,
                    ItemKind::Reasoning {
                        text,
                        signature,
                        redacted,
                        completion_id: completion_id.to_string(),
                        replay: capabilities.reasoning_replay,
                        duration_ms,
                    },
                ),
                Part::Tool {
                    id,
                    call_id,
                    name,
                    arguments,
                    ..
                } => {
                    if !include_calls {
                        let _ = self.events.send(AgentEvent::ItemDiscarded { item_id: id });
                        continue;
                    }
                    (
                        id,
                        ItemKind::ToolCall {
                            call_id,
                            name,
                            arguments,
                            completion_id: completion_id.to_string(),
                        },
                    )
                }
            };
            let mut item = Item::new(turn_id, kind);
            item.id = id;
            items.push(item);
        }
        items
    }

    async fn wait_reply(&mut self, call_id: &str, cancel: &AgentToolCancellation) -> Option<Reply> {
        let (sender, receiver) = oneshot::channel();
        self.control
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(call_id.to_string(), sender);
        self.status(SessionStatus::WaitingForUser);
        let reply = tokio::select! {
            reply = receiver => reply.ok(),
            _ = cancel.cancelled() => None,
        };
        self.control
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(call_id);
        reply
    }

    fn record_invalid(&mut self, signature: String) -> bool {
        let count = match &mut self.invalid_streak {
            Some((last, count)) if *last == signature => {
                *count += 1;
                *count
            }
            _ => {
                self.invalid_streak = Some((signature, 1));
                1
            }
        };
        count >= self.config.limits.max_repeated_invalid_calls
    }

    async fn run_tools(
        &mut self,
        turn_id: &str,
        calls: Vec<ToolCallRef>,
        cancel: &AgentToolCancellation,
    ) -> Result<(), TurnEndReason> {
        let specs = self.tools.specs(&self.tool_options());
        let read_only_mode = self.config.approval.mode == ApprovalMode::ReadOnly;

        enum Plan {
            Done(ToolOutcome),
            Engine(Value),
            Run(ToolInvocation, ToolClass),
        }

        let mut plans: Vec<(ToolCallRef, Plan)> = Vec::new();
        let mut abort_invalid = false;
        let mut any_valid = false;

        // Phase 1: validate, assess and decide (approvals in model order).
        for call in calls {
            if cancel.is_cancelled_runtime() {
                plans.push((call, Plan::Done(ToolOutcome::cancelled())));
                continue;
            }
            let spec = specs.iter().find(|spec| spec.name == call.name);
            let (Some(spec), Some(class)) = (spec, tool_class(&call.name)) else {
                let names: Vec<&str> = specs.iter().map(|spec| spec.name.as_str()).collect();
                abort_invalid |= self.record_invalid(format!("unknown:{}", call.name));
                let text = format!(
                    "Unknown tool `{}`. Available tools: {}.",
                    call.name,
                    names.join(", ")
                );
                plans.push((call, Plan::Done(ToolOutcome::failed(text))));
                continue;
            };
            let arguments: Value = if call.arguments.trim().is_empty() {
                Value::Object(Default::default())
            } else {
                match serde_json::from_str(&call.arguments) {
                    Ok(value) => value,
                    Err(error) => {
                        abort_invalid |= self.record_invalid(format!("json:{}", call.name));
                        let text = format!(
                            "Invalid arguments for `{}`: not valid JSON ({error}). Send one JSON object that matches the tool schema.",
                            call.name
                        );
                        plans.push((call, Plan::Done(ToolOutcome::failed(text))));
                        continue;
                    }
                }
            };
            if let Err(error) = validate_arguments(spec, &arguments) {
                abort_invalid |= self.record_invalid(format!("schema:{}:{error}", call.name));
                let text = format!(
                    "Invalid arguments for `{}`: {error}. Fix the arguments and call again.",
                    call.name
                );
                plans.push((call, Plan::Done(ToolOutcome::failed(text))));
                continue;
            }
            any_valid = true;
            if class == ToolClass::Engine {
                plans.push((call, Plan::Engine(arguments)));
                continue;
            }
            if read_only_mode && class == ToolClass::FileWrite {
                let outcome = denied(false, Some(
                    "read-only mode does not allow file changes; propose the change to the user instead".into(),
                ));
                plans.push((call, Plan::Done(outcome)));
                continue;
            }

            let mut invocation = ToolInvocation {
                call_id: call.call_id.clone(),
                name: call.name.clone(),
                arguments,
            };
            let assessment = match self.tools.assess(&invocation).await {
                Ok(assessment) => assessment,
                Err(error) => {
                    plans.push((call, Plan::Done(ToolOutcome::failed(error))));
                    continue;
                }
            };
            let mut verdict = self
                .config
                .approval
                .verdict(assessment.risk, &assessment.reason);
            if matches!(verdict, Verdict::Ask { .. })
                && assessment.risk < AgentRiskLevel::L4Forbidden
                && self
                    .grants
                    .is_granted(&assessment.target, assessment.grant_prefix.as_deref())
            {
                verdict = Verdict::Allow;
            }
            match verdict {
                Verdict::Allow => plans.push((call, Plan::Run(invocation, assessment.class))),
                Verdict::Deny { reason } => {
                    plans.push((call, Plan::Done(denied(false, Some(reason)))));
                }
                Verdict::Ask { reason } => {
                    self.emit(AgentEvent::ApprovalRequested(ApprovalRequest {
                        call_id: call.call_id.clone(),
                        tool: call.name.clone(),
                        target: assessment.target.clone(),
                        risk: assessment.risk,
                        reason,
                        summary: assessment.summary.clone(),
                        editable: assessment.editable.clone(),
                        grant_prefix: assessment.grant_prefix.clone(),
                    }));
                    let reply = self.wait_reply(&call.call_id, cancel).await;
                    let plan = match reply {
                        Some(Reply::Decision(ApprovalDecision::Approve)) => {
                            Plan::Run(invocation, assessment.class)
                        }
                        Some(Reply::Decision(ApprovalDecision::ApproveAndGrant)) => {
                            if let Some(prefix) = &assessment.grant_prefix {
                                self.grants.grant(&assessment.target, prefix);
                            }
                            Plan::Run(invocation, assessment.class)
                        }
                        Some(Reply::Decision(ApprovalDecision::ApproveEdited { edited })) => {
                            invocation.arguments = self.tools.apply_edit(&invocation, &edited);
                            match self.tools.assess(&invocation).await {
                                Ok(edited_assessment)
                                    if edited_assessment.risk < AgentRiskLevel::L4Forbidden =>
                                {
                                    Plan::Run(invocation, edited_assessment.class)
                                }
                                Ok(edited_assessment) => Plan::Done(denied(
                                    false,
                                    Some(format!("forbidden: {}", edited_assessment.reason)),
                                )),
                                Err(error) => Plan::Done(ToolOutcome::failed(error)),
                            }
                        }
                        Some(Reply::Decision(ApprovalDecision::Deny { reason })) => {
                            Plan::Done(denied(true, reason))
                        }
                        Some(Reply::Answer(_)) | None => Plan::Done(ToolOutcome::cancelled()),
                    };
                    plans.push((call, plan));
                }
            }
        }
        if any_valid && !abort_invalid {
            self.invalid_streak = None;
        }

        // Phase 2: execute. Consecutive read-only calls run concurrently;
        // side effects run one at a time, in model order.
        self.status(SessionStatus::RunningTools);
        let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<ToolProgress>();
        let events = self.events.clone();
        let forwarder = tokio::spawn(async move {
            while let Some(progress) = progress_rx.recv().await {
                let _ = events.send(AgentEvent::ToolProgress {
                    call_id: progress.call_id,
                    text: progress.text,
                });
            }
        });

        let mut index = 0;
        let mut plans: Vec<(ToolCallRef, Option<Plan>)> = plans
            .into_iter()
            .map(|(call, plan)| (call, Some(plan)))
            .collect();
        while index < plans.len() {
            let is_read = matches!(plans[index].1, Some(Plan::Run(_, ToolClass::ReadOnly)));
            if is_read {
                let mut end = index;
                while end < plans.len()
                    && matches!(plans[end].1, Some(Plan::Run(_, ToolClass::ReadOnly)))
                {
                    end += 1;
                }
                let mut futures = Vec::new();
                for (_, plan) in &mut plans[index..end] {
                    let Some(Plan::Run(invocation, _)) = plan.take() else {
                        unreachable!()
                    };
                    let tools = self.tools.clone();
                    let cancel = cancel.clone();
                    let progress = progress_tx.clone();
                    futures.push(async move {
                        if cancel.is_cancelled_runtime() {
                            return ToolOutcome::cancelled();
                        }
                        tools.execute(invocation, cancel, progress).await
                    });
                }
                let outcomes: Vec<ToolOutcome> = futures::stream::iter(futures)
                    .buffered(self.config.limits.max_parallel_reads.max(1))
                    .collect()
                    .await;
                let mut results = Vec::new();
                for ((call, _), outcome) in plans[index..end].iter().zip(outcomes) {
                    results.push(result_item(turn_id, call, outcome));
                }
                self.commit(results);
                index = end;
                continue;
            }

            let plan = plans[index].1.take();
            let call = ToolCallRef {
                call_id: plans[index].0.call_id.clone(),
                name: plans[index].0.name.clone(),
                arguments: plans[index].0.arguments.clone(),
            };
            let outcome = match plan {
                Some(Plan::Done(outcome)) => outcome,
                Some(Plan::Engine(arguments)) => {
                    if cancel.is_cancelled_runtime() {
                        ToolOutcome::cancelled()
                    } else {
                        let outcome = self.run_engine_tool(&call, arguments, cancel).await;
                        self.status(SessionStatus::RunningTools);
                        outcome
                    }
                }
                Some(Plan::Run(invocation, _)) => {
                    if cancel.is_cancelled_runtime() {
                        ToolOutcome::cancelled()
                    } else {
                        self.tools
                            .execute(invocation, cancel.clone(), progress_tx.clone())
                            .await
                    }
                }
                None => ToolOutcome::cancelled(),
            };
            self.commit(vec![result_item(turn_id, &call, outcome)]);
            index += 1;
        }
        drop(progress_tx);
        let _ = forwarder.await;

        if abort_invalid {
            self.notice(
                turn_id,
                NoticeLevel::Error,
                "模型连续多次发出相同的无效工具调用，本轮已停止。",
            );
            return Err(TurnEndReason::Failed);
        }
        if cancel.is_cancelled_runtime() {
            return Err(TurnEndReason::Interrupted);
        }
        Ok(())
    }

    async fn run_engine_tool(
        &mut self,
        call: &ToolCallRef,
        arguments: Value,
        cancel: &AgentToolCancellation,
    ) -> ToolOutcome {
        match call.name.as_str() {
            "update_plan" => {
                let items: Vec<PlanItem> = arguments
                    .get("items")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| {
                                let step = item.get("step")?.as_str()?.to_string();
                                let status = match item.get("status")?.as_str()? {
                                    "in_progress" => PlanStatus::InProgress,
                                    "completed" => PlanStatus::Completed,
                                    _ => PlanStatus::Pending,
                                };
                                Some(PlanItem { step, status })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                ToolOutcome::ok("Plan updated.", ToolDisplay::Plan { items })
            }
            "ask_user" => {
                let question = arguments
                    .get("question")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let options = arguments
                    .get("choices")
                    .and_then(Value::as_array)
                    .map(|choices| {
                        choices
                            .iter()
                            .filter_map(|choice| choice.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                self.emit(AgentEvent::QuestionAsked {
                    call_id: call.call_id.clone(),
                    question: question.clone(),
                    options,
                });
                match self.wait_reply(&call.call_id, cancel).await {
                    Some(Reply::Answer(answer)) => ToolOutcome::ok(
                        format!("User answered: {answer}"),
                        ToolDisplay::Answer { question, answer },
                    ),
                    Some(Reply::Decision(ApprovalDecision::Deny { reason })) => {
                        denied(true, reason)
                    }
                    _ => ToolOutcome::cancelled(),
                }
            }
            other => ToolOutcome::failed(format!("Tool `{other}` is not an engine tool.")),
        }
    }
}

fn close_reasoning(parts: &mut [Part]) {
    if let Some(Part::Reasoning {
        started,
        duration_ms,
        ..
    }) = parts.last_mut()
        && duration_ms.is_none()
    {
        *duration_ms = Some(started.elapsed().as_millis() as u64);
    }
}

fn denied(by_user: bool, reason: Option<String>) -> ToolOutcome {
    let model_text = if by_user {
        match &reason {
            Some(reason) if !reason.trim().is_empty() => {
                format!("User denied this call. Reason: {reason}")
            }
            _ => "User denied this call.".to_string(),
        }
    } else {
        format!(
            "Denied by policy: {}. Choose a safer approach or explain to the user what is needed.",
            reason.clone().unwrap_or_default()
        )
    };
    ToolOutcome {
        status: ToolResultStatus::Denied { by_user, reason },
        model_text,
        display: ToolDisplay::None,
    }
}

fn result_item(turn_id: &str, call: &ToolCallRef, outcome: ToolOutcome) -> Item {
    Item::new(
        turn_id,
        ItemKind::ToolResult {
            call_id: call.call_id.clone(),
            name: call.name.clone(),
            status: outcome.status,
            model_text: outcome.model_text,
            display: outcome.display,
            elided: false,
        },
    )
}

fn classify_error(error: AgentError) -> StreamFailure {
    match &error {
        AgentError::Cancelled => StreamFailure::Interrupted,
        AgentError::Model(model) if model.retryable || model.context_overflow => {
            StreamFailure::Retry(error)
        }
        _ => StreamFailure::Fatal(error),
    }
}

fn is_context_overflow(error: &AgentError) -> bool {
    matches!(error, AgentError::Model(model) if model.context_overflow)
}

fn error_text(error: &AgentError) -> String {
    match error {
        AgentError::Model(model) => match model.status {
            Some(status) => format!("HTTP {status}: {}", model.message),
            None => model.message.clone(),
        },
        other => other.to_string(),
    }
}

pub(super) fn estimate_messages(messages: &[WireMessage]) -> usize {
    messages
        .iter()
        .map(|message| match message {
            WireMessage::User(parts) => {
                parts
                    .iter()
                    .map(|part| match part {
                        UserPart::Text(text) => estimate_tokens(text),
                        UserPart::Image { .. } => 1000,
                    })
                    .sum::<usize>()
                    + 4
            }
            WireMessage::Assistant(parts) => {
                parts
                    .iter()
                    .map(|part| match part {
                        AssistantPart::Text(text) => estimate_tokens(text),
                        AssistantPart::Reasoning { text, .. } => estimate_tokens(text),
                        AssistantPart::RedactedReasoning(data) => data.len() / 4,
                        AssistantPart::ToolCall {
                            name, arguments, ..
                        } => estimate_tokens(name) + estimate_tokens(arguments) + 8,
                    })
                    .sum::<usize>()
                    + 4
            }
            WireMessage::ToolResult { content, .. } => estimate_tokens(content) + 8,
        })
        .sum()
}
