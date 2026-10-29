//! OpenAI-compatible Chat Completions client.
//!
//! Request building ([`build_request_body`]) and stream decoding
//! ([`OpenAiStreamDecoder`]) are pure so they can be tested without network.

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
use miaominal_settings::AiProviderKind;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

pub(crate) struct OpenAiChatClient {
    profile: ModelProfile,
    plan: ReasoningRequestPlan,
    http: reqwest::Client,
    url: String,
}

impl OpenAiChatClient {
    pub(crate) fn new(profile: ModelProfile) -> AgentResult<Self> {
        let plan = plan_reasoning_request(&profile)?;
        let http = build_client()?;
        let url = chat_completions_url(&profile.base_url);
        Ok(Self {
            profile,
            plan,
            http,
            url,
        })
    }

    fn http_request(&self, body: &Value) -> reqwest::RequestBuilder {
        let mut request = self
            .http
            .post(&self.url)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .json(body);
        let api_key = self.profile.api_key.trim();
        if !api_key.is_empty() {
            request = request.bearer_auth(api_key);
        }
        if self.profile.kind == AiProviderKind::OpenRouter {
            request = request
                .header("HTTP-Referer", "https://miaominal.app")
                .header("X-Title", "Miaominal");
        }
        request
    }
}

impl ModelClient for OpenAiChatClient {
    fn capabilities(&self) -> ModelCapabilities {
        super::model_capabilities(&self.profile)
    }

    fn stream(&self, request: ModelRequest) -> BoxFuture<'_, AgentResult<ModelEventStream>> {
        Box::pin(async move {
            let body = build_request_body(&self.profile, &self.plan, &request, true);
            let response = match send_checked(self.http_request(&body)).await {
                Ok(response) => response,
                Err(AgentError::Model(error)) if rejects_stream_options(&error) => {
                    log::info!(
                        "provider `{}` rejected stream_options; retrying without usage reporting",
                        self.profile.provider_name
                    );
                    let body = build_request_body(&self.profile, &self.plan, &request, false);
                    send_checked(self.http_request(&body)).await?
                }
                Err(error) => return Err(error),
            };
            Ok(model_events(
                sse_events(response),
                OpenAiStreamDecoder::default(),
            ))
        })
    }
}

fn rejects_stream_options(error: &crate::error::ModelError) -> bool {
    matches!(error.status, Some(400 | 422))
        && error
            .message
            .to_ascii_lowercase()
            .contains("stream_options")
}

pub(crate) fn chat_completions_url(base_url: &str) -> String {
    format!("{}/chat/completions", trim_base_url(base_url))
}

/// Build the JSON body for a streaming Chat Completions request.
pub(crate) fn build_request_body(
    profile: &ModelProfile,
    plan: &ReasoningRequestPlan,
    request: &ModelRequest,
    include_stream_options: bool,
) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(profile.model));
    body.insert(
        "messages".into(),
        Value::Array(map_messages(&request.system, &request.messages)),
    );
    if !request.tools.is_empty() {
        let tools = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    body.insert("stream".into(), json!(true));
    if include_stream_options {
        body.insert("stream_options".into(), json!({ "include_usage": true }));
    }
    if let Some(temperature) = profile.temperature
        && !plan.suppress_temperature
    {
        body.insert("temperature".into(), json!(temperature));
    }
    if let Some(max_tokens) = profile.max_tokens {
        let key = if profile.kind == AiProviderKind::OpenAi {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        body.insert(key.into(), json!(max_tokens));
    }
    let mut body = Value::Object(body);
    if let Some(extra) = &plan.additional_params {
        merge_json(&mut body, extra);
    }
    body
}

fn map_messages(system: &str, messages: &[WireMessage]) -> Vec<Value> {
    let mut out = Vec::with_capacity(messages.len() + 1);
    if !system.is_empty() {
        out.push(json!({ "role": "system", "content": system }));
    }
    for message in messages {
        match message {
            WireMessage::User(parts) => out.push(json!({
                "role": "user",
                "content": user_content(parts),
            })),
            WireMessage::Assistant(parts) => out.push(assistant_message(parts)),
            WireMessage::ToolResult {
                call_id, content, ..
            } => out.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": content,
            })),
        }
    }
    out
}

fn user_content(parts: &[UserPart]) -> Value {
    if parts.iter().all(|part| matches!(part, UserPart::Text(_))) {
        let text: Vec<&str> = parts
            .iter()
            .filter_map(|part| match part {
                UserPart::Text(text) => Some(text.as_str()),
                UserPart::Image { .. } => None,
            })
            .collect();
        return Value::String(text.join("\n"));
    }
    Value::Array(
        parts
            .iter()
            .map(|part| match part {
                UserPart::Text(text) => json!({ "type": "text", "text": text }),
                UserPart::Image {
                    mime_type,
                    data_base64,
                } => json!({
                    "type": "image_url",
                    "image_url": { "url": format!("data:{mime_type};base64,{data_base64}") },
                }),
            })
            .collect(),
    )
}

fn assistant_message(parts: &[AssistantPart]) -> Value {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut has_reasoning = false;
    let mut tool_calls = Vec::new();
    for part in parts {
        match part {
            AssistantPart::Text(part) => text.push_str(part),
            AssistantPart::Reasoning { text: part, .. } => {
                has_reasoning = true;
                reasoning.push_str(part);
            }
            // Anthropic-only payload; meaningless to OpenAI-compatible APIs.
            AssistantPart::RedactedReasoning(_) => {}
            AssistantPart::ToolCall {
                id,
                name,
                arguments,
            } => tool_calls.push(json!({
                "id": id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": if arguments.trim().is_empty() { "{}" } else { arguments.as_str() },
                },
            })),
        }
    }
    let mut message = Map::new();
    message.insert("role".into(), json!("assistant"));
    let content = if !text.is_empty() {
        Value::String(text)
    } else if tool_calls.is_empty() {
        Value::String(String::new())
    } else {
        Value::Null
    };
    message.insert("content".into(), content);
    if has_reasoning {
        message.insert("reasoning_content".into(), Value::String(reasoning));
    }
    if !tool_calls.is_empty() {
        message.insert("tool_calls".into(), Value::Array(tool_calls));
    }
    Value::Object(message)
}

#[derive(Debug)]
struct ToolCallState {
    id: Option<String>,
    name: String,
    started: bool,
    pending_args: String,
}

/// Chat Completions SSE → [`ModelEvent`] state machine.
#[derive(Debug, Default)]
pub(crate) struct OpenAiStreamDecoder {
    calls: Vec<ToolCallState>,
    /// Server-side tool call index → position in `calls`.
    slots: HashMap<u64, usize>,
    finish_reason: Option<String>,
    usage: Option<TokenUsage>,
    done: bool,
}

impl OpenAiStreamDecoder {
    fn push_tool_call(&mut self, position: usize, call: &Value, events: &mut Vec<ModelEvent>) {
        let key = call
            .get("index")
            .and_then(Value::as_u64)
            .unwrap_or(position as u64);
        let id = non_empty_str(call.get("id"));
        let function = call.get("function");
        let name = non_empty_str(function.and_then(|function| function.get("name")));
        let arguments = match function.and_then(|function| function.get("arguments")) {
            Some(Value::String(arguments)) => arguments.clone(),
            Some(Value::Null) | None => String::new(),
            // Some servers send already-parsed arguments.
            Some(other) => other.to_string(),
        };

        let existing = self.slots.get(&key).copied().filter(|slot| {
            // A different id at the same index means a new call (servers that
            // reuse index 0 for every parallel call).
            match (&id, &self.calls[*slot].id) {
                (Some(new), Some(old)) => new == old,
                _ => true,
            }
        });
        let slot = match existing {
            Some(slot) => slot,
            None => {
                self.calls.push(ToolCallState {
                    id: None,
                    name: String::new(),
                    started: false,
                    pending_args: String::new(),
                });
                let slot = self.calls.len() - 1;
                self.slots.insert(key, slot);
                slot
            }
        };
        let state = &mut self.calls[slot];
        if !state.started {
            if state.id.is_none() {
                state.id = id;
            }
            if let Some(name) = name {
                state.name = name;
            }
        }
        if !state.started && !state.name.is_empty() {
            start_call(slot, state, events);
        }
        if !arguments.is_empty() {
            if state.started {
                events.push(ModelEvent::ToolCallArgsDelta {
                    index: slot,
                    delta: arguments,
                });
            } else {
                state.pending_args.push_str(&arguments);
            }
        }
    }
}

fn start_call(slot: usize, state: &mut ToolCallState, events: &mut Vec<ModelEvent>) {
    let id = state
        .id
        .get_or_insert_with(|| format!("call_{}", uuid::Uuid::new_v4().simple()))
        .clone();
    state.started = true;
    events.push(ModelEvent::ToolCallStart {
        index: slot,
        id,
        name: state.name.clone(),
    });
    if !state.pending_args.is_empty() {
        events.push(ModelEvent::ToolCallArgsDelta {
            index: slot,
            delta: std::mem::take(&mut state.pending_args),
        });
    }
}

fn non_empty_str(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn parse_usage(usage: &Value) -> TokenUsage {
    let number = |value: Option<&Value>| value.and_then(Value::as_u64).unwrap_or(0);
    let cached = usage
        .get("prompt_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64)
        .or_else(|| usage.get("prompt_cache_hit_tokens").and_then(Value::as_u64))
        .unwrap_or(0);
    TokenUsage {
        input_tokens: number(usage.get("prompt_tokens")),
        output_tokens: number(usage.get("completion_tokens")),
        cached_input_tokens: cached,
    }
}

impl StreamDecoder for OpenAiStreamDecoder {
    fn push(&mut self, event: &SseEvent) -> AgentResult<Vec<ModelEvent>> {
        let data = event.data.trim();
        if data.is_empty() {
            return Ok(Vec::new());
        }
        if data == "[DONE]" {
            self.done = true;
            return Ok(Vec::new());
        }
        let value: Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(error) => {
                log::warn!("ignoring malformed chat completion chunk: {error}");
                return Ok(Vec::new());
            }
        };
        if value.get("error").is_some_and(|error| !error.is_null()) {
            return Err(AgentError::Model(error_from_payload(&value)));
        }
        if let Some(usage) = value.get("usage").filter(|usage| usage.is_object()) {
            self.usage = Some(parse_usage(usage));
        }
        let mut events = Vec::new();
        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return Ok(events);
        };
        if let Some(delta) = choice.get("delta") {
            let reasoning = non_empty_str(delta.get("reasoning_content"))
                .or_else(|| non_empty_str(delta.get("reasoning")));
            if let Some(reasoning) = reasoning {
                events.push(ModelEvent::ReasoningDelta(reasoning));
            }
            if let Some(content) = non_empty_str(delta.get("content")) {
                events.push(ModelEvent::TextDelta(content));
            }
            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for (position, call) in calls.iter().enumerate() {
                    self.push_tool_call(position, call, &mut events);
                }
            }
        }
        if let Some(reason) = non_empty_str(choice.get("finish_reason")) {
            self.finish_reason = Some(reason);
        }
        Ok(events)
    }

    fn is_done(&self) -> bool {
        self.done
    }

    fn finish(&mut self) -> AgentResult<Vec<ModelEvent>> {
        if !self.done && self.finish_reason.is_none() {
            return Err(truncated_stream_error());
        }
        let mut events = Vec::new();
        for (slot, state) in self.calls.iter_mut().enumerate() {
            if !state.started {
                // The name never arrived; surface the call anyway so the
                // engine can report an unknown tool instead of losing it.
                start_call(slot, state, &mut events);
            }
        }
        if let Some(usage) = self.usage.take() {
            events.push(ModelEvent::Usage(usage));
        }
        let has_calls = !self.calls.is_empty();
        let reason = match self.finish_reason.take().as_deref() {
            Some("stop") | None if has_calls => FinishReason::ToolCalls,
            Some("stop") | None => FinishReason::Stop,
            Some("tool_calls" | "function_call") => FinishReason::ToolCalls,
            Some("length") => FinishReason::Length,
            Some(other) => FinishReason::Other(other.to_owned()),
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
    use miaominal_settings::AiReasoningEffort;

    async fn decode(fixture: &str) -> Vec<AgentResult<ModelEvent>> {
        run_fixture(fixture, OpenAiStreamDecoder::default()).await
    }

    const OPENAI_TEXT: &str = r#"data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","content":"","refusal":null},"logprobs":null,"finish_reason":null}],"usage":null}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"Hello"},"logprobs":null,"finish_reason":null}],"usage":null}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":" world"},"logprobs":null,"finish_reason":null}],"usage":null}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[{"index":0,"delta":{},"logprobs":null,"finish_reason":"stop"}],"usage":null}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":1,"model":"gpt-4o","choices":[],"usage":{"prompt_tokens":20,"completion_tokens":2,"total_tokens":22,"prompt_tokens_details":{"cached_tokens":8}}}

data: [DONE]

"#;

    #[tokio::test]
    async fn decodes_text_stream_with_trailing_usage() {
        assert_eq!(
            ok_events(decode(OPENAI_TEXT).await),
            vec![
                ModelEvent::TextDelta("Hello".into()),
                ModelEvent::TextDelta(" world".into()),
                ModelEvent::Usage(TokenUsage {
                    input_tokens: 20,
                    output_tokens: 2,
                    cached_input_tokens: 8,
                }),
                ModelEvent::Finish(FinishReason::Stop),
            ]
        );
    }

    const OPENAI_PARALLEL_TOOLS: &str = r#"data: {"id":"c","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"read","arguments":""}}]},"finish_reason":null}]}

data: {"id":"c","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"pa"}}]},"finish_reason":null}]}

data: {"id":"c","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_b","type":"function","function":{"name":"glob","arguments":""}}]},"finish_reason":null}]}

data: {"id":"c","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"a.rs\"}"}}]},"finish_reason":null}]}

data: {"id":"c","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"arguments":"{\"pattern\":\"*.md\"}"}}]},"finish_reason":null}]}

data: {"id":"c","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}

data: [DONE]

"#;

    #[tokio::test]
    async fn decodes_parallel_tool_calls_split_across_chunks() {
        assert_eq!(
            ok_events(decode(OPENAI_PARALLEL_TOOLS).await),
            vec![
                ModelEvent::ToolCallStart {
                    index: 0,
                    id: "call_a".into(),
                    name: "read".into()
                },
                ModelEvent::ToolCallArgsDelta {
                    index: 0,
                    delta: "{\"pa".into()
                },
                ModelEvent::ToolCallStart {
                    index: 1,
                    id: "call_b".into(),
                    name: "glob".into()
                },
                ModelEvent::ToolCallArgsDelta {
                    index: 0,
                    delta: "th\":\"a.rs\"}".into()
                },
                ModelEvent::ToolCallArgsDelta {
                    index: 1,
                    delta: "{\"pattern\":\"*.md\"}".into()
                },
                ModelEvent::Finish(FinishReason::ToolCalls),
            ]
        );
    }

    const DEEPSEEK_REASONING_TOOL: &str = r#"data: {"id":"d","object":"chat.completion.chunk","model":"deepseek-v4-pro","choices":[{"index":0,"delta":{"role":"assistant","content":null,"reasoning_content":""},"finish_reason":null}]}

data: {"id":"d","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":null,"reasoning_content":"Need to list"},"finish_reason":null}]}

data: {"id":"d","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":null,"reasoning_content":" files."},"finish_reason":null}]}

data: {"id":"d","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"Checking."},"finish_reason":null}]}

data: {"id":"d","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_00_x","type":"function","function":{"name":"list","arguments":"{\"path\":\".\"}"}}]},"finish_reason":null}]}

data: {"id":"d","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":""},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":100,"completion_tokens":30,"total_tokens":130,"prompt_cache_hit_tokens":64,"prompt_cache_miss_tokens":36}}

data: [DONE]

"#;

    #[tokio::test]
    async fn decodes_deepseek_reasoning_and_tool_call() {
        assert_eq!(
            ok_events(decode(DEEPSEEK_REASONING_TOOL).await),
            vec![
                ModelEvent::ReasoningDelta("Need to list".into()),
                ModelEvent::ReasoningDelta(" files.".into()),
                ModelEvent::TextDelta("Checking.".into()),
                ModelEvent::ToolCallStart {
                    index: 0,
                    id: "call_00_x".into(),
                    name: "list".into()
                },
                ModelEvent::ToolCallArgsDelta {
                    index: 0,
                    delta: "{\"path\":\".\"}".into()
                },
                ModelEvent::Usage(TokenUsage {
                    input_tokens: 100,
                    output_tokens: 30,
                    cached_input_tokens: 64,
                }),
                ModelEvent::Finish(FinishReason::ToolCalls),
            ]
        );
    }

    #[tokio::test]
    async fn usage_before_finish_and_missing_done_are_accepted() {
        let fixture = r#"data: {"choices":[{"index":0,"delta":{"reasoning":"hm"}}]}

data: {"choices":[{"index":0,"delta":{"content":"ok"}}],"usage":{"prompt_tokens":5,"completion_tokens":1}}

data: {"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}

"#;
        assert_eq!(
            ok_events(decode(fixture).await),
            vec![
                ModelEvent::ReasoningDelta("hm".into()),
                ModelEvent::TextDelta("ok".into()),
                ModelEvent::Usage(TokenUsage {
                    input_tokens: 5,
                    output_tokens: 1,
                    cached_input_tokens: 0,
                }),
                ModelEvent::Finish(FinishReason::Length),
            ]
        );
    }

    #[tokio::test]
    async fn delays_start_until_name_and_generates_missing_ids() {
        let fixture = r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"a\":"}}]}}]}

data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"name":"read","arguments":"1}"}}]}}]}

data: {"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: [DONE]
"#;
        let events = ok_events(decode(fixture).await);
        let ModelEvent::ToolCallStart { index, id, name } = &events[0] else {
            panic!("expected start, got {events:?}");
        };
        assert_eq!((*index, name.as_str()), (0, "read"));
        assert!(id.starts_with("call_") && id.len() > 10);
        assert_eq!(
            events[1..],
            [
                ModelEvent::ToolCallArgsDelta {
                    index: 0,
                    delta: "{\"a\":".into()
                },
                ModelEvent::ToolCallArgsDelta {
                    index: 0,
                    delta: "1}".into()
                },
                // `stop` with tool calls is normalized to ToolCalls.
                ModelEvent::Finish(FinishReason::ToolCalls),
            ]
        );
    }

    #[tokio::test]
    async fn reused_index_with_new_id_is_a_new_call() {
        let fixture = r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"read","arguments":"{}"}}]}}]}

data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"b","function":{"name":"list","arguments":"{}"}}]}}]}

data: [DONE]
"#;
        let events = ok_events(decode(fixture).await);
        assert_eq!(
            events,
            vec![
                ModelEvent::ToolCallStart {
                    index: 0,
                    id: "a".into(),
                    name: "read".into()
                },
                ModelEvent::ToolCallArgsDelta {
                    index: 0,
                    delta: "{}".into()
                },
                ModelEvent::ToolCallStart {
                    index: 1,
                    id: "b".into(),
                    name: "list".into()
                },
                ModelEvent::ToolCallArgsDelta {
                    index: 1,
                    delta: "{}".into()
                },
                ModelEvent::Finish(FinishReason::ToolCalls),
            ]
        );
    }

    #[tokio::test]
    async fn mid_stream_error_payload_is_an_error() {
        let fixture = r#"data: {"choices":[{"index":0,"delta":{"content":"par"}}]}

data: {"id":"x","object":"chat.completion.chunk","error":{"code":502,"message":"Upstream provider disconnected"},"choices":[{"index":0,"delta":{"content":""},"finish_reason":"error"}]}

data: [DONE]
"#;
        let results = decode(fixture).await;
        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0].as_ref().unwrap(),
            &ModelEvent::TextDelta("par".into())
        );
        let error = last_model_error(&results);
        assert_eq!(error.message, "Upstream provider disconnected");
        assert!(error.retryable);
    }

    #[tokio::test]
    async fn abrupt_end_is_retryable_error() {
        let fixture = r#"data: {"choices":[{"index":0,"delta":{"content":"half"}}]}

data: {"choices":[{"index":0,"delta":{"content":" an ans"#;
        let results = decode(fixture).await;
        assert_eq!(
            results[0].as_ref().unwrap(),
            &ModelEvent::TextDelta("half".into())
        );
        let error = last_model_error(&results);
        assert!(error.retryable);
        assert_eq!(error.status, None);
        assert_eq!(results.len(), 2);
    }

    fn sample_request() -> ModelRequest {
        ModelRequest {
            system: "You are helpful.".into(),
            messages: vec![
                WireMessage::User(vec![UserPart::Text("List files".into())]),
                WireMessage::Assistant(vec![
                    AssistantPart::Reasoning {
                        text: "Need ".into(),
                        signature: None,
                    },
                    AssistantPart::Reasoning {
                        text: "tools.".into(),
                        signature: None,
                    },
                    AssistantPart::ToolCall {
                        id: "call_1".into(),
                        name: "list".into(),
                        arguments: "{\"path\":\".\"}".into(),
                    },
                    AssistantPart::ToolCall {
                        id: "call_2".into(),
                        name: "read".into(),
                        arguments: "".into(),
                    },
                ]),
                WireMessage::ToolResult {
                    call_id: "call_1".into(),
                    content: "a.rs".into(),
                    is_error: false,
                },
                WireMessage::ToolResult {
                    call_id: "call_2".into(),
                    content: "denied".into(),
                    is_error: true,
                },
                WireMessage::User(vec![
                    UserPart::Text("What is this?".into()),
                    UserPart::Image {
                        mime_type: "image/png".into(),
                        data_base64: "iVBOR".into(),
                    },
                ]),
                WireMessage::Assistant(vec![AssistantPart::Text("A cat.".into())]),
            ],
            tools: vec![ToolSpec {
                name: "list".into(),
                description: "List a directory".into(),
                parameters: json!({"type":"object","properties":{"path":{"type":"string"}}}),
            }],
        }
    }

    #[test]
    fn builds_request_body_with_deepseek_reasoning_replay() {
        let mut deepseek = profile(
            AiProviderKind::DeepSeek,
            "deepseek-v4-pro",
            AiReasoningEffort::High,
        );
        deepseek.max_tokens = Some(4096);
        let plan = plan_reasoning_request(&deepseek).unwrap();
        let body = build_request_body(&deepseek, &plan, &sample_request(), true);
        assert_eq!(
            body,
            json!({
                "model": "deepseek-v4-pro",
                "messages": [
                    {"role": "system", "content": "You are helpful."},
                    {"role": "user", "content": "List files"},
                    {
                        "role": "assistant",
                        "content": null,
                        "reasoning_content": "Need tools.",
                        "tool_calls": [
                            {"id": "call_1", "type": "function", "function": {"name": "list", "arguments": "{\"path\":\".\"}"}},
                            {"id": "call_2", "type": "function", "function": {"name": "read", "arguments": "{}"}}
                        ]
                    },
                    {"role": "tool", "tool_call_id": "call_1", "content": "a.rs"},
                    {"role": "tool", "tool_call_id": "call_2", "content": "denied"},
                    {"role": "user", "content": [
                        {"type": "text", "text": "What is this?"},
                        {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBOR"}}
                    ]},
                    {"role": "assistant", "content": "A cat."}
                ],
                "tools": [{"type": "function", "function": {
                    "name": "list",
                    "description": "List a directory",
                    "parameters": {"type":"object","properties":{"path":{"type":"string"}}}
                }}],
                "stream": true,
                "stream_options": {"include_usage": true},
                "temperature": 0.7,
                "max_tokens": 4096,
                "thinking": {"type": "enabled"},
                "reasoning_effort": "high"
            })
        );
    }

    #[test]
    fn openai_uses_max_completion_tokens_and_omits_optional_fields() {
        let mut openai = profile(AiProviderKind::OpenAi, "gpt-5", AiReasoningEffort::Default);
        openai.max_tokens = Some(1000);
        openai.temperature = None;
        let request = ModelRequest {
            system: String::new(),
            messages: vec![WireMessage::User(vec![UserPart::Text("hi".into())])],
            tools: Vec::new(),
        };
        let body = build_request_body(&openai, &ReasoningRequestPlan::default(), &request, false);
        assert_eq!(
            body,
            json!({
                "model": "gpt-5",
                "messages": [{"role": "user", "content": "hi"}],
                "stream": true,
                "max_completion_tokens": 1000
            })
        );
    }

    #[test]
    fn suppressed_temperature_is_omitted() {
        let openai = profile(AiProviderKind::OpenAi, "gpt-5", AiReasoningEffort::Default);
        let plan = ReasoningRequestPlan {
            additional_params: Some(json!({"reasoning_effort": "low"})),
            suppress_temperature: true,
        };
        let body = build_request_body(&openai, &plan, &sample_request(), true);
        assert!(body.get("temperature").is_none());
        assert_eq!(body["reasoning_effort"], "low");
    }

    #[test]
    fn builds_urls_and_detects_stream_options_rejection() {
        assert_eq!(
            chat_completions_url("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/chat/completions"
        );
        let rejection = crate::model::http::error_from_status(
            400,
            r#"{"error":{"message":"Unrecognized request argument supplied: stream_options"}}"#,
        );
        assert!(rejects_stream_options(&rejection));
        assert!(!rejects_stream_options(
            &crate::model::http::error_from_status(400, "bad")
        ));
    }

    #[test]
    fn new_rejects_unsupported_reasoning() {
        let unsupported = profile(AiProviderKind::OpenAi, "gpt-4o", AiReasoningEffort::High);
        assert!(matches!(
            OpenAiChatClient::new(unsupported),
            Err(AgentError::UnsupportedReasoningEffort { .. })
        ));
    }
}
