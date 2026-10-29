//! Anthropic Messages client.
//!
//! Request building ([`build_request_body`]) and stream decoding
//! ([`AnthropicStreamDecoder`]) are pure so they can be tested without network.

use super::http::{
    StreamDecoder, build_client, error_from_payload, merge_json, model_events, send_checked,
    sse_events, trim_base_url, truncated_stream_error,
};
use super::reasoning::{ReasoningRequestPlan, plan_reasoning_request};
use super::sse::SseEvent;
use super::{
    AssistantPart, FinishReason, ModelCapabilities, ModelClient, ModelEvent, ModelEventStream,
    ModelProfile, ModelRequest, TokenUsage, UserPart, WireMessage,
};
use crate::error::{AgentError, AgentResult};
use futures::future::BoxFuture;
use serde_json::{Map, Value, json};

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u64 = 16_384;
/// Output room added on top of a thinking budget when the user did not
/// configure `max_tokens` (the API requires `max_tokens > budget_tokens`).
const THINKING_OUTPUT_HEADROOM: u64 = 16_384;

pub(crate) struct AnthropicClient {
    profile: ModelProfile,
    plan: ReasoningRequestPlan,
    http: reqwest::Client,
    url: String,
}

impl AnthropicClient {
    pub(crate) fn new(profile: ModelProfile) -> AgentResult<Self> {
        let plan = plan_reasoning_request(&profile)?;
        let http = build_client()?;
        let url = messages_url(&profile.base_url);
        Ok(Self {
            profile,
            plan,
            http,
            url,
        })
    }
}

impl ModelClient for AnthropicClient {
    fn capabilities(&self) -> ModelCapabilities {
        super::model_capabilities(&self.profile)
    }

    fn stream(&self, request: ModelRequest) -> BoxFuture<'_, AgentResult<ModelEventStream>> {
        Box::pin(async move {
            let body = build_request_body(&self.profile, &self.plan, &request);
            let http_request = self
                .http
                .post(&self.url)
                .header("x-api-key", self.profile.api_key.trim())
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .json(&body);
            let response = send_checked(http_request).await?;
            Ok(model_events(
                sse_events(response),
                AnthropicStreamDecoder::default(),
            ))
        })
    }
}

pub(crate) fn messages_url(base_url: &str) -> String {
    let base = trim_base_url(base_url);
    if base.ends_with("/v1") {
        format!("{base}/messages")
    } else {
        format!("{base}/v1/messages")
    }
}

fn ephemeral() -> Value {
    json!({ "type": "ephemeral" })
}

/// Build the JSON body for a streaming Messages request.
pub(crate) fn build_request_body(
    profile: &ModelProfile,
    plan: &ReasoningRequestPlan,
    request: &ModelRequest,
) -> Value {
    let thinking_budget = plan
        .additional_params
        .as_ref()
        .and_then(|params| params.pointer("/thinking/budget_tokens"))
        .and_then(Value::as_u64);
    let mut max_tokens = profile.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
    if let Some(budget) = thinking_budget
        && max_tokens <= budget
    {
        max_tokens = budget + THINKING_OUTPUT_HEADROOM;
    }

    let mut body = Map::new();
    body.insert("model".into(), json!(profile.model));
    body.insert("max_tokens".into(), json!(max_tokens));
    if !request.system.is_empty() {
        body.insert(
            "system".into(),
            json!([{ "type": "text", "text": request.system, "cache_control": ephemeral() }]),
        );
    }
    body.insert(
        "messages".into(),
        Value::Array(map_messages(&request.messages)),
    );
    if !request.tools.is_empty() {
        let tools = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.parameters,
                })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    body.insert("stream".into(), json!(true));
    if let Some(temperature) = profile.temperature
        && !plan.suppress_temperature
    {
        body.insert("temperature".into(), json!(temperature));
    }
    let mut body = Value::Object(body);
    if let Some(extra) = &plan.additional_params {
        merge_json(&mut body, extra);
    }
    body
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    User,
    Assistant,
}

fn map_messages(messages: &[WireMessage]) -> Vec<Value> {
    let mut grouped: Vec<(Role, Vec<Value>)> = Vec::new();
    for message in messages {
        let (role, blocks) = match message {
            WireMessage::User(parts) => (Role::User, user_blocks(parts)),
            WireMessage::Assistant(parts) => (Role::Assistant, assistant_blocks(parts)),
            WireMessage::ToolResult {
                call_id,
                content,
                is_error,
            } => (
                Role::User,
                vec![json!({
                    "type": "tool_result",
                    "tool_use_id": call_id,
                    "content": content,
                    "is_error": is_error,
                })],
            ),
        };
        if blocks.is_empty() {
            continue;
        }
        match grouped.last_mut() {
            Some((last_role, last_blocks)) if *last_role == role => last_blocks.extend(blocks),
            _ => grouped.push((role, blocks)),
        }
    }

    // Tool results must lead a user message.
    for (role, blocks) in &mut grouped {
        if *role == Role::User {
            let (mut results, rest): (Vec<Value>, Vec<Value>) = std::mem::take(blocks)
                .into_iter()
                .partition(|block| block["type"] == "tool_result");
            results.extend(rest);
            *blocks = results;
        }
    }

    if let Some((_, blocks)) = grouped
        .iter_mut()
        .rev()
        .find(|(role, _)| *role == Role::User)
        && let Some(Value::Object(block)) = blocks.last_mut()
    {
        block.insert("cache_control".into(), ephemeral());
    }

    grouped
        .into_iter()
        .map(|(role, blocks)| {
            json!({
                "role": match role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                },
                "content": blocks,
            })
        })
        .collect()
}

fn user_blocks(parts: &[UserPart]) -> Vec<Value> {
    parts
        .iter()
        .filter_map(|part| match part {
            UserPart::Text(text) if text.is_empty() => None,
            UserPart::Text(text) => Some(json!({ "type": "text", "text": text })),
            UserPart::Image {
                mime_type,
                data_base64,
            } => Some(json!({
                "type": "image",
                "source": { "type": "base64", "media_type": mime_type, "data": data_base64 },
            })),
        })
        .collect()
}

fn assistant_blocks(parts: &[AssistantPart]) -> Vec<Value> {
    parts
        .iter()
        .filter_map(|part| match part {
            AssistantPart::Reasoning {
                text,
                signature: Some(signature),
            } => Some(json!({ "type": "thinking", "thinking": text, "signature": signature })),
            // Unsigned thinking cannot be replayed to Anthropic.
            AssistantPart::Reasoning {
                signature: None, ..
            } => None,
            AssistantPart::RedactedReasoning(data) => {
                Some(json!({ "type": "redacted_thinking", "data": data }))
            }
            AssistantPart::Text(text) if text.is_empty() => None,
            AssistantPart::Text(text) => Some(json!({ "type": "text", "text": text })),
            AssistantPart::ToolCall {
                id,
                name,
                arguments,
            } => {
                let input = serde_json::from_str::<Value>(arguments)
                    .ok()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({}));
                Some(json!({ "type": "tool_use", "id": id, "name": name, "input": input }))
            }
        })
        .collect()
}

/// Messages SSE → [`ModelEvent`] state machine.
#[derive(Debug, Default)]
pub(crate) struct AnthropicStreamDecoder {
    input_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    output_tokens: u64,
    saw_usage: bool,
    saw_tool_use: bool,
    stop_reason: Option<String>,
    done: bool,
}

impl AnthropicStreamDecoder {
    fn apply_usage(&mut self, usage: Option<&Value>) {
        let Some(usage) = usage.filter(|usage| usage.is_object()) else {
            return;
        };
        self.saw_usage = true;
        let number = |key: &str| usage.get(key).and_then(Value::as_u64);
        if let Some(value) = number("input_tokens") {
            self.input_tokens = value;
        }
        if let Some(value) = number("cache_read_input_tokens") {
            self.cache_read_tokens = value;
        }
        if let Some(value) = number("cache_creation_input_tokens") {
            self.cache_creation_tokens = value;
        }
        if let Some(value) = number("output_tokens") {
            self.output_tokens = value;
        }
    }
}

fn text_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

impl StreamDecoder for AnthropicStreamDecoder {
    fn push(&mut self, event: &SseEvent) -> AgentResult<Vec<ModelEvent>> {
        let data = event.data.trim();
        if data.is_empty() {
            return Ok(Vec::new());
        }
        let value: Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(error) => {
                log::warn!("ignoring malformed Anthropic stream event: {error}");
                return Ok(Vec::new());
            }
        };
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .or(event.event.as_deref())
            .unwrap_or("");
        let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        let mut events = Vec::new();
        match kind {
            "message_start" => {
                self.apply_usage(
                    value
                        .get("message")
                        .and_then(|message| message.get("usage")),
                );
            }
            "content_block_start" => {
                let block = value.get("content_block").unwrap_or(&Value::Null);
                match block.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text" => {
                        if let Some(text) = text_field(block, "text") {
                            events.push(ModelEvent::TextDelta(text));
                        }
                    }
                    "thinking" => {
                        if let Some(text) = text_field(block, "thinking") {
                            events.push(ModelEvent::ReasoningDelta(text));
                        }
                        if let Some(signature) = text_field(block, "signature") {
                            events.push(ModelEvent::ReasoningSignature(signature));
                        }
                    }
                    "redacted_thinking" => {
                        events.push(ModelEvent::RedactedReasoning(
                            text_field(block, "data").unwrap_or_default(),
                        ));
                    }
                    "tool_use" => {
                        self.saw_tool_use = true;
                        events.push(ModelEvent::ToolCallStart {
                            index,
                            id: text_field(block, "id").unwrap_or_default(),
                            name: text_field(block, "name").unwrap_or_default(),
                        });
                        if let Some(input) = block
                            .get("input")
                            .and_then(Value::as_object)
                            .filter(|input| !input.is_empty())
                        {
                            events.push(ModelEvent::ToolCallArgsDelta {
                                index,
                                delta: Value::Object(input.clone()).to_string(),
                            });
                        }
                    }
                    _ => {}
                }
            }
            "content_block_delta" => {
                let delta = value.get("delta").unwrap_or(&Value::Null);
                match delta.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text_delta" => {
                        if let Some(text) = text_field(delta, "text") {
                            events.push(ModelEvent::TextDelta(text));
                        }
                    }
                    "thinking_delta" => {
                        if let Some(text) = text_field(delta, "thinking") {
                            events.push(ModelEvent::ReasoningDelta(text));
                        }
                    }
                    "signature_delta" => {
                        if let Some(signature) = text_field(delta, "signature") {
                            events.push(ModelEvent::ReasoningSignature(signature));
                        }
                    }
                    "input_json_delta" => {
                        if let Some(partial) = text_field(delta, "partial_json") {
                            events.push(ModelEvent::ToolCallArgsDelta {
                                index,
                                delta: partial,
                            });
                        }
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(reason) = value
                    .get("delta")
                    .and_then(|delta| text_field(delta, "stop_reason"))
                {
                    self.stop_reason = Some(reason);
                }
                self.apply_usage(value.get("usage"));
            }
            "message_stop" => self.done = true,
            "error" => return Err(AgentError::Model(error_from_payload(&value))),
            // ping, content_block_stop and future event types.
            _ => {}
        }
        Ok(events)
    }

    fn is_done(&self) -> bool {
        self.done
    }

    fn finish(&mut self) -> AgentResult<Vec<ModelEvent>> {
        if !self.done && self.stop_reason.is_none() {
            return Err(truncated_stream_error());
        }
        let mut events = Vec::new();
        if self.saw_usage {
            events.push(ModelEvent::Usage(TokenUsage {
                // Anthropic reports uncached input separately from cache
                // reads/writes; expose the total prompt size.
                input_tokens: self.input_tokens
                    + self.cache_read_tokens
                    + self.cache_creation_tokens,
                output_tokens: self.output_tokens,
                cached_input_tokens: self.cache_read_tokens,
            }));
        }
        let reason = match self.stop_reason.take().as_deref() {
            Some("end_turn" | "stop_sequence") => FinishReason::Stop,
            Some("tool_use") => FinishReason::ToolCalls,
            Some("max_tokens") => FinishReason::Length,
            Some(other) => FinishReason::Other(other.to_owned()),
            None if self.saw_tool_use => FinishReason::ToolCalls,
            None => FinishReason::Stop,
        };
        events.push(ModelEvent::Finish(reason));
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ToolSpec;
    use crate::model::http::test_support::{last_model_error, ok_events, run_fixture};
    use crate::model::reasoning::tests::profile;
    use miaominal_settings::{AiProviderKind, AiReasoningEffort};

    async fn decode(fixture: &str) -> Vec<AgentResult<ModelEvent>> {
        run_fixture(fixture, AnthropicStreamDecoder::default()).await
    }

    const THINKING_TOOL_USE: &str = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_01","type":"message","role":"assistant","model":"claude-sonnet-4-6","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":120,"cache_creation_input_tokens":0,"cache_read_input_tokens":2000,"output_tokens":3}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}

event: ping
data: {"type": "ping"}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"I should read "}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"the file."}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQBCgIYAhIM1gbcDa9GJwZA2b3h"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Reading it now."}}

event: content_block_stop
data: {"type":"content_block_stop","index":1}

event: content_block_start
data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_01A","name":"read","input":{}}}

event: content_block_delta
data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"path\": \"src/"}}

event: content_block_delta
data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"main.rs\"}"}}

event: content_block_stop
data: {"type":"content_block_stop","index":2}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":89}}

event: message_stop
data: {"type":"message_stop"}

"#;

    #[tokio::test]
    async fn decodes_thinking_signature_and_tool_use() {
        assert_eq!(
            ok_events(decode(THINKING_TOOL_USE).await),
            vec![
                ModelEvent::ReasoningDelta("I should read ".into()),
                ModelEvent::ReasoningDelta("the file.".into()),
                ModelEvent::ReasoningSignature("EqQBCgIYAhIM1gbcDa9GJwZA2b3h".into()),
                ModelEvent::TextDelta("Reading it now.".into()),
                ModelEvent::ToolCallStart {
                    index: 2,
                    id: "toolu_01A".into(),
                    name: "read".into()
                },
                ModelEvent::ToolCallArgsDelta {
                    index: 2,
                    delta: "{\"path\": \"src/".into()
                },
                ModelEvent::ToolCallArgsDelta {
                    index: 2,
                    delta: "main.rs\"}".into()
                },
                ModelEvent::Usage(TokenUsage {
                    input_tokens: 2120,
                    output_tokens: 89,
                    cached_input_tokens: 2000,
                }),
                ModelEvent::Finish(FinishReason::ToolCalls),
            ]
        );
    }

    const REDACTED: &str = r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_02","type":"message","role":"assistant","content":[],"usage":{"input_tokens":10,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"EmwKAhgBEgy3va3pzix/LafPsn4a"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Done."}}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":12}}

event: message_stop
data: {"type":"message_stop"}
"#;

    #[tokio::test]
    async fn decodes_redacted_thinking() {
        assert_eq!(
            ok_events(decode(REDACTED).await),
            vec![
                ModelEvent::RedactedReasoning("EmwKAhgBEgy3va3pzix/LafPsn4a".into()),
                ModelEvent::TextDelta("Done.".into()),
                ModelEvent::Usage(TokenUsage {
                    input_tokens: 10,
                    output_tokens: 12,
                    cached_input_tokens: 0,
                }),
                ModelEvent::Finish(FinishReason::Stop),
            ]
        );
    }

    #[tokio::test]
    async fn error_event_is_retryable_when_overloaded() {
        let fixture = r#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":1}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}

event: error
data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}

"#;
        let results = decode(fixture).await;
        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0].as_ref().unwrap(),
            &ModelEvent::TextDelta("Hi".into())
        );
        let error = last_model_error(&results);
        assert!(error.retryable);
        assert_eq!(error.message, "Overloaded");
    }

    #[tokio::test]
    async fn abrupt_end_and_max_tokens() {
        let cut = r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}
"#;
        let results = decode(cut).await;
        assert!(last_model_error(&results).retryable);

        let length = r#"data: {"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":5}}

data: {"type":"message_stop"}
"#;
        assert_eq!(
            ok_events(decode(length).await).last(),
            Some(&ModelEvent::Finish(FinishReason::Length))
        );
    }

    #[test]
    fn builds_urls() {
        assert_eq!(
            messages_url("https://api.anthropic.com/"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            messages_url("https://proxy.example/anthropic/v1"),
            "https://proxy.example/anthropic/v1/messages"
        );
    }

    #[test]
    fn builds_request_body_with_merging_and_cache_control() {
        let mut claude = profile(
            AiProviderKind::Anthropic,
            "claude-3-7-sonnet-latest",
            AiReasoningEffort::Low,
        );
        claude.max_tokens = Some(8192);
        let plan = plan_reasoning_request(&claude).unwrap();
        let request = ModelRequest {
            system: "Be brief.".into(),
            messages: vec![
                WireMessage::User(vec![
                    UserPart::Text("Look".into()),
                    UserPart::Image {
                        mime_type: "image/jpeg".into(),
                        data_base64: "/9j/".into(),
                    },
                ]),
                WireMessage::Assistant(vec![
                    AssistantPart::Reasoning {
                        text: "Plan".into(),
                        signature: Some("sig".into()),
                    },
                    AssistantPart::Reasoning {
                        text: "unsigned".into(),
                        signature: None,
                    },
                    AssistantPart::RedactedReasoning("opaque".into()),
                    AssistantPart::Text(String::new()),
                    AssistantPart::Text("Calling.".into()),
                    AssistantPart::ToolCall {
                        id: "toolu_1".into(),
                        name: "read".into(),
                        arguments: "{\"path\":\"a\"}".into(),
                    },
                    AssistantPart::ToolCall {
                        id: "toolu_2".into(),
                        name: "list".into(),
                        arguments: "not json".into(),
                    },
                ]),
                WireMessage::ToolResult {
                    call_id: "toolu_1".into(),
                    content: "contents".into(),
                    is_error: false,
                },
                WireMessage::User(vec![UserPart::Text("Also this".into())]),
                WireMessage::ToolResult {
                    call_id: "toolu_2".into(),
                    content: "failed".into(),
                    is_error: true,
                },
            ],
            tools: vec![ToolSpec {
                name: "read".into(),
                description: "Read a file".into(),
                parameters: json!({"type":"object"}),
            }],
        };
        let body = build_request_body(&claude, &plan, &request);
        assert_eq!(
            body,
            json!({
                "model": "claude-3-7-sonnet-latest",
                "max_tokens": 8192,
                "system": [{"type": "text", "text": "Be brief.", "cache_control": {"type": "ephemeral"}}],
                "messages": [
                    {"role": "user", "content": [
                        {"type": "text", "text": "Look"},
                        {"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": "/9j/"}}
                    ]},
                    {"role": "assistant", "content": [
                        {"type": "thinking", "thinking": "Plan", "signature": "sig"},
                        {"type": "redacted_thinking", "data": "opaque"},
                        {"type": "text", "text": "Calling."},
                        {"type": "tool_use", "id": "toolu_1", "name": "read", "input": {"path": "a"}},
                        {"type": "tool_use", "id": "toolu_2", "name": "list", "input": {}}
                    ]},
                    {"role": "user", "content": [
                        {"type": "tool_result", "tool_use_id": "toolu_1", "content": "contents", "is_error": false},
                        {"type": "tool_result", "tool_use_id": "toolu_2", "content": "failed", "is_error": true},
                        {"type": "text", "text": "Also this", "cache_control": {"type": "ephemeral"}}
                    ]}
                ],
                "tools": [{"name": "read", "description": "Read a file", "input_schema": {"type": "object"}}],
                "stream": true,
                "thinking": {"type": "enabled", "budget_tokens": 4096}
            })
        );
    }

    #[test]
    fn default_max_tokens_exceeds_thinking_budget_and_keeps_temperature() {
        let claude = profile(
            AiProviderKind::Anthropic,
            "claude-opus-4-1",
            AiReasoningEffort::High,
        );
        let plan = plan_reasoning_request(&claude).unwrap();
        let request = ModelRequest {
            system: String::new(),
            messages: vec![WireMessage::User(vec![UserPart::Text("hi".into())])],
            tools: Vec::new(),
        };
        let body = build_request_body(&claude, &plan, &request);
        assert_eq!(body["thinking"]["budget_tokens"], 16384);
        assert!(body["max_tokens"].as_u64().unwrap() > 16384);
        assert!(body.get("temperature").is_none());
        assert!(body.get("system").is_none());
        assert!(body.get("tools").is_none());

        let plain = profile(
            AiProviderKind::Anthropic,
            "claude-3-5-sonnet-latest",
            AiReasoningEffort::Default,
        );
        let body = build_request_body(&plain, &ReasoningRequestPlan::default(), &request);
        assert_eq!(body["max_tokens"], 16384);
        assert_eq!(body["temperature"], 0.7);
        assert_eq!(
            body["messages"],
            json!([{"role": "user", "content": [
                {"type": "text", "text": "hi", "cache_control": {"type": "ephemeral"}}
            ]}])
        );
    }
}
