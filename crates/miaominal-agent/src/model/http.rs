//! Shared HTTP helpers for model clients: client construction, error
//! classification and turning a streaming response into model events.

use super::ModelEventStream;
use super::sse::{SseDecoder, SseEvent};
use crate::error::{AgentError, AgentResult, ModelError};
use futures::stream::{self, BoxStream, StreamExt};
use serde_json::Value;
use std::collections::VecDeque;
use std::time::Duration;

use super::ModelEvent;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Maximum silence between two body reads. There is deliberately no total
/// timeout because completions can stream for many minutes.
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_ERROR_BODY_CHARS: usize = 2000;

/// Build the HTTP client shared by all requests of one model client.
pub(crate) fn build_client() -> AgentResult<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_IDLE_TIMEOUT)
        .build()
        .map_err(|error| {
            AgentError::Model(ModelError {
                status: None,
                message: format!("failed to build HTTP client: {}", error_chain(&error)),
                retryable: false,
                context_overflow: false,
            })
        })
}

pub(crate) fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 409 | 429 | 500 | 502 | 503 | 504 | 529)
}

pub(crate) fn is_context_overflow(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "context_length_exceeded",
        "maximum context length",
        "prompt is too long",
        "too many tokens",
        "context window",
    ]
    .iter()
    .any(|pattern| text.contains(pattern))
}

/// Extract a human readable message from a JSON error body.
pub(crate) fn extract_error_message(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body.trim()).ok()?;
    message_from_value(&value)
}

fn message_from_value(value: &Value) -> Option<String> {
    match value {
        // Gemini wraps errors in an array.
        Value::Array(items) => items.iter().find_map(message_from_value),
        Value::Object(object) => {
            if let Some(error) = object.get("error") {
                match error {
                    Value::String(message) => return Some(message.clone()),
                    Value::Object(inner) => {
                        if let Some(Value::String(message)) = inner.get("message") {
                            return Some(message.clone());
                        }
                    }
                    _ => {}
                }
            }
            match object.get("message").or_else(|| object.get("detail")) {
                Some(Value::String(message)) => Some(message.clone()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Map a non-success HTTP response to a [`ModelError`].
pub(crate) fn error_from_status(status: u16, body: &str) -> ModelError {
    let message = extract_error_message(body).unwrap_or_else(|| {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            format!("HTTP {status}")
        } else {
            truncate(trimmed, MAX_ERROR_BODY_CHARS)
        }
    });
    let context_overflow = is_context_overflow(&message) || is_context_overflow(body);
    ModelError {
        status: Some(status),
        message,
        retryable: is_retryable_status(status) && !context_overflow,
        context_overflow,
    }
}

/// Map an error payload received inside a stream (`{"error": {...}}` or an
/// Anthropic `error` event) to a [`ModelError`].
pub(crate) fn error_from_payload(value: &Value) -> ModelError {
    let error = value.get("error").unwrap_or(value);
    let message = message_from_value(value)
        .or_else(|| error.as_str().map(str::to_owned))
        .unwrap_or_else(|| truncate(&error.to_string(), MAX_ERROR_BODY_CHARS));
    let kind = error.get("type").and_then(Value::as_str).unwrap_or("");
    let code = error.get("code");
    let numeric_code = code
        .and_then(|code| {
            code.as_u64()
                .or_else(|| code.as_str().and_then(|code| code.parse().ok()))
        })
        .filter(|code| (100..600).contains(code))
        .map(|code| code as u16);
    let code_text = code.and_then(Value::as_str).unwrap_or("");
    const RETRYABLE_KINDS: &[&str] = &[
        "overloaded_error",
        "rate_limit_error",
        "rate_limit_exceeded",
        "api_error",
        "server_error",
        "timeout_error",
        "service_unavailable",
    ];
    let context_overflow = is_context_overflow(&message) || is_context_overflow(code_text);
    let retryable = !context_overflow
        && (RETRYABLE_KINDS.contains(&kind)
            || RETRYABLE_KINDS.contains(&code_text)
            || numeric_code.is_some_and(is_retryable_status));
    ModelError {
        status: numeric_code,
        message,
        retryable,
        context_overflow,
    }
}

/// Map a transport error to a [`ModelError`].
pub(crate) fn error_from_reqwest(error: &reqwest::Error) -> ModelError {
    let status = error.status().map(|status| status.as_u16());
    let retryable = error.is_timeout()
        || error.is_connect()
        || error.is_request()
        || error.is_body()
        || status.is_some_and(is_retryable_status);
    ModelError {
        status,
        message: error_chain(error),
        retryable,
        context_overflow: false,
    }
}

fn error_chain(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !message.contains(&cause_text) {
            message.push_str(": ");
            message.push_str(&cause_text);
        }
        source = cause.source();
    }
    message
}

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_owned(),
    }
}

/// Send a request and turn non-success statuses into [`AgentError::Model`].
pub(crate) async fn send_checked(
    request: reqwest::RequestBuilder,
) -> AgentResult<reqwest::Response> {
    let response = request
        .send()
        .await
        .map_err(|error| AgentError::Model(error_from_reqwest(&error)))?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    Err(AgentError::Model(error_from_status(status.as_u16(), &body)))
}

/// Read a response body incrementally (without reqwest's `stream` feature)
/// and decode it as server-sent events.
pub(crate) fn sse_events(response: reqwest::Response) -> BoxStream<'static, AgentResult<SseEvent>> {
    struct State {
        response: reqwest::Response,
        decoder: SseDecoder,
        queue: VecDeque<SseEvent>,
        done: bool,
    }
    let state = State {
        response,
        decoder: SseDecoder::new(),
        queue: VecDeque::new(),
        done: false,
    };
    stream::unfold(state, |mut state| async move {
        loop {
            if let Some(event) = state.queue.pop_front() {
                return Some((Ok(event), state));
            }
            if state.done {
                return None;
            }
            match state.response.chunk().await {
                Ok(Some(bytes)) => state.queue.extend(state.decoder.push(&bytes)),
                Ok(None) => {
                    state.done = true;
                    state.queue.extend(state.decoder.finish());
                }
                Err(error) => {
                    state.done = true;
                    let mut model_error = error_from_reqwest(&error);
                    model_error.message =
                        format!("response stream interrupted: {}", model_error.message);
                    model_error.retryable = true;
                    return Some((Err(AgentError::Model(model_error)), state));
                }
            }
        }
    })
    .boxed()
}

/// Protocol-specific SSE → [`ModelEvent`] state machine.
pub(crate) trait StreamDecoder: Send + 'static {
    /// Consume one SSE event. A returned error terminates the stream.
    fn push(&mut self, event: &SseEvent) -> AgentResult<Vec<ModelEvent>>;
    /// The protocol signalled the end of the completion (`[DONE]`,
    /// `message_stop`); no further input is read.
    fn is_done(&self) -> bool;
    /// Called exactly once when the completion ends (done or end of input).
    /// Emits the trailing `Usage`/`Finish` events, or an error when the stream
    /// was cut short.
    fn finish(&mut self) -> AgentResult<Vec<ModelEvent>>;
}

/// Drive a decoder over an SSE event stream.
pub(crate) fn model_events<D: StreamDecoder>(
    input: BoxStream<'static, AgentResult<SseEvent>>,
    decoder: D,
) -> ModelEventStream {
    struct State<D> {
        input: BoxStream<'static, AgentResult<SseEvent>>,
        decoder: D,
        queue: VecDeque<ModelEvent>,
        finished: bool,
    }
    let state = State {
        input,
        decoder,
        queue: VecDeque::new(),
        finished: false,
    };
    stream::unfold(state, |mut state| async move {
        loop {
            if let Some(event) = state.queue.pop_front() {
                return Some((Ok(event), state));
            }
            if state.finished {
                return None;
            }
            let next = if state.decoder.is_done() {
                None
            } else {
                state.input.next().await
            };
            match next {
                Some(Ok(event)) => match state.decoder.push(&event) {
                    Ok(events) => state.queue.extend(events),
                    Err(error) => {
                        state.finished = true;
                        return Some((Err(error), state));
                    }
                },
                Some(Err(error)) => {
                    state.finished = true;
                    return Some((Err(error), state));
                }
                None => {
                    state.finished = true;
                    match state.decoder.finish() {
                        Ok(events) => state.queue.extend(events),
                        Err(error) => return Some((Err(error), state)),
                    }
                }
            }
        }
    })
    .boxed()
}

/// Error for a stream that ended before the protocol's completion marker.
pub(crate) fn truncated_stream_error() -> AgentError {
    AgentError::Model(ModelError {
        status: None,
        message: "model response stream ended unexpectedly".to_owned(),
        retryable: true,
        context_overflow: false,
    })
}

/// Deep-merge `extra` into `target`: objects merge recursively, any other
/// value replaces the existing one.
pub(crate) fn merge_json(target: &mut Value, extra: &Value) {
    match (target, extra) {
        (Value::Object(target), Value::Object(extra)) => {
            for (key, value) in extra {
                match target.get_mut(key) {
                    Some(existing) => merge_json(existing, value),
                    None => {
                        target.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (target, extra) => *target = extra.clone(),
    }
}

/// Trim trailing slashes from a resolved base URL.
pub(crate) fn trim_base_url(base_url: &str) -> &str {
    base_url.trim().trim_end_matches('/')
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// Decode a recorded SSE fixture into model events through the same
    /// driver used for live responses.
    pub(crate) async fn run_fixture<D: StreamDecoder>(
        fixture: &str,
        decoder: D,
    ) -> Vec<AgentResult<ModelEvent>> {
        let mut sse = SseDecoder::new();
        let mut events: Vec<AgentResult<SseEvent>> =
            sse.push(fixture.as_bytes()).into_iter().map(Ok).collect();
        events.extend(sse.finish().into_iter().map(Ok));
        model_events(stream::iter(events).boxed(), decoder)
            .collect()
            .await
    }

    /// Unwrap all events, panicking on the first error.
    pub(crate) fn ok_events(results: Vec<AgentResult<ModelEvent>>) -> Vec<ModelEvent> {
        results
            .into_iter()
            .map(|result| result.unwrap_or_else(|error| panic!("unexpected error: {error}")))
            .collect()
    }

    pub(crate) fn last_model_error(results: &[AgentResult<ModelEvent>]) -> &ModelError {
        match results.last() {
            Some(Err(AgentError::Model(error))) => error,
            other => panic!("expected trailing model error, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_http_statuses() {
        for status in [408, 409, 429, 500, 502, 503, 504, 529] {
            let error = error_from_status(status, "");
            assert!(error.retryable, "{status}");
            assert_eq!(error.status, Some(status));
            assert_eq!(error.message, format!("HTTP {status}"));
        }
        for status in [400, 401, 403, 404, 422] {
            assert!(!error_from_status(status, "{}").retryable, "{status}");
        }
    }

    #[test]
    fn extracts_messages_from_common_error_bodies() {
        let openai = r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error","code":"invalid_api_key"}}"#;
        assert_eq!(
            error_from_status(401, openai).message,
            "Incorrect API key provided"
        );
        let anthropic =
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
        let error = error_from_status(529, anthropic);
        assert_eq!(error.message, "Overloaded");
        assert!(error.retryable);
        let gemini =
            r#"[{"error":{"code":400,"message":"API key not valid","status":"INVALID_ARGUMENT"}}]"#;
        assert_eq!(error_from_status(400, gemini).message, "API key not valid");
        assert_eq!(
            error_from_status(502, "<html>Bad Gateway</html>").message,
            "<html>Bad Gateway</html>"
        );
        let long = "x".repeat(5000);
        assert_eq!(
            error_from_status(500, &long).message.chars().count(),
            MAX_ERROR_BODY_CHARS + 1
        );
    }

    #[test]
    fn detects_context_overflow() {
        let openai = r#"{"error":{"message":"This model's maximum context length is 128000 tokens. However, your messages resulted in 130000 tokens.","type":"invalid_request_error","code":"context_length_exceeded"}}"#;
        let error = error_from_status(400, openai);
        assert!(error.context_overflow);
        assert!(!error.retryable);
        let anthropic = r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 210000 tokens > 200000 maximum"}}"#;
        assert!(error_from_status(400, anthropic).context_overflow);
        let code_only = r#"{"error":{"message":"bad request","code":"context_length_exceeded"}}"#;
        assert!(error_from_status(400, code_only).context_overflow);
        assert!(error_from_status(413, "Too many tokens in request").context_overflow);
        assert!(
            !error_from_status(400, r#"{"error":{"message":"invalid model"}}"#).context_overflow
        );
    }

    #[test]
    fn classifies_stream_error_payloads() {
        let overloaded = error_from_payload(
            &json!({"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}),
        );
        assert!(overloaded.retryable);
        assert_eq!(overloaded.message, "Overloaded");

        let invalid = error_from_payload(
            &json!({"error":{"type":"invalid_request_error","message":"bad tool schema"}}),
        );
        assert!(!invalid.retryable);

        let coded = error_from_payload(&json!({"error":{"code":429,"message":"rate limited"}}));
        assert!(coded.retryable);
        assert_eq!(coded.status, Some(429));

        let overflow = error_from_payload(
            &json!({"error":{"code":"context_length_exceeded","message":"too long"}}),
        );
        assert!(overflow.context_overflow);
        assert!(!overflow.retryable);
    }

    #[test]
    fn deep_merges_json() {
        let mut target = json!({"a": 1, "nested": {"x": 1, "y": 2}});
        merge_json(&mut target, &json!({"b": 2, "nested": {"y": 3, "z": 4}}));
        assert_eq!(
            target,
            json!({"a": 1, "b": 2, "nested": {"x": 1, "y": 3, "z": 4}})
        );
    }

    #[test]
    fn trims_base_url() {
        assert_eq!(trim_base_url(" https://x/v1// "), "https://x/v1");
    }

    #[test]
    fn builds_client() {
        build_client().unwrap();
    }
}
