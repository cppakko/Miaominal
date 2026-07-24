use crate::chat::{AgentChatProvider, AgentChatProviderKind};
use crate::error::{AgentError, AgentResult};
use miaominal_settings::AiReasoningEffort;
use rig_core::providers::gemini::completion::gemini_api_types::{
    AdditionalParameters, GenerationConfig, ThinkingConfig, ThinkingLevel,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentReasoningSupport {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct ReasoningRequestPlan {
    pub additional_params: Option<serde_json::Value>,
    pub suppress_temperature: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnthropicThinkingMode {
    Adaptive,
    Budget,
}

pub fn agent_reasoning_support(kind: AgentChatProviderKind, model: &str) -> AgentReasoningSupport {
    let model = model.trim().to_ascii_lowercase();
    match kind {
        AgentChatProviderKind::OpenAi => {
            if starts_with_any(&model, &["o1", "o3", "o4", "gpt-5"]) {
                AgentReasoningSupport::Supported
            } else if model.starts_with("gpt-4o") {
                AgentReasoningSupport::Unsupported
            } else {
                AgentReasoningSupport::Unknown
            }
        }
        AgentChatProviderKind::Gemini => {
            if model.contains("gemini-3") || model.contains("gemini-2.5") {
                AgentReasoningSupport::Supported
            } else if model.contains("gemini-1.5") {
                AgentReasoningSupport::Unsupported
            } else {
                AgentReasoningSupport::Unknown
            }
        }
        AgentChatProviderKind::Anthropic => {
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
        AgentChatProviderKind::Xai => {
            if model.contains("grok-3-mini") {
                AgentReasoningSupport::Supported
            } else if model.contains("grok-2") {
                AgentReasoningSupport::Unsupported
            } else {
                AgentReasoningSupport::Unknown
            }
        }
        AgentChatProviderKind::DeepSeek => {
            if model.contains("deepseek-v4-pro") {
                AgentReasoningSupport::Supported
            } else if contains_any(&model, &["deepseek-r1", "deepseek-reasoner"]) {
                AgentReasoningSupport::Unsupported
            } else {
                AgentReasoningSupport::Unknown
            }
        }
        AgentChatProviderKind::OpenRouter => {
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
        AgentChatProviderKind::Mistral => {
            if model.contains("mistral-large-3") {
                AgentReasoningSupport::Supported
            } else {
                AgentReasoningSupport::Unknown
            }
        }
        AgentChatProviderKind::HuggingFace => {
            if model == "meta-llama/meta-llama-3.1-8b-instruct" {
                AgentReasoningSupport::Unsupported
            } else {
                AgentReasoningSupport::Unknown
            }
        }
        AgentChatProviderKind::Cohere | AgentChatProviderKind::Together => {
            AgentReasoningSupport::Unknown
        }
        AgentChatProviderKind::ChatGpt
        | AgentChatProviderKind::Copilot
        | AgentChatProviderKind::Custom => AgentReasoningSupport::Unsupported,
    }
}

pub(crate) fn plan_reasoning_request(
    provider: &AgentChatProvider,
) -> AgentResult<ReasoningRequestPlan> {
    let Some(effort) = provider.reasoning_effort.api_value() else {
        return Ok(ReasoningRequestPlan::default());
    };

    if agent_reasoning_support(provider.kind, &provider.model) == AgentReasoningSupport::Unsupported
    {
        return Err(AgentError::UnsupportedReasoningEffort {
            provider: provider.name.clone(),
            model: provider.model.clone(),
        });
    }

    let additional_params = match provider.kind {
        AgentChatProviderKind::OpenAi => serde_json::json!({
            "reasoning_effort": effort
        }),
        AgentChatProviderKind::OpenRouter => serde_json::json!({
            "reasoning": { "effort": effort }
        }),
        AgentChatProviderKind::Xai => serde_json::json!({
            "reasoning_effort": match provider.reasoning_effort {
                AiReasoningEffort::Low => "low",
                AiReasoningEffort::Medium | AiReasoningEffort::High => "high",
                AiReasoningEffort::Default => unreachable!(),
            }
        }),
        AgentChatProviderKind::Gemini => gemini_params(&provider.model, provider.reasoning_effort)?,
        AgentChatProviderKind::Anthropic => {
            let mode = anthropic_thinking_mode(&provider.model.to_ascii_lowercase())
                .unwrap_or(AnthropicThinkingMode::Adaptive);
            let params = match mode {
                AnthropicThinkingMode::Adaptive => serde_json::json!({
                    "thinking": { "type": "adaptive" },
                    "output_config": { "effort": effort }
                }),
                AnthropicThinkingMode::Budget => serde_json::json!({
                    "thinking": {
                        "type": "enabled",
                        "budget_tokens": anthropic_budget(provider)?
                    }
                }),
            };
            return Ok(ReasoningRequestPlan {
                additional_params: Some(params),
                suppress_temperature: true,
            });
        }
        AgentChatProviderKind::DeepSeek
            if provider
                .model
                .to_ascii_lowercase()
                .contains("deepseek-v4-pro") =>
        {
            serde_json::json!({
                "thinking": { "type": "enabled" },
                "reasoning_effort": "high"
            })
        }
        AgentChatProviderKind::Mistral
            if provider
                .model
                .to_ascii_lowercase()
                .contains("mistral-large-3") =>
        {
            serde_json::json!({ "reasoning_effort": "high" })
        }
        AgentChatProviderKind::Cohere
        | AgentChatProviderKind::DeepSeek
        | AgentChatProviderKind::HuggingFace
        | AgentChatProviderKind::Mistral
        | AgentChatProviderKind::Together => serde_json::json!({
            "reasoning_effort": effort
        }),
        AgentChatProviderKind::ChatGpt
        | AgentChatProviderKind::Copilot
        | AgentChatProviderKind::Custom => {
            return Err(AgentError::UnsupportedReasoningEffort {
                provider: provider.name.clone(),
                model: provider.model.clone(),
            });
        }
    };

    Ok(ReasoningRequestPlan {
        additional_params: Some(additional_params),
        suppress_temperature: false,
    })
}

fn gemini_params(model: &str, effort: AiReasoningEffort) -> AgentResult<serde_json::Value> {
    let model = model.to_ascii_lowercase();
    let thinking_config = if model.contains("gemini-2.5") || model.contains("thinking") {
        ThinkingConfig {
            thinking_budget: Some(match effort {
                AiReasoningEffort::Low => 1024,
                AiReasoningEffort::Medium => 2048,
                AiReasoningEffort::High => 4096,
                AiReasoningEffort::Default => unreachable!(),
            }),
            thinking_level: None,
            include_thoughts: Some(true),
        }
    } else {
        ThinkingConfig {
            thinking_budget: None,
            thinking_level: Some(match effort {
                AiReasoningEffort::Low => ThinkingLevel::Low,
                AiReasoningEffort::Medium => ThinkingLevel::Medium,
                AiReasoningEffort::High => ThinkingLevel::High,
                AiReasoningEffort::Default => unreachable!(),
            }),
            include_thoughts: Some(true),
        }
    };
    let mut generation_config = GenerationConfig::default();
    // Rig's Gemini default also sets temperature=1 and maxOutputTokens=4096.
    // Keep those controlled by AgentBuilder/provider defaults and serialize only thinking here.
    generation_config.temperature = None;
    generation_config.max_output_tokens = None;
    generation_config.thinking_config = Some(thinking_config);
    serde_json::to_value(AdditionalParameters::default().with_config(generation_config))
        .map_err(|error| AgentError::Backend(error.into()))
}

fn anthropic_budget(provider: &AgentChatProvider) -> AgentResult<u64> {
    let requested = match provider.reasoning_effort {
        AiReasoningEffort::Low => 4096,
        AiReasoningEffort::Medium => 8192,
        AiReasoningEffort::High => 16384,
        AiReasoningEffort::Default => unreachable!(),
    };
    let Some(max_tokens) = provider.max_tokens else {
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
mod tests {
    use super::*;

    fn provider(
        kind: AgentChatProviderKind,
        model: &str,
        effort: AiReasoningEffort,
    ) -> AgentChatProvider {
        AgentChatProvider {
            id: "provider".into(),
            name: "Provider".into(),
            kind,
            model: model.into(),
            base_url: String::new(),
            api_key: "secret".into(),
            temperature: Some(0.7),
            max_tokens: None,
            reasoning_effort: effort,
        }
    }

    #[test]
    fn classifies_known_and_unknown_models() {
        for (kind, model, expected) in [
            (
                AgentChatProviderKind::OpenAi,
                "gpt-5",
                AgentReasoningSupport::Supported,
            ),
            (
                AgentChatProviderKind::OpenAi,
                "gpt-4o",
                AgentReasoningSupport::Unsupported,
            ),
            (
                AgentChatProviderKind::OpenAi,
                "future-model",
                AgentReasoningSupport::Unknown,
            ),
            (
                AgentChatProviderKind::Gemini,
                "gemini-2.5-pro",
                AgentReasoningSupport::Supported,
            ),
            (
                AgentChatProviderKind::Anthropic,
                "claude-3-5-sonnet-latest",
                AgentReasoningSupport::Unsupported,
            ),
            (
                AgentChatProviderKind::OpenRouter,
                "openai/gpt-5",
                AgentReasoningSupport::Supported,
            ),
            (
                AgentChatProviderKind::Xai,
                "grok-2-latest",
                AgentReasoningSupport::Unsupported,
            ),
            (
                AgentChatProviderKind::DeepSeek,
                "deepseek-reasoner",
                AgentReasoningSupport::Unsupported,
            ),
            (
                AgentChatProviderKind::Mistral,
                "mistral-large-3",
                AgentReasoningSupport::Supported,
            ),
            (
                AgentChatProviderKind::HuggingFace,
                "future-reasoner",
                AgentReasoningSupport::Unknown,
            ),
            (
                AgentChatProviderKind::Cohere,
                "command-r-plus",
                AgentReasoningSupport::Unknown,
            ),
            (
                AgentChatProviderKind::Together,
                "future-reasoner",
                AgentReasoningSupport::Unknown,
            ),
            (
                AgentChatProviderKind::ChatGpt,
                "gpt-5",
                AgentReasoningSupport::Unsupported,
            ),
            (
                AgentChatProviderKind::Copilot,
                "gpt-5",
                AgentReasoningSupport::Unsupported,
            ),
            (
                AgentChatProviderKind::Custom,
                "future-reasoner",
                AgentReasoningSupport::Unsupported,
            ),
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
        for kind in [
            AgentChatProviderKind::OpenAi,
            AgentChatProviderKind::Anthropic,
            AgentChatProviderKind::Cohere,
            AgentChatProviderKind::DeepSeek,
            AgentChatProviderKind::Gemini,
            AgentChatProviderKind::HuggingFace,
            AgentChatProviderKind::Mistral,
            AgentChatProviderKind::OpenRouter,
            AgentChatProviderKind::Together,
            AgentChatProviderKind::Xai,
            AgentChatProviderKind::ChatGpt,
            AgentChatProviderKind::Copilot,
            AgentChatProviderKind::Custom,
        ] {
            let provider = provider(kind, "known-or-unknown-model", AiReasoningEffort::Default);
            assert_eq!(
                plan_reasoning_request(&provider).unwrap(),
                ReasoningRequestPlan::default(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn maps_openai_openrouter_and_xai_shapes() {
        let openai = provider(
            AgentChatProviderKind::OpenAi,
            "gpt-5",
            AiReasoningEffort::High,
        );
        assert_eq!(
            plan_reasoning_request(&openai).unwrap().additional_params,
            Some(serde_json::json!({ "reasoning_effort": "high" }))
        );

        let openrouter = provider(
            AgentChatProviderKind::OpenRouter,
            "openai/gpt-5",
            AiReasoningEffort::Medium,
        );
        assert_eq!(
            plan_reasoning_request(&openrouter)
                .unwrap()
                .additional_params,
            Some(serde_json::json!({ "reasoning": { "effort": "medium" } }))
        );

        let xai = provider(
            AgentChatProviderKind::Xai,
            "grok-3-mini",
            AiReasoningEffort::Medium,
        );
        assert_eq!(
            plan_reasoning_request(&xai).unwrap().additional_params,
            Some(serde_json::json!({ "reasoning_effort": "high" }))
        );
    }

    #[test]
    fn maps_gemini_models_with_typed_config() {
        let gemini3 = provider(
            AgentChatProviderKind::Gemini,
            "gemini-3-pro",
            AiReasoningEffort::Medium,
        );
        assert_eq!(
            plan_reasoning_request(&gemini3).unwrap().additional_params,
            Some(serde_json::json!({
                "generationConfig": {
                    "thinkingConfig": {
                        "thinkingLevel": "medium",
                        "includeThoughts": true
                    }
                }
            }))
        );

        let gemini25 = provider(
            AgentChatProviderKind::Gemini,
            "gemini-2.5-pro",
            AiReasoningEffort::High,
        );
        assert_eq!(
            plan_reasoning_request(&gemini25).unwrap().additional_params,
            Some(serde_json::json!({
                "generationConfig": {
                    "thinkingConfig": {
                        "thinkingBudget": 4096,
                        "includeThoughts": true
                    }
                }
            }))
        );
    }

    #[test]
    fn maps_anthropic_modes_and_suppresses_temperature() {
        let adaptive = provider(
            AgentChatProviderKind::Anthropic,
            "claude-sonnet-4-6",
            AiReasoningEffort::High,
        );
        let adaptive_plan = plan_reasoning_request(&adaptive).unwrap();
        assert!(adaptive_plan.suppress_temperature);
        assert_eq!(
            adaptive_plan.additional_params,
            Some(serde_json::json!({
                "thinking": { "type": "adaptive" },
                "output_config": { "effort": "high" }
            }))
        );

        let mut budget = provider(
            AgentChatProviderKind::Anthropic,
            "claude-3-7-sonnet-latest",
            AiReasoningEffort::High,
        );
        budget.max_tokens = Some(8192);
        let budget_plan = plan_reasoning_request(&budget).unwrap();
        assert!(budget_plan.suppress_temperature);
        assert_eq!(
            budget_plan.additional_params,
            Some(serde_json::json!({
                "thinking": { "type": "enabled", "budget_tokens": 8191 }
            }))
        );
    }

    #[test]
    fn rejects_invalid_anthropic_budget_and_known_unsupported_model() {
        let mut anthropic = provider(
            AgentChatProviderKind::Anthropic,
            "claude-3-7-sonnet-latest",
            AiReasoningEffort::Low,
        );
        anthropic.max_tokens = Some(1024);
        assert!(matches!(
            plan_reasoning_request(&anthropic),
            Err(AgentError::InvalidArguments(_))
        ));

        let unsupported = provider(
            AgentChatProviderKind::OpenAi,
            "gpt-4o",
            AiReasoningEffort::High,
        );
        assert!(matches!(
            plan_reasoning_request(&unsupported),
            Err(AgentError::UnsupportedReasoningEffort { .. })
        ));
    }

    #[test]
    fn unknown_models_are_forwarded_best_effort() {
        for kind in [
            AgentChatProviderKind::Cohere,
            AgentChatProviderKind::HuggingFace,
            AgentChatProviderKind::Together,
        ] {
            let unknown = provider(kind, "future-reasoner", AiReasoningEffort::Medium);
            assert_eq!(
                plan_reasoning_request(&unknown).unwrap().additional_params,
                Some(serde_json::json!({ "reasoning_effort": "medium" })),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn maps_deepseek_and_mistral_supported_shapes() {
        let deepseek = provider(
            AgentChatProviderKind::DeepSeek,
            "deepseek-v4-pro",
            AiReasoningEffort::Low,
        );
        assert_eq!(
            plan_reasoning_request(&deepseek).unwrap().additional_params,
            Some(serde_json::json!({
                "thinking": { "type": "enabled" },
                "reasoning_effort": "high"
            }))
        );

        let mistral = provider(
            AgentChatProviderKind::Mistral,
            "mistral-large-3",
            AiReasoningEffort::Low,
        );
        assert_eq!(
            plan_reasoning_request(&mistral).unwrap().additional_params,
            Some(serde_json::json!({ "reasoning_effort": "high" }))
        );
    }
}
