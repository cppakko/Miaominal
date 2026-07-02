use super::*;

pub(in crate::ui::shell) fn ai_provider_kind_label_key(kind: AiProviderKind) -> &'static str {
    match kind {
        AiProviderKind::Anthropic => "settings.ai_providers.kinds.anthropic",
        AiProviderKind::ChatGpt => "settings.ai_providers.kinds.chat_gpt",
        AiProviderKind::Cohere => "settings.ai_providers.kinds.cohere",
        AiProviderKind::Copilot => "settings.ai_providers.kinds.copilot",
        AiProviderKind::DeepSeek => "settings.ai_providers.kinds.deepseek",
        AiProviderKind::Gemini => "settings.ai_providers.kinds.gemini",
        AiProviderKind::HuggingFace => "settings.ai_providers.kinds.hugging_face",
        AiProviderKind::Mistral => "settings.ai_providers.kinds.mistral",
        AiProviderKind::OpenAi => "settings.ai_providers.kinds.open_ai",
        AiProviderKind::OpenRouter => "settings.ai_providers.kinds.open_router",
        AiProviderKind::Together => "settings.ai_providers.kinds.together",
        AiProviderKind::Xai => "settings.ai_providers.kinds.xai",
        AiProviderKind::Custom => "settings.ai_providers.kinds.custom",
    }
}

pub(in crate::ui::shell) const fn ai_provider_kind_chat_supported(kind: AiProviderKind) -> bool {
    matches!(
        kind,
        AiProviderKind::Anthropic
            | AiProviderKind::Cohere
            | AiProviderKind::DeepSeek
            | AiProviderKind::Gemini
            | AiProviderKind::HuggingFace
            | AiProviderKind::Mistral
            | AiProviderKind::OpenAi
            | AiProviderKind::OpenRouter
            | AiProviderKind::Together
            | AiProviderKind::Xai
    )
}

pub(in crate::ui::shell) fn ai_provider_select_options(
    settings: &miaominal_settings::AppSettings,
) -> Vec<SelectOption<String>> {
    settings
        .ai_providers
        .iter()
        .filter(|provider| provider.enabled && ai_provider_kind_chat_supported(provider.kind))
        .map(|provider| SelectOption::new(provider.id.clone(), provider.name.clone()))
        .collect()
}

pub(in crate::ui::shell) fn web_search_provider_kind_label_key(
    kind: WebSearchProviderKind,
) -> &'static str {
    match kind {
        WebSearchProviderKind::Tavily => "settings.web_search.kinds.tavily",
        WebSearchProviderKind::Exa => "settings.web_search.kinds.exa",
        WebSearchProviderKind::Bocha => "settings.web_search.kinds.bocha",
        WebSearchProviderKind::Zhipu => "settings.web_search.kinds.zhipu",
        WebSearchProviderKind::SearXng => "settings.web_search.kinds.sear_xng",
    }
}

pub(in crate::ui::shell) fn web_search_endpoint_placeholder(kind: WebSearchProviderKind) -> String {
    let key = match kind {
        WebSearchProviderKind::Tavily => "settings.web_search.placeholders.endpoint_tavily",
        WebSearchProviderKind::Exa => "settings.web_search.placeholders.endpoint_exa",
        WebSearchProviderKind::Bocha => "settings.web_search.placeholders.endpoint_bocha",
        WebSearchProviderKind::Zhipu => "settings.web_search.placeholders.endpoint_zhipu",
        WebSearchProviderKind::SearXng => "settings.web_search.placeholders.endpoint_sear_xng",
    };
    i18n::string(key)
}
