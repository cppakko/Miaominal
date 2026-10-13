use super::*;
use crate::ui::components::forms::set_input_masked;
use miaominal_secrets::SecretKind;

impl SettingsController {
    pub(in crate::ui) fn set_secret_visibility(
        &mut self,
        target: SecretRevealTarget,
        visible: bool,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(input) = self.secret_input(&target) else {
            return false;
        };
        self.secret_visibility.set_visible(target, visible);
        set_input_masked(&input, !visible, focus, window, cx);
        cx.notify();
        true
    }

    pub(in crate::ui) fn continue_reveal_secret_after_unlock(
        &mut self,
        request: SecretRevealRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlay_revisions.current(request.surface_key()) != request.editor_revision()
            || !self.secret_reveal_surface_is_active(request.surface_key())
        {
            return;
        }
        self.reveal_secret_input(request.into_target(), window, cx);
    }

    pub(in crate::ui) fn toggle_secret_visibility(
        &mut self,
        target: SecretRevealTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(input) = self.secret_input(&target) else {
            return false;
        };

        if self.secret_visibility.is_visible(&target) {
            return self.set_secret_visibility(target, false, true, window, cx);
        }

        let has_text = !input.read(cx).value().is_empty();
        if has_text || !target.uses_stored_secret() || !self.secret_target_has_stored_value(&target)
        {
            return self.set_secret_visibility(target, true, true, window, cx);
        }

        if self.local_vault_status == LocalVaultStatus::Locked {
            let key = match &target {
                SecretRevealTarget::AiProviderApiKey(..) => "ai-provider",
                SecretRevealTarget::WebSearchApiKey => "web-search-config",
                SecretRevealTarget::SyncGithubToken | SecretRevealTarget::SyncWebdavPassword => {
                    "sync-provider-config"
                }
                _ => return false,
            };
            let request =
                SecretRevealRequest::new(target, key, self.overlay_revisions.current(key));
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                SettingsDeferredCommand::RevealSecret(request),
            )));
            cx.notify();
            return true;
        }

        self.reveal_secret_input(target, window, cx);
        true
    }

    fn secret_reveal_surface_is_active(&self, key: &'static str) -> bool {
        match key {
            "ai-provider" => self.ai_provider_popup.is_some(),
            "web-search-config" => self.web_search_config_popup.is_some(),
            "sync-provider-config" => self.sync_provider_config_popup.is_some(),
            _ => false,
        }
    }

    fn secret_input(&self, target: &SecretRevealTarget) -> Option<Entity<InputState>> {
        match target {
            SecretRevealTarget::SyncGithubToken => Some(self.forms.sync_github_token_input.clone()),
            SecretRevealTarget::SyncWebdavPassword => {
                Some(self.forms.sync_webdav_password_input.clone())
            }
            SecretRevealTarget::SyncPassphrase => Some(self.forms.sync_passphrase_input.clone()),
            SecretRevealTarget::SyncPassphraseConfirmation => {
                Some(self.forms.sync_passphrase_confirmation_input.clone())
            }
            SecretRevealTarget::LocalVaultCurrentPassphrase => {
                Some(self.forms.local_vault_current_passphrase_input.clone())
            }
            SecretRevealTarget::LocalVaultPassphrase => {
                Some(self.forms.local_vault_passphrase_input.clone())
            }
            SecretRevealTarget::LocalVaultPassphraseConfirmation => {
                Some(self.forms.local_vault_passphrase_confirmation_input.clone())
            }
            SecretRevealTarget::AiProviderApiKey(_) => {
                Some(self.forms.ai_provider_api_key_input.clone())
            }
            SecretRevealTarget::WebSearchApiKey => {
                Some(self.forms.web_search_api_key_input.clone())
            }
        }
    }

    fn secret_target_has_stored_value(&self, target: &SecretRevealTarget) -> bool {
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
            SecretRevealTarget::AiProviderApiKey(provider_id) => self
                .settings_store
                .settings()
                .ai_providers
                .iter()
                .any(|provider| provider.id == *provider_id && provider.has_api_key),
            SecretRevealTarget::WebSearchApiKey => {
                self.settings_store.settings().web_search.has_api_key
            }
            SecretRevealTarget::SyncPassphrase
            | SecretRevealTarget::SyncPassphraseConfirmation
            | SecretRevealTarget::LocalVaultCurrentPassphrase
            | SecretRevealTarget::LocalVaultPassphrase
            | SecretRevealTarget::LocalVaultPassphraseConfirmation => false,
        }
    }

    fn load_secret_input_for_reveal(
        &mut self,
        target: &SecretRevealTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        match target {
            SecretRevealTarget::SyncGithubToken | SecretRevealTarget::SyncWebdavPassword => {
                self.refresh_sync_secret_inputs(window, cx);
            }
            SecretRevealTarget::AiProviderApiKey(provider_id) => {
                let api_key = self
                    .secrets
                    .get(provider_id, SecretKind::AiProviderApiKey)?
                    .unwrap_or_default();
                set_input_value(&self.forms.ai_provider_api_key_input, api_key, window, cx);
            }
            SecretRevealTarget::WebSearchApiKey => {
                let api_key = self
                    .secrets
                    .get("web_search", SecretKind::WebSearchApiKey)?
                    .unwrap_or_default();
                set_input_value(&self.forms.web_search_api_key_input, api_key, window, cx);
            }
            SecretRevealTarget::SyncPassphrase
            | SecretRevealTarget::SyncPassphraseConfirmation
            | SecretRevealTarget::LocalVaultCurrentPassphrase
            | SecretRevealTarget::LocalVaultPassphrase
            | SecretRevealTarget::LocalVaultPassphraseConfirmation => {}
        }
        Ok(())
    }

    fn reveal_secret_input(
        &mut self,
        target: SecretRevealTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.load_secret_input_for_reveal(&target, window, cx) {
            let message = error.to_string();
            crate::ui::components::notifications::push_app_notification(
                window,
                error_notification(
                    i18n::string("settings.sync.vault.notifications.failed_title"),
                    message.clone(),
                ),
                cx,
            );
            cx.emit(AppCommand::Feedback(message));
            cx.notify();
            return;
        }

        self.set_secret_visibility(target, true, true, window, cx);
    }
}
