use super::*;

impl AppView {
    pub(in crate::ui::shell) fn update_font_family(
        &mut self,
        value: String,
        cx: &mut Context<Self>,
    ) {
        let trimmed = value.trim();
        let next = if trimmed.is_empty() {
            miaominal_settings::default_font_family()
        } else {
            trimmed.to_string()
        };

        let changed = self.settings_store.update(|s| s.font_family = next.clone());
        if changed {
            miaominal_settings::sync_component_theme(cx);
            self.status_message = i18n::string_args("status.font_set", &[("font", &next)]);
            self.invalidate_terminal_metrics();
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn reset_font_family(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let default_font = miaominal_settings::default_font_family();
        let changed = self
            .settings_store
            .update(|s| s.font_family = default_font.clone());
        self.panel_forms
            .settings
            .font_family_select
            .update(cx, |select, cx| {
                select.set_selected_value(&default_font, window, cx);
            });
        if changed {
            miaominal_settings::sync_component_theme(cx);
            self.status_message =
                i18n::string_args("status.font_reset", &[("font", &default_font)]);
            self.invalidate_terminal_metrics();
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn update_font_fallbacks(
        &mut self,
        value: String,
        cx: &mut Context<Self>,
    ) {
        let fallbacks: Vec<String> = value
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let changed = self.settings_store.update(|s| s.font_fallbacks = fallbacks);
        if changed {
            self.invalidate_terminal_metrics();
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn reset_font_fallbacks(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let defaults = miaominal_settings::default_font_fallbacks();
        let value = defaults.join(", ");
        let changed = self.settings_store.update(|s| s.font_fallbacks = defaults);
        set_input_value(
            &self.panel_forms.settings.font_fallbacks_input,
            value,
            window,
            cx,
        );
        if changed {
            self.invalidate_terminal_metrics();
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn adjust_font_size(&mut self, delta: f32, cx: &mut Context<Self>) {
        if let Some(target) = SettingsService::adjust_font_size(&mut self.settings_store, delta) {
            miaominal_settings::sync_component_theme(cx);
            let value = format!("{target:.1}");
            self.status_message = i18n::string_args("status.font_size", &[("value", &value)]);
            self.invalidate_terminal_metrics();
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn adjust_line_height(&mut self, delta: f32, cx: &mut Context<Self>) {
        if let Some(target) = SettingsService::adjust_line_height(&mut self.settings_store, delta) {
            miaominal_settings::sync_component_theme(cx);
            let value = format!("{target:.1}");
            self.status_message = i18n::string_args("status.line_height", &[("value", &value)]);
            self.invalidate_terminal_metrics();
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn update_seed_color(
        &mut self,
        value: String,
        cx: &mut Context<Self>,
    ) {
        let Some(normalized) = crate::ui::theme::normalize_seed_color(&value) else {
            self.notify_validation_failure(
                ValidationNotificationKind::InvalidInput,
                i18n::string("status.invalid_seed_color"),
                cx,
            );
            return;
        };

        let changed = self
            .settings_store
            .update(|settings| settings.seed_color = normalized.clone());
        if changed {
            miaominal_settings::sync_component_theme(cx);
            self.status_message = i18n::string_args("status.theme_seed", &[("value", &normalized)]);
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn reset_seed_color(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let default_seed = crate::ui::theme::DEFAULT_SEED_COLOR.to_string();
        let changed = self
            .settings_store
            .update(|settings| settings.seed_color = default_seed.clone());
        let default_color =
            miaominal_settings::Theme::from_settings(self.settings_store.settings())
                .material
                .source;
        self.panel_forms
            .settings
            .seed_color_picker
            .update(cx, |picker, cx| {
                picker.set_value(rgb(default_color), window, cx);
            });
        if changed {
            miaominal_settings::sync_component_theme(cx);
            self.status_message =
                i18n::string_args("status.theme_seed_reset", &[("value", &default_seed)]);
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn set_theme(&mut self, theme_id: ThemeId, cx: &mut Context<Self>) {
        let changed = self.settings_store.update(|s| s.theme_id = theme_id);
        if changed {
            miaominal_settings::sync_component_theme(cx);
            let theme = theme_id_label(theme_id);
            self.status_message = i18n::string_args("status.theme_changed", &[("theme", &theme)]);
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn set_language(
        &mut self,
        language: AppLanguage,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .settings_store
            .update(|settings| settings.language = language);
        if changed {
            crate::ui::i18n::set_language(language);
            if let Some(window_handle) = cx.active_window()
                && let Err(error) = window_handle.update(cx, |_, window, cx| {
                    self.refresh_localized_placeholders(window, cx);
                })
            {
                log::debug!(
                    "failed to refresh localized placeholders after language change: {error:?}"
                );
            }
            self.status_message = crate::ui::i18n::string_args(
                "status.language_changed",
                &[("language", language.native_name())],
            );
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn adjust_recent_connections_count(
        &mut self,
        delta: i16,
        cx: &mut Context<Self>,
    ) {
        let current = self.settings_store.settings().recent_connections_count as i16;
        let next = (current + delta).clamp(
            miaominal_settings::RECENT_CONNECTIONS_COUNT_MIN as i16,
            miaominal_settings::RECENT_CONNECTIONS_COUNT_MAX as i16,
        ) as u8;
        let changed = self
            .settings_store
            .update(|s| s.recent_connections_count = next);
        if changed {
            self.status_message = if next == 0 {
                i18n::string("status.recent_connections_hidden")
            } else {
                let count = next.to_string();
                i18n::string_args("status.recent_connections_show_count", &[("count", &count)])
            };
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn set_auto_collect_session_monitoring(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .settings_store
            .update(|settings| settings.auto_collect_session_monitoring = enabled);
        if changed {
            let profile_ids: Vec<_> = self
                .workspace_state
                .tabs
                .iter()
                .filter_map(|tab| {
                    tab.as_session().and_then(|session| {
                        (session.purpose == SessionPurpose::Terminal)
                            .then_some(session.profile_id.clone())
                    })
                })
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();

            for profile_id in profile_ids {
                if let Err(error) = self.set_profile_monitoring_enabled(&profile_id, enabled) {
                    log::debug!("failed to toggle session monitoring: {error}");
                }
            }
            self.status_message = if enabled {
                i18n::string("status.auto_collect_session_monitoring_enabled")
            } else {
                i18n::string("status.auto_collect_session_monitoring_disabled")
            };
            cx.notify();
        }
    }
    pub(super) fn invalidate_terminal_metrics(&mut self) {
        // Force the terminal canvas prepaint path to recompute on the next frame
        // by resetting cached metrics; the next paint will reseed them from the
        // latest font settings and resize the active PTY accordingly.
        self.workspace_state.workspace.active_pane.terminal_bounds = None;
        self.workspace_state
            .workspace
            .active_pane
            .terminal_cell_width = terminal_cell_width_default();
        self.workspace_state
            .workspace
            .active_pane
            .terminal_line_height = terminal_line_height_default();

        for parked in self.workspace_state.workspace.parked_panes.values_mut() {
            parked.terminal_bounds = None;
            parked.terminal_cell_width = terminal_cell_width_default();
            parked.terminal_line_height = terminal_line_height_default();
        }
    }
}
