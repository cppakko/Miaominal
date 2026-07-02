use super::*;

impl AppView {
    pub(in crate::ui::shell) fn begin_recording_key_binding(
        &mut self,
        slot: KeyBindingSlot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.panel_forms.settings.recording_binding = Some(slot);
        self.panel_forms.settings.pending_preview = None;
        self.panel_forms.settings.pending_binding = None;
        self.panel_forms
            .settings
            .key_capture_focus
            .focus(window, cx);
        cx.notify();
    }
    pub(in crate::ui::shell) fn commit_recorded_key_binding(
        &mut self,
        binding: KeyBinding,
        cx: &mut Context<Self>,
    ) {
        self.panel_forms.settings.pending_preview = None;
        self.panel_forms.settings.pending_binding = None;
        let Some(slot) = self.panel_forms.settings.recording_binding.take() else {
            return;
        };
        let changed = self.settings_store.update(|s| match slot {
            KeyBindingSlot::NextTab => s.key_bindings.next_tab = binding.clone(),
            KeyBindingSlot::CloseTab => s.key_bindings.close_tab = binding.clone(),
            KeyBindingSlot::ReopenTab => s.key_bindings.reopen_tab = binding.clone(),
            KeyBindingSlot::OpenSettings => s.key_bindings.open_settings = binding.clone(),
            KeyBindingSlot::Copy => s.key_bindings.copy = binding.clone(),
            KeyBindingSlot::Paste => s.key_bindings.paste = binding.clone(),
            KeyBindingSlot::Search => s.key_bindings.search = binding.clone(),
            KeyBindingSlot::SplitRight => s.key_bindings.split_right = binding.clone(),
            KeyBindingSlot::SplitDown => s.key_bindings.split_down = binding.clone(),
            KeyBindingSlot::ClosePane => s.key_bindings.close_pane = binding.clone(),
        });
        if changed {
            let name = slot.label();
            let binding = binding.display();
            self.status_message = i18n::string_args(
                "status.key_binding_updated",
                &[("name", &name), ("binding", &binding)],
            );
        }
        cx.notify();
    }
    pub(in crate::ui::shell) fn cancel_recording_key_binding(&mut self, cx: &mut Context<Self>) {
        self.panel_forms.settings.pending_preview = None;
        self.panel_forms.settings.pending_binding = None;
        if self.panel_forms.settings.recording_binding.take().is_some() {
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn accept_pending_key_binding(&mut self, cx: &mut Context<Self>) {
        let Some(binding) = self.panel_forms.settings.pending_binding.take() else {
            return;
        };
        self.commit_recorded_key_binding(binding, cx);
    }
    pub(in crate::ui::shell) fn update_key_preview(
        &mut self,
        preview: String,
        binding: Option<KeyBinding>,
        cx: &mut Context<Self>,
    ) {
        self.panel_forms.settings.pending_preview = Some(preview);
        self.panel_forms.settings.pending_binding = binding;
        cx.notify();
    }
    pub(in crate::ui::shell) fn reset_key_binding(
        &mut self,
        slot: KeyBindingSlot,
        cx: &mut Context<Self>,
    ) {
        use miaominal_settings::TerminalKeyBindings;
        let defaults = TerminalKeyBindings::default();
        let default_binding = match slot {
            KeyBindingSlot::NextTab => defaults.next_tab,
            KeyBindingSlot::CloseTab => defaults.close_tab,
            KeyBindingSlot::ReopenTab => defaults.reopen_tab,
            KeyBindingSlot::OpenSettings => defaults.open_settings,
            KeyBindingSlot::Copy => defaults.copy,
            KeyBindingSlot::Paste => defaults.paste,
            KeyBindingSlot::Search => defaults.search,
            KeyBindingSlot::SplitRight => defaults.split_right,
            KeyBindingSlot::SplitDown => defaults.split_down,
            KeyBindingSlot::ClosePane => defaults.close_pane,
        };
        let changed = self.settings_store.update(|s| match slot {
            KeyBindingSlot::NextTab => s.key_bindings.next_tab = default_binding.clone(),
            KeyBindingSlot::CloseTab => s.key_bindings.close_tab = default_binding.clone(),
            KeyBindingSlot::ReopenTab => s.key_bindings.reopen_tab = default_binding.clone(),
            KeyBindingSlot::OpenSettings => s.key_bindings.open_settings = default_binding.clone(),
            KeyBindingSlot::Copy => s.key_bindings.copy = default_binding.clone(),
            KeyBindingSlot::Paste => s.key_bindings.paste = default_binding.clone(),
            KeyBindingSlot::Search => s.key_bindings.search = default_binding.clone(),
            KeyBindingSlot::SplitRight => s.key_bindings.split_right = default_binding.clone(),
            KeyBindingSlot::SplitDown => s.key_bindings.split_down = default_binding.clone(),
            KeyBindingSlot::ClosePane => s.key_bindings.close_pane = default_binding.clone(),
        });
        if changed {
            let name = slot.label();
            let binding = default_binding.display();
            self.status_message = i18n::string_args(
                "status.key_binding_reset",
                &[("name", &name), ("binding", &binding)],
            );
        }
        cx.notify();
    }
}
