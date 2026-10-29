//! Bridge between the agent engine and the panel: projects engine items and
//! events onto [`SessionAgentState`] messages. The engine owns the loop,
//! approvals and persistence; the UI only translates actions into
//! [`AgentCommand`]s and renders what the engine reports.

use miaominal_agent::engine::{
    AgentCommand, AgentEvent, AgentHandle, ApprovalRequest, ItemHint, SessionStatus, TurnEndReason,
};
use miaominal_agent::policy::AgentRiskLevel;
use miaominal_agent::transcript::{Item, ItemKind, NoticeLevel, ToolDisplay, ToolResultStatus};
use serde_json::{Value, json};

use super::{
    SessionAgentMessage, SessionAgentMessageRole, SessionAgentState, SessionAgentThinking,
    SessionAgentToolCall, SessionAgentToolStatus,
};
use crate::ui::i18n;

/// A running engine session owned by one conversation.
pub(in crate::ui) struct SessionEngine {
    pub(in crate::ui) handle: AgentHandle,
    pub(super) _pump: gpui_kit::Task<()>,
}

impl SessionEngine {
    pub(in crate::ui) fn send(&self, command: AgentCommand) -> bool {
        self.handle.send(command)
    }
}

impl Drop for SessionEngine {
    fn drop(&mut self) {
        self.handle.send(AgentCommand::Shutdown);
    }
}

/// How the conversation view must change after applying an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) enum MessageChange {
    /// New messages were appended starting at this index.
    Pushed(usize),
    Updated(usize),
    Appended(usize, String),
    /// Indices shifted; rebuild the projection.
    Rebuild,
    /// Busy/idle changed.
    Generating,
}

/// Side effects the controller performs after state changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) enum EngineSignal {
    Feedback(String),
    ApprovalRequired { tool_name: String },
    UserInputRequired { tool_name: String },
    TurnFinished(TurnEndReason),
}

/// Engine tool names → names the existing tool views understand.
pub(in crate::ui) fn display_tool_name(name: &str) -> String {
    match name {
        "shell" => "run_shell",
        "read_file" => "read",
        "list_dir" => "list",
        "host_info" => "workspace_info",
        other => other,
    }
    .to_string()
}

/// Engine arguments → the argument shape the tool views parse.
pub(in crate::ui) fn display_tool_arguments(name: &str, arguments: &str) -> String {
    if name != "ask_user" {
        return arguments.to_string();
    }
    let Ok(value) = serde_json::from_str::<Value>(arguments) else {
        return arguments.to_string();
    };
    let question = value
        .get("question")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let choices: Vec<Value> = value
        .get("choices")
        .and_then(Value::as_array)
        .map(|choices| {
            choices
                .iter()
                .filter_map(Value::as_str)
                .map(|label| json!({ "label": label }))
                .collect()
        })
        .unwrap_or_default();
    json!({ "message": question, "choices": choices, "allow_custom": true }).to_string()
}

fn tool_status(status: &ToolResultStatus) -> SessionAgentToolStatus {
    match status {
        ToolResultStatus::Ok => SessionAgentToolStatus::Completed,
        ToolResultStatus::Failed | ToolResultStatus::TimedOut => SessionAgentToolStatus::Failed,
        ToolResultStatus::Denied { .. } | ToolResultStatus::Cancelled => {
            SessionAgentToolStatus::Rejected
        }
    }
}

fn result_note(status: &ToolResultStatus, model_text: &str, display: &ToolDisplay) -> String {
    match status {
        ToolResultStatus::Cancelled => {
            return i18n::string("workspace.panel.agent.messages.stopped_by_user");
        }
        ToolResultStatus::Denied {
            by_user: true,
            reason,
        } => {
            let denied = i18n::string("workspace.panel.agent.messages.tool_denied");
            return match reason {
                Some(reason) if !reason.trim().is_empty() => format!("{denied}：{reason}"),
                _ => denied,
            };
        }
        _ => {}
    }
    match display {
        ToolDisplay::Shell {
            exit_code,
            stdout,
            stderr,
            timed_out,
            ..
        } => {
            let mut text = String::new();
            if *timed_out {
                text.push_str("timed out\n");
            } else if let Some(code) = exit_code {
                text.push_str(&format!("exit_code: {code}\n"));
            }
            if !stdout.trim().is_empty() {
                text.push_str(stdout.trim_end());
                text.push('\n');
            }
            if !stderr.trim().is_empty() {
                text.push_str(stderr.trim_end());
            }
            text.trim_end().to_string()
        }
        ToolDisplay::Diff { diff, .. } => diff.clone(),
        _ => model_text.to_string(),
    }
}

fn tool_message(call_id: &str, name: &str, arguments: &str) -> SessionAgentMessage {
    let display_name = display_tool_name(name);
    let arguments = display_tool_arguments(name, arguments);
    let summary = if arguments.trim().is_empty() {
        "No arguments".to_string()
    } else {
        arguments.clone()
    };
    SessionAgentMessage::tool_call(SessionAgentToolCall {
        id: call_id.to_string(),
        name: display_name,
        arguments: summary.clone(),
        summary,
        status: SessionAgentToolStatus::Pending,
        requires_confirmation: false,
        confirmation_note: None,
        expanded: false,
    })
}

/// Project persisted items into panel messages (history load).
#[cfg(test)]
pub(in crate::ui) fn messages_from_items(items: &[Item]) -> Vec<SessionAgentMessage> {
    let mut state = SessionAgentState::default();
    for item in items {
        state.apply_committed_item(item.clone(), false);
    }
    state.messages
}

impl SessionAgentState {
    pub(in crate::ui) fn engine_message_index(&self, item_id: &str) -> Option<usize> {
        self.item_messages.get(item_id).copied()
    }

    fn call_message_index(&self, call_id: &str) -> Option<usize> {
        self.call_messages.get(call_id).copied().or_else(|| {
            self.messages.iter().rposition(|message| {
                message
                    .tool_call
                    .as_ref()
                    .is_some_and(|tool| tool.id == call_id)
            })
        })
    }

    fn push_engine_message(
        &mut self,
        item_id: Option<&str>,
        message: SessionAgentMessage,
        animate: bool,
    ) -> usize {
        if animate {
            self.push_message_with_enter_motion(message);
        } else {
            self.messages.push(message);
        }
        let index = self.messages.len() - 1;
        if let Some(item_id) = item_id {
            self.item_messages.insert(item_id.to_string(), index);
        }
        index
    }

    fn remove_engine_message(&mut self, index: usize) {
        if index >= self.messages.len() {
            return;
        }
        self.messages.remove(index);
        for map in [&mut self.item_messages, &mut self.call_messages] {
            map.retain(|_, value| *value != index);
            for value in map.values_mut() {
                if *value > index {
                    *value -= 1;
                }
            }
        }
    }

    fn finish_thinking_at(&mut self, index: usize, duration_ms: Option<u64>) {
        if let Some(thinking) = self
            .messages
            .get_mut(index)
            .and_then(|message| message.thinking.as_mut())
            && thinking.elapsed_ms.is_none()
        {
            thinking.elapsed_ms = Some(
                duration_ms
                    .map(u128::from)
                    .unwrap_or_else(|| thinking.started_at.elapsed().as_millis()),
            );
        }
    }

    /// Apply a persisted item. `live` items replace their streamed
    /// placeholder; history items are appended without animation.
    pub(in crate::ui) fn apply_committed_item(
        &mut self,
        item: Item,
        live: bool,
    ) -> Vec<MessageChange> {
        let existing = self.item_messages.get(&item.id).copied();
        let mut changes = Vec::new();
        match item.kind {
            ItemKind::UserMessage {
                text, attachments, ..
            } => {
                let message = if attachments.is_empty() {
                    SessionAgentMessage::user(text)
                } else {
                    SessionAgentMessage::user_with_attachments(text, attachments)
                };
                let index = self.push_engine_message(Some(&item.id), message, live);
                changes.push(MessageChange::Pushed(index));
            }
            ItemKind::AssistantText { text, .. } => match existing {
                Some(index) => {
                    if let Some(message) = self.messages.get_mut(index) {
                        message.content = text;
                    }
                    changes.push(MessageChange::Updated(index));
                }
                None if !text.trim().is_empty() => {
                    let index = self.push_engine_message(
                        Some(&item.id),
                        SessionAgentMessage::assistant_raw(text),
                        live,
                    );
                    changes.push(MessageChange::Pushed(index));
                }
                None => {}
            },
            ItemKind::Reasoning {
                text, duration_ms, ..
            } => match existing {
                Some(index) => {
                    if text.trim().is_empty() {
                        self.remove_engine_message(index);
                        changes.push(MessageChange::Rebuild);
                    } else {
                        if let Some(message) = self.messages.get_mut(index) {
                            message.content = text;
                        }
                        self.finish_thinking_at(index, duration_ms);
                        changes.push(MessageChange::Updated(index));
                    }
                }
                None if !text.trim().is_empty() => {
                    let mut message = SessionAgentMessage::thinking_from_history(text);
                    if live && let Some(thinking) = message.thinking.as_mut() {
                        thinking.elapsed_ms = duration_ms.map(u128::from).or(Some(0));
                    }
                    let index = self.push_engine_message(Some(&item.id), message, live);
                    changes.push(MessageChange::Pushed(index));
                }
                None => {}
            },
            ItemKind::ToolCall {
                call_id,
                name,
                arguments,
                ..
            } => {
                let fresh = tool_message(&call_id, &name, &arguments);
                let index = match existing {
                    Some(index) => {
                        if let Some(message) = self.messages.get_mut(index) {
                            let expanded =
                                message.tool_call.as_ref().is_some_and(|tool| tool.expanded);
                            *message = SessionAgentMessage {
                                motion: message.motion,
                                ..fresh
                            };
                            if let Some(tool) = message.tool_call.as_mut() {
                                tool.expanded = expanded;
                            }
                        }
                        changes.push(MessageChange::Updated(index));
                        index
                    }
                    None => {
                        let index = self.push_engine_message(Some(&item.id), fresh, live);
                        changes.push(MessageChange::Pushed(index));
                        index
                    }
                };
                self.call_messages.insert(call_id, index);
            }
            ItemKind::ToolResult {
                call_id,
                name,
                status,
                model_text,
                display,
                ..
            } => {
                self.pending_approvals.remove(&call_id);
                let index = match self.call_message_index(&call_id) {
                    Some(index) => index,
                    None => {
                        let index =
                            self.push_engine_message(None, tool_message(&call_id, &name, ""), live);
                        self.call_messages.insert(call_id.clone(), index);
                        changes.push(MessageChange::Pushed(index));
                        index
                    }
                };
                if let Some(tool) = self
                    .messages
                    .get_mut(index)
                    .and_then(|message| message.tool_call.as_mut())
                {
                    tool.status = tool_status(&status);
                    tool.requires_confirmation = false;
                    let note = result_note(&status, &model_text, &display);
                    tool.confirmation_note = (!note.trim().is_empty()).then_some(note);
                }
                changes.push(MessageChange::Updated(index));
            }
            ItemKind::Notice { level, text } => {
                let message = match level {
                    NoticeLevel::Error | NoticeLevel::Warning => SessionAgentMessage::error(text),
                    NoticeLevel::Info | NoticeLevel::Legacy => {
                        SessionAgentMessage::assistant_raw(text)
                    }
                };
                let index = self.push_engine_message(Some(&item.id), message, live);
                changes.push(MessageChange::Pushed(index));
            }
            ItemKind::Usage { usage, .. } => {
                self.last_usage = Some(usage);
            }
            ItemKind::Summary { .. } => {}
        }
        changes
    }

    /// Apply one engine event; returns view changes and controller signals.
    pub(in crate::ui) fn apply_engine_event(
        &mut self,
        event: AgentEvent,
    ) -> (Vec<MessageChange>, Vec<EngineSignal>) {
        let mut changes = Vec::new();
        let mut signals = Vec::new();
        match event {
            AgentEvent::ItemStarted {
                item_id,
                hint,
                name,
            } => {
                let message = match hint {
                    ItemHint::AssistantText => SessionAgentMessage::assistant_raw(""),
                    ItemHint::Reasoning => SessionAgentMessage {
                        thinking: Some(SessionAgentThinking {
                            started_at: std::time::Instant::now(),
                            elapsed_ms: None,
                            expanded: false,
                        }),
                        ..SessionAgentMessage::thinking_raw("")
                    },
                    ItemHint::ToolCall => {
                        tool_message(&item_id, name.as_deref().unwrap_or_default(), "")
                    }
                };
                // Finish a thinking block that is still running.
                if let Some(index) = self.messages.len().checked_sub(1)
                    && self.messages[index].role == SessionAgentMessageRole::Thinking
                    && self.messages[index]
                        .thinking
                        .as_ref()
                        .is_some_and(|thinking| thinking.elapsed_ms.is_none())
                {
                    self.finish_thinking_at(index, None);
                    changes.push(MessageChange::Updated(index));
                }
                let index = self.push_engine_message(Some(&item_id), message, true);
                changes.push(MessageChange::Pushed(index));
            }
            AgentEvent::TextDelta { item_id, delta }
            | AgentEvent::ReasoningDelta { item_id, delta } => {
                if let Some(index) = self.engine_message_index(&item_id)
                    && let Some(message) = self.messages.get_mut(index)
                {
                    message.content.push_str(&delta);
                    changes.push(MessageChange::Appended(index, delta));
                }
            }
            AgentEvent::ToolArgumentsDelta { item_id, delta } => {
                if let Some(index) = self.engine_message_index(&item_id)
                    && let Some(tool) = self
                        .messages
                        .get_mut(index)
                        .and_then(|message| message.tool_call.as_mut())
                {
                    if tool.summary == "No arguments" {
                        tool.summary.clear();
                        tool.arguments.clear();
                    }
                    tool.summary.push_str(&delta);
                    tool.arguments.push_str(&delta);
                    changes.push(MessageChange::Updated(index));
                }
            }
            AgentEvent::ItemDiscarded { item_id } => {
                if let Some(index) = self.item_messages.get(&item_id).copied() {
                    self.remove_engine_message(index);
                    changes.push(MessageChange::Rebuild);
                }
            }
            AgentEvent::ItemCommitted(item) => {
                changes.extend(self.apply_committed_item(item, true));
            }
            AgentEvent::ApprovalRequested(request) => {
                if let Some(index) = self.call_message_index(&request.call_id)
                    && let Some(tool) = self
                        .messages
                        .get_mut(index)
                        .and_then(|message| message.tool_call.as_mut())
                {
                    tool.status = SessionAgentToolStatus::WaitingForConfirmation;
                    tool.requires_confirmation = true;
                    tool.confirmation_note = Some(approval_note(&request));
                    tool.expanded = true;
                    signals.push(EngineSignal::ApprovalRequired {
                        tool_name: tool.name.clone(),
                    });
                    changes.push(MessageChange::Updated(index));
                }
                self.pending_approvals
                    .insert(request.call_id.clone(), request);
            }
            AgentEvent::QuestionAsked { call_id, .. } => {
                if let Some(index) = self.call_message_index(&call_id)
                    && let Some(tool) = self
                        .messages
                        .get_mut(index)
                        .and_then(|message| message.tool_call.as_mut())
                {
                    tool.status = SessionAgentToolStatus::WaitingForConfirmation;
                    tool.expanded = true;
                    signals.push(EngineSignal::UserInputRequired {
                        tool_name: tool.name.clone(),
                    });
                    changes.push(MessageChange::Updated(index));
                }
            }
            AgentEvent::ToolProgress { .. } => {}
            AgentEvent::Usage {
                context_used,
                context_window,
                ..
            } => {
                self.context_usage = Some((context_used, context_window));
            }
            AgentEvent::TurnFinished { reason, .. } => {
                self.engine_busy = false;
                for index in 0..self.messages.len() {
                    if self.messages[index]
                        .thinking
                        .as_ref()
                        .is_some_and(|thinking| thinking.elapsed_ms.is_none())
                    {
                        self.finish_thinking_at(index, None);
                        changes.push(MessageChange::Updated(index));
                    }
                }
                self.pending_approvals.clear();
                self.active_exec_context = None;
                changes.push(MessageChange::Generating);
                signals.push(EngineSignal::TurnFinished(reason));
            }
            AgentEvent::Status(status) => {
                let busy = status != SessionStatus::Idle;
                if busy != self.engine_busy {
                    self.engine_busy = busy;
                    changes.push(MessageChange::Generating);
                }
                if status == SessionStatus::RunningTools {
                    for index in 0..self.messages.len() {
                        if let Some(tool) = self.messages[index].tool_call.as_mut()
                            && tool.status == SessionAgentToolStatus::Pending
                            && self.call_messages.values().any(|value| *value == index)
                        {
                            tool.status = SessionAgentToolStatus::InProgress;
                            changes.push(MessageChange::Updated(index));
                        }
                    }
                }
            }
            AgentEvent::Info { text, .. } => signals.push(EngineSignal::Feedback(text)),
        }
        (changes, signals)
    }

    /// The pending approval for a tool call, if any.
    pub(in crate::ui) fn pending_approval(&self, call_id: &str) -> Option<&ApprovalRequest> {
        self.pending_approvals.get(call_id)
    }

    /// The user decided on an approval card: hide the buttons and show the
    /// call as running (approved) or rejected until the result arrives.
    pub(in crate::ui) fn mark_decision_sent(
        &mut self,
        call_id: &str,
        approved: bool,
    ) -> Option<usize> {
        self.pending_approvals.remove(call_id);
        let index = self.call_message_index(call_id)?;
        let tool = self.messages.get_mut(index)?.tool_call.as_mut()?;
        tool.requires_confirmation = false;
        tool.confirmation_note = None;
        tool.status = if approved {
            SessionAgentToolStatus::InProgress
        } else {
            SessionAgentToolStatus::Rejected
        };
        Some(index)
    }
}

fn approval_note(request: &ApprovalRequest) -> String {
    let level = match request.risk {
        AgentRiskLevel::L0ReadOnly => "L0",
        AgentRiskLevel::L1LowMutation => "L1",
        AgentRiskLevel::L2ServiceImpacting => "L2",
        AgentRiskLevel::L3Dangerous => "L3",
        AgentRiskLevel::L4Forbidden => "L4",
    };
    i18n::string_args(
        "workspace.panel.agent.messages.approval_needed",
        &[("level", level), ("reason", &request.reason)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_agent::transcript::Item;

    fn committed(kind: ItemKind) -> Item {
        Item::new("t", kind)
    }

    #[test]
    fn streamed_items_are_replaced_on_commit() {
        let mut state = SessionAgentState::default();
        let (changes, _) = state.apply_engine_event(AgentEvent::ItemStarted {
            item_id: "a".into(),
            hint: ItemHint::AssistantText,
            name: None,
        });
        assert_eq!(changes, vec![MessageChange::Pushed(0)]);
        state.apply_engine_event(AgentEvent::TextDelta {
            item_id: "a".into(),
            delta: "hel".into(),
        });
        assert_eq!(state.messages[0].content, "hel");
        let mut item = committed(ItemKind::AssistantText {
            text: "hello".into(),
            completion_id: "c".into(),
        });
        item.id = "a".into();
        let (changes, _) = state.apply_engine_event(AgentEvent::ItemCommitted(item));
        assert_eq!(changes, vec![MessageChange::Updated(0)]);
        assert_eq!(state.messages[0].content, "hello");
        assert_eq!(state.messages.len(), 1);
    }

    #[test]
    fn tool_lifecycle_maps_to_tool_card_states() {
        let mut state = SessionAgentState::default();
        state.apply_engine_event(AgentEvent::ItemStarted {
            item_id: "i".into(),
            hint: ItemHint::ToolCall,
            name: Some("shell".into()),
        });
        let mut call = committed(ItemKind::ToolCall {
            call_id: "c1".into(),
            name: "shell".into(),
            arguments: "{\"command\":\"systemctl restart nginx\"}".into(),
            completion_id: "x".into(),
        });
        call.id = "i".into();
        state.apply_engine_event(AgentEvent::ItemCommitted(call));
        let tool = state.messages[0].tool_call.clone().unwrap();
        assert_eq!(tool.id, "c1");
        assert_eq!(tool.name, "run_shell");

        let (_, signals) =
            state.apply_engine_event(AgentEvent::ApprovalRequested(ApprovalRequest {
                call_id: "c1".into(),
                tool: "shell".into(),
                target: "web".into(),
                risk: AgentRiskLevel::L2ServiceImpacting,
                reason: "changes service state".into(),
                summary: "systemctl restart nginx".into(),
                editable: Some("systemctl restart nginx".into()),
                grant_prefix: Some("systemctl restart".into()),
            }));
        assert!(matches!(signals[0], EngineSignal::ApprovalRequired { .. }));
        assert!(state.has_tool_call_waiting_for_confirmation());
        assert!(state.pending_approval("c1").is_some());

        // Approving hides the buttons immediately and shows the call running.
        assert_eq!(state.mark_decision_sent("c1", true), Some(0));
        let tool = state.messages[0].tool_call.clone().unwrap();
        assert_eq!(tool.status, SessionAgentToolStatus::InProgress);
        assert!(!tool.requires_confirmation);
        assert!(!state.has_tool_call_waiting_for_confirmation());
        assert!(state.pending_approval("c1").is_none());

        state.apply_engine_event(AgentEvent::ItemCommitted(committed(ItemKind::ToolResult {
            call_id: "c1".into(),
            name: "shell".into(),
            status: ToolResultStatus::Denied {
                by_user: true,
                reason: None,
            },
            model_text: "User denied this call.".into(),
            display: ToolDisplay::None,
            elided: false,
        })));
        let tool = state.messages[0].tool_call.clone().unwrap();
        assert_eq!(tool.status, SessionAgentToolStatus::Rejected);
        assert!(state.pending_approval("c1").is_none());
    }

    #[test]
    fn discarded_items_are_removed_and_indices_shift() {
        let mut state = SessionAgentState::default();
        for id in ["a", "b"] {
            state.apply_engine_event(AgentEvent::ItemStarted {
                item_id: id.into(),
                hint: ItemHint::AssistantText,
                name: None,
            });
        }
        let (changes, _) = state.apply_engine_event(AgentEvent::ItemDiscarded {
            item_id: "a".into(),
        });
        assert_eq!(changes, vec![MessageChange::Rebuild]);
        assert_eq!(state.engine_message_index("b"), Some(0));
    }

    #[test]
    fn history_projection_and_ask_user_arguments() {
        let items = vec![
            committed(ItemKind::UserMessage {
                text: "hi".into(),
                attachments: vec![],
                context: None,
                targets: vec![],
            }),
            committed(ItemKind::ToolCall {
                call_id: "q".into(),
                name: "ask_user".into(),
                arguments: "{\"question\":\"Which?\",\"choices\":[\"a\",\"b\"]}".into(),
                completion_id: "c".into(),
            }),
            committed(ItemKind::Notice {
                level: NoticeLevel::Error,
                text: "boom".into(),
            }),
        ];
        let messages = messages_from_items(&items);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, SessionAgentMessageRole::User);
        let tool = messages[1].tool_call.as_ref().unwrap();
        let args: Value = serde_json::from_str(&tool.arguments).unwrap();
        assert_eq!(args["message"], "Which?");
        assert_eq!(args["choices"][1]["label"], "b");
        assert_eq!(messages[2].role, SessionAgentMessageRole::Error);
    }
}
