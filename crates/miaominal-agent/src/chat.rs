use crate::channel::{AgentToolCallResponse, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::tools::{AgentToolCancellation, AgentToolSet};
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
use std::collections::{HashMap, HashSet};
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
    /// All tools, policy bypassed entirely, tool calls require user approval before execution.
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
    ToolCallCancelled {
        id: String,
    },
    ToolCallAutoExecuteRequired {
        id: String,
    },
    ToolCallApprovalRequired {
        id: String,
        message: String,
    },
    ToolCallUserInputRequired {
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

const SESSION_AGENT_PREAMBLE: &str = "You are Miaominal's terminal-side assistant. Help with shell, SSH, SFTP, and general development questions. Be concise, practical, and ask for clarification only when needed.\n\nTool contract:\n- Use only the tools listed in this session. Do not invent tool names.\n- There is no `write`, `edit`, or `replace` tool. For any file creation or modification, use `apply_patch` with a unified patch.\n- Use `read`, `list`, `glob`, and `grep` to inspect files before patching.\n- Treat `workspace_info.shell` as the command syntax used by `run_shell` on the exec channel, even if the SSH login/default shell is different. If it is `cmd`, use CMD syntax such as `dir` and `type`; do not use POSIX commands or bare PowerShell syntax unless you explicitly invoke `powershell.exe -NoProfile -Command ...`.\n- Use `run_shell` for commands expected to finish quickly. Use `start_job` only for long-running commands such as servers, watchers, deploys, logs, or slow test suites.\n- After `start_job`, keep track of the returned job_id and call `poll_job` until the job exits or you explicitly tell the user it is still running. If you forget the id, use `list_jobs`.\n- Use `ask_user` when you need user input before continuing. Provide a clear message and at most three concise choices; the user can also enter a custom response.\n- If a file change needs approval, call `apply_patch` normally and let Miaominal request approval.";
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
        Err(_error) => {
            #[cfg(debug_assertions)]
            log::info!("title generation failed: {_error:?}");
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
            | AgentChatEvent::ToolCallCancelled { .. }
            | AgentChatEvent::ToolCallAutoExecuteRequired { .. }
            | AgentChatEvent::ToolCallApprovalRequired { .. }
            | AgentChatEvent::ToolCallUserInputRequired { .. }
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
            let tool_mode = the_tools.mode();
            let tool_cancellation = the_tools.cancellation();
            spawn_stream_chat(
                agent_builder.tools(the_tools.into_rig_tools()).build(),
                $prompt,
                $history,
                $sender,
                Some(tool_mode),
                Some(tool_cancellation),
            );
        } else {
            spawn_stream_chat(
                agent_builder.build(),
                $prompt,
                $history,
                $sender,
                None,
                None,
            );
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
    tool_mode: Option<AgentMode>,
    tool_cancellation: Option<AgentToolCancellation>,
) where
    M: CompletionModel + Send + Sync + 'static,
    M::StreamingResponse: Send + Unpin + GetTokenUsage + 'static,
{
    tokio::spawn(async move {
        let Some(mut stream) =
            await_or_chat_receiver_closed(&sender, tool_cancellation.as_ref(), true, async move {
                agent.stream_chat(prompt, history).await
            })
            .await
        else {
            return;
        };
        let mut final_reply = String::new();

        // Track streamed tool calls to work around a rig_core bug where
        // tool calls delivered only as ToolCallDelta (no complete ToolCall)
        // are buffered internally but never executed, so the agent emits
        // FinalResponse without running them.
        let mut pending_streamed_tools: HashMap<String, PendingStreamedTool> = HashMap::new();
        let mut active_tool_ids = HashSet::new();

        loop {
            // A UI Stop cancels the shared tool set before it closes this receiver. Finish the
            // stream item already being polled so a completed tool result can be delivered, but
            // never poll another item (and potentially another tool) after cancellation.
            if tool_cancellation
                .as_ref()
                .is_some_and(AgentToolCancellation::is_cancelled_runtime)
                && active_tool_ids.is_empty()
            {
                break;
            }
            let Some(item) = await_or_chat_receiver_closed(
                &sender,
                tool_cancellation.as_ref(),
                active_tool_ids.is_empty(),
                stream.next(),
            )
            .await
            else {
                break;
            };
            let Some(item) = item else {
                break;
            };
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
                    let usage = completion_call.usage;
                    let _ = sender
                        .send(Ok(AgentChatEvent::TokenUsage {
                            input_tokens: usage.input_tokens,
                            output_tokens: usage.output_tokens,
                        }))
                        .await;
                    #[cfg(debug_assertions)]
                    log::info!("agent llm completion call boundary");
                    None
                }
                Ok(MultiTurnStreamItem::FinalResponse(response)) => {
                    if !pending_streamed_tools.is_empty() {
                        if tool_cancellation
                            .as_ref()
                            .is_some_and(AgentToolCancellation::is_cancelled_runtime)
                        {
                            break;
                        }
                        let auto_execute = matches!(tool_mode, Some(AgentMode::FullAuto));
                        log::warn!(
                            "rig_core did not execute {} streamed tool call(s); forwarding to client",
                            pending_streamed_tools.len()
                        );
                        for (_cid, tool) in pending_streamed_tools.drain() {
                            if !send_pending_streamed_tool_handoff(&sender, tool, auto_execute)
                                .await
                            {
                                break;
                            }
                        }
                        continue; // skip Finished, let the loop drain the stream
                    }
                    let response = response.response().to_string();
                    #[cfg(debug_assertions)]
                    log::info!("agent llm final response: {:?}", response);
                    Some(Ok(AgentChatEvent::Finished(response)))
                }
                Ok(_) => None,
                Err(error) => {
                    #[cfg(debug_assertions)]
                    log::info!("agent llm stream error: {error:?}");
                    Some(Err(streaming_error(error)))
                }
            };

            let approval_required = matches!(
                event.as_ref(),
                Some(Ok(AgentChatEvent::ToolCallApprovalRequired { .. }))
                    | Some(Ok(AgentChatEvent::ToolCallUserInputRequired { .. }))
            );
            let stream_failed = matches!(event.as_ref(), Some(Err(_)));
            if let Some(Ok(event)) = event.as_ref() {
                match event {
                    AgentChatEvent::ToolCallStarted(tool) => {
                        active_tool_ids.insert(tool.id.clone());
                    }
                    AgentChatEvent::ToolCallCompleted { id, .. }
                    | AgentChatEvent::ToolCallCancelled { id }
                    | AgentChatEvent::ToolCallAutoExecuteRequired { id }
                    | AgentChatEvent::ToolCallApprovalRequired { id, .. }
                    | AgentChatEvent::ToolCallUserInputRequired { id, .. } => {
                        active_tool_ids.remove(id);
                    }
                    _ => {}
                }
            }
            if let Some(event) = event
                && sender.send(event).await.is_err()
            {
                if let Some(tool_cancellation) = tool_cancellation.as_ref() {
                    tool_cancellation.cancel_and_wait().await;
                }
                break;
            }
            if approval_required {
                break;
            }
            if stream_failed
                && tool_cancellation
                    .as_ref()
                    .is_some_and(AgentToolCancellation::is_cancelled_runtime)
            {
                break;
            }
        }

        // If there are still unexecuted tool calls after the loop (e.g.,
        // FinalResponse was handled without us intercepting it), hand them to
        // the UI for approval or automatic execution depending on agent mode.
        if !pending_streamed_tools.is_empty()
            && !tool_cancellation
                .as_ref()
                .is_some_and(AgentToolCancellation::is_cancelled_runtime)
        {
            let auto_execute = matches!(tool_mode, Some(AgentMode::FullAuto));
            log::warn!(
                "rig_core stream ended with {} unexecuted tool call(s); forwarding to client",
                pending_streamed_tools.len()
            );
            for (_cid, tool) in pending_streamed_tools.drain() {
                if !send_pending_streamed_tool_handoff(&sender, tool, auto_execute).await {
                    break;
                }
            }
        }

        #[cfg(debug_assertions)]
        log::info!(
            "agent llm stream ended; accumulated_text_delta={:?}",
            final_reply
        );
    });
}

async fn await_or_chat_receiver_closed<T>(
    sender: &mpsc::Sender<AgentResult<AgentChatEvent>>,
    tool_cancellation: Option<&AgentToolCancellation>,
    stop_on_tool_cancellation: bool,
    future: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::pin!(future);
    let cancellation_requested = async {
        if stop_on_tool_cancellation {
            match tool_cancellation {
                Some(tool_cancellation) => tool_cancellation.cancelled().await,
                None => std::future::pending::<()>().await,
            }
        } else {
            std::future::pending::<()>().await;
        }
    };
    tokio::pin!(cancellation_requested);
    tokio::select! {
        biased;
        _ = &mut cancellation_requested => {
            if let Some(tool_cancellation) = tool_cancellation {
                tool_cancellation.cancel_and_wait().await;
            }
            None
        },
        output = &mut future => Some(output),
        _ = sender.closed() => {
            if let Some(tool_cancellation) = tool_cancellation {
                tool_cancellation.cancel_and_wait().await;
            }
            None
        },
    }
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

async fn send_pending_streamed_tool_handoff(
    sender: &mpsc::Sender<AgentResult<AgentChatEvent>>,
    tool: PendingStreamedTool,
    auto_execute: bool,
) -> bool {
    let PendingStreamedTool {
        internal_call_id,
        name,
        arguments,
    } = tool;

    if sender
        .send(Ok(AgentChatEvent::ToolCallStarted(AgentChatToolEvent {
            id: internal_call_id.clone(),
            name: name.clone(),
            arguments,
        })))
        .await
        .is_err()
    {
        return false;
    }

    let event = if auto_execute {
        AgentChatEvent::ToolCallAutoExecuteRequired {
            id: internal_call_id,
        }
    } else {
        AgentChatEvent::ToolCallApprovalRequired {
            id: internal_call_id,
            message: format!("tool `{name}` requires user approval"),
        }
    };

    sender.send(Ok(event)).await.is_ok()
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
            #[cfg(debug_assertions)]
            log::info!(
                "agent llm tool result returned to model: id={} result={:?}",
                internal_call_id,
                result
            );
            match structured_tool_pause(&result) {
                Some(ToolPause::ApprovalRequired { message }) => {
                    Some(Ok(AgentChatEvent::ToolCallApprovalRequired {
                        id: internal_call_id,
                        message,
                    }))
                }
                Some(ToolPause::UserInputRequired { message }) => {
                    Some(Ok(AgentChatEvent::ToolCallUserInputRequired {
                        id: internal_call_id,
                        message,
                    }))
                }
                Some(ToolPause::Cancelled) => Some(Ok(AgentChatEvent::ToolCallCancelled {
                    id: internal_call_id,
                })),
                None => Some(Ok(AgentChatEvent::ToolCallCompleted {
                    id: internal_call_id,
                    result,
                })),
            }
        }
    }
}

enum ToolPause {
    ApprovalRequired { message: String },
    UserInputRequired { message: String },
    Cancelled,
}

fn structured_tool_pause(result: &str) -> Option<ToolPause> {
    let response: AgentToolCallResponse = serde_json::from_str(result).ok()?;
    match response.output {
        ToolOutput::Cancelled => Some(ToolPause::Cancelled),
        ToolOutput::Approval { message, .. } => Some(ToolPause::ApprovalRequired { message }),
        ToolOutput::UserQuestion { message, .. } => Some(ToolPause::UserInputRequired { message }),
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
    fn structured_tool_pause_reads_tool_output_approval() {
        let response = AgentToolCallResponse {
            tool_name: "apply_patch".to_string(),
            route: BackendRoute::SshExec,
            output: ToolOutput::Approval {
                message: "approval required".to_string(),
                operation_hash: Some("op-1".to_string()),
            },
        };
        let json = serde_json::to_string(&response).expect("response serializes");

        match structured_tool_pause(&json) {
            Some(ToolPause::ApprovalRequired { message }) => {
                assert_eq!(message, "approval required")
            }
            _ => panic!("expected approval pause"),
        }
    }

    #[test]
    fn structured_tool_pause_reads_user_question() {
        let response = AgentToolCallResponse {
            tool_name: "ask_user".to_string(),
            route: BackendRoute::SshExec,
            output: ToolOutput::UserQuestion {
                message: "Which branch?".to_string(),
                choices: Vec::new(),
                allow_custom: true,
                operation_hash: None,
            },
        };
        let json = serde_json::to_string(&response).expect("response serializes");

        match structured_tool_pause(&json) {
            Some(ToolPause::UserInputRequired { message }) => {
                assert_eq!(message, "Which branch?")
            }
            _ => panic!("expected user input pause"),
        }
    }

    #[test]
    fn structured_tool_cancellation_maps_to_a_cancelled_event() {
        let response = AgentToolCallResponse {
            tool_name: "run_shell".to_string(),
            route: BackendRoute::SshExec,
            output: ToolOutput::Cancelled,
        };
        let json = serde_json::to_string(&response).expect("response serializes");
        let event = chat_event_from_user_content(StreamedUserContent::ToolResult {
            tool_result: rig_core::message::ToolResult {
                id: "provider-call".to_string(),
                call_id: None,
                content: OneOrMany::one(ToolResultContent::text(json)),
            },
            internal_call_id: "tool-1".to_string(),
        })
        .expect("tool result should map to an event")
        .expect("structured cancellation should not be a stream error");

        assert_eq!(
            event,
            AgentChatEvent::ToolCallCancelled {
                id: "tool-1".to_string(),
            }
        );
    }

    #[test]
    fn structured_tool_pause_ignores_plain_text() {
        assert!(structured_tool_pause("tool `x` requires user approval").is_none());
    }

    #[tokio::test]
    async fn pending_streamed_handoff_auto_executes_when_requested() {
        let (sender, mut receiver) = mpsc::channel(2);
        let sent = send_pending_streamed_tool_handoff(
            &sender,
            PendingStreamedTool {
                internal_call_id: "call-1".to_string(),
                name: "run_shell".to_string(),
                arguments: "{\"command\":\"dir\"}".to_string(),
            },
            true,
        )
        .await;

        assert!(sent);
        assert_eq!(
            receiver.recv().await.expect("started event").unwrap(),
            AgentChatEvent::ToolCallStarted(AgentChatToolEvent {
                id: "call-1".to_string(),
                name: "run_shell".to_string(),
                arguments: "{\"command\":\"dir\"}".to_string(),
            })
        );
        assert_eq!(
            receiver.recv().await.expect("auto event").unwrap(),
            AgentChatEvent::ToolCallAutoExecuteRequired {
                id: "call-1".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn pending_streamed_handoff_requests_approval_when_not_auto() {
        let (sender, mut receiver) = mpsc::channel(2);
        let sent = send_pending_streamed_tool_handoff(
            &sender,
            PendingStreamedTool {
                internal_call_id: "call-2".to_string(),
                name: "apply_patch".to_string(),
                arguments: "{}".to_string(),
            },
            false,
        )
        .await;

        assert!(sent);
        let _ = receiver.recv().await.expect("started event").unwrap();
        assert_eq!(
            receiver.recv().await.expect("approval event").unwrap(),
            AgentChatEvent::ToolCallApprovalRequired {
                id: "call-2".to_string(),
                message: "tool `apply_patch` requires user approval".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn dropping_chat_receiver_cancels_the_in_flight_stream_item() {
        struct DropFlag(std::sync::Arc<std::sync::atomic::AtomicBool>);

        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::Release);
            }
        }

        let (sender, receiver) = mpsc::channel(1);
        let tool_cancellation = AgentToolCancellation::new();
        let started = std::sync::Arc::new(tokio::sync::Notify::new());
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stream_started = started.clone();
        let stream_dropped = dropped.clone();
        let stream_item = async move {
            let _drop_flag = DropFlag(stream_dropped);
            stream_started.notify_one();
            std::future::pending::<()>().await;
        };
        let close_receiver = async move {
            started.notified().await;
            drop(receiver);
        };

        let (result, ()) = tokio::join!(
            await_or_chat_receiver_closed(&sender, Some(&tool_cancellation), true, stream_item,),
            close_receiver,
        );

        assert!(result.is_none());
        assert!(dropped.load(std::sync::atomic::Ordering::Acquire));
        assert!(tool_cancellation.is_cancelled());
    }

    #[tokio::test]
    async fn completed_stream_item_wins_over_a_simultaneously_closed_receiver() {
        let (sender, receiver) = mpsc::channel(1);
        let tool_cancellation = AgentToolCancellation::new();
        drop(receiver);

        let result = await_or_chat_receiver_closed(
            &sender,
            Some(&tool_cancellation),
            true,
            std::future::ready("completed"),
        )
        .await;

        assert_eq!(result, Some("completed"));
        assert!(!tool_cancellation.is_cancelled());
    }

    #[tokio::test]
    async fn explicit_cancellation_interrupts_a_pending_item_without_closing_the_receiver() {
        let (sender, _receiver) = mpsc::channel(1);
        let tool_cancellation = AgentToolCancellation::new();
        tool_cancellation.cancel();

        let result = await_or_chat_receiver_closed(
            &sender,
            Some(&tool_cancellation),
            true,
            std::future::pending::<()>(),
        )
        .await;

        assert!(result.is_none());
        assert!(tool_cancellation.is_cancelled());
    }
}
