use super::{
    AgentController, AgentExecMode, SessionAgentExecutionContext, SessionAgentMessage,
    SessionAgentToolCall, SessionAgentToolStatus,
};
use crate::ui::{
    i18n,
    shell::{AppCommand, set_input_value},
};
use gpui::{Context, Window};
use miaominal_agent::AgentMode;
use miaominal_agent::{AgentChatToolEvent, AgentToolCallResponse, BackendRoute, ToolOutput};
use serde_json::Value;

pub(in crate::ui::shell) struct AgentToolApprovalCommit {
    pub(in crate::ui::shell) session_id: String,
    pub(in crate::ui::shell) reasoning: Option<String>,
    pub(in crate::ui::shell) agent_mode: AgentMode,
}

pub(in crate::ui::shell) struct AgentContinuationPreparation {
    pub(in crate::ui::shell) session_id: String,
    pub(in crate::ui::shell) request_id: u64,
    pub(in crate::ui::shell) history_messages: Vec<SessionAgentMessage>,
}

pub(in crate::ui::shell) enum AgentToolExecutionCommit {
    Ignored,
    Stopped,
    FinishedPendingStop,
    Continue { result: String, failed: bool },
}

pub(in crate::ui::shell) struct AgentToolContinuation {
    pub(in crate::ui::shell) tool_call: AgentChatToolEvent,
    pub(in crate::ui::shell) reasoning: Option<String>,
    pub(in crate::ui::shell) result: String,
    pub(in crate::ui::shell) failed: bool,
}

impl AgentController {
    pub(in crate::ui::shell) fn tool_call_for_approval_in_session(
        &self,
        session_id: &str,
        tool_id: &str,
    ) -> Option<SessionAgentToolCall> {
        self.runtime
            .borrow()
            .session(session_id)
            .and_then(|state| state.tool_call(tool_id))
    }

    pub(in crate::ui::shell) fn active_execution_context_for_session(
        &self,
        session_id: &str,
    ) -> Option<SessionAgentExecutionContext> {
        self.runtime
            .borrow()
            .session(session_id)
            .and_then(|state| state.active_exec_context.clone())
    }

    pub(in crate::ui::shell) fn approve_tool_for_execution_in_session(
        &mut self,
        session_id: &str,
        tool_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<AgentToolApprovalCommit> {
        let is_foreground = self.runtime.borrow().session_is_foreground(session_id);
        let (reasoning, agent_mode) = {
            let state = self.runtime.get_mut().session_mut(session_id)?;
            let reasoning = state.reasoning_before_tool_call(tool_id);
            state.tool_call(tool_id)?;
            state.approve_tool_call(tool_id);
            (reasoning, state.agent_mode)
        };
        if is_foreground
            && let Some(index) = self.tool_message_index_for_session(session_id, tool_id)
        {
            self.sync_conversation_message_view(index, cx);
        }
        cx.emit(AppCommand::Feedback(i18n::string(
            "workspace.panel.agent.messages.tool_approved_running",
        )));
        cx.notify();
        Some(AgentToolApprovalCommit {
            session_id: session_id.to_string(),
            reasoning,
            agent_mode,
        })
    }

    pub(in crate::ui::shell) fn begin_tool_execution_for_session(
        &mut self,
        session_id: &str,
    ) -> Option<u64> {
        let state = self.runtime.get_mut().session_mut(session_id)?;
        let request_id = state.next_request_id();
        state.active_request_id = request_id;
        Some(request_id)
    }

    pub(in crate::ui::shell) fn commit_tool_execution_result_for_session(
        &mut self,
        session_id: &str,
        tool_id: &str,
        request_id: u64,
        result: anyhow::Result<String>,
        stop_after_finished: bool,
        cx: &mut Context<Self>,
    ) -> AgentToolExecutionCommit {
        let is_foreground = self.runtime.borrow().session_is_foreground(session_id);
        let (active_request_id, tool_status) = {
            let runtime = self.runtime.borrow();
            let Some(state) = runtime.session(session_id) else {
                return AgentToolExecutionCommit::Ignored;
            };
            (
                state.active_request_id,
                state.tool_call(tool_id).map(|tool_call| tool_call.status),
            )
        };
        if active_request_id != request_id {
            return AgentToolExecutionCommit::Ignored;
        }
        if !matches!(tool_status, Some(SessionAgentToolStatus::InProgress)) {
            self.take_pending_task_for_session(session_id, cx);
            if let Some(state) = self.runtime.get_mut().session_mut(session_id) {
                state.active_request_id = 0;
            }
            if is_foreground {
                self.emit_tool_feedback(i18n::string("workspace.panel.agent.messages.stopped"), cx);
            } else {
                cx.notify();
            }
            return AgentToolExecutionCommit::Stopped;
        }

        let (result, failed, status) = {
            let state = self
                .runtime
                .get_mut()
                .session_mut(session_id)
                .expect("agent session disappeared during tool completion");
            match result {
                Ok(result) => {
                    state.complete_tool_call(tool_id, result.clone());
                    (
                        result,
                        false,
                        i18n::string("workspace.panel.agent.messages.tool_finished_continuing"),
                    )
                }
                Err(error) => {
                    let result = format!("tool failed after approval: {error}");
                    state.fail_tool_call(tool_id, result.clone());
                    (
                        result,
                        true,
                        i18n::string("workspace.panel.agent.messages.tool_failed_continuing"),
                    )
                }
            }
        };
        if is_foreground
            && let Some(index) = self.tool_message_index_for_session(session_id, tool_id)
        {
            self.sync_conversation_message_view(index, cx);
        }
        if is_foreground {
            self.emit_tool_feedback(status, cx);
        } else {
            cx.notify();
        }
        if stop_after_finished {
            return AgentToolExecutionCommit::FinishedPendingStop;
        }
        self.take_pending_task_for_session(session_id, cx);
        if let Some(state) = self.runtime.get_mut().session_mut(session_id) {
            state.active_request_id = 0;
        }
        AgentToolExecutionCommit::Continue { result, failed }
    }

    pub(in crate::ui::shell) fn agent_mode(&self) -> AgentMode {
        self.session_agent().agent_mode
    }

    pub(in crate::ui::shell) fn running_execution_mode(&self) -> AgentExecMode {
        self.session_agent().execution_mode_for_running_tools()
    }

    pub(in crate::ui::shell) fn agent_mode_for_session(
        &self,
        session_id: &str,
    ) -> Option<AgentMode> {
        self.runtime
            .borrow()
            .session(session_id)
            .map(|state| state.agent_mode)
    }

    pub(in crate::ui::shell) fn running_execution_mode_for_session(
        &self,
        session_id: &str,
    ) -> Option<AgentExecMode> {
        self.runtime
            .borrow()
            .session(session_id)
            .map(|state| state.execution_mode_for_running_tools())
    }

    pub(in crate::ui::shell) fn active_target_names_for_session(
        &self,
        session_id: &str,
    ) -> Option<Vec<String>> {
        self.runtime
            .borrow()
            .session(session_id)
            .map(|state| state.active_at_targets.clone())
    }

    pub(in crate::ui::shell) fn ensure_continuation_idle_for_session(
        &mut self,
        session_id: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.has_pending_task_for_session(session_id) {
            return true;
        }
        let is_foreground = self.runtime.borrow().session_is_foreground(session_id);
        let previous_message_count = {
            let Some(state) = self.runtime.get_mut().session_mut(session_id) else {
                return false;
            };
            let previous_message_count = state.messages.len();
            state.push_message_with_enter_motion(SessionAgentMessage::error(i18n::string(
                "workspace.panel.agent.messages.approved_tool_result_skipped",
            )));
            previous_message_count
        };
        if is_foreground {
            self.push_message_views_from(previous_message_count, cx);
        }
        self.emit_tool_feedback(
            i18n::string("workspace.panel.agent.messages.already_processing"),
            cx,
        );
        false
    }

    pub(in crate::ui::shell) fn record_continuation_setup_error_for_session(
        &mut self,
        session_id: &str,
        message: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let is_foreground = self.runtime.borrow().session_is_foreground(session_id);
        let previous_message_count = {
            let Some(state) = self.runtime.get_mut().session_mut(session_id) else {
                return false;
            };
            state.last_error = Some(message.clone());
            let previous_message_count = state.messages.len();
            state.push_message_with_enter_motion(SessionAgentMessage::error(message.clone()));
            previous_message_count
        };
        if is_foreground {
            self.push_message_views_from(previous_message_count, cx);
        }
        self.emit_tool_feedback(message, cx);
        true
    }

    pub(in crate::ui::shell) fn prepare_continuation_request_for_session(
        &mut self,
        session_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<AgentContinuationPreparation> {
        let is_foreground = self.runtime.borrow().session_is_foreground(session_id);
        let thinking_index = self.active_thinking_index_for_session(session_id);
        let (request_id, history_messages, previous_message_count) = {
            let state = self.runtime.get_mut().session_mut(session_id)?;
            let request_id = state.next_request_id();
            let history_messages = state.messages.clone();
            state.active_request_id = request_id;
            state.last_error = None;
            let previous_message_count = state.messages.len();
            state.start_assistant_reply();
            (request_id, history_messages, previous_message_count)
        };
        if is_foreground {
            if let Some(index) = thinking_index {
                self.sync_conversation_message_view(index, cx);
            }
            self.push_message_views_from(previous_message_count, cx);
        }
        Some(AgentContinuationPreparation {
            session_id: session_id.to_string(),
            request_id,
            history_messages,
        })
    }

    pub(in crate::ui::shell) fn prepare_active_user_answer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AgentToolContinuation> {
        let answer = self
            .forms
            .ask_user_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let Some(tool_call) = self.session_agent().active_ask_user_tool_call() else {
            self.emit_tool_feedback(
                i18n::string("workspace.panel.agent.messages.tool_not_found"),
                cx,
            );
            return None;
        };
        self.prepare_user_answer(tool_call.id, answer, None, true, window, cx)
    }

    pub(in crate::ui::shell) fn prepare_user_answer(
        &mut self,
        tool_id: String,
        answer: String,
        selected_index: Option<usize>,
        custom: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AgentToolContinuation> {
        let answer = answer.trim().to_string();
        if answer.is_empty() {
            self.emit_tool_feedback(
                i18n::string("workspace.panel.agent.messages.answer_required"),
                cx,
            );
            return None;
        }

        let Some(tool_call) = self.session_agent().tool_call(&tool_id) else {
            self.emit_tool_feedback(
                i18n::string("workspace.panel.agent.messages.tool_not_found"),
                cx,
            );
            return None;
        };
        if tool_call.name != "ask_user"
            || tool_call.status != SessionAgentToolStatus::WaitingForConfirmation
        {
            self.emit_tool_feedback(
                i18n::string("workspace.panel.agent.messages.tool_not_found"),
                cx,
            );
            return None;
        }

        let arguments = parse_tool_arguments(&tool_call.arguments);
        let operation_hash = arguments
            .get("operation_hash")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let reasoning = self.session_agent().reasoning_before_tool_call(&tool_id);
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
        let result = match serde_json::to_string(&response) {
            Ok(result) => result,
            Err(error) => {
                self.emit_tool_feedback(error.to_string(), cx);
                return None;
            }
        };

        self.session_agent_mut()
            .complete_tool_call(&tool_id, result.clone());
        if let Some(index) = self.tool_message_index(&tool_id) {
            self.sync_conversation_message_view(index, cx);
        }
        set_input_value(&self.forms.ask_user_input, String::new(), window, cx);
        self.emit_tool_feedback(
            i18n::string("workspace.panel.agent.messages.user_answer_sent"),
            cx,
        );

        Some(AgentToolContinuation {
            tool_call: AgentChatToolEvent {
                id: tool_id,
                name: tool_name,
                arguments: tool_arguments,
            },
            reasoning,
            result,
            failed: false,
        })
    }

    fn emit_tool_feedback(&mut self, message: String, cx: &mut Context<Self>) {
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }
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
