use crate::channel::{AgentToolCallResponse, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::tools::AgentToolSet;
use anyhow::Context as _;
use futures::StreamExt as _;
use miaominal_core::chat_attachment::ChatImage;
use rig_core::OneOrMany;
use rig_core::agent::{Agent, AgentBuilder, MultiTurnStreamItem, StreamingError};
use rig_core::client::CompletionClient;
use rig_core::completion::{AssistantContent, CompletionModel, GetTokenUsage, Message};
use rig_core::message::{
    ImageMediaType, MimeType, ReasoningContent, ToolResultContent, UserContent,
};
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AgentMode {
    /// Only read-only tools, full policy review, every tool call requires confirmation.
    Ask,
    /// All tools, full policy review, sensitive operations require confirmation. (default)
    #[default]
    Execute,
    /// All tools, policy enforced, but tool calls are auto-approved (no confirmation prompts).
    NonBlocking,
    /// All tools, policy bypassed entirely, all tool calls auto-executed.
    FullAuto,
}

#[derive(Clone, Debug)]
pub struct AgentChatProvider {
    pub id: String,
    pub name: String,
    pub kind: AgentChatProviderKind,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
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
    /// Images attached to this message (empty for assistant messages and
    /// text-only user messages). Text file attachments are embedded into
    /// `content` by the UI layer before conversion.
    pub images: Vec<ChatImage>,
}

#[derive(Clone)]
pub struct AgentChatRequest {
    pub provider: AgentChatProvider,
    pub messages: Vec<AgentChatMessage>,
    pub prompt: String,
    /// Images attached to the current prompt (sent as multimodal content to
    /// vision-capable providers, or as a text marker fallback otherwise).
    pub prompt_images: Vec<ChatImage>,
    pub tools: Option<AgentToolSet>,
    pub target_guidance: Option<String>,
}

#[derive(Clone)]
pub struct AgentToolResultContinuationRequest {
    pub provider: AgentChatProvider,
    pub messages: Vec<AgentChatMessage>,
    pub tool_call: AgentChatToolEvent,
    pub reasoning: Option<String>,
    pub result: String,
    pub tools: Option<AgentToolSet>,
    pub target_guidance: Option<String>,
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
    ToolCallApprovalRequired {
        id: String,
        message: String,
    },
    Finished(String),
    /// Token usage for the most recent completion request.
    TokenUsage {
        input_tokens: u64,
        output_tokens: u64,
    },
}

const SESSION_AGENT_PREAMBLE: &str = "You are Miaominal's terminal-side assistant. Help with shell, SSH, SFTP, and general development questions. Be concise, practical, and ask for clarification only when needed.\n\nTool contract:\n- Use only the tools listed in this session. Do not invent tool names.\n- There is no `write`, `edit`, or `replace` tool. For any file creation or modification, use `apply_patch` with a unified patch.\n- Use `read`, `list`, `glob`, and `grep` to inspect files before patching.\n- Use `run_shell` for commands expected to finish quickly. Use `start_job` only for long-running commands such as servers, watchers, deploys, logs, or slow test suites.\n- After `start_job`, keep track of the returned job_id and call `poll_job` until the job exits or you explicitly tell the user it is still running. If you forget the id, use `list_jobs`.\n- If a file change needs approval, call `apply_patch` normally and let Miaominal request approval.";
const SESSION_AGENT_MAX_TURNS: usize = 40;

fn chat_history(messages: Vec<AgentChatMessage>, vision_supported: bool) -> Vec<Message> {
    messages
        .into_iter()
        .filter(|message| !message.content.trim().is_empty() || !message.images.is_empty())
        .map(|message| match message.role {
            AgentChatRole::User => {
                build_user_message(&message.content, &message.images, vision_supported)
            }
            AgentChatRole::Assistant => Message::assistant(message.content),
        })
        .collect::<Vec<_>>()
}

/// Returns `true` when the provider kind is known to support image content
/// blocks in user messages. Providers not in this list receive a text marker
/// fallback so the chat still works without crashing.
fn provider_supports_vision(kind: AgentChatProviderKind) -> bool {
    matches!(
        kind,
        AgentChatProviderKind::OpenAi
            | AgentChatProviderKind::Anthropic
            | AgentChatProviderKind::Gemini
            | AgentChatProviderKind::OpenRouter
            | AgentChatProviderKind::Xai
    )
}

/// Builds a user `Message` from text and optional images. When the provider
/// supports vision, images become `UserContent::Image` blocks alongside the
/// text. Otherwise, a `[Image attached: N image(s)]` marker is appended to the
/// text so the model is at least aware that images were shared.
fn build_user_message(text: &str, images: &[ChatImage], vision_supported: bool) -> Message {
    if images.is_empty() {
        return Message::user(text);
    }
    if !vision_supported {
        let marker = format!("[Image attached: {} image(s)]", images.len());
        let combined = if text.trim().is_empty() {
            marker
        } else {
            format!("{text}\n\n{marker}")
        };
        return Message::user(combined);
    }
    let mut content = OneOrMany::one(UserContent::text(text));
    let image_count = images.len();
    for image in images {
        let media_type = ImageMediaType::from_mime_type(&image.mime_type);
        content.push(UserContent::image_base64(
            &image.data_base64,
            media_type,
            None,
        ));
    }
    let prefix = format!(
        "[The user attached {} image(s) below. Analyze them as visual input.]\n\n",
        image_count
    );
    content.insert(0, UserContent::text(prefix));
    Message::User { content }
}

/// Generate a concise title (3-8 words) from the first user-assistant exchange.
/// Returns None on any failure — callers should silently keep the title empty.
pub async fn generate_title(
    provider: AgentChatProvider,
    user_message: &str,
    assistant_reply: &str,
) -> Option<String> {
    let prompt = format!(
        "Generate a concise title (3-8 words) for the following conversation. The title must be in the same language as the user's message. Output only the title, no quotes, punctuation, or extra text.\n\nUser: {user_message}\nAssistant: {assistant_reply}"
    );
    let request = AgentChatRequest {
        provider,
        messages: Vec::new(),
        prompt,
        prompt_images: Vec::new(),
        tools: None,
        target_guidance: None,
    };
    match send_chat(request).await {
        Ok(reply) => {
            let title = reply
                .trim()
                .trim_matches(|c: char| {
                    c == '"'
                        || c == '\''
                        || c == '。'
                        || c == '.'
                        || c == '，'
                        || c == ','
                        || c == '！'
                        || c == '!'
                        || c == '？'
                        || c == '?'
                })
                .to_string();
            if title.is_empty() { None } else { Some(title) }
        }
        Err(error) => {
            log::info!("title generation failed: {error:?}");
            None
        }
    }
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
            | AgentChatEvent::ToolCallApprovalRequired { .. }
            | AgentChatEvent::TokenUsage { .. } => {}
        }
    }
    Ok(reply)
}

pub async fn stream_chat(
    request: AgentChatRequest,
) -> AgentResult<mpsc::Receiver<AgentResult<AgentChatEvent>>> {
    let vision_supported = provider_supports_vision(request.provider.kind);
    let prompt = build_user_message(&request.prompt, &request.prompt_images, vision_supported);
    stream_chat_with_history(
        request.provider,
        chat_history(request.messages, vision_supported),
        prompt,
        request.tools,
        request.target_guidance,
    )
    .await
}

pub async fn stream_chat_after_tool_result(
    request: AgentToolResultContinuationRequest,
) -> AgentResult<mpsc::Receiver<AgentResult<AgentChatEvent>>> {
    let provider = request.provider;
    let vision_supported = provider_supports_vision(provider.kind);
    let mut history = chat_history(request.messages, vision_supported);
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
    stream_chat_with_history(
        provider,
        history,
        prompt,
        request.tools,
        request.target_guidance,
    )
    .await
}

macro_rules! build_provider_agent {
    ($client_module:ident, $error_msg:expr, $provider:expr, $preamble:expr, $tools:expr, $prompt:expr, $history:expr, $sender:expr) => {{
        let mut builder = $client_module::Client::builder().api_key($provider.api_key);
        if !$provider.base_url.trim().is_empty() {
            builder = builder.base_url($provider.base_url);
        }
        let client = builder.build().context($error_msg)?;
        let mut agent_builder = AgentBuilder::new(client.completion_model($provider.model))
            .preamble($preamble)
            .default_max_turns(SESSION_AGENT_MAX_TURNS);
        if let Some(t) = $provider.temperature {
            agent_builder = agent_builder.temperature(t);
        }
        if let Some(mt) = $provider.max_tokens {
            agent_builder = agent_builder.max_tokens(mt);
        }
        if let Some(the_tools) = $tools {
            spawn_stream_chat(
                agent_builder.tools(the_tools.into_rig_tools()).build(),
                $prompt,
                $history,
                $sender,
            );
        } else {
            spawn_stream_chat(agent_builder.build(), $prompt, $history, $sender);
        }
    }};
}

async fn stream_chat_with_history(
    provider: AgentChatProvider,
    history: Vec<Message>,
    prompt: Message,
    tools: Option<AgentToolSet>,
    target_guidance: Option<String>,
) -> AgentResult<mpsc::Receiver<AgentResult<AgentChatEvent>>> {
    let (sender, receiver) = mpsc::channel(64);
    let preamble = match target_guidance {
        Some(guidance) if !guidance.trim().is_empty() => {
            format!("{SESSION_AGENT_PREAMBLE}\n\n{guidance}")
        }
        _ => SESSION_AGENT_PREAMBLE.to_string(),
    };

    match provider.kind {
        AgentChatProviderKind::OpenAi => {
            build_provider_agent!(
                openai,
                "failed to build OpenAI chat client",
                provider,
                &preamble,
                tools,
                prompt,
                history,
                sender
            );
        }
        AgentChatProviderKind::Anthropic => {
            build_provider_agent!(
                anthropic,
                "failed to build Anthropic chat client",
                provider,
                &preamble,
                tools,
                prompt,
                history,
                sender
            );
        }
        AgentChatProviderKind::DeepSeek => {
            build_provider_agent!(
                deepseek,
                "failed to build DeepSeek chat client",
                provider,
                &preamble,
                tools,
                prompt,
                history,
                sender
            );
        }
        AgentChatProviderKind::Gemini => {
            build_provider_agent!(
                gemini,
                "failed to build Gemini chat client",
                provider,
                &preamble,
                tools,
                prompt,
                history,
                sender
            );
        }
        AgentChatProviderKind::OpenRouter => {
            build_provider_agent!(
                openrouter,
                "failed to build OpenRouter chat client",
                provider,
                &preamble,
                tools,
                prompt,
                history,
                sender
            );
        }
        AgentChatProviderKind::Mistral => {
            build_provider_agent!(
                mistral,
                "failed to build Mistral chat client",
                provider,
                &preamble,
                tools,
                prompt,
                history,
                sender
            );
        }
        AgentChatProviderKind::Cohere => {
            build_provider_agent!(
                cohere,
                "failed to build Cohere chat client",
                provider,
                &preamble,
                tools,
                prompt,
                history,
                sender
            );
        }
        AgentChatProviderKind::Together => {
            build_provider_agent!(
                together,
                "failed to build Together AI chat client",
                provider,
                &preamble,
                tools,
                prompt,
                history,
                sender
            );
        }
        AgentChatProviderKind::Xai => {
            build_provider_agent!(
                xai,
                "failed to build xAI chat client",
                provider,
                &preamble,
                tools,
                prompt,
                history,
                sender
            );
        }
        AgentChatProviderKind::HuggingFace => {
            build_provider_agent!(
                huggingface,
                "failed to build Hugging Face chat client",
                provider,
                &preamble,
                tools,
                prompt,
                history,
                sender
            );
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
                Ok(MultiTurnStreamItem::CompletionCall(completion_call)) => {
                    if let Some(usage) = completion_call.usage {
                        let _ = sender
                            .send(Ok(AgentChatEvent::TokenUsage {
                                input_tokens: usage.input_tokens,
                                output_tokens: usage.output_tokens,
                            }))
                            .await;
                    }
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
            #[cfg(debug_assertions)]
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
            #[cfg(debug_assertions)]
            log::info!("agent llm reasoning block: {:?}", text);
            (!text.is_empty()).then_some(Ok(AgentChatEvent::ThinkingDelta(text)))
        }
        StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
            #[cfg(debug_assertions)]
            log::info!("agent llm reasoning delta: {:?}", reasoning);
            (!reasoning.is_empty()).then_some(Ok(AgentChatEvent::ThinkingDelta(reasoning)))
        }
        StreamedAssistantContent::ToolCall {
            tool_call,
            internal_call_id,
        } => {
            #[cfg(debug_assertions)]
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
            #[cfg(debug_assertions)]
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
            #[cfg(debug_assertions)]
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
            if let Some(message) = structured_approval_message(&result) {
                Some(Ok(AgentChatEvent::ToolCallApprovalRequired {
                    id: internal_call_id,
                    message,
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

fn structured_approval_message(result: &str) -> Option<String> {
    let response: AgentToolCallResponse = serde_json::from_str(result).ok()?;
    match response.output {
        ToolOutput::Approval { message, .. } => Some(message),
        _ => None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendRoute;

    #[test]
    fn structured_approval_message_reads_tool_output_approval() {
        let response = AgentToolCallResponse {
            tool_name: "apply_patch".to_string(),
            route: BackendRoute::SshExec,
            output: ToolOutput::Approval {
                message: "approval required".to_string(),
                operation_hash: Some("op-1".to_string()),
            },
        };
        let json = serde_json::to_string(&response).expect("response serializes");

        assert_eq!(
            structured_approval_message(&json).as_deref(),
            Some("approval required")
        );
    }

    #[test]
    fn structured_approval_message_ignores_plain_text() {
        assert_eq!(structured_approval_message("tool `x` requires user approval"), None);
    }
}
