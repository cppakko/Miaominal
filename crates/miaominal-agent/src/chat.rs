use crate::error::{AgentError, AgentResult};
use crate::tools::AgentToolSet;
use anyhow::Context as _;
use futures::StreamExt as _;
use rig_core::OneOrMany;
use rig_core::agent::{Agent, AgentBuilder, MultiTurnStreamItem, StreamingError};
use rig_core::client::CompletionClient;
use rig_core::completion::{AssistantContent, CompletionModel, GetTokenUsage, Message};
use rig_core::message::{ReasoningContent, ToolResultContent};
use rig_core::providers::{
    anthropic, cohere, deepseek, gemini, huggingface, mistral, openai, openrouter, together, xai,
};
use rig_core::streaming::{
    StreamedAssistantContent, StreamedUserContent, StreamingChat, ToolCallDeltaContent,
};
use std::collections::HashMap;
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

#[derive(Clone)]
pub struct AgentToolResultContinuationRequest {
    pub provider: AgentChatProvider,
    pub messages: Vec<AgentChatMessage>,
    pub tool_call: AgentChatToolEvent,
    pub reasoning: Option<String>,
    pub result: String,
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
    ToolCallDelta { id: String, delta: String },
    ToolCallCompleted { id: String, result: String },
    ToolCallApprovalRequired { id: String, message: String },
    Finished(String),
}

const SESSION_AGENT_PREAMBLE: &str = "You are Miaominal's terminal-side assistant. Help with shell, SSH, SFTP, and general development questions. Be concise, practical, and ask for clarification only when needed.\n\nTool contract:\n- Use only the tools listed in this session. Do not invent tool names.\n- There is no `write`, `edit`, or `replace` tool. For any file creation or modification, use `apply_patch` with a unified patch.\n- Use `read`, `list`, `glob`, and `grep` to inspect files before patching.\n- Use `run_shell` for commands expected to finish quickly. Use `start_job` only for long-running commands such as servers, watchers, deploys, logs, or slow test suites.\n- After `start_job`, keep track of the returned job_id and call `poll_job` until the job exits or you explicitly tell the user it is still running. If you forget the id, use `list_jobs`.\n- If a file change needs approval, call `apply_patch` normally and let Miaominal request approval.";
const SESSION_AGENT_MAX_TURNS: usize = 40;

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
            | AgentChatEvent::ToolCallCompleted { .. }
            | AgentChatEvent::ToolCallApprovalRequired { .. } => {}
        }
    }
    Ok(reply)
}

pub async fn stream_chat(
    request: AgentChatRequest,
) -> AgentResult<mpsc::Receiver<AgentResult<AgentChatEvent>>> {
    stream_chat_with_history(
        request.provider,
        chat_history(request.messages),
        Message::user(request.prompt),
        request.tools,
    )
    .await
}

pub async fn stream_chat_after_tool_result(
    request: AgentToolResultContinuationRequest,
) -> AgentResult<mpsc::Receiver<AgentResult<AgentChatEvent>>> {
    let provider = request.provider;
    let mut history = chat_history(request.messages);
    let tool_arguments = serde_json::from_str(&request.tool_call.arguments)
        .unwrap_or_else(|_| serde_json::Value::String(request.tool_call.arguments.clone()));
    let tool_call_content = AssistantContent::tool_call(
        request.tool_call.id.clone(),
        request.tool_call.name,
        tool_arguments,
    );
    let content = if let Some(reasoning) = request
        .reasoning
        .filter(|reasoning| !reasoning.trim().is_empty())
    {
        let mut content = OneOrMany::one(AssistantContent::reasoning(reasoning));
        content.push(tool_call_content);
        content
    } else {
        OneOrMany::one(tool_call_content)
    };
    history.push(Message::Assistant { id: None, content });
    let prompt = Message::tool_result(request.tool_call.id, request.result);
    stream_chat_with_history(provider, history, prompt, request.tools).await
}

async fn stream_chat_with_history(
    provider: AgentChatProvider,
    history: Vec<Message>,
    prompt: Message,
    tools: Option<AgentToolSet>,
) -> AgentResult<mpsc::Receiver<AgentResult<AgentChatEvent>>> {
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
                .default_max_turns(SESSION_AGENT_MAX_TURNS);
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
                .default_max_turns(SESSION_AGENT_MAX_TURNS);
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
                .default_max_turns(SESSION_AGENT_MAX_TURNS);
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
                .default_max_turns(SESSION_AGENT_MAX_TURNS);
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
                .default_max_turns(SESSION_AGENT_MAX_TURNS);
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
                .default_max_turns(SESSION_AGENT_MAX_TURNS);
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
                .default_max_turns(SESSION_AGENT_MAX_TURNS);
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
                .default_max_turns(SESSION_AGENT_MAX_TURNS);
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
                .default_max_turns(SESSION_AGENT_MAX_TURNS);
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
                .default_max_turns(SESSION_AGENT_MAX_TURNS);
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
    prompt: Message,
    history: Vec<Message>,
    sender: mpsc::Sender<AgentResult<AgentChatEvent>>,
) where
    M: CompletionModel + Send + Sync + 'static,
    M::StreamingResponse: Send + Unpin + GetTokenUsage + 'static,
{
    tokio::spawn(async move {
        let mut stream = agent.stream_chat(prompt, history).await;
        let mut final_reply = String::new();

        // Track streamed tool calls to work around a rig_core bug where
        // tool calls delivered only as ToolCallDelta (no complete ToolCall)
        // are buffered internally but never executed, so the agent emits
        // FinalResponse without running them.
        let mut pending_streamed_tools: HashMap<String, PendingStreamedTool> = HashMap::new();

        while let Some(item) = stream.next().await {
            let event = match item {
                Ok(MultiTurnStreamItem::StreamAssistantItem(content)) => {
                    // Track streamed tool call deltas for the workaround above.
                    match &content {
                        StreamedAssistantContent::ToolCallDelta {
                            internal_call_id,
                            content,
                            ..
                        } => {
                            let entry = pending_streamed_tools
                                .entry(internal_call_id.clone())
                                .or_insert_with(|| {
                                    PendingStreamedTool::new(internal_call_id.clone())
                                });
                            match content {
                                ToolCallDeltaContent::Name(name) => {
                                    entry.name = name.clone();
                                }
                                ToolCallDeltaContent::Delta(delta) => {
                                    entry.arguments.push_str(delta);
                                }
                            }
                        }
                        // Non-streamed tool calls will be executed normally by rig_core.
                        StreamedAssistantContent::ToolCall {
                            internal_call_id, ..
                        } => {
                            pending_streamed_tools.remove(internal_call_id);
                        }
                        _ => {}
                    }
                    chat_event_from_assistant_content(content, &mut final_reply)
                }
                Ok(MultiTurnStreamItem::StreamUserItem(content)) => {
                    // Tool was executed by rig_core; remove from our tracking.
                    let StreamedUserContent::ToolResult {
                        internal_call_id, ..
                    } = &content;
                    pending_streamed_tools.remove(internal_call_id);
                    chat_event_from_user_content(content)
                }
                Ok(MultiTurnStreamItem::CompletionCall(_)) => {
                    log::info!("agent llm completion call boundary");
                    None
                }
                Ok(MultiTurnStreamItem::FinalResponse(response)) => {
                    if !pending_streamed_tools.is_empty() {
                        log::warn!(
                            "rig_core did not execute {} streamed tool call(s); forwarding for approval",
                            pending_streamed_tools.len()
                        );
                        for (_cid, tool) in pending_streamed_tools.drain() {
                            if sender
                                .send(Ok(AgentChatEvent::ToolCallStarted(AgentChatToolEvent {
                                    id: tool.internal_call_id.clone(),
                                    name: tool.name.clone(),
                                    arguments: tool.arguments,
                                })))
                                .await
                                .is_err()
                            {
                                break;
                            }
                            if sender
                                .send(Ok(AgentChatEvent::ToolCallApprovalRequired {
                                    id: tool.internal_call_id,
                                    message: format!("tool `{}` requires user approval", tool.name),
                                }))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        continue; // skip Finished, let the loop drain the stream
                    }
                    let response = response.response().to_string();
                    log::info!("agent llm final response: {:?}", response);
                    Some(Ok(AgentChatEvent::Finished(response)))
                }
                Ok(_) => None,
                Err(error) => {
                    log::info!("agent llm stream error: {error:?}");
                    Some(Err(streaming_error(error)))
                }
            };

            let approval_required = matches!(
                event.as_ref(),
                Some(Ok(AgentChatEvent::ToolCallApprovalRequired { .. }))
            );
            if let Some(event) = event
                && sender.send(event).await.is_err()
            {
                break;
            }
            if approval_required {
                break;
            }
        }

        // If there are still unexecuted tool calls after the loop (e.g.,
        // FinalResponse was handled without us intercepting it), send approval.
        if !pending_streamed_tools.is_empty() {
            log::warn!(
                "rig_core stream ended with {} unexecuted tool call(s); forwarding for approval",
                pending_streamed_tools.len()
            );
            for (_cid, tool) in pending_streamed_tools.drain() {
                let _ = sender
                    .send(Ok(AgentChatEvent::ToolCallStarted(AgentChatToolEvent {
                        id: tool.internal_call_id.clone(),
                        name: tool.name.clone(),
                        arguments: tool.arguments,
                    })))
                    .await;
                let _ = sender
                    .send(Ok(AgentChatEvent::ToolCallApprovalRequired {
                        id: tool.internal_call_id,
                        message: format!("tool `{}` requires user approval", tool.name),
                    }))
                    .await;
            }
        }

        log::info!(
            "agent llm stream ended; accumulated_text_delta={:?}",
            final_reply
        );
    });
}

struct PendingStreamedTool {
    internal_call_id: String,
    name: String,
    arguments: String,
}

impl PendingStreamedTool {
    fn new(internal_call_id: String) -> Self {
        Self {
            internal_call_id,
            name: String::new(),
            arguments: String::new(),
        }
    }
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
            log::info!("agent llm text delta: {:?}", text.text);
            final_reply.push_str(&text.text);
            Some(Ok(AgentChatEvent::TextDelta(text.text)))
        }
        StreamedAssistantContent::Reasoning(reasoning) => {
            let text = reasoning
                .content
                .iter()
                .map(reasoning_content_text)
                .collect::<String>();
            log::info!("agent llm reasoning block: {:?}", text);
            (!text.is_empty()).then_some(Ok(AgentChatEvent::ThinkingDelta(text)))
        }
        StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
            log::info!("agent llm reasoning delta: {:?}", reasoning);
            (!reasoning.is_empty()).then_some(Ok(AgentChatEvent::ThinkingDelta(reasoning)))
        }
        StreamedAssistantContent::ToolCall {
            tool_call,
            internal_call_id,
        } => {
            log::info!(
                "agent llm tool call: id={} name={} arguments={}",
                internal_call_id,
                tool_call.function.name,
                tool_call.function.arguments
            );
            Some(Ok(AgentChatEvent::ToolCallStarted(AgentChatToolEvent {
                id: internal_call_id,
                name: tool_call.function.name,
                arguments: tool_call.function.arguments.to_string(),
            })))
        }
        StreamedAssistantContent::ToolCallDelta {
            internal_call_id,
            content,
            ..
        } => {
            let delta = match content {
                rig_core::streaming::ToolCallDeltaContent::Name(name) => name,
                rig_core::streaming::ToolCallDeltaContent::Delta(delta) => delta,
            };
            log::info!(
                "agent llm tool call delta: id={} delta={:?}",
                internal_call_id,
                delta
            );
            Some(Ok(AgentChatEvent::ToolCallDelta {
                id: internal_call_id,
                delta,
            }))
        }
        StreamedAssistantContent::Final(_) => {
            log::info!("agent llm assistant final marker");
            None
        }
    }
}

fn chat_event_from_user_content(
    content: StreamedUserContent,
) -> Option<AgentResult<AgentChatEvent>> {
    match content {
        StreamedUserContent::ToolResult {
            tool_result,
            internal_call_id,
        } => {
            let result = tool_result_content_text(&tool_result.content);
            log::info!(
                "agent llm tool result returned to model: id={} result={:?}",
                internal_call_id,
                result
            );
            if result.contains("requires user approval") {
                Some(Ok(AgentChatEvent::ToolCallApprovalRequired {
                    id: internal_call_id,
                    message: result,
                }))
            } else {
                Some(Ok(AgentChatEvent::ToolCallCompleted {
                    id: internal_call_id,
                    result,
                }))
            }
        }
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
