//! Model access layer.
//!
//! The engine talks to language models only through [`ModelClient`]. Two wire
//! protocols are implemented in-house (no third-party agent framework):
//! OpenAI-compatible Chat Completions and Anthropic Messages. Provider presets
//! from Settings select defaults (base URL, protocol, capability table).

mod anthropic;
mod capabilities;
mod http;
mod openai_chat;
mod reasoning;
mod sse;

pub use capabilities::{ProviderPreset, model_capabilities, provider_preset};
pub use reasoning::{AgentReasoningSupport, agent_reasoning_support};

use crate::error::AgentResult;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use miaominal_settings::{AiProviderKind, AiReasoningEffort};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// Wire protocol spoken by a provider endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireProtocol {
    /// `POST {base_url}/chat/completions` with SSE streaming.
    OpenAiChat,
    /// `POST {base_url}/v1/messages` with SSE streaming.
    Anthropic,
}

/// Fully resolved model configuration used for one session.
#[derive(Clone, Debug)]
pub struct ModelProfile {
    pub provider_id: String,
    pub provider_name: String,
    pub kind: AiProviderKind,
    pub protocol: WireProtocol,
    pub model: String,
    /// Already resolved: empty user input is replaced by the preset default.
    pub base_url: String,
    pub api_key: String,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u64>,
    pub reasoning_effort: AiReasoningEffort,
    /// User override of the context window in tokens.
    pub context_window: Option<u64>,
}

/// How reasoning produced by the model must be replayed in later requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningReplay {
    /// Never send reasoning back.
    Never,
    /// Send back only for completions inside the current tool loop (i.e. after
    /// the latest user message). Required by DeepSeek thinking mode with tools
    /// and by Anthropic extended thinking with tool use.
    CurrentTurn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelCapabilities {
    pub vision: bool,
    pub parallel_tool_calls: bool,
    pub reasoning_replay: ReasoningReplay,
    /// Context window in tokens (override applied).
    pub context_window: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON schema object for the arguments.
    pub parameters: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserPart {
    Text(String),
    Image {
        mime_type: String,
        data_base64: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssistantPart {
    Text(String),
    /// Model reasoning. `signature` is Anthropic's thinking signature.
    Reasoning {
        text: String,
        signature: Option<String>,
    },
    /// Anthropic `redacted_thinking` block payload.
    RedactedReasoning(String),
    ToolCall {
        id: String,
        name: String,
        /// Raw JSON argument text exactly as produced by the model.
        arguments: String,
    },
}

/// Provider-neutral conversation message.
///
/// Adapters are responsible for protocol-specific grouping, e.g. Anthropic
/// requires consecutive tool results (and any following user text) to be
/// merged into one `user` message, while OpenAI uses one `tool` message per
/// result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireMessage {
    User(Vec<UserPart>),
    Assistant(Vec<AssistantPart>),
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelRequest {
    pub system: String,
    pub messages: Vec<WireMessage>,
    pub tools: Vec<ToolSpec>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Input tokens served from the provider prompt cache, when reported.
    pub cached_input_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    Other(String),
}

/// Streaming event produced by a model adapter.
///
/// Ordering contract: events arrive in the order the model produced content.
/// A `ReasoningSignature` closes the reasoning block currently being streamed.
/// Tool call `index` values are unique within one completion; argument deltas
/// for an index always follow its `ToolCallStart`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ReasoningSignature(String),
    RedactedReasoning(String),
    ToolCallStart {
        index: usize,
        id: String,
        name: String,
    },
    ToolCallArgsDelta {
        index: usize,
        delta: String,
    },
    Usage(TokenUsage),
    Finish(FinishReason),
}

pub type ModelEventStream = BoxStream<'static, AgentResult<ModelEvent>>;

pub trait ModelClient: Send + Sync {
    fn capabilities(&self) -> ModelCapabilities;

    /// Start one streaming completion. Errors before the first byte (HTTP
    /// status, connection) are returned from the future; errors mid-stream are
    /// yielded as stream items. Dropping the stream aborts the request.
    fn stream(&self, request: ModelRequest) -> BoxFuture<'_, AgentResult<ModelEventStream>>;
}

impl ModelProfile {
    /// Resolve a Settings provider entry. Fails for providers that need
    /// sign-in flows not supported yet (ChatGPT/Copilot OAuth) and for
    /// custom endpoints without a base URL.
    pub fn from_provider(
        config: &miaominal_settings::AiProviderConfig,
        api_key: String,
    ) -> AgentResult<Self> {
        let preset = provider_preset(config.kind);
        if !preset.supported {
            return Err(crate::error::AgentError::UnsupportedProvider(format!(
                "{} 需要账号登录，当前版本的助手暂不支持，请改用 API Key 方式的服务商。",
                config.name
            )));
        }
        let base_url = if config.base_url.trim().is_empty() {
            preset.default_base_url.to_string()
        } else {
            config.base_url.trim().trim_end_matches('/').to_string()
        };
        if base_url.is_empty() {
            return Err(crate::error::AgentError::UnsupportedProvider(format!(
                "服务商 {} 没有填写 Base URL。",
                config.name
            )));
        }
        Ok(Self {
            provider_id: config.id.clone(),
            provider_name: config.name.clone(),
            kind: config.kind,
            protocol: preset.protocol,
            model: config.model.trim().to_string(),
            base_url,
            api_key,
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            reasoning_effort: config.reasoning_effort,
            context_window: config.context_window,
        })
    }
}

/// Build the client for a resolved profile.
pub fn client_for(profile: ModelProfile) -> AgentResult<Arc<dyn ModelClient>> {
    match profile.protocol {
        WireProtocol::OpenAiChat => Ok(Arc::new(openai_chat::OpenAiChatClient::new(profile)?)),
        WireProtocol::Anthropic => Ok(Arc::new(anthropic::AnthropicClient::new(profile)?)),
    }
}
