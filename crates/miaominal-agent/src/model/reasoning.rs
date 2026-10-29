//! Reasoning effort support table and request parameters.
//!
//! Ported from the former rig-based implementation. Parameters are plain JSON
//! that the protocol adapters deep-merge into the request body.

use super::ModelProfile;
use crate::error::{AgentError, AgentResult};
use miaominal_settings::{AiProviderKind, AiReasoningEffort};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentReasoningSupport {
    Supported,
    Unsupported,
    Unknown,
}

/// Extra request parameters needed for the configured reasoning effort.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ReasoningRequestPlan {
    pub additional_params: Option<Value>,
    pub suppress_temperature: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnthropicThinkingMode {
    Adaptive,
    Budget,
}

pub fn agent_reasoning_support(kind: AiProviderKind, model: &str) -> AgentReasoningSupport {
    let model = model.trim().to_ascii_lowercase();
    match kind {
        AiProviderKind::OpenAi => {
            if starts_with_any(&model, &["o1", "o3", "o4", "gpt-5"]) {
                AgentReasoningSupport::Supported
            } else if model.starts_with("gpt-4o") {
                AgentReasoningSupport::Unsupported
            } else {
                AgentReasoningSupport::Unknown
            }
        }
        AiProviderKind::Gemini => {
            if model.contains("gemini-3") || model.contains("gemini-2.5") {
                AgentReasoningSupport::Supported
            } else if model.contains("gemini-1.5") {
                AgentReasoningSupport::Unsupported
            } else {
                AgentReasoningSupport::Unknown
            }
        }
        AiProviderKind::Anthropic => {
            if anthropic_thinking_mode(&model).is_some() {
                AgentReasoningSupport::Supported
            } else if contains_any(
                &model,
                &[
                    "claude-3-5",
                    "claude-3.5",
                    "claude-3-opus",
                    "claude-3-haiku",
                    "claude-4-5-sonnet",
                    "claude-sonnet-4-5",
                    "claude-2",
                ],
            ) {
                AgentReasoningSupport::Unsupported
            } else {
                AgentReasoningSupport::Unknown
            }
        }
        AiProviderKind::Xai => {
            if model.contains("grok-3-mini") {
                AgentReasoningSupport::Supported
            } else if model.contains("grok-2") {
                AgentReasoningSupport::Unsupported
            } else {
                AgentReasoningSupport::Unknown
            }
        }
        AiProviderKind::DeepSeek => {
            if model.contains("deepseek-v4-pro") {
                AgentReasoningSupport::Supported
            } else if contains_any(&model, &["deepseek-r1", "deepseek-reasoner"]) {
                AgentReasoningSupport::Unsupported
            } else {
                AgentReasoningSupport::Unknown
            }
        }
        AiProviderKind::OpenRouter => {
            if contains_any(
                &model,
                &[
                    "/o1",
                    "/o3",
                    "/o4",
                    "/gpt-5",
                    "deepseek-r1",
                    "deepseek-reasoner",
                    "qwen3",
                    "qwq",
                    ":thinking",
                    "claude-sonnet-4-6",
                    "claude-opus-4-",
                ],
            ) {
                AgentReasoningSupport::Supported
            } else if model.contains("gpt-4o") {
                AgentReasoningSupport::Unsupported
            } else {
                AgentReasoningSupport::Unknown
            }
        }
        AiProviderKind::Mistral => {
            if model.contains("mistral-large-3") {
                AgentReasoningSupport::Supported
            } else {
                AgentReasoningSupport::Unknown
            }
        }
        AiProviderKind::HuggingFace => {
            if model == "meta-llama/meta-llama-3.1-8b-instruct" {
                AgentReasoningSupport::Unsupported
            } else {
                AgentReasoningSupport::Unknown
            }
        }
        AiProviderKind::Cohere | AiProviderKind::Together => AgentReasoningSupport::Unknown,
        AiProviderKind::ChatGpt | AiProviderKind::Copilot | AiProviderKind::Custom => {
            AgentReasoningSupport::Unsupported
        }
    }
}

pub(crate) fn plan_reasoning_request(profile: &ModelProfile) -> AgentResult<ReasoningRequestPlan> {
    let Some(effort) = profile.reasoning_effort.api_value() else {
        return Ok(ReasoningRequestPlan::default());
    };
    let unsupported = || AgentError::UnsupportedReasoningEffort {
        provider: profile.provider_name.clone(),
        model: profile.model.clone(),
    };

    if agent_reasoning_support(profile.kind, &profile.model) == AgentReasoningSupport::Unsupported {
        return Err(unsupported());
    }

    let model = profile.model.to_ascii_lowercase();
    let additional_params = match profile.kind {
        // Gemini is reached through its OpenAI-compatible endpoint, which
        // accepts the standard `reasoning_effort` parameter.
        AiProviderKind::OpenAi | AiProviderKind::Gemini => json!({
            "reasoning_effort": effort
        }),
        AiProviderKind::OpenRouter => json!({
            "reasoning": { "effort": effort }
        }),
        AiProviderKind::Xai => json!({
            "reasoning_effort": match profile.reasoning_effort {
                AiReasoningEffort::Low => "low",
                _ => "high",
            }
        }),
        AiProviderKind::Anthropic => {
            let mode = anthropic_thinking_mode(&model).unwrap_or(AnthropicThinkingMode::Adaptive);
            let params = match mode {
                AnthropicThinkingMode::Adaptive => json!({
                    "thinking": { "type": "adaptive" },
                    "output_config": { "effort": effort }
                }),
                AnthropicThinkingMode::Budget => json!({
                    "thinking": {
                        "type": "enabled",
                        "budget_tokens": anthropic_budget(profile)?
                    }
                }),
            };
            return Ok(ReasoningRequestPlan {
                additional_params: Some(params),
                suppress_temperature: true,
            });
        }
        AiProviderKind::DeepSeek if model.contains("deepseek-v4-pro") => json!({
            "thinking": { "type": "enabled" },
            "reasoning_effort": "high"
        }),
        AiProviderKind::Mistral if model.contains("mistral-large-3") => {
            json!({ "reasoning_effort": "high" })
        }
        AiProviderKind::Cohere
        | AiProviderKind::DeepSeek
        | AiProviderKind::HuggingFace
        | AiProviderKind::Mistral
        | AiProviderKind::Together => json!({
            "reasoning_effort": effort
        }),
        AiProviderKind::ChatGpt | AiProviderKind::Copilot | AiProviderKind::Custom => {
            return Err(unsupported());
        }
    };

    Ok(ReasoningRequestPlan {
        additional_params: Some(additional_params),
        suppress_temperature: false,
    })
}

fn anthropic_budget(profile: &ModelProfile) -> AgentResult<u64> {
    let requested = match profile.reasoning_effort {
        AiReasoningEffort::Low => 4096,
        AiReasoningEffort::Medium => 8192,
        AiReasoningEffort::High | AiReasoningEffort::Default => 16384,
    };
    let Some(max_tokens) = profile.max_tokens else {
        return Ok(requested);
    };
    if max_tokens <= 1024 {
        return Err(AgentError::InvalidArguments(format!(
            "Anthropic thinking requires max tokens greater than 1024; configured value is {max_tokens}"
        )));
    }
    Ok(requested.min(max_tokens - 1))
}

fn anthropic_thinking_mode(model: &str) -> Option<AnthropicThinkingMode> {
    if contains_any(
        model,
        &[
            "claude-sonnet-4-6",
            "claude-opus-4-6",
            "claude-opus-4-7",
            "claude-opus-4-8",
            "claude-sonnet-5",
            "claude-fable-5",
            "claude-mythos-5",
        ],
    ) {
        return Some(AnthropicThinkingMode::Adaptive);
    }
    if contains_any(
        model,
        &[
            "claude-3-7",
            "claude-4-0",
            "claude-4-1",
            "claude-4-2",
            "claude-4-3",
            "claude-4-4",
            "claude-sonnet-4-0",
            "claude-sonnet-4-1",
            "claude-sonnet-4-2",
            "claude-sonnet-4-3",
            "claude-sonnet-4-4",
            "claude-opus-4-0",
            "claude-opus-4-1",
            "claude-opus-4-2",
            "claude-opus-4-3",
            "claude-opus-4-4",
            "claude-haiku-4-5",
        ],
    ) {
        return Some(AnthropicThinkingMode::Budget);
    }
    None
}

fn starts_with_any(value: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| value.starts_with(pattern))
}

fn contains_any(value: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| value.contains(pattern))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::WireProtocol;

    pub(crate) fn profile(
        kind: AiProviderKind,
        model: &str,
        effort: AiReasoningEffort,
    ) -> ModelProfile {
        ModelProfile {
            provider_id: "provider".into(),
            provider_name: "Provider".into(),
            kind,
            protocol: if kind == AiProviderKind::Anthropic {
                WireProtocol::Anthropic
            } else {
                WireProtocol::OpenAiChat
            },
            model: model.into(),
            base_url: String::new(),
            api_key: "secret".into(),
            temperature: Some(0.7),
            max_tokens: None,
            reasoning_effort: effort,
            context_window: None,
        }
    }

    #[test]
    fn classifies_known_and_unknown_models() {
        use AgentReasoningSupport::*;
        for (kind, model, expected) in [
            (AiProviderKind::OpenAi, "gpt-5", Supported),
            (AiProviderKind::OpenAi, "gpt-4o", Unsupported),
            (AiProviderKind::OpenAi, "future-model", Unknown),
            (AiProviderKind::Gemini, "gemini-2.5-pro", Supported),
            (
                AiProviderKind::Anthropic,
                "claude-3-5-sonnet-latest",
                Unsupported,
            ),
            (AiProviderKind::OpenRouter, "openai/gpt-5", Supported),
            (AiProviderKind::Xai, "grok-2-latest", Unsupported),
            (AiProviderKind::DeepSeek, "deepseek-reasoner", Unsupported),
            (AiProviderKind::Mistral, "mistral-large-3", Supported),
            (AiProviderKind::HuggingFace, "future-reasoner", Unknown),
            (AiProviderKind::Cohere, "command-r-plus", Unknown),
            (AiProviderKind::Together, "future-reasoner", Unknown),
            (AiProviderKind::ChatGpt, "gpt-5", Unsupported),
            (AiProviderKind::Copilot, "gpt-5", Unsupported),
            (AiProviderKind::Custom, "future-reasoner", Unsupported),
        ] {
            assert_eq!(
                agent_reasoning_support(kind, model),
                expected,
                "{kind:?} {model}"
            );
        }
    }

    #[test]
    fn default_effort_never_generates_parameters() {
        for kind in AiProviderKind::all() {
            let profile = profile(*kind, "known-or-unknown-model", AiReasoningEffort::Default);
            assert_eq!(
                plan_reasoning_request(&profile).unwrap(),
                ReasoningRequestPlan::default(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn maps_openai_openrouter_and_xai_shapes() {
        let openai = profile(AiProviderKind::OpenAi, "gpt-5", AiReasoningEffort::High);
        assert_eq!(
            plan_reasoning_request(&openai).unwrap().additional_params,
            Some(json!({ "reasoning_effort": "high" }))
        );

        let openrouter = profile(
            AiProviderKind::OpenRouter,
            "openai/gpt-5",
            AiReasoningEffort::Medium,
        );
        assert_eq!(
            plan_reasoning_request(&openrouter)
                .unwrap()
                .additional_params,
            Some(json!({ "reasoning": { "effort": "medium" } }))
        );

        let xai = profile(
            AiProviderKind::Xai,
            "grok-3-mini",
            AiReasoningEffort::Medium,
        );
        assert_eq!(
            plan_reasoning_request(&xai).unwrap().additional_params,
            Some(json!({ "reasoning_effort": "high" }))
        );
    }

    #[test]
    fn maps_gemini_to_openai_compatible_reasoning_effort() {
        for (model, effort, expected) in [
            ("gemini-3-pro", AiReasoningEffort::Medium, "medium"),
            ("gemini-2.5-pro", AiReasoningEffort::High, "high"),
            ("gemini-2.5-flash", AiReasoningEffort::Low, "low"),
        ] {
            let gemini = profile(AiProviderKind::Gemini, model, effort);
            let plan = plan_reasoning_request(&gemini).unwrap();
            assert!(!plan.suppress_temperature);
            assert_eq!(
                plan.additional_params,
                Some(json!({ "reasoning_effort": expected })),
                "{model}"
            );
        }
        let old = profile(
            AiProviderKind::Gemini,
            "gemini-1.5-pro",
            AiReasoningEffort::High,
        );
        assert!(matches!(
            plan_reasoning_request(&old),
            Err(AgentError::UnsupportedReasoningEffort { .. })
        ));
    }

    #[test]
    fn maps_anthropic_modes_and_suppresses_temperature() {
        let adaptive = profile(
            AiProviderKind::Anthropic,
            "claude-sonnet-4-6",
            AiReasoningEffort::High,
        );
        let adaptive_plan = plan_reasoning_request(&adaptive).unwrap();
        assert!(adaptive_plan.suppress_temperature);
        assert_eq!(
            adaptive_plan.additional_params,
            Some(json!({
                "thinking": { "type": "adaptive" },
                "output_config": { "effort": "high" }
            }))
        );

        let mut budget = profile(
            AiProviderKind::Anthropic,
            "claude-3-7-sonnet-latest",
            AiReasoningEffort::High,
        );
        budget.max_tokens = Some(8192);
        let budget_plan = plan_reasoning_request(&budget).unwrap();
        assert!(budget_plan.suppress_temperature);
        assert_eq!(
            budget_plan.additional_params,
            Some(json!({
                "thinking": { "type": "enabled", "budget_tokens": 8191 }
            }))
        );
    }

    #[test]
    fn rejects_invalid_anthropic_budget_and_known_unsupported_model() {
        let mut anthropic = profile(
            AiProviderKind::Anthropic,
            "claude-3-7-sonnet-latest",
            AiReasoningEffort::Low,
        );
        anthropic.max_tokens = Some(1024);
        assert!(matches!(
            plan_reasoning_request(&anthropic),
            Err(AgentError::InvalidArguments(_))
        ));

        let unsupported = profile(AiProviderKind::OpenAi, "gpt-4o", AiReasoningEffort::High);
        assert!(matches!(
            plan_reasoning_request(&unsupported),
            Err(AgentError::UnsupportedReasoningEffort { .. })
        ));

        let custom = profile(AiProviderKind::Custom, "anything", AiReasoningEffort::Low);
        assert!(matches!(
            plan_reasoning_request(&custom),
            Err(AgentError::UnsupportedReasoningEffort { .. })
        ));
    }

    #[test]
    fn unknown_models_are_forwarded_best_effort() {
        for kind in [
            AiProviderKind::Cohere,
            AiProviderKind::HuggingFace,
            AiProviderKind::Together,
        ] {
            let unknown = profile(kind, "future-reasoner", AiReasoningEffort::Medium);
            assert_eq!(
                plan_reasoning_request(&unknown).unwrap().additional_params,
                Some(json!({ "reasoning_effort": "medium" })),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn maps_deepseek_and_mistral_supported_shapes() {
        let deepseek = profile(
            AiProviderKind::DeepSeek,
            "deepseek-v4-pro",
            AiReasoningEffort::Low,
        );
        assert_eq!(
            plan_reasoning_request(&deepseek).unwrap().additional_params,
            Some(json!({
                "thinking": { "type": "enabled" },
                "reasoning_effort": "high"
            }))
        );

        let mistral = profile(
            AiProviderKind::Mistral,
            "mistral-large-3",
            AiReasoningEffort::Low,
        );
        assert_eq!(
            plan_reasoning_request(&mistral).unwrap().additional_params,
            Some(json!({ "reasoning_effort": "high" }))
        );
    }
}
