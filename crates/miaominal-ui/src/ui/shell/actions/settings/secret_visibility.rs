use super::*;

impl AppView {
    pub(in crate::ui::shell) fn secret_reveal_icon(&self, target: SecretRevealTarget) -> AppIcon {
        if self.secret_visibility.is_visible(&target) {
            AppIcon::EyeOff
        } else {
            AppIcon::Eye
        }
    }
    pub(super) fn secret_input(&self, target: SecretRevealTarget) -> Entity<InputState> {
        match target {
            SecretRevealTarget::SyncGithubToken => {
                self.panel_forms.settings.sync_github_token_input.clone()
            }
            SecretRevealTarget::SyncWebdavPassword => {
                self.panel_forms.settings.sync_webdav_password_input.clone()
            }
            SecretRevealTarget::HostPassword => self.host_editor_forms.password_input.clone(),
            SecretRevealTarget::SyncPassphrase => {
                self.panel_forms.settings.sync_passphrase_input.clone()
            }
            SecretRevealTarget::SyncPassphraseConfirmation => self
                .panel_forms
                .settings
                .sync_passphrase_confirmation_input
                .clone(),
            SecretRevealTarget::LocalVaultPassphrase => self
                .panel_forms
                .settings
                .local_vault_passphrase_input
                .clone(),
            SecretRevealTarget::LocalVaultPassphraseConfirmation => self
                .panel_forms
                .settings
                .local_vault_passphrase_confirmation_input
                .clone(),
            SecretRevealTarget::AiProviderApiKey(_) => {
                self.panel_forms.settings.ai_provider_api_key_input.clone()
            }
            SecretRevealTarget::WebSearchApiKey => {
                self.panel_forms.settings.web_search_api_key_input.clone()
            }
        }
    }
    pub(in crate::ui::shell) fn set_secret_visibility(
        &mut self,
        target: SecretRevealTarget,
        visible: bool,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.secret_visibility.set_visible(target.clone(), visible);
        let input = self.secret_input(target);
        set_input_masked(&input, !visible, focus, window, cx);
        cx.notify();
    }
    pub(super) fn secret_target_has_stored_value(&self, target: SecretRevealTarget) -> bool {
        match target {
            SecretRevealTarget::SyncGithubToken => {
                self.sync.sync_engine.config_store.config.has_github_token
            }
            SecretRevealTarget::SyncWebdavPassword => {
                self.sync
                    .sync_engine
                    .config_store
                    .config
                    .has_webdav_password
            }
            SecretRevealTarget::HostPassword => self
                .data
                .selected_profile
                .and_then(|index| self.data.sessions.get(index))
                .is_some_and(|profile| profile.has_stored_password),
            SecretRevealTarget::SyncPassphrase
            | SecretRevealTarget::SyncPassphraseConfirmation
            | SecretRevealTarget::LocalVaultPassphrase
            | SecretRevealTarget::LocalVaultPassphraseConfirmation => false,
            SecretRevealTarget::AiProviderApiKey(provider_id) => self
                .settings_store
                .settings()
                .ai_providers
                .iter()
                .any(|provider| provider.id == provider_id && provider.has_api_key),
            SecretRevealTarget::WebSearchApiKey => {
                self.settings_store.settings().web_search.has_api_key
            }
        }
    }
    pub(super) fn hide_storage_backed_secret_visibility(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for target in [
            SecretRevealTarget::SyncGithubToken,
            SecretRevealTarget::SyncWebdavPassword,
            SecretRevealTarget::HostPassword,
        ] {
            self.set_secret_visibility(target, false, false, window, cx);
        }
    }
    pub(super) fn prepare_host_password_for_lock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.data.selected_profile else {
            return;
        };
        let Some(profile) = self.data.sessions.get(index) else {
            return;
        };

        if !profile.has_stored_password {
            return;
        }

        let current_password = self
            .host_editor_forms
            .password_input
            .read(cx)
            .value()
            .to_string();

        if current_password.is_empty() {
            self.set_secret_visibility(SecretRevealTarget::HostPassword, false, false, window, cx);
            return;
        }

        match self.services.secrets.get(&profile.id, SecretKind::Password) {
            Ok(Some(stored_password)) if stored_password == current_password => {
                set_input_value(&self.host_editor_forms.password_input, "", window, cx);
            }
            Ok(_) => {}
            Err(error) => {
                log::warn!(
                    "failed to compare stored host password before locking local vault: {error:?}"
                );
            }
        }

        self.set_secret_visibility(SecretRevealTarget::HostPassword, false, false, window, cx);
    }
    pub(super) fn load_secret_input_for_reveal(
        &mut self,
        target: SecretRevealTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        match target {
            SecretRevealTarget::SyncGithubToken | SecretRevealTarget::SyncWebdavPassword => {
                self.refresh_sync_secret_inputs(window, cx);
                Ok(())
            }
            SecretRevealTarget::HostPassword => {
                self.load_selected_profile_password_input(window, cx)
            }
            SecretRevealTarget::AiProviderApiKey(provider_id) => {
                let api_key = self
                    .services
                    .secrets
                    .get(&provider_id, SecretKind::AiProviderApiKey)?
                    .unwrap_or_default();
                set_input_value(
                    &self.panel_forms.settings.ai_provider_api_key_input,
                    api_key,
                    window,
                    cx,
                );
                Ok(())
            }
            SecretRevealTarget::WebSearchApiKey => {
                let api_key = self
                    .services
                    .secrets
                    .get("web_search", SecretKind::WebSearchApiKey)?
                    .unwrap_or_default();
                set_input_value(
                    &self.panel_forms.settings.web_search_api_key_input,
                    api_key,
                    window,
                    cx,
                );
                Ok(())
            }
            SecretRevealTarget::SyncPassphrase
            | SecretRevealTarget::SyncPassphraseConfirmation
            | SecretRevealTarget::LocalVaultPassphrase
            | SecretRevealTarget::LocalVaultPassphraseConfirmation => Ok(()),
        }
    }
    pub(super) fn notify_secret_reveal_failed(
        &mut self,
        window: &mut Window,
        error: &anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        let message = error.to_string();
        self.status_message = message.clone();
        window.push_notification(
            Self::error_notification(
                i18n::string("settings.sync.vault.notifications.failed_title"),
                message,
            ),
            cx,
        );
        cx.notify();
    }
    pub(super) fn reveal_secret_input(
        &mut self,
        target: SecretRevealTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.load_secret_input_for_reveal(target.clone(), window, cx) {
            self.notify_secret_reveal_failed(window, &error, cx);
            return;
        }

        self.set_secret_visibility(target, true, true, window, cx);
    }
    pub(super) fn continue_reveal_secret_after_unlock(
        &mut self,
        target: SecretRevealTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reveal_secret_input(target, window, cx);
    }
    pub(in crate::ui::shell) fn toggle_secret_visibility(
        &mut self,
        target: SecretRevealTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.secret_visibility.is_visible(&target) {
            self.set_secret_visibility(target.clone(), false, true, window, cx);
            return;
        }

        let input = self.secret_input(target.clone());
        let has_text = !input.read(cx).value().is_empty();

        if has_text
            || !target.uses_stored_secret()
            || !self.secret_target_has_stored_value(target.clone())
        {
            self.set_secret_visibility(target, true, true, window, cx);
            return;
        }

        if self.local_vault_status == LocalVaultStatus::Locked {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::RevealSecret(target),
                window,
                cx,
            );
            return;
        }

        self.reveal_secret_input(target, window, cx);
    }
}
