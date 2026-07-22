use super::*;
use crate::ui::shell::support::set_input_masked;
use crate::ui::shell::{AppIcon, DeferredAppCommand, SettingsDeferredCommand, error_notification};
use gpui_component::WindowExt as _;
use miaominal_secrets::SecretKind;

impl SettingsController {
    pub(in crate::ui::shell) fn secret_reveal_icon(&self, target: &SecretRevealTarget) -> AppIcon {
        if self.secret_visibility.is_visible(target) {
            AppIcon::EyeOff
        } else {
            AppIcon::Eye
        }
    }

    pub(in crate::ui::shell) fn set_secret_visibility(
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

    pub(in crate::ui::shell) fn continue_reveal_secret_after_unlock(
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
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                SettingsDeferredCommand::RevealSecret(target),
            )));
            cx.notify();
            return true;
        }

        self.reveal_secret_input(target, window, cx);
        true
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
            SecretRevealTarget::HostPassword => None,
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
            | SecretRevealTarget::LocalVaultPassphraseConfirmation
            | SecretRevealTarget::HostPassword => false,
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
            SecretRevealTarget::HostPassword => return Ok(()),
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
            window.push_notification(
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
