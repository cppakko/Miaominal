use super::*;
use crate::ui::contracts::preferences::{AgentPreferencesPort, AgentPreferencesSnapshot};

impl SettingsController {
    pub(in crate::ui) fn agent_preferences_port(entity: &Entity<Self>) -> AgentPreferencesPort {
        let reader = entity.downgrade();
        let provider_writer = entity.downgrade();
        let reasoning_writer = entity.downgrade();
        AgentPreferencesPort::new(
            move |cx| {
                let Some(entity) = reader.upgrade() else {
                    return AgentPreferencesSnapshot::default();
                };
                let settings = entity.read(cx).settings();
                AgentPreferencesSnapshot::new(
                    settings.ai_providers.clone(),
                    settings.selected_ai_provider_id.clone(),
                )
            },
            move |provider_id, window, cx| {
                let _ = provider_writer.update(cx, |settings, cx| {
                    settings.select_ai_provider(provider_id, window, cx);
                });
            },
            move |provider_id, effort, cx| {
                let _ = reasoning_writer.update(cx, |settings, cx| {
                    settings.set_ai_provider_reasoning_effort(provider_id, effort, cx);
                });
            },
        )
    }
}
