use std::collections::HashMap;

use gpui::Context;
use miaominal_agent::{
    AgentChatEvent, AgentChatProvider, AgentChatRequest, AgentChatToolEvent, AgentExecChannel,
    AgentToolCallRequest, AgentToolCancellation, AgentToolResultContinuationRequest,
    TerminalExecHandle,
};
use miaominal_core::profile::SessionProfile;
use miaominal_secrets::{SecretKind, SecretStore};
use miaominal_services::AgentService;
use miaominal_storage::known_hosts_store::KnownHostsStore;
use tokio::sync::watch;

use super::{
    AgentController, AgentFinishStreamOutcome, AgentStreamFollowUp, AgentToolExecutionCommit,
    ChatPanelView, SessionAgentBackgroundNotificationKind,
};
use crate::ui::i18n;
use crate::ui::shell::session_agent::{
    SessionAgentApprovedToolOutcome, SessionAgentPtyInterrupts,
    abort_and_wait_for_session_agent_tool_worker, approved_tool_should_stop_after_finished,
    approved_tool_was_stopped, build_session_agent_history,
    is_recoverable_session_agent_prompt_error, receive_session_agent_event_batch,
    retire_session_agent_pty_leases, session_agent_event_releases_pty_lease,
    wait_for_session_agent_stop, wait_for_session_agent_tool_or_stop,
};
use crate::ui::shell::{AppCommand, TerminalLease, truncate_with_ellipsis};

pub(in crate::ui::shell) enum AgentStreamTaskRequest {
    Chat(AgentChatRequest),
    Continuation(AgentToolResultContinuationRequest),
}

pub(in crate::ui::shell) struct AgentStreamTask {
    pub session_id: String,
    pub request_id: u64,
    pub request: AgentStreamTaskRequest,
    pub active_pty_lease: Option<TerminalLease>,
    pub target_pty_leases: Vec<TerminalLease>,
    pub pty_interrupts: SessionAgentPtyInterrupts,
    pub agent_cancellation: Option<AgentToolCancellation>,
    pub wait_for_producer_close: bool,
    pub error_context: &'static str,
}

pub(in crate::ui::shell) struct AgentApprovedToolTask {
    pub session_id: String,
    pub tool_id: String,
    pub execution_request_id: u64,
    pub reasoning: Option<String>,
    pub tool_name: String,
    pub tool_arguments: String,
    pub arguments: serde_json::Value,
    pub approved: bool,
    pub skip_policy: bool,
    pub agent_service: AgentService,
    pub profile: SessionProfile,
    pub profiles: Vec<SessionProfile>,
    pub secrets: SecretStore,
    pub known_hosts: KnownHostsStore,
    pub web_search_config: miaominal_settings::WebSearchConfig,
    pub terminal_exec: Option<TerminalExecHandle>,
    pub aux_channels: HashMap<String, AgentExecChannel>,
    pub active_target_marker: Option<String>,
    pub active_pty_lease: Option<TerminalLease>,
    pub target_pty_leases: Vec<TerminalLease>,
    pub pty_interrupts: SessionAgentPtyInterrupts,
}

impl AgentController {
    pub(in crate::ui::shell) fn spawn_stream_task(
        &mut self,
        task: AgentStreamTask,
        cx: &mut Context<Self>,
    ) {
        let AgentStreamTask {
            session_id,
            request_id,
            request,
            active_pty_lease,
            target_pty_leases,
            pty_interrupts,
            agent_cancellation,
            wait_for_producer_close,
            error_context,
        } = task;
        let runtime = self.task_runtime.clone();
        let pending_session_id = session_id.clone();
        let (stream_stop, mut stream_stop_receiver) = watch::channel(false);
        let task = cx.spawn(async move |this, cx| {
            let error_session_id = session_id.clone();
            let mut stream_task = runtime.spawn(async move {
                match request {
                    AgentStreamTaskRequest::Chat(request) => miaominal_agent::stream_chat(request)
                        .await
                        .map_err(anyhow::Error::from),
                    AgentStreamTaskRequest::Continuation(request) => {
                        miaominal_agent::stream_chat_after_tool_result(request)
                            .await
                            .map_err(anyhow::Error::from)
                    }
                }
            });
            let stream_result = tokio::select! {
                biased;
                _ = wait_for_session_agent_stop(&mut stream_stop_receiver) => {
                    pty_interrupts.cancel_commands();
                    stream_task.abort();
                    let stopped_session_id = session_id.clone();
                    let stop_interrupts = pty_interrupts.clone();
                    let cleanup_active = active_pty_lease.clone();
                    let cleanup_targets = target_pty_leases.clone();
                    let _ = this.update(cx, move |controller, cx| {
                        controller.finalize_stopped_for_session(
                            &stopped_session_id,
                            Some(request_id),
                            cx,
                        );
                        stop_interrupts.cancel_commands();
                        retire_task_leases(cleanup_active.as_ref(), &cleanup_targets);
                    });
                    return;
                }
                result = &mut stream_task => result.unwrap_or_else(|error| {
                    Err(anyhow::anyhow!("{error_context} task cancelled: {error}"))
                }),
            };

            let mut receiver = match stream_result {
                Ok(receiver) => receiver,
                Err(error) => {
                    pty_interrupts.cancel_commands();
                    let cleanup_active = active_pty_lease.clone();
                    let cleanup_targets = target_pty_leases.clone();
                    let _ = this.update(cx, move |controller, cx| {
                        controller.handle_owned_stream_error(
                            &error_session_id,
                            request_id,
                            error,
                            cx,
                        );
                        retire_task_leases(cleanup_active.as_ref(), &cleanup_targets);
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
                    let release_active = releases_pty_lease
                        .then(|| active_pty_lease.clone())
                        .flatten();
                    let release_targets = if releases_pty_lease {
                        target_pty_leases.clone()
                    } else {
                        Default::default()
                    };
                    let event_session_id = session_id.clone();
                    if let Err(error) = this.update(cx, move |controller, cx| {
                        retire_task_leases(release_active.as_ref(), &release_targets);
                        controller.apply_owned_stream_events(
                            &event_session_id,
                            request_id,
                            received.events,
                            cx,
                        );
                    }) {
                        log::debug!("failed to apply {error_context} events: {error:?}");
                        break;
                    }
                }

                if received.stopped {
                    drop(receiver);
                    pty_interrupts.cancel_commands();
                    let stopped_session_id = session_id.clone();
                    let cleanup_active = active_pty_lease.clone();
                    let cleanup_targets = target_pty_leases.clone();
                    let producer_stop_ack_timed_out = received.producer_stop_ack_timed_out;
                    let _ = this.update(cx, move |controller, cx| {
                        if producer_stop_ack_timed_out {
                            controller.mark_tools_unconfirmed_for_session(
                                &stopped_session_id,
                                request_id,
                                cx,
                            );
                        }
                        controller.finalize_stopped_for_session(
                            &stopped_session_id,
                            Some(request_id),
                            cx,
                        );
                        retire_task_leases(cleanup_active.as_ref(), &cleanup_targets);
                    });
                    return;
                }

                if let Some(error) = received.error {
                    pty_interrupts.cancel_commands();
                    let error_session_id = session_id.clone();
                    let cleanup_active = active_pty_lease.clone();
                    let cleanup_targets = target_pty_leases.clone();
                    let _ = this
                        .update(cx, move |controller, cx| {
                            controller.handle_owned_stream_error(
                                &error_session_id,
                                request_id,
                                anyhow::Error::from(error),
                                cx,
                            );
                            retire_task_leases(cleanup_active.as_ref(), &cleanup_targets);
                        })
                        .map_err(|error| {
                            log::debug!("failed to apply {error_context} error: {error:?}");
                        });
                    return;
                }

                if received.finished || received.stream_closed {
                    break;
                }
            }

            pty_interrupts.cancel_commands();
            let finish_session_id = session_id.clone();
            let cleanup_active = active_pty_lease.clone();
            let cleanup_targets = target_pty_leases.clone();
            let _ = this.update(cx, move |controller, cx| {
                controller.finish_owned_stream(&finish_session_id, request_id, cx);
                retire_task_leases(cleanup_active.as_ref(), &cleanup_targets);
            });
        });

        self.install_pending_task_for_session(
            &pending_session_id,
            task,
            stream_stop,
            agent_cancellation,
            cx,
        );
    }

    pub(in crate::ui::shell) fn spawn_approved_tool_task(
        &mut self,
        task: AgentApprovedToolTask,
        cx: &mut Context<Self>,
    ) {
        let AgentApprovedToolTask {
            session_id,
            tool_id,
            execution_request_id,
            reasoning,
            tool_name,
            tool_arguments,
            arguments,
            approved,
            skip_policy,
            agent_service,
            profile,
            profiles,
            secrets,
            known_hosts,
            web_search_config,
            terminal_exec,
            mut aux_channels,
            active_target_marker,
            active_pty_lease,
            target_pty_leases,
            pty_interrupts,
        } = task;
        let pending_session_id = session_id.clone();
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
                            &profiles,
                            secrets.clone(),
                            known_hosts,
                        );
                        if web_search_config.enabled {
                            let web_search_api_key =
                                secrets.get("web_search", SecretKind::WebSearchApiKey)?;
                            channel = channel
                                .with_web_search_config(web_search_config, web_search_api_key);
                        }
                        if let Some(terminal_exec) = terminal_exec {
                            channel = channel.with_terminal_exec(terminal_exec);
                        }
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
                                approved,
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

            let _ = this.update(cx, move |controller, cx| {
                if approved_tool_was_stopped(&outcome) {
                    pty_interrupts.cancel_commands();
                    controller.finalize_stopped_for_session(
                        &session_id,
                        Some(execution_request_id),
                        cx,
                    );
                    retire_task_leases(active_pty_lease.as_ref(), &target_pty_leases);
                    return;
                }

                let stop_after_finished =
                    approved_tool_should_stop_after_finished(&outcome, &tool_stop_receiver);
                pty_interrupts.cancel_commands();
                retire_task_leases(active_pty_lease.as_ref(), &target_pty_leases);
                let SessionAgentApprovedToolOutcome::Finished(result) = outcome else {
                    unreachable!("stopped tool execution returned past stop handling");
                };
                let commit = controller.commit_tool_execution_result_for_session(
                    &session_id,
                    &tool_id,
                    execution_request_id,
                    result,
                    stop_after_finished,
                    cx,
                );
                let mut finished_was_committed = false;
                match commit {
                    AgentToolExecutionCommit::Ignored | AgentToolExecutionCommit::Stopped => {}
                    AgentToolExecutionCommit::FinishedPendingStop => {
                        finished_was_committed = true;
                    }
                    AgentToolExecutionCommit::Continue { result, failed } => {
                        finished_was_committed = true;
                        controller.continue_session_agent_after_tool_result_for_session(
                            &session_id,
                            AgentChatToolEvent {
                                id: tool_id,
                                name: tool_name,
                                arguments: tool_arguments,
                            },
                            reasoning,
                            result,
                            failed,
                            cx,
                        );
                    }
                }
                if stop_after_finished && finished_was_committed {
                    controller.finalize_stopped_for_session(
                        &session_id,
                        Some(execution_request_id),
                        cx,
                    );
                }
                cx.notify();
            });
        });

        self.install_pending_task_for_session(&pending_session_id, task, tool_stop, None, cx);
    }

    fn apply_owned_stream_events(
        &mut self,
        session_id: &str,
        request_id: u64,
        events: Vec<AgentChatEvent>,
        cx: &mut Context<Self>,
    ) {
        if events.is_empty() || !self.session_exists(session_id) {
            return;
        }

        let is_loaded_session = self.session_is_foreground(session_id);
        let should_background_notify = !self.stream_session_is_foreground(session_id);
        let chat_label = self.stream_notification_chat_label(session_id);
        let requires_root_notify = events.iter().any(|event| {
            !matches!(
                event,
                AgentChatEvent::TextDelta(_)
                    | AgentChatEvent::ThinkingDelta(_)
                    | AgentChatEvent::ToolCallDelta { .. }
                    | AgentChatEvent::TokenUsage { .. }
            )
        });

        for event in events {
            let Some(outcome) =
                self.apply_stream_event_for_session(session_id, request_id, event, cx)
            else {
                continue;
            };
            if is_loaded_session && let Some(message) = outcome.status_message {
                cx.emit(AppCommand::Feedback(message));
            }
            let mut notification = outcome.notification;
            match outcome.follow_up {
                AgentStreamFollowUp::None => {}
                AgentStreamFollowUp::ApproveTool { tool_id } => {
                    self.approve_session_agent_tool_call_for_session(session_id, tool_id, cx);
                }
                AgentStreamFollowUp::FinishStream => {
                    self.finish_owned_stream(session_id, request_id, cx);
                }
                AgentStreamFollowUp::FinishReply => {
                    if self.finish_owned_stream(session_id, request_id, cx)
                        && !self.session_has_active_tool_call(session_id)
                    {
                        notification = Some(SessionAgentBackgroundNotificationKind::ReplyReady);
                    }
                }
            }
            if should_background_notify && let Some(kind) = notification {
                self.emit_background_notification(chat_label.clone(), kind, cx);
            }
        }

        if !is_loaded_session && requires_root_notify {
            self.refresh_chat_sessions(cx);
        }
        if requires_root_notify {
            cx.notify();
        }
    }

    fn finish_owned_stream(
        &mut self,
        session_id: &str,
        request_id: u64,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.session_exists(session_id) {
            return false;
        }
        let is_loaded_session = self.session_is_foreground(session_id);
        let should_notify = !self.stream_session_is_foreground(session_id);
        let Some(AgentFinishStreamOutcome { title_seed }) =
            self.finish_stream_for_session(session_id, request_id, cx)
        else {
            return false;
        };

        if let Some((user_message, assistant_message)) = title_seed {
            self.generate_session_agent_title_for_session(
                session_id,
                user_message,
                assistant_message,
                cx,
            );
        }
        if should_notify && !self.session_has_active_tool_call(session_id) {
            self.emit_background_notification(
                self.stream_notification_chat_label(session_id),
                SessionAgentBackgroundNotificationKind::ReplyReady,
                cx,
            );
        }
        if !is_loaded_session {
            self.refresh_chat_sessions(cx);
        }
        cx.notify();
        true
    }

    fn handle_owned_stream_error(
        &mut self,
        session_id: &str,
        request_id: u64,
        error: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        if !self.session_exists(session_id) {
            return;
        }
        let is_loaded_session = self.session_is_foreground(session_id);
        let should_background_notify = !self.stream_session_is_foreground(session_id);
        let message = error.to_string();
        let notification_error = if is_recoverable_session_agent_prompt_error(&message) {
            self.recover_session_agent_prompt_for_session(session_id, request_id, message, cx);
            None
        } else if self.fail_stream_for_session(session_id, request_id, message.clone(), cx) {
            Some(message)
        } else {
            None
        };

        if should_background_notify && let Some(error) = notification_error {
            self.emit_background_notification(
                self.stream_notification_chat_label(session_id),
                SessionAgentBackgroundNotificationKind::StreamFailed {
                    error: truncate_with_ellipsis(&error, 160),
                },
                cx,
            );
        }
        if !is_loaded_session {
            self.refresh_chat_sessions(cx);
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn continue_prompt_recovery_with_provider(
        &mut self,
        session_id: &str,
        message: String,
        provider: AgentChatProvider,
        cx: &mut Context<Self>,
    ) -> Option<String> {
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
        let Some(preparation) = self.prepare_recovery_request_for_session(session_id, cx) else {
            self.fail_prompt_recovery_setup(session_id, message.clone(), cx);
            return Some(message);
        };
        let history = build_session_agent_history(&preparation.history_messages);
        self.spawn_stream_task(
            AgentStreamTask {
                session_id: preparation.session_id,
                request_id: preparation.request_id,
                request: AgentStreamTaskRequest::Chat(AgentChatRequest {
                    provider,
                    messages: history,
                    prompt,
                    prompt_images: Vec::new(),
                    tools: None,
                    target_guidance: None,
                }),
                active_pty_lease: None,
                target_pty_leases: Vec::new(),
                pty_interrupts: SessionAgentPtyInterrupts::default(),
                agent_cancellation: None,
                wait_for_producer_close: false,
                error_context: "session agent recovery",
            },
            cx,
        );
        None
    }

    pub(in crate::ui::shell) fn fail_prompt_recovery_setup(
        &mut self,
        session_id: &str,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if !self.session_exists(session_id) {
            return;
        }
        let is_loaded_session = self.session_is_foreground(session_id);
        let should_background_notify = !self.stream_session_is_foreground(session_id);
        self.record_recovery_setup_error_for_session(session_id, message.clone(), cx);
        if should_background_notify {
            self.emit_background_notification(
                self.stream_notification_chat_label(session_id),
                SessionAgentBackgroundNotificationKind::StreamFailed {
                    error: truncate_with_ellipsis(&message, 160),
                },
                cx,
            );
        }
        if !is_loaded_session {
            self.refresh_chat_sessions(cx);
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn spawn_title_task(
        &mut self,
        session_id: String,
        provider: AgentChatProvider,
        user_message: String,
        assistant_message: String,
        cx: &mut Context<Self>,
    ) {
        let runtime = self.task_runtime.clone();
        cx.spawn(async move |this, cx| {
            let title = runtime
                .spawn(async move {
                    miaominal_agent::generate_title(provider, &user_message, &assistant_message)
                        .await
                })
                .await
                .inspect_err(|_error| {
                    #[cfg(debug_assertions)]
                    log::info!("title generation task cancelled: {_error:?}");
                })
                .unwrap_or_default();
            if let Some(title) = title {
                let _ = this.update(cx, move |controller, cx| {
                    controller.update_session_title_for_session(&session_id, Some(title), cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn stream_session_is_foreground(&self, session_id: &str) -> bool {
        self.panel_open()
            && self.session_terminal.active_target().is_some()
            && self.session_is_foreground(session_id)
            && self.session_panel_view(session_id) == Some(ChatPanelView::Conversation)
    }

    fn stream_notification_chat_label(&self, session_id: &str) -> String {
        self.session_chat_label(session_id)
            .unwrap_or_else(|| i18n::string("workspace.panel.agent.sidebar_title"))
    }

    fn emit_background_notification(
        &mut self,
        chat_label: String,
        kind: SessionAgentBackgroundNotificationKind,
        cx: &mut Context<Self>,
    ) {
        self.notify_background_session_agent(chat_label, kind, cx);
    }
}

fn retire_task_leases(active: Option<&TerminalLease>, targets: &[TerminalLease]) {
    if let Some(active) = active {
        retire_session_agent_pty_leases(std::slice::from_ref(active));
    }
    retire_session_agent_pty_leases(targets);
}
