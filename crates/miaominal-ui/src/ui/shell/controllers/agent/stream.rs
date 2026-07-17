use super::{AgentController, SessionAgentMessageRole, SessionAgentState, SessionAgentToolStatus};
use crate::ui::i18n;
use crate::ui::shell::AppCommand;
use gpui::Context;
use miaominal_agent::{AgentChatEvent, AgentMode, AgentToolCancellation};
use tokio::sync::watch;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum SessionAgentBackgroundNotificationKind {
    ToolApprovalRequired { tool_name: String },
    UserInputRequired { tool_name: String },
    ReplyReady,
    StreamFailed { error: String },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::ui::shell) enum AgentStreamFollowUp {
    #[default]
    None,
    ApproveTool {
        tool_id: String,
    },
    FinishStream,
    FinishReply,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::ui::shell) struct AgentStreamMutationOutcome {
    pub(in crate::ui::shell) status_message: Option<String>,
    pub(in crate::ui::shell) notification: Option<SessionAgentBackgroundNotificationKind>,
    pub(in crate::ui::shell) follow_up: AgentStreamFollowUp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AgentStreamProjection {
    SyncMessage(usize),
    PushMessagesFrom(usize),
    AppendMessageDelta { index: usize, delta: String },
    ScheduleSearchRefresh(usize),
    RefreshSearchMessage(usize),
    SyncGenerating,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct AgentStreamEventApplication {
    outcome: AgentStreamMutationOutcome,
    projections: Vec<AgentStreamProjection>,
}

impl AgentController {
    pub(in crate::ui::shell) fn install_pending_task_for_session(
        &mut self,
        session_id: &str,
        task: gpui::Task<()>,
        stop: watch::Sender<bool>,
        agent_cancellation: Option<AgentToolCancellation>,
        cx: &mut Context<Self>,
    ) -> bool {
        let is_foreground = self.runtime.borrow().session_is_foreground(session_id);
        let Some(state) = self.runtime.get_mut().session_mut(session_id) else {
            return false;
        };
        state.pending_stream_stop = Some(stop);
        state.pending_agent_cancellation = agent_cancellation;
        state.pending_task = Some(task);
        if is_foreground {
            self.sync_conversation_generating_view(cx);
        }
        true
    }

    pub(in crate::ui::shell) fn take_pending_task(&mut self, cx: &mut Context<Self>) -> bool {
        let had_pending_task = clear_pending_task(&mut self.runtime.get_mut().foreground);
        self.sync_conversation_generating_view(cx);
        had_pending_task
    }

    pub(in crate::ui::shell) fn take_pending_task_for_session(
        &mut self,
        session_id: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let is_foreground = self.runtime.borrow().session_is_foreground(session_id);
        let Some(state) = self.runtime.get_mut().session_mut(session_id) else {
            return false;
        };
        let had_pending_task = clear_pending_task(state);
        if is_foreground {
            self.sync_conversation_generating_view(cx);
        }
        had_pending_task
    }

    pub(in crate::ui::shell) fn request_stream_stop(&self) -> bool {
        request_stream_stop(&self.runtime.borrow().foreground)
    }

    pub(in crate::ui::shell) fn has_pending_task(&self) -> bool {
        self.runtime.borrow().foreground.has_pending_task()
    }

    pub(in crate::ui::shell) fn has_pending_task_for_session(&self, session_id: &str) -> bool {
        self.runtime
            .borrow()
            .session(session_id)
            .is_some_and(SessionAgentState::has_pending_task)
    }

    pub(in crate::ui::shell) fn apply_stream_event_for_session(
        &mut self,
        session_id: &str,
        request_id: u64,
        event: AgentChatEvent,
        cx: &mut Context<Self>,
    ) -> Option<AgentStreamMutationOutcome> {
        let is_foreground = self.runtime.borrow().session_is_foreground(session_id);
        let application = {
            let runtime = self.runtime.get_mut();
            mutate_stream_event(runtime.session_mut(session_id)?, request_id, event)?
        };
        if is_foreground {
            for projection in application.projections {
                match projection {
                    AgentStreamProjection::SyncMessage(index) => {
                        self.sync_conversation_message_view(index, cx);
                    }
                    AgentStreamProjection::PushMessagesFrom(start) => {
                        self.push_message_views_from(start, cx);
                    }
                    AgentStreamProjection::AppendMessageDelta { index, delta } => {
                        self.append_conversation_message_view_delta(index, &delta, cx);
                    }
                    AgentStreamProjection::ScheduleSearchRefresh(index) => {
                        self.schedule_conversation_search_message_refresh(index, cx);
                    }
                    AgentStreamProjection::RefreshSearchMessage(index) => {
                        self.refresh_conversation_search_message(index, cx);
                    }
                    AgentStreamProjection::SyncGenerating => {
                        self.sync_conversation_generating_view(cx);
                    }
                }
            }
        }
        Some(application.outcome)
    }

    pub(in crate::ui::shell) fn active_thinking_index(&self) -> Option<usize> {
        active_thinking_index(&self.runtime.borrow().foreground)
    }

    pub(in crate::ui::shell) fn tool_message_index(&self, tool_id: &str) -> Option<usize> {
        tool_message_index(&self.runtime.borrow().foreground, tool_id)
    }

    pub(in crate::ui::shell) fn active_tool_message_indices(&self) -> Vec<usize> {
        active_tool_message_indices(&self.runtime.borrow().foreground)
    }

    pub(in crate::ui::shell) fn active_thinking_index_for_session(
        &self,
        session_id: &str,
    ) -> Option<usize> {
        self.runtime
            .borrow()
            .session(session_id)
            .and_then(active_thinking_index)
    }

    pub(in crate::ui::shell) fn tool_message_index_for_session(
        &self,
        session_id: &str,
        tool_id: &str,
    ) -> Option<usize> {
        self.runtime
            .borrow()
            .session(session_id)
            .and_then(|state| tool_message_index(state, tool_id))
    }

    pub(in crate::ui::shell) fn active_tool_message_indices_for_session(
        &self,
        session_id: &str,
    ) -> Vec<usize> {
        self.runtime
            .borrow()
            .session(session_id)
            .map(active_tool_message_indices)
            .unwrap_or_default()
    }

    pub(in crate::ui::shell) fn mark_tools_unconfirmed_for_session(
        &mut self,
        session_id: &str,
        request_id: u64,
        cx: &mut Context<Self>,
    ) {
        let message = i18n::string("workspace.panel.agent.messages.tool_stop_unconfirmed");
        let (is_foreground, indices) = {
            let runtime = self.runtime.get_mut();
            let is_foreground = runtime.foreground.session_id.as_deref() == Some(session_id);
            let Some(state) = runtime.session_mut(session_id) else {
                return;
            };
            if state.active_request_id != request_id {
                return;
            }
            let indices = active_tool_message_indices(state);
            if !state.fail_active_tool_calls(&message) {
                return;
            }
            (is_foreground, indices)
        };

        if is_foreground {
            for index in indices {
                self.sync_conversation_message_view(index, cx);
            }
        }
    }

    pub(in crate::ui::shell) fn deny_tool_call(&mut self, tool_id: String, cx: &mut Context<Self>) {
        let index = {
            let state = &mut self.runtime.get_mut().foreground;
            state.reject_tool_call(&tool_id);
            tool_message_index(state, &tool_id)
        };
        if let Some(index) = index {
            self.sync_conversation_message_view(index, cx);
        }
        let message = i18n::string("workspace.panel.agent.messages.tool_denied");
        cx.emit(AppCommand::Feedback(message));
        self.persist_foreground_chat(cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn push_message_views_from(
        &mut self,
        start: usize,
        cx: &mut Context<Self>,
    ) {
        let (start, messages) = {
            let state = &self.runtime.borrow().foreground;
            let start = start.min(state.messages.len());
            (start, state.messages[start..].to_vec())
        };
        for (offset, message) in messages.into_iter().enumerate() {
            let index = start + offset;
            self.push_conversation_message_view(message, cx);
            self.refresh_conversation_search_message(index, cx);
        }
    }
}

fn mutate_stream_event(
    state: &mut SessionAgentState,
    request_id: u64,
    event: AgentChatEvent,
) -> Option<AgentStreamEventApplication> {
    if state.active_request_id != request_id {
        return None;
    }

    let mut application = AgentStreamEventApplication::default();
    match event {
        AgentChatEvent::TextDelta(delta) => {
            let previous_message_count = state.messages.len();
            let thinking_index = active_thinking_index(state);
            state.append_assistant_delta(&delta);
            if let Some(index) = thinking_index {
                application
                    .projections
                    .push(AgentStreamProjection::SyncMessage(index));
            }
            if state.messages.len() > previous_message_count {
                application
                    .projections
                    .push(AgentStreamProjection::PushMessagesFrom(
                        previous_message_count,
                    ));
            } else if !delta.is_empty()
                && state
                    .messages
                    .last()
                    .is_some_and(|message| message.role == SessionAgentMessageRole::Assistant)
            {
                application
                    .projections
                    .push(AgentStreamProjection::AppendMessageDelta {
                        index: state.messages.len().saturating_sub(1),
                        delta: delta.clone(),
                    });
            }
            if !delta.is_empty()
                && let Some(index) = state.messages.len().checked_sub(1).filter(|index| {
                    state.messages[*index].role == SessionAgentMessageRole::Assistant
                })
            {
                application
                    .projections
                    .push(AgentStreamProjection::ScheduleSearchRefresh(index));
            }
            state.last_error = None;
        }
        AgentChatEvent::ThinkingDelta(delta) => {
            let previous_message_count = state.messages.len();
            let changed = !delta.trim().is_empty();
            state.append_thinking_delta(&delta);
            if state.messages.len() > previous_message_count {
                application
                    .projections
                    .push(AgentStreamProjection::PushMessagesFrom(
                        previous_message_count,
                    ));
            } else if changed {
                application
                    .projections
                    .push(AgentStreamProjection::AppendMessageDelta {
                        index: state.messages.len().saturating_sub(1),
                        delta,
                    });
            }
            application.outcome.status_message =
                Some(i18n::string("workspace.panel.agent.thinking"));
        }
        AgentChatEvent::ToolCallStarted(tool) => {
            let previous_message_count = state.messages.len();
            let thinking_index = active_thinking_index(state);
            state.push_tool_call(
                tool.id,
                tool.name,
                tool.arguments,
                SessionAgentToolStatus::InProgress,
            );
            if let Some(index) = thinking_index {
                application
                    .projections
                    .push(AgentStreamProjection::SyncMessage(index));
            }
            application
                .projections
                .push(AgentStreamProjection::PushMessagesFrom(
                    previous_message_count,
                ));
        }
        AgentChatEvent::ToolCallDelta { id, delta } => {
            let index = tool_message_index(state, &id);
            let changed = !delta.trim().is_empty();
            state.append_tool_call_delta(&id, delta);
            if changed && let Some(index) = index {
                application
                    .projections
                    .push(AgentStreamProjection::SyncMessage(index));
            }
        }
        AgentChatEvent::ToolCallCompleted { id, result } => {
            let index = tool_message_index(state, &id);
            state.complete_tool_call(&id, result);
            if let Some(index) = index {
                application
                    .projections
                    .push(AgentStreamProjection::SyncMessage(index));
            }
            application.outcome.notification = state.tool_call(&id).and_then(|tool_call| {
                (tool_call.status == SessionAgentToolStatus::WaitingForConfirmation).then_some(
                    SessionAgentBackgroundNotificationKind::ToolApprovalRequired {
                        tool_name: tool_call.name,
                    },
                )
            });
        }
        AgentChatEvent::ToolCallCancelled { id } => {
            let index = tool_message_index(state, &id);
            state.reject_tool_call_with_message(
                &id,
                i18n::string("workspace.panel.agent.messages.stopped_by_user"),
            );
            if let Some(index) = index {
                application
                    .projections
                    .push(AgentStreamProjection::SyncMessage(index));
            }
        }
        AgentChatEvent::ToolCallAutoExecuteRequired { id } => {
            clear_pending_task(state);
            application
                .projections
                .push(AgentStreamProjection::SyncGenerating);
            state.active_request_id = 0;
            application.outcome.follow_up = AgentStreamFollowUp::ApproveTool { tool_id: id };
        }
        AgentChatEvent::ToolCallApprovalRequired { id, message } => {
            if state.agent_mode == AgentMode::FullAuto {
                clear_pending_task(state);
                application
                    .projections
                    .push(AgentStreamProjection::SyncGenerating);
                state.active_request_id = 0;
                application.outcome.follow_up = AgentStreamFollowUp::ApproveTool { tool_id: id };
            } else {
                let index = tool_message_index(state, &id);
                let tool_name = state
                    .tool_call(&id)
                    .map(|tool_call| tool_call.name)
                    .unwrap_or_else(|| i18n::string("workspace.panel.agent.tool"));
                state.require_tool_call_confirmation(&id, message);
                if let Some(index) = index {
                    application
                        .projections
                        .push(AgentStreamProjection::SyncMessage(index));
                }
                application.outcome.notification = Some(
                    SessionAgentBackgroundNotificationKind::ToolApprovalRequired { tool_name },
                );
                application.outcome.follow_up = AgentStreamFollowUp::FinishStream;
            }
        }
        AgentChatEvent::ToolCallUserInputRequired { id, message } => {
            let index = tool_message_index(state, &id);
            let tool_name = state
                .tool_call(&id)
                .map(|tool_call| tool_call.name)
                .unwrap_or_else(|| i18n::string("workspace.panel.agent.tool"));
            state.require_tool_call_confirmation(&id, message);
            if let Some(index) = index {
                application
                    .projections
                    .push(AgentStreamProjection::SyncMessage(index));
            }
            application.outcome.notification =
                Some(SessionAgentBackgroundNotificationKind::UserInputRequired { tool_name });
            application.outcome.follow_up = AgentStreamFollowUp::FinishStream;
        }
        AgentChatEvent::Finished(reply) => {
            let previous_message_count = state.messages.len();
            let thinking_index = active_thinking_index(state);
            state.finish_assistant_reply(reply);
            if let Some(index) = thinking_index {
                application
                    .projections
                    .push(AgentStreamProjection::SyncMessage(index));
            }
            if state.messages.len() > previous_message_count {
                application
                    .projections
                    .push(AgentStreamProjection::PushMessagesFrom(
                        previous_message_count,
                    ));
            } else if let Some(index) =
                state.messages.len().checked_sub(1).filter(|index| {
                    state.messages[*index].role == SessionAgentMessageRole::Assistant
                })
            {
                application
                    .projections
                    .push(AgentStreamProjection::SyncMessage(index));
            }
            if let Some(index) =
                state.messages.len().checked_sub(1).filter(|index| {
                    state.messages[*index].role == SessionAgentMessageRole::Assistant
                })
            {
                application
                    .projections
                    .push(AgentStreamProjection::RefreshSearchMessage(index));
            }
            application.outcome.follow_up = AgentStreamFollowUp::FinishReply;
        }
        AgentChatEvent::TokenUsage { .. } => {}
    }

    Some(application)
}

fn clear_pending_task(state: &mut SessionAgentState) -> bool {
    state.pending_stream_stop = None;
    state.pending_agent_cancellation = None;
    state.pending_task.take().is_some()
}

fn request_stream_stop(state: &SessionAgentState) -> bool {
    let Some(stop) = state.pending_stream_stop.as_ref() else {
        return false;
    };
    if let Some(cancellation) = state.pending_agent_cancellation.as_ref() {
        cancellation.cancel();
    }
    stop.send(true).is_ok()
}

fn active_thinking_index(state: &SessionAgentState) -> Option<usize> {
    state.messages.len().checked_sub(1).filter(|&index| {
        state.messages[index].role == SessionAgentMessageRole::Thinking
            && state.messages[index]
                .thinking
                .as_ref()
                .is_some_and(|thinking| thinking.elapsed_ms.is_none())
    })
}

fn tool_message_index(state: &SessionAgentState, tool_id: &str) -> Option<usize> {
    state.messages.iter().rposition(|message| {
        message
            .tool_call
            .as_ref()
            .is_some_and(|tool_call| tool_call.id == tool_id)
    })
}

fn active_tool_message_indices(state: &SessionAgentState) -> Vec<usize> {
    state
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
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::shell::SessionAgentMessage;

    fn active_stream_state(request_id: u64) -> SessionAgentState {
        let mut state = SessionAgentState::default();
        state.active_request_id = request_id;
        state
    }

    #[test]
    fn stale_stream_event_does_not_mutate_the_active_conversation() {
        let mut state = active_stream_state(2);

        assert!(
            mutate_stream_event(&mut state, 1, AgentChatEvent::TextDelta("late".into())).is_none()
        );
        assert!(state.messages.is_empty());
        assert_eq!(state.active_request_id, 2);
    }

    #[test]
    fn text_delta_finishes_thinking_before_projecting_the_assistant_reply() {
        let mut state = active_stream_state(7);
        state.last_error = Some("old error".into());
        state.append_thinking_delta("checking");

        let application =
            mutate_stream_event(&mut state, 7, AgentChatEvent::TextDelta("answer".into()))
                .expect("active request should accept the event");

        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[1].role, SessionAgentMessageRole::Assistant);
        assert_eq!(state.messages[1].content, "answer");
        assert!(
            state.messages[0]
                .thinking
                .as_ref()
                .is_some_and(|thinking| thinking.elapsed_ms.is_some())
        );
        assert!(state.last_error.is_none());
        assert_eq!(
            application.projections,
            vec![
                AgentStreamProjection::SyncMessage(0),
                AgentStreamProjection::PushMessagesFrom(1),
                AgentStreamProjection::ScheduleSearchRefresh(1),
            ]
        );
        assert_eq!(application.outcome, AgentStreamMutationOutcome::default());
    }

    #[test]
    fn interactive_tool_approval_returns_finish_and_notification_follow_up() {
        let mut state = active_stream_state(9);
        state.push_tool_call(
            "tool-1".into(),
            "run_shell".into(),
            "{}".into(),
            SessionAgentToolStatus::InProgress,
        );

        let application = mutate_stream_event(
            &mut state,
            9,
            AgentChatEvent::ToolCallApprovalRequired {
                id: "tool-1".into(),
                message: "confirm".into(),
            },
        )
        .expect("active request should accept the event");

        let tool = state
            .tool_call("tool-1")
            .expect("tool should remain present");
        assert_eq!(tool.status, SessionAgentToolStatus::WaitingForConfirmation);
        assert_eq!(tool.confirmation_note.as_deref(), Some("confirm"));
        assert_eq!(
            application.projections,
            vec![AgentStreamProjection::SyncMessage(0)]
        );
        assert_eq!(
            application.outcome,
            AgentStreamMutationOutcome {
                status_message: None,
                notification: Some(
                    SessionAgentBackgroundNotificationKind::ToolApprovalRequired {
                        tool_name: "run_shell".into(),
                    }
                ),
                follow_up: AgentStreamFollowUp::FinishStream,
            }
        );
    }

    #[test]
    fn full_auto_tool_approval_retires_the_old_stream_before_root_execution() {
        let (stop, receiver) = watch::channel(false);
        let mut state = active_stream_state(11);
        state.agent_mode = AgentMode::FullAuto;
        state.pending_stream_stop = Some(stop);

        let application = mutate_stream_event(
            &mut state,
            11,
            AgentChatEvent::ToolCallApprovalRequired {
                id: "tool-1".into(),
                message: "confirm".into(),
            },
        )
        .expect("active request should accept the event");

        assert_eq!(state.active_request_id, 0);
        assert!(state.pending_stream_stop.is_none());
        assert!(receiver.has_changed().is_err());
        assert_eq!(
            application.projections,
            vec![AgentStreamProjection::SyncGenerating]
        );
        assert_eq!(
            application.outcome.follow_up,
            AgentStreamFollowUp::ApproveTool {
                tool_id: "tool-1".into(),
            }
        );
    }

    #[test]
    fn finished_reply_updates_the_existing_placeholder_before_root_finalization() {
        let mut state = active_stream_state(13);
        state.start_assistant_reply();

        let application =
            mutate_stream_event(&mut state, 13, AgentChatEvent::Finished("done".into()))
                .expect("active request should accept the event");

        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].content, "done");
        assert_eq!(
            application.projections,
            vec![
                AgentStreamProjection::SyncMessage(0),
                AgentStreamProjection::RefreshSearchMessage(0),
            ]
        );
        assert_eq!(
            application.outcome.follow_up,
            AgentStreamFollowUp::FinishReply
        );
    }

    #[test]
    fn stream_stop_signals_the_current_producer() {
        let (stop, mut receiver) = watch::channel(false);
        let state = SessionAgentState {
            pending_stream_stop: Some(stop),
            ..Default::default()
        };

        assert!(request_stream_stop(&state));
        assert!(
            receiver
                .has_changed()
                .expect("stop sender should remain open")
        );
        assert!(*receiver.borrow_and_update());
    }

    #[test]
    fn stream_indices_track_only_live_tail_and_active_tools() {
        let mut state = SessionAgentState::default();
        state.push_tool_call(
            "completed".to_string(),
            "run_shell".to_string(),
            "{}".to_string(),
            SessionAgentToolStatus::Completed,
        );
        state.push_tool_call(
            "waiting".to_string(),
            "run_shell".to_string(),
            "{}".to_string(),
            SessionAgentToolStatus::WaitingForConfirmation,
        );
        state.push_tool_call(
            "running".to_string(),
            "run_shell".to_string(),
            "{}".to_string(),
            SessionAgentToolStatus::InProgress,
        );

        assert_eq!(tool_message_index(&state, "waiting"), Some(1));
        assert_eq!(active_tool_message_indices(&state), vec![1, 2]);
        assert_eq!(active_thinking_index(&state), None);

        state
            .messages
            .push(SessionAgentMessage::thinking_raw("working"));
        assert_eq!(active_thinking_index(&state), Some(3));
        state.finish_active_thinking();
        assert_eq!(active_thinking_index(&state), None);
    }

    #[test]
    fn clearing_stream_handles_drops_stop_sender_without_a_task() {
        let (stop, receiver) = watch::channel(false);
        let mut state = SessionAgentState {
            pending_stream_stop: Some(stop),
            ..Default::default()
        };

        assert!(!clear_pending_task(&mut state));
        assert!(state.pending_stream_stop.is_none());
        assert!(receiver.has_changed().is_err());
    }
}
