use super::super::SelectOption;
use crate::ui::i18n;
use miaominal_settings::{self, AiProviderKind, WebSearchProviderKind};

#[path = "settings/labels.rs"]
mod labels;
pub(in crate::ui::shell) use labels::{
    ai_provider_kind_chat_supported, ai_provider_kind_label_key, ai_provider_select_options,
    web_search_endpoint_placeholder, web_search_provider_kind_label_key,
};
