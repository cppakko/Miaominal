//! Commands and immutable provider preferences shared by Settings and Agent.
//!
//! Settings owns preference persistence. Shell supplies a weak Settings adapter
//! when it wires Agent; consumers retain no Settings controller or form handles.

use gpui_kit::{App, Window};
use miaominal_settings::{AiProviderConfig, AiProviderKind, AiReasoningEffort};
use std::rc::Rc;

#[derive(Clone, Default)]
pub(in crate::ui) struct AgentPreferencesSnapshot {
    providers: Vec<AiProviderConfig>,
    selected_provider_id: Option<String>,
}

impl AgentPreferencesSnapshot {
    pub(in crate::ui) fn new(
        providers: Vec<AiProviderConfig>,
        selected_provider_id: Option<String>,
    ) -> Self {
        Self {
            providers,
            selected_provider_id,
        }
    }

    pub(in crate::ui) fn providers(&self) -> &[AiProviderConfig] {
        &self.providers
    }
    pub(in crate::ui) fn selected_provider_id(&self) -> Option<&str> {
        self.selected_provider_id.as_deref()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum SftpBrowserSide {
    Local,
    Remote,
}

type ReadPreferences = dyn Fn(&App) -> AgentPreferencesSnapshot;
type SelectProvider = dyn Fn(String, &mut Window, &mut App);
type SetReasoningEffort = dyn Fn(&str, AiReasoningEffort, &mut App);

pub(in crate::ui) const fn ai_provider_kind_chat_supported(kind: AiProviderKind) -> bool {
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

#[derive(Clone)]
pub(in crate::ui) struct AgentPreferencesPort {
    read: Rc<ReadPreferences>,
    select_provider: Rc<SelectProvider>,
    set_reasoning_effort: Rc<SetReasoningEffort>,
}

impl AgentPreferencesPort {
    pub(in crate::ui) fn new(
        read: impl Fn(&App) -> AgentPreferencesSnapshot + 'static,
        select_provider: impl Fn(String, &mut Window, &mut App) + 'static,
        set_reasoning_effort: impl Fn(&str, AiReasoningEffort, &mut App) + 'static,
    ) -> Self {
        Self {
            read: Rc::new(read),
            select_provider: Rc::new(select_provider),
            set_reasoning_effort: Rc::new(set_reasoning_effort),
        }
    }

    pub(in crate::ui) fn snapshot(&self, cx: &App) -> AgentPreferencesSnapshot {
        (self.read)(cx)
    }

    pub(in crate::ui) fn select_provider(
        &self,
        provider_id: String,
        window: &mut Window,
        cx: &mut App,
    ) {
        (self.select_provider)(provider_id, window, cx);
    }

    pub(in crate::ui) fn set_reasoning_effort(
        &self,
        provider_id: &str,
        effort: AiReasoningEffort,
        cx: &mut App,
    ) {
        (self.set_reasoning_effort)(provider_id, effort, cx);
    }
}
