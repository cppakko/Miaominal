//! Provider presets and model capability table.
use super::{ModelCapabilities, ModelProfile, ReasoningReplay, WireProtocol};
use miaominal_settings::AiProviderKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderPreset {
    pub protocol: WireProtocol,
    pub default_base_url: &'static str,
    /// The provider is usable by the agent (false for OAuth-only providers).
    pub supported: bool,
}

const DEFAULT_CONTEXT_WINDOW: u64 = 128_000;

pub fn provider_preset(kind: AiProviderKind) -> ProviderPreset {
    let openai = |default_base_url| ProviderPreset {
        protocol: WireProtocol::OpenAiChat,
        default_base_url,
        supported: true,
    };
    match kind {
        AiProviderKind::OpenAi => openai("https://api.openai.com/v1"),
        AiProviderKind::Anthropic => ProviderPreset {
            protocol: WireProtocol::Anthropic,
            default_base_url: "https://api.anthropic.com",
            supported: true,
        },
        AiProviderKind::DeepSeek => openai("https://api.deepseek.com"),
        AiProviderKind::Gemini => openai("https://generativelanguage.googleapis.com/v1beta/openai"),
        AiProviderKind::OpenRouter => openai("https://openrouter.ai/api/v1"),
        AiProviderKind::Mistral => openai("https://api.mistral.ai/v1"),
        AiProviderKind::Cohere => openai("https://api.cohere.ai/compatibility/v1"),
        AiProviderKind::Together => openai("https://api.together.xyz/v1"),
        AiProviderKind::Xai => openai("https://api.x.ai/v1"),
        AiProviderKind::HuggingFace => openai("https://router.huggingface.co/v1"),
        AiProviderKind::Custom => openai(""),
        AiProviderKind::ChatGpt | AiProviderKind::Copilot => ProviderPreset {
            protocol: WireProtocol::OpenAiChat,
            default_base_url: "",
            supported: false,
        },
    }
}

pub fn model_capabilities(profile: &ModelProfile) -> ModelCapabilities {
    let model = profile.model.trim().to_ascii_lowercase();
    let reasoning_replay = if matches!(
        profile.kind,
        AiProviderKind::Anthropic | AiProviderKind::DeepSeek
    ) || profile.protocol == WireProtocol::Anthropic
    {
        ReasoningReplay::CurrentTurn
    } else {
        ReasoningReplay::Never
    };
    ModelCapabilities {
        vision: supports_vision(profile.kind, &model),
        parallel_tool_calls: true,
        reasoning_replay,
        context_window: profile
            .context_window
            .filter(|window| *window > 0)
            .unwrap_or_else(|| default_context_window(&model)),
    }
}

fn supports_vision(kind: AiProviderKind, model: &str) -> bool {
    match kind {
        AiProviderKind::OpenAi => {
            if ["o1-mini", "o1-preview", "o3-mini"]
                .iter()
                .any(|text_only| model.starts_with(text_only))
            {
                return false;
            }
            ["gpt-4o", "gpt-4.1", "gpt-5"]
                .iter()
                .any(|prefix| model.starts_with(prefix))
                || is_o_series(model)
        }
        AiProviderKind::Anthropic => {
            model.contains("claude") && !model.contains("claude-2") && !model.contains("instant")
        }
        AiProviderKind::Gemini | AiProviderKind::OpenRouter => true,
        AiProviderKind::Xai => model.contains("vision") || model.contains("grok-4"),
        _ => false,
    }
}

/// `o1`, `o3`, `o4-mini`, ... (letter `o` followed by a digit).
fn is_o_series(model: &str) -> bool {
    let mut chars = model.chars();
    chars.next() == Some('o') && chars.next().is_some_and(|c| c.is_ascii_digit())
}

fn default_context_window(model: &str) -> u64 {
    let model_name = model.rsplit('/').next().unwrap_or(model);
    if model.contains("claude") {
        200_000
    } else if model.contains("gemini") || model.contains("gpt-4.1") {
        1_000_000
    } else if model.contains("gpt-5") {
        400_000
    } else if model.contains("grok-4") {
        256_000
    } else if model.contains("deepseek") || model.contains("gpt-4o") || is_o_series(model_name) {
        128_000
    } else {
        DEFAULT_CONTEXT_WINDOW
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::reasoning::tests::profile;
    use miaominal_settings::AiReasoningEffort;

    fn caps(kind: AiProviderKind, model: &str) -> ModelCapabilities {
        model_capabilities(&profile(kind, model, AiReasoningEffort::Default))
    }

    #[test]
    fn presets_cover_every_provider() {
        for kind in AiProviderKind::all() {
            let preset = provider_preset(*kind);
            let expect_supported =
                !matches!(kind, AiProviderKind::ChatGpt | AiProviderKind::Copilot);
            assert_eq!(preset.supported, expect_supported, "{kind:?}");
            if expect_supported && *kind != AiProviderKind::Custom {
                assert!(preset.default_base_url.starts_with("https://"), "{kind:?}");
                assert!(!preset.default_base_url.ends_with('/'), "{kind:?}");
            }
        }
        assert_eq!(
            provider_preset(AiProviderKind::Anthropic).protocol,
            WireProtocol::Anthropic
        );
        assert_eq!(
            provider_preset(AiProviderKind::Gemini),
            ProviderPreset {
                protocol: WireProtocol::OpenAiChat,
                default_base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
                supported: true,
            }
        );
        assert_eq!(provider_preset(AiProviderKind::Custom).default_base_url, "");
    }

    #[test]
    fn vision_table() {
        for (kind, model, expected) in [
            (AiProviderKind::OpenAi, "gpt-4o-mini", true),
            (AiProviderKind::OpenAi, "gpt-4.1", true),
            (AiProviderKind::OpenAi, "gpt-5", true),
            (AiProviderKind::OpenAi, "o3", true),
            (AiProviderKind::OpenAi, "o4-mini", true),
            (AiProviderKind::OpenAi, "o3-mini", false),
            (AiProviderKind::OpenAi, "gpt-3.5-turbo", false),
            (AiProviderKind::Anthropic, "claude-3-5-sonnet-latest", true),
            (AiProviderKind::Anthropic, "claude-sonnet-4-6", true),
            (AiProviderKind::Anthropic, "claude-2.1", false),
            (AiProviderKind::Gemini, "gemini-2.5-pro", true),
            (AiProviderKind::Xai, "grok-2-vision-1212", true),
            (AiProviderKind::Xai, "grok-4", true),
            (AiProviderKind::Xai, "grok-3-mini", false),
            (AiProviderKind::OpenRouter, "anything/model", true),
            (AiProviderKind::DeepSeek, "deepseek-chat", false),
            (AiProviderKind::Custom, "llava", false),
        ] {
            assert_eq!(caps(kind, model).vision, expected, "{kind:?} {model}");
        }
    }

    #[test]
    fn reasoning_replay_and_parallel_tools() {
        assert_eq!(
            caps(AiProviderKind::Anthropic, "claude-sonnet-4-6").reasoning_replay,
            ReasoningReplay::CurrentTurn
        );
        assert_eq!(
            caps(AiProviderKind::DeepSeek, "deepseek-v4-pro").reasoning_replay,
            ReasoningReplay::CurrentTurn
        );
        assert_eq!(
            caps(AiProviderKind::OpenAi, "gpt-5").reasoning_replay,
            ReasoningReplay::Never
        );
        assert!(caps(AiProviderKind::Mistral, "x").parallel_tool_calls);
    }

    #[test]
    fn context_window_table_and_override() {
        for (kind, model, expected) in [
            (AiProviderKind::Anthropic, "claude-opus-4-1", 200_000),
            (
                AiProviderKind::OpenRouter,
                "anthropic/claude-sonnet-4",
                200_000,
            ),
            (AiProviderKind::Gemini, "gemini-2.5-pro", 1_000_000),
            (AiProviderKind::OpenAi, "gpt-4.1-mini", 1_000_000),
            (AiProviderKind::OpenAi, "gpt-5", 400_000),
            (AiProviderKind::OpenAi, "gpt-4o", 128_000),
            (AiProviderKind::OpenAi, "o3", 128_000),
            (AiProviderKind::DeepSeek, "deepseek-chat", 128_000),
            (AiProviderKind::Xai, "grok-4", 256_000),
            (AiProviderKind::Custom, "llama3", 128_000),
        ] {
            assert_eq!(caps(kind, model).context_window, expected, "{model}");
        }
        let mut overridden = profile(AiProviderKind::OpenAi, "gpt-5", AiReasoningEffort::Default);
        overridden.context_window = Some(32_000);
        assert_eq!(model_capabilities(&overridden).context_window, 32_000);
    }
}
