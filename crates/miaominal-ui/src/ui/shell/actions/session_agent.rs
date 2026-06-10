use super::super::*;
use crate::ui::i18n;
use miaominal_agent::{
    AgentChatEvent, AgentChatMessage, AgentChatProvider, AgentChatProviderKind, AgentChatRequest,
    AgentChatRole, AgentToolSet,
};
use miaominal_secrets::SecretKind;
use miaominal_settings::{AiProviderConfig, AiProviderKind};

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
        if self.session_agent.is_waiting() {
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

        let tools = self.active_profile().cloned().map(|profile| {
            AgentToolSet::for_channel(miaominal_agent::AgentExecChannel::for_profile(
                profile,
                self.data.sessions.clone(),
                self.services.secrets.clone(),
                self.services.known_hosts.clone(),
            ))
        });
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

        self.session_agent
            .messages
            .push(SessionAgentMessage::user(prompt.clone()));
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
                    Err(anyhow::anyhow!("session agent stream task cancelled: {error}"))
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
                            log::debug!("failed to apply session agent chat event: {error:?}");
                            break;
                        }
                        if done {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = this.update(cx, move |this, cx| {
                            this.fail_session_agent_stream(
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

    fn apply_session_agent_event(
        &mut self,
        request_id: u64,
        event: AgentChatEvent,
        cx: &mut Context<Self>,
    ) {
        if self.session_agent.active_request_id != request_id {
            return;
        }

        match event {
            AgentChatEvent::TextDelta(delta) => {
                self.session_agent.append_assistant_delta(delta);
                self.session_agent.last_error = None;
            }
            AgentChatEvent::ThinkingDelta(delta) => {
                self.session_agent.append_thinking_delta(delta);
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
            AgentChatEvent::Finished(reply) => {
                self.session_agent.finish_assistant_reply(reply);
                self.finish_session_agent_stream(request_id, cx);
            }
        }

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
                matches!(
                    message.role,
                    SessionAgentMessageRole::Assistant
                        | SessionAgentMessageRole::Thinking
                        | SessionAgentMessageRole::ToolCall
                )
            });
        if !turn_has_output {
            self.session_agent
                .messages
                .push(SessionAgentMessage::assistant(i18n::string(
                    "workspace.panel.agent.empty_reply",
                )));
        }
        self.session_agent.last_error = None;
        self.status_message = i18n::string("workspace.panel.agent.reply_ready");

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
        self.session_agent
            .messages
            .push(SessionAgentMessage::error(message.clone()));
        self.status_message = message;
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
