use super::super::*;
use crate::ui::i18n;
use miaominal_agent::{
    AgentChatEvent, AgentChatMessage, AgentChatProvider, AgentChatProviderKind, AgentChatRequest,
    AgentChatRole, AgentChatToolEvent, AgentExecChannel, AgentToolCallRequest,
    AgentToolResultContinuationRequest, AgentToolSet,
};
use miaominal_secrets::SecretKind;
use miaominal_settings::{AiProviderConfig, AiProviderKind};
use serde_json::Value;

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
        Self {
            role: match message.role {
                SessionAgentMessageRole::User => AgentChatRole::User,
                SessionAgentMessageRole::Assistant
                | SessionAgentMessageRole::Thinking
                | SessionAgentMessageRole::ToolCall
                | SessionAgentMessageRole::Error => AgentChatRole::Assistant,
            },
            content: message.content.clone(),
        }
    }
}

impl AppView {
    fn session_agent_is_scrolled_to_bottom(&self) -> bool {
        let scroll_handle = &self.workspace_state.session_agent_scroll_handle;
        let offset = scroll_handle.offset();
        let max_offset = scroll_handle.max_offset();
        (offset.y + max_offset.y).abs() <= px(2.0)
    }

    fn scroll_session_agent_to_bottom_if_following(
        &self,
        previous_message_count: usize,
        was_scrolled_to_bottom: bool,
        content_may_have_grown: bool,
    ) {
        let new_block_added = self.session_agent.messages.len() > previous_message_count;
        if was_scrolled_to_bottom && (new_block_added || content_may_have_grown) {
            self.workspace_state
                .session_agent_scroll_handle
                .scroll_to_bottom();
        }
    }

    fn push_session_agent_message(&mut self, message: SessionAgentMessage, cx: &mut Context<Self>) {
        let previous_message_count = self.session_agent.messages.len();
        let was_scrolled_to_bottom = self.session_agent_is_scrolled_to_bottom();
        self.session_agent.messages.push(message);
        let index = self.session_agent.messages.len() - 1;
        self.session_agent.ensure_plain_markdown(index, cx);
        self.scroll_session_agent_to_bottom_if_following(
            previous_message_count,
            was_scrolled_to_bottom,
            false,
        );
    }

    pub(in crate::ui::shell) fn reset_session_agent_chat(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.session_agent.messages.clear();
        self.session_agent.last_error = None;
        self.session_agent.active_request_id = self.session_agent.active_request_id.wrapping_add(1);
        self.session_agent.pending_task = None;
        set_input_value(
            &self.workspace_forms.agent.prompt_input,
            String::new(),
            window,
            cx,
        );
        self.status_message = i18n::string("workspace.panel.agent.new_chat_started");
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
        if prompt.is_empty() {
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

        let tools = self
            .active_profile()
            .cloned()
            .map(|profile| AgentToolSet::for_channel(self.agent_exec_channel_for_profile(profile)));
        let request_id = self.session_agent.next_request_id();
        let history = self
            .session_agent
            .messages
            .iter()
            .filter(|message| {
                matches!(
                    message.role,
                    SessionAgentMessageRole::User | SessionAgentMessageRole::Assistant
                )
            })
            .map(AgentChatMessage::from)
            .collect::<Vec<_>>();

        self.push_session_agent_message(SessionAgentMessage::user(prompt.clone()), cx);
        self.workspace_state
            .session_agent_scroll_handle
            .scroll_to_bottom();
        self.session_agent.active_request_id = request_id;
        self.session_agent.last_error = None;
        self.status_message = i18n::string("workspace.panel.agent.send_pending");
        set_input_value(
            &self.workspace_forms.agent.prompt_input,
            String::new(),
            window,
            cx,
        );

        let runtime = self.services.runtime.clone();
        let task = cx.spawn(async move |this, cx| {
            let stream_result = runtime
                .spawn(async move {
                    miaominal_agent::stream_chat(AgentChatRequest {
                        provider,
                        messages: history,
                        prompt,
                        tools,
                    })
                    .await
                    .map_err(anyhow::Error::from)
                })
                .await
                .unwrap_or_else(|error| {
                    Err(anyhow::anyhow!(
                        "session agent stream task cancelled: {error}"
                    ))
                });

            let mut receiver = match stream_result {
                Ok(receiver) => receiver,
                Err(error) => {
                    let _ = this.update(cx, move |this, cx| {
                        this.handle_session_agent_stream_error(request_id, error, cx);
                    });
                    return;
                }
            };

            while let Some(event) = receiver.recv().await {
                match event {
                    Ok(event) => {
                        let done = matches!(event, AgentChatEvent::Finished(_));
                        if let Err(error) = this.update(cx, move |this, cx| {
                            this.apply_session_agent_event(request_id, event, cx);
                        }) {
                            log::debug!("failed to apply session agent chat event: {error:?}");
                            break;
                        }
                        if done {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = this
                            .update(cx, move |this, cx| {
                                this.handle_session_agent_stream_error(
                                    request_id,
                                    anyhow::Error::from(error),
                                    cx,
                                );
                            })
                            .map_err(|error| {
                                log::debug!("failed to apply session agent chat error: {error:?}");
                            });
                        return;
                    }
                }
            }

            let _ = this.update(cx, move |this, cx| {
                this.finish_session_agent_stream(request_id, cx);
            });
        });
        self.session_agent.pending_task = Some(task);
        cx.notify();
    }

    pub(in crate::ui::shell) fn stop_session_agent_stream(&mut self, cx: &mut Context<Self>) {
        let had_pending_task = self.session_agent.pending_task.take().is_some();
        let had_active_tool = self
            .session_agent
            .reject_active_tool_calls("Stopped by user.");
        if !had_pending_task && !had_active_tool {
            return;
        }

        self.session_agent.active_request_id = self.session_agent.active_request_id.wrapping_add(1);
        self.status_message = "Agent stopped.".into();
        self.session_agent.last_error = None;
        cx.notify();
    }

    pub(in crate::ui::shell) fn approve_session_agent_tool_call(
        &mut self,
        tool_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(tool_call) = self.session_agent.tool_call(&tool_id) else {
            self.status_message = "Tool call was not found.".into();
            cx.notify();
            return;
        };
        let Some(profile) = self.active_profile().cloned() else {
            self.status_message = "No active session for tool approval.".into();
            cx.notify();
            return;
        };

        let arguments = parse_tool_arguments(&tool_call.arguments);
        let reasoning = self.session_agent.reasoning_before_tool_call(&tool_id);
        self.session_agent.approve_tool_call(&tool_id);
        self.status_message = "Tool approved. Running...".into();

        let sessions = self.data.sessions.clone();
        let secrets = self.services.secrets.clone();
        let known_hosts = self.services.known_hosts.clone();
        let web_search_config = self.settings_store.settings().web_search.clone();
        let tool_name = tool_call.name.clone();
        let tool_arguments = tool_call.arguments.clone();
        let task = cx.spawn(async move |this, cx| {
            let (sender, receiver) = tokio::sync::oneshot::channel();
            let worker_tool_name = tool_name.clone();
            let spawn_result = std::thread::Builder::new()
                .name(format!("session-agent-approved-tool-{worker_tool_name}"))
                .spawn(move || {
                    let result = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| anyhow::anyhow!(error))
                        .and_then(|runtime| {
                            runtime.block_on(async move {
                                let mut channel = AgentExecChannel::for_profile(
                                    profile,
                                    sessions,
                                    secrets.clone(),
                                    known_hosts,
                                );
                                if web_search_config.enabled {
                                    let web_search_api_key = secrets
                                        .get("web_search", SecretKind::WebSearchApiKey)
                                        .map_err(anyhow::Error::from)?;
                                    channel = channel.with_web_search_config(
                                        web_search_config,
                                        web_search_api_key,
                                    );
                                }
                                channel
                                    .call_tool(AgentToolCallRequest {
                                        tool_name: worker_tool_name,
                                        arguments,
                                        approved: true,
                                        route: None,
                                    })
                                    .await
                                    .map_err(anyhow::Error::from)
                                    .and_then(|response| {
                                        serde_json::to_string(&response)
                                            .map_err(|error| anyhow::anyhow!(error))
                                    })
                            })
                        });
                    let _ = sender.send(result);
                });

            let result = match spawn_result {
                Ok(_) => receiver.await.unwrap_or_else(|_| {
                    Err(anyhow::anyhow!(
                        "approved tool worker stopped before returning a result"
                    ))
                }),
                Err(error) => Err(anyhow::anyhow!(error)),
            };

            let _ = this.update(cx, move |this, cx| {
                let previous_message_count = this.session_agent.messages.len();
                let was_scrolled_to_bottom = this.session_agent_is_scrolled_to_bottom();
                let (tool_result, failed) = match result {
                    Ok(result) => {
                        if !matches!(
                            this.session_agent
                                .tool_call(&tool_id)
                                .map(|tool_call| tool_call.status),
                            Some(SessionAgentToolStatus::InProgress)
                        ) {
                            this.status_message = "Agent stopped.".into();
                            cx.notify();
                            return;
                        }
                        this.session_agent
                            .complete_tool_call(&tool_id, result.clone());
                        this.status_message = "Approved tool finished. Continuing...".into();
                        (result, false)
                    }
                    Err(error) => {
                        if !matches!(
                            this.session_agent
                                .tool_call(&tool_id)
                                .map(|tool_call| tool_call.status),
                            Some(SessionAgentToolStatus::InProgress)
                        ) {
                            this.status_message = "Agent stopped.".into();
                            cx.notify();
                            return;
                        }
                        let result = format!("tool failed after approval: {error}");
                        this.session_agent.fail_tool_call(&tool_id, result.clone());
                        this.status_message = "Approved tool failed. Continuing...".into();
                        (result, true)
                    }
                };
                this.scroll_session_agent_to_bottom_if_following(
                    previous_message_count,
                    was_scrolled_to_bottom,
                    true,
                );
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
                cx.notify();
            });
        });
        task.detach();
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
            self.push_session_agent_message(SessionAgentMessage::error(
                "Agent is already processing another response; approved tool result was not sent to the model.",
            ), cx);
            self.status_message = "Agent is already processing.".into();
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

        let tools = self
            .active_profile()
            .cloned()
            .map(|profile| AgentToolSet::for_channel(self.agent_exec_channel_for_profile(profile)));
        let request_id = self.session_agent.next_request_id();
        let history = self
            .session_agent
            .messages
            .iter()
            .filter(|message| {
                matches!(
                    message.role,
                    SessionAgentMessageRole::User | SessionAgentMessageRole::Assistant
                )
            })
            .map(AgentChatMessage::from)
            .collect::<Vec<_>>();

        self.session_agent.active_request_id = request_id;
        self.session_agent.last_error = None;
        let previous_message_count = self.session_agent.messages.len();
        let was_scrolled_to_bottom = self.session_agent_is_scrolled_to_bottom();
        self.session_agent.start_assistant_reply(cx);
        self.scroll_session_agent_to_bottom_if_following(
            previous_message_count,
            was_scrolled_to_bottom,
            false,
        );
        self.status_message = i18n::string("workspace.panel.agent.thinking");

        let runtime = self.services.runtime.clone();
        let task = cx.spawn(async move |this, cx| {
            let stream_result = runtime
                .spawn(async move {
                    let result = if failed {
                        format!("ERROR: {result}")
                    } else {
                        result
                    };
                    miaominal_agent::stream_chat_after_tool_result(
                        AgentToolResultContinuationRequest {
                            provider,
                            messages: history,
                            tool_call,
                            reasoning,
                            result,
                            tools,
                        },
                    )
                    .await
                    .map_err(anyhow::Error::from)
                })
                .await
                .unwrap_or_else(|error| {
                    Err(anyhow::anyhow!(
                        "session agent continuation task cancelled: {error}"
                    ))
                });

            let mut receiver = match stream_result {
                Ok(receiver) => receiver,
                Err(error) => {
                    let _ = this.update(cx, move |this, cx| {
                        this.handle_session_agent_stream_error(request_id, error, cx);
                    });
                    return;
                }
            };

            while let Some(event) = receiver.recv().await {
                match event {
                    Ok(event) => {
                        let done = matches!(event, AgentChatEvent::Finished(_));
                        if let Err(error) = this.update(cx, move |this, cx| {
                            this.apply_session_agent_event(request_id, event, cx);
                        }) {
                            log::debug!(
                                "failed to apply session agent continuation event: {error:?}"
                            );
                            break;
                        }
                        if done {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = this
                            .update(cx, move |this, cx| {
                                this.handle_session_agent_stream_error(
                                    request_id,
                                    anyhow::Error::from(error),
                                    cx,
                                );
                            })
                            .map_err(|error| {
                                log::debug!(
                                    "failed to apply session agent continuation error: {error:?}"
                                );
                            });
                        return;
                    }
                }
            }

            let _ = this.update(cx, move |this, cx| {
                this.finish_session_agent_stream(request_id, cx);
            });
        });
        self.session_agent.pending_task = Some(task);
    }

    pub(in crate::ui::shell) fn deny_session_agent_tool_call(
        &mut self,
        tool_id: String,
        cx: &mut Context<Self>,
    ) {
        self.session_agent.reject_tool_call(&tool_id);
        self.status_message = "Tool denied.".into();
        cx.notify();
    }

    pub(in crate::ui::shell) fn copy_session_agent_text(
        &mut self,
        label: &str,
        text: String,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.status_message = format!("Copied {label}.");
        cx.notify();
    }

    fn apply_session_agent_event(
        &mut self,
        request_id: u64,
        event: AgentChatEvent,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent.active_request_id != request_id {
            return;
        }

        let previous_message_count = self.session_agent.messages.len();
        let was_scrolled_to_bottom = self.session_agent_is_scrolled_to_bottom();
        let content_may_have_grown = matches!(
            event,
            AgentChatEvent::TextDelta(_)
                | AgentChatEvent::ThinkingDelta(_)
                | AgentChatEvent::ToolCallDelta { .. }
                | AgentChatEvent::ToolCallCompleted { .. }
                | AgentChatEvent::ToolCallApprovalRequired { .. }
                | AgentChatEvent::Finished(_)
        );
        match event {
            AgentChatEvent::TextDelta(delta) => {
                self.session_agent.append_assistant_delta(delta, cx);
                self.session_agent.last_error = None;
            }
            AgentChatEvent::ThinkingDelta(delta) => {
                self.session_agent.append_thinking_delta(delta, cx);
                self.status_message = i18n::string("workspace.panel.agent.thinking");
            }
            AgentChatEvent::ToolCallStarted(tool) => {
                self.session_agent.push_tool_call(
                    tool.id,
                    tool.name,
                    tool.arguments,
                    SessionAgentToolStatus::InProgress,
                );
            }
            AgentChatEvent::ToolCallDelta { id, delta } => {
                self.session_agent.append_tool_call_delta(&id, delta);
            }
            AgentChatEvent::ToolCallCompleted { id, result } => {
                self.session_agent.complete_tool_call(&id, result);
            }
            AgentChatEvent::ToolCallApprovalRequired { id, message } => {
                self.session_agent
                    .require_tool_call_confirmation(&id, message);
                self.finish_session_agent_stream(request_id, cx);
            }
            AgentChatEvent::Finished(reply) => {
                self.session_agent.finish_assistant_reply(reply);
                // Sync the markdown entity with the final authoritative content.
                // finish_assistant_reply overwrites message.content but does not
                // update markdown_entity, so we need to do it here.
                let last_assistant_idx = self
                    .session_agent
                    .messages
                    .iter()
                    .rposition(|m| m.role == SessionAgentMessageRole::Assistant);
                if let Some(idx) = last_assistant_idx {
                    self.session_agent.ensure_assistant_markdown(idx, cx);
                }
                self.finish_session_agent_stream(request_id, cx);
            }
        }
        self.scroll_session_agent_to_bottom_if_following(
            previous_message_count,
            was_scrolled_to_bottom,
            content_may_have_grown,
        );

        cx.notify();
    }

    fn finish_session_agent_stream(&mut self, request_id: u64, cx: &mut Context<Self>) {
        if self.session_agent.active_request_id != request_id {
            return;
        }

        self.session_agent.pending_task = None;
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
            "Waiting for tool approval.".into()
        } else {
            i18n::string("workspace.panel.agent.reply_ready")
        };

        cx.notify();
    }

    fn fail_session_agent_stream(
        &mut self,
        request_id: u64,
        error: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent.active_request_id != request_id {
            return;
        }

        self.session_agent.pending_task = None;
        self.session_agent.active_request_id = 0;
        let message = error.to_string();
        self.session_agent.last_error = Some(message.clone());
        self.push_session_agent_message(SessionAgentMessage::error(message.clone()), cx);
        self.status_message = message;
        cx.notify();
    }

    fn handle_session_agent_stream_error(
        &mut self,
        request_id: u64,
        error: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        let message = error.to_string();
        if is_recoverable_session_agent_prompt_error(&message) {
            self.recover_session_agent_prompt_error(request_id, message, cx);
        } else {
            self.fail_session_agent_stream(request_id, error, cx);
        }
    }

    fn recover_session_agent_prompt_error(
        &mut self,
        request_id: u64,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent.active_request_id != request_id {
            return;
        }

        self.session_agent.pending_task = None;
        self.session_agent.active_request_id = 0;
        self.session_agent.last_error = None;
        self.push_session_agent_message(
            SessionAgentMessage::error(format!(
                "Agent tool-loop error returned to model: {message}"
            )),
            cx,
        );
        self.status_message = "Agent tool-loop error returned to model.".into();

        let Some(provider_id) = self.selected_ai_provider_id(cx) else {
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

        let history = self
            .session_agent
            .messages
            .iter()
            .filter(|message| {
                matches!(
                    message.role,
                    SessionAgentMessageRole::User | SessionAgentMessageRole::Assistant
                )
            })
            .map(AgentChatMessage::from)
            .collect::<Vec<_>>();
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
        let previous_message_count = self.session_agent.messages.len();
        let was_scrolled_to_bottom = self.session_agent_is_scrolled_to_bottom();
        self.session_agent.start_assistant_reply(cx);
        self.scroll_session_agent_to_bottom_if_following(
            previous_message_count,
            was_scrolled_to_bottom,
            false,
        );
        self.status_message = i18n::string("workspace.panel.agent.thinking");

        let runtime = self.services.runtime.clone();
        let task = cx.spawn(async move |this, cx| {
            let stream_result = runtime
                .spawn(async move {
                    miaominal_agent::stream_chat(AgentChatRequest {
                        provider,
                        messages: history,
                        prompt,
                        tools: None,
                    })
                    .await
                    .map_err(anyhow::Error::from)
                })
                .await
                .unwrap_or_else(|error| {
                    Err(anyhow::anyhow!(
                        "session agent recovery task cancelled: {error}"
                    ))
                });

            let mut receiver = match stream_result {
                Ok(receiver) => receiver,
                Err(error) => {
                    let _ = this.update(cx, move |this, cx| {
                        this.fail_session_agent_stream(request_id, error, cx);
                    });
                    return;
                }
            };

            while let Some(event) = receiver.recv().await {
                match event {
                    Ok(event) => {
                        let done = matches!(event, AgentChatEvent::Finished(_));
                        if let Err(error) = this.update(cx, move |this, cx| {
                            this.apply_session_agent_event(request_id, event, cx);
                        }) {
                            log::debug!("failed to apply session agent recovery event: {error:?}");
                            break;
                        }
                        if done {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = this
                            .update(cx, move |this, cx| {
                                this.fail_session_agent_stream(
                                    request_id,
                                    anyhow::Error::from(error),
                                    cx,
                                );
                            })
                            .map_err(|error| {
                                log::debug!(
                                    "failed to apply session agent recovery error: {error:?}"
                                );
                            });
                        return;
                    }
                }
            }

            let _ = this.update(cx, move |this, cx| {
                this.finish_session_agent_stream(request_id, cx);
            });
        });
        self.session_agent.pending_task = Some(task);
        cx.notify();
    }

    fn build_session_agent_provider(&self, provider_id: &str) -> anyhow::Result<AgentChatProvider> {
        let provider = self
            .settings_store
            .settings()
            .ai_providers
            .iter()
            .find(|provider| provider.id == provider_id)
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
        })
    }

    fn agent_exec_channel_for_profile(&self, profile: SessionProfile) -> AgentExecChannel {
        let mut channel = AgentExecChannel::for_profile(
            profile,
            self.data.sessions.clone(),
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
