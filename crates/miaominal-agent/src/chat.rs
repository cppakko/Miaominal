use crate::error::{AgentError, AgentResult};
use crate::tools::AgentToolSet;
use anyhow::Context as _;
use futures::StreamExt as _;
use rig_core::agent::{Agent, AgentBuilder, MultiTurnStreamItem, StreamingError};
use rig_core::client::CompletionClient;
use rig_core::completion::{CompletionModel, GetTokenUsage, Message};
use rig_core::message::{ReasoningContent, ToolResultContent};
use rig_core::providers::{
    anthropic, cohere, deepseek, gemini, huggingface, mistral, openai, openrouter, together, xai,
};
use rig_core::streaming::{StreamedAssistantContent, StreamedUserContent, StreamingChat};
use tokio::sync::mpsc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentChatProviderKind {
    Anthropic,
    ChatGpt,
    Cohere,
    Copilot,
    DeepSeek,
    Gemini,
    HuggingFace,
    Mistral,
    OpenAi,
    OpenRouter,
    Together,
    Xai,
    Custom,
}

#[derive(Clone, Debug)]
pub struct AgentChatProvider {
    pub id: String,
    pub name: String,
    pub kind: AgentChatProviderKind,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentChatRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentChatMessage {
    pub role: AgentChatRole,
    pub content: String,
}

#[derive(Clone)]
pub struct AgentChatRequest {
    pub provider: AgentChatProvider,
    pub messages: Vec<AgentChatMessage>,
    pub prompt: String,
    pub tools: Option<AgentToolSet>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentChatToolEvent {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentChatEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallStarted(AgentChatToolEvent),
    ToolCallDelta {
        id: String,
        delta: String,
    },
    ToolCallCompleted {
        id: String,
        result: String,
    },
    Finished(String),
}

const SESSION_AGENT_PREAMBLE: &str = "You are Miaominal's terminal-side assistant. Help with shell, SSH, SFTP, and general development questions. Be concise, practical, and ask for clarification only when needed.";

fn chat_history(messages: Vec<AgentChatMessage>) -> Vec<Message> {
    messages
        .into_iter()
        .filter(|message| !message.content.trim().is_empty())
        .map(|message| match message.role {
            AgentChatRole::User => Message::user(message.content),
            AgentChatRole::Assistant => Message::assistant(message.content),
        })
        .collect::<Vec<_>>()
}

pub async fn send_chat(request: AgentChatRequest) -> AgentResult<String> {
    let mut receiver = stream_chat(request).await?;
    let mut reply = String::new();
    while let Some(event) = receiver.recv().await {
        match event? {
            AgentChatEvent::TextDelta(delta) => reply.push_str(&delta),
            AgentChatEvent::Finished(final_reply) => {
                if !final_reply.trim().is_empty() {
                    reply = final_reply;
                }
            }
            AgentChatEvent::ThinkingDelta(_)
            | AgentChatEvent::ToolCallStarted(_)
            | AgentChatEvent::ToolCallDelta { .. }
            | AgentChatEvent::ToolCallCompleted { .. } => {}
        }
    }
    Ok(reply)
}

pub async fn stream_chat(
    request: AgentChatRequest,
) -> AgentResult<mpsc::Receiver<AgentResult<AgentChatEvent>>> {
    let provider = request.provider;
    let history = chat_history(request.messages);
    let prompt = request.prompt;
    let tools = request.tools;
    let (sender, receiver) = mpsc::channel(64);

    match provider.kind {
        AgentChatProviderKind::OpenAi => {
            let mut builder = openai::Client::builder().api_key(provider.api_key);
            if !provider.base_url.trim().is_empty() {
                builder = builder.base_url(provider.base_url);
            }
            let client = builder
                .build()
                .context("failed to build OpenAI chat client")?;
            let builder = AgentBuilder::new(client.completion_model(provider.model))
                .preamble(SESSION_AGENT_PREAMBLE)
                .default_max_turns(4);
            if let Some(tools) = tools {
                spawn_stream_chat(
                    builder.tools(tools.into_rig_tools()).build(),
                    prompt,
                    history,
                    sender,
                );
            } else {
                spawn_stream_chat(builder.build(), prompt, history, sender);
            }
        }
        AgentChatProviderKind::Anthropic => {
            let mut builder = anthropic::Client::builder().api_key(provider.api_key);
            if !provider.base_url.trim().is_empty() {
                builder = builder.base_url(provider.base_url);
            }
            let client = builder
                .build()
                .context("failed to build Anthropic chat client")?;
            let builder = AgentBuilder::new(client.completion_model(provider.model))
                .preamble(SESSION_AGENT_PREAMBLE)
                .default_max_turns(4);
            if let Some(tools) = tools {
                spawn_stream_chat(
                    builder.tools(tools.into_rig_tools()).build(),
                    prompt,
                    history,
                    sender,
                );
            } else {
                spawn_stream_chat(builder.build(), prompt, history, sender);
            }
        }
        AgentChatProviderKind::DeepSeek => {
            let mut builder = deepseek::Client::builder().api_key(provider.api_key);
            if !provider.base_url.trim().is_empty() {
                builder = builder.base_url(provider.base_url);
            }
            let client = builder
                .build()
                .context("failed to build DeepSeek chat client")?;
            let builder = AgentBuilder::new(client.completion_model(provider.model))
                .preamble(SESSION_AGENT_PREAMBLE)
                .default_max_turns(4);
            if let Some(tools) = tools {
                spawn_stream_chat(
                    builder.tools(tools.into_rig_tools()).build(),
                    prompt,
                    history,
                    sender,
                );
            } else {
                spawn_stream_chat(builder.build(), prompt, history, sender);
            }
        }
        AgentChatProviderKind::Gemini => {
            let mut builder = gemini::Client::builder().api_key(provider.api_key);
            if !provider.base_url.trim().is_empty() {
                builder = builder.base_url(provider.base_url);
            }
            let client = builder
                .build()
                .context("failed to build Gemini chat client")?;
            let builder = AgentBuilder::new(client.completion_model(provider.model))
                .preamble(SESSION_AGENT_PREAMBLE)
                .default_max_turns(4);
            if let Some(tools) = tools {
                spawn_stream_chat(
                    builder.tools(tools.into_rig_tools()).build(),
                    prompt,
                    history,
                    sender,
                );
            } else {
                spawn_stream_chat(builder.build(), prompt, history, sender);
            }
        }
        AgentChatProviderKind::OpenRouter => {
            let mut builder = openrouter::Client::builder().api_key(provider.api_key);
            if !provider.base_url.trim().is_empty() {
                builder = builder.base_url(provider.base_url);
            }
            let client = builder
                .build()
                .context("failed to build OpenRouter chat client")?;
            let builder = AgentBuilder::new(client.completion_model(provider.model))
                .preamble(SESSION_AGENT_PREAMBLE)
                .default_max_turns(4);
            if let Some(tools) = tools {
                spawn_stream_chat(
                    builder.tools(tools.into_rig_tools()).build(),
                    prompt,
                    history,
                    sender,
                );
            } else {
                spawn_stream_chat(builder.build(), prompt, history, sender);
            }
        }
        AgentChatProviderKind::Mistral => {
            let mut builder = mistral::Client::builder().api_key(provider.api_key);
            if !provider.base_url.trim().is_empty() {
                builder = builder.base_url(provider.base_url);
            }
            let client = builder
                .build()
                .context("failed to build Mistral chat client")?;
            let builder = AgentBuilder::new(client.completion_model(provider.model))
                .preamble(SESSION_AGENT_PREAMBLE)
                .default_max_turns(4);
            if let Some(tools) = tools {
                spawn_stream_chat(
                    builder.tools(tools.into_rig_tools()).build(),
                    prompt,
                    history,
                    sender,
                );
            } else {
                spawn_stream_chat(builder.build(), prompt, history, sender);
            }
        }
        AgentChatProviderKind::Cohere => {
            let mut builder = cohere::Client::builder().api_key(provider.api_key);
            if !provider.base_url.trim().is_empty() {
                builder = builder.base_url(provider.base_url);
            }
            let client = builder
                .build()
                .context("failed to build Cohere chat client")?;
            let builder = AgentBuilder::new(client.completion_model(provider.model))
                .preamble(SESSION_AGENT_PREAMBLE)
                .default_max_turns(4);
            if let Some(tools) = tools {
                spawn_stream_chat(
                    builder.tools(tools.into_rig_tools()).build(),
                    prompt,
                    history,
                    sender,
                );
            } else {
                spawn_stream_chat(builder.build(), prompt, history, sender);
            }
        }
        AgentChatProviderKind::Together => {
            let mut builder = together::Client::builder().api_key(provider.api_key);
            if !provider.base_url.trim().is_empty() {
                builder = builder.base_url(provider.base_url);
            }
            let client = builder
                .build()
                .context("failed to build Together AI chat client")?;
            let builder = AgentBuilder::new(client.completion_model(provider.model))
                .preamble(SESSION_AGENT_PREAMBLE)
                .default_max_turns(4);
            if let Some(tools) = tools {
                spawn_stream_chat(
                    builder.tools(tools.into_rig_tools()).build(),
                    prompt,
                    history,
                    sender,
                );
            } else {
                spawn_stream_chat(builder.build(), prompt, history, sender);
            }
        }
        AgentChatProviderKind::Xai => {
            let mut builder = xai::Client::builder().api_key(provider.api_key);
            if !provider.base_url.trim().is_empty() {
                builder = builder.base_url(provider.base_url);
            }
            let client = builder.build().context("failed to build xAI chat client")?;
            let builder = AgentBuilder::new(client.completion_model(provider.model))
                .preamble(SESSION_AGENT_PREAMBLE)
                .default_max_turns(4);
            if let Some(tools) = tools {
                spawn_stream_chat(
                    builder.tools(tools.into_rig_tools()).build(),
                    prompt,
                    history,
                    sender,
                );
            } else {
                spawn_stream_chat(builder.build(), prompt, history, sender);
            }
        }
        AgentChatProviderKind::HuggingFace => {
            let mut builder = huggingface::Client::builder().api_key(provider.api_key);
            if !provider.base_url.trim().is_empty() {
                builder = builder.base_url(provider.base_url);
            }
            let client = builder
                .build()
                .context("failed to build Hugging Face chat client")?;
            let builder = AgentBuilder::new(client.completion_model(provider.model))
                .preamble(SESSION_AGENT_PREAMBLE)
                .default_max_turns(4);
            if let Some(tools) = tools {
                spawn_stream_chat(
                    builder.tools(tools.into_rig_tools()).build(),
                    prompt,
                    history,
                    sender,
                );
            } else {
                spawn_stream_chat(builder.build(), prompt, history, sender);
            }
        }
        AgentChatProviderKind::ChatGpt
        | AgentChatProviderKind::Copilot
        | AgentChatProviderKind::Custom => {
            return Err(AgentError::UnsupportedProvider(format!(
                "{} is not supported by the terminal chat yet",
                provider.name
            )));
        }
    }

    Ok(receiver)
}

fn spawn_stream_chat<M>(
    agent: Agent<M>,
    prompt: String,
    history: Vec<Message>,
    sender: mpsc::Sender<AgentResult<AgentChatEvent>>,
) where
    M: CompletionModel + Send + Sync + 'static,
    M::StreamingResponse: Send + Unpin + GetTokenUsage + 'static,
{
    tokio::spawn(async move {
        let mut stream = agent.stream_chat(prompt, history).await;
        let mut final_reply = String::new();

        while let Some(item) = stream.next().await {
            let event = match item {
                Ok(MultiTurnStreamItem::StreamAssistantItem(content)) => {
                    chat_event_from_assistant_content(content, &mut final_reply)
                }
                Ok(MultiTurnStreamItem::StreamUserItem(content)) => {
                    chat_event_from_user_content(content)
                }
                Ok(MultiTurnStreamItem::CompletionCall(_)) => None,
                Ok(MultiTurnStreamItem::FinalResponse(response)) => {
                    Some(Ok(AgentChatEvent::Finished(response.response().to_string())))
                }
                Ok(_) => None,
                Err(error) => Some(Err(streaming_error(error))),
            };

            if let Some(event) = event
                && sender.send(event).await.is_err()
            {
                break;
            }
        }
    });
}

fn chat_event_from_assistant_content<R>(
    content: StreamedAssistantContent<R>,
    final_reply: &mut String,
) -> Option<AgentResult<AgentChatEvent>>
where
    R: Clone + Unpin,
{
    match content {
        StreamedAssistantContent::Text(text) => {
            final_reply.push_str(&text.text);
            Some(Ok(AgentChatEvent::TextDelta(text.text)))
        }
        StreamedAssistantContent::Reasoning(reasoning) => {
            let text = reasoning
                .content
                .iter()
                .map(reasoning_content_text)
                .collect::<String>();
            (!text.is_empty()).then_some(Ok(AgentChatEvent::ThinkingDelta(text)))
        }
        StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
            (!reasoning.is_empty()).then_some(Ok(AgentChatEvent::ThinkingDelta(reasoning)))
        }
        StreamedAssistantContent::ToolCall {
            tool_call,
            internal_call_id,
        } => Some(Ok(AgentChatEvent::ToolCallStarted(AgentChatToolEvent {
            id: internal_call_id,
            name: tool_call.function.name,
            arguments: tool_call.function.arguments.to_string(),
        }))),
        StreamedAssistantContent::ToolCallDelta {
            internal_call_id,
            content,
            ..
        } => {
            let delta = match content {
                rig_core::streaming::ToolCallDeltaContent::Name(name) => name,
                rig_core::streaming::ToolCallDeltaContent::Delta(delta) => delta,
            };
            Some(Ok(AgentChatEvent::ToolCallDelta {
                id: internal_call_id,
                delta,
            }))
        }
        StreamedAssistantContent::Final(_) => None,
    }
}

fn chat_event_from_user_content(
    content: StreamedUserContent,
) -> Option<AgentResult<AgentChatEvent>> {
    match content {
        StreamedUserContent::ToolResult {
            tool_result,
            internal_call_id,
        } => Some(Ok(AgentChatEvent::ToolCallCompleted {
            id: internal_call_id,
            result: tool_result_content_text(&tool_result.content),
        })),
    }
}

fn reasoning_content_text(content: &ReasoningContent) -> String {
    match content {
        ReasoningContent::Text { text, .. } | ReasoningContent::Summary(text) => text.clone(),
        ReasoningContent::Encrypted { .. } => String::new(),
        _ => String::new(),
    }
}

fn tool_result_content_text(content: &rig_core::OneOrMany<ToolResultContent>) -> String {
    content
        .iter()
        .map(|content| match content {
            ToolResultContent::Text(text) => text.text.clone(),
            ToolResultContent::Image(image) => match &image.data {
                rig_core::message::DocumentSourceKind::Url(url)
                | rig_core::message::DocumentSourceKind::Base64(url)
                | rig_core::message::DocumentSourceKind::FileId(url)
                | rig_core::message::DocumentSourceKind::String(url) => url.clone(),
                rig_core::message::DocumentSourceKind::Raw(bytes) => {
                    format!("<{} bytes image>", bytes.len())
                }
                rig_core::message::DocumentSourceKind::Unknown => "<image>".to_string(),
                _ => "<image>".to_string(),
            },
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn streaming_error(error: StreamingError) -> AgentError {
    AgentError::Backend(anyhow::Error::new(error))
}
