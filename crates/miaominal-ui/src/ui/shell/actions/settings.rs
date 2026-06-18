use super::super::state::{
    PendingLocalDataResetConfirmState, PendingLocalDataResetConfirmationPopupState,
    PendingSyncPassphraseClearConfirmPopupState, PendingSyncPassphrasePopupState,
    SyncPassphraseOperation, SyncSecretSaveOperation,
};
use super::super::support::set_input_masked;
use super::super::*;
use crate::ui::i18n;
use gpui_component::WindowExt;
use miaominal_secrets::{SecretKind, SecretStore};
use miaominal_services::{
    LocalVaultMode, LocalVaultPassphraseChangeOutcome, LocalVaultTransition, SettingsService,
};
use miaominal_settings::{
    self, AiProviderConfig, AiProviderKind, AppLanguage, KeyBinding, LastTabCloseBehavior,
    LocalVaultAutoLockDuration, MonitorHistoryDuration, TerminalRightClickBehavior, ThemeId,
    WebSearchConfig, WebSearchProviderKind,
};
use miaominal_sync::engine::SyncEngine;
use miaominal_sync::{SyncConfig, SyncProvider, SyncStatus};

const LOCAL_DATA_RESET_CONFIRMATION_TOKEN: &str = "RESET";

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

fn normalize_github_gist_id(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }

    let without_query = trimmed
        .split_once('?')
        .map_or(trimmed, |(before, _)| before);
    let without_fragment = without_query
        .split_once('#')
        .map_or(without_query, |(before, _)| before);

    without_fragment
        .rsplit('/')
        .next()
        .unwrap_or(without_fragment)
        .trim()
        .to_string()
}

struct LocalVaultSyncSecretInputs {
    github_token: String,
    webdav_password: String,
    sync_passphrase: String,
}

struct LocalVaultUnlockResult {
    transition: LocalVaultTransition,
    sync_secret_inputs: LocalVaultSyncSecretInputs,
}

struct LocalVaultEnableResult {
    passphrase: String,
    vault_secrets: SecretStore,
    vault_sync_engine: SyncEngine,
    sync_secret_inputs: LocalVaultSyncSecretInputs,
    session_ids: Vec<String>,
    managed_key_ids: Vec<String>,
    ai_provider_ids: Vec<String>,
}

struct LocalVaultChangePassphraseResult {
    outcome: LocalVaultPassphraseChangeOutcome,
    sync_secret_inputs: LocalVaultSyncSecretInputs,
}

enum SyncSecretSaveTaskRequest {
    GithubToken(String),
    WebdavPassword(String),
}

struct SyncSecretSaveTaskResult {
    operation: SyncSecretSaveOperation,
    updated_config: SyncConfig,
}

impl SyncSecretSaveTaskRequest {
    fn operation(&self) -> SyncSecretSaveOperation {
        match self {
            Self::GithubToken(_) => SyncSecretSaveOperation::GithubToken,
            Self::WebdavPassword(_) => SyncSecretSaveOperation::WebdavPassword,
        }
    }

    fn worker_name(&self) -> &'static str {
        match self {
            Self::GithubToken(_) => "sync-github-token-save",
            Self::WebdavPassword(_) => "sync-webdav-password-save",
        }
    }

    fn cancelled_message(&self) -> &'static str {
        match self {
            Self::GithubToken(_) => "sync GitHub token save task cancelled",
            Self::WebdavPassword(_) => "sync WebDAV password save task cancelled",
        }
    }
}

enum SyncPassphraseTaskRequest {
    Save(String),
    Clear,
}

struct SyncPassphraseTaskResult {
    operation: SyncPassphraseOperation,
    configured: bool,
    updated_config: SyncConfig,
}

struct AiProviderSaveTaskResult {
    provider: AiProviderConfig,
}

struct AiProviderApiKeyLoadTaskResult {
    provider_id: String,
    api_key: Result<Option<String>>,
}

struct WebSearchSaveTaskResult {
    config: WebSearchConfig,
}

impl SyncPassphraseTaskRequest {
    fn operation(&self) -> SyncPassphraseOperation {
        match self {
            Self::Save(_) => SyncPassphraseOperation::Save,
            Self::Clear => SyncPassphraseOperation::Clear,
        }
    }

    fn worker_name(&self) -> &'static str {
        match self {
            Self::Save(_) => "sync-passphrase-save",
            Self::Clear => "sync-passphrase-clear",
        }
    }

    fn cancelled_message(&self) -> &'static str {
        match self {
            Self::Save(_) => "sync passphrase save task cancelled",
            Self::Clear => "sync passphrase clear task cancelled",
        }
    }
}

impl AppView {
    pub(in crate::ui::shell) fn apply_ai_provider_kind_defaults(
        &mut self,
        kind: AiProviderKind,
        cx: &mut Context<Self>,
    ) {
        if self.panel_forms.settings.editing_ai_provider_id.is_none() {
            self.status_message = i18n::string_args(
                "settings.ai_providers.status.kind_selected",
                &[("kind", &i18n::string(ai_provider_kind_label_key(kind)))],
            );
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn secret_reveal_icon(&self, target: SecretRevealTarget) -> AppIcon {
        if self.secret_visibility.is_visible(&target) {
            AppIcon::EyeOff
        } else {
            AppIcon::Eye
        }
    }

    fn secret_input(&self, target: SecretRevealTarget) -> Entity<InputState> {
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

    fn secret_target_has_stored_value(&self, target: SecretRevealTarget) -> bool {
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

    fn hide_storage_backed_secret_visibility(
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

    fn prepare_host_password_for_lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn load_secret_input_for_reveal(
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

    fn notify_secret_reveal_failed(
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

    fn reveal_secret_input(
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

    fn continue_reveal_secret_after_unlock(
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

    pub(in crate::ui::shell) fn open_local_data_reset_confirm(&mut self, cx: &mut Context<Self>) {
        if self.local_data_reset_in_progress {
            return;
        }

        self.dialogs.pending_local_data_reset_confirm = Some(PendingLocalDataResetConfirmState);
        cx.notify();
    }

    pub(in crate::ui::shell) fn cancel_local_data_reset_confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(prompt) = self.dialogs.pending_local_data_reset_confirm.take() {
            self.start_dialog_exit(DialogOverlaySnapshot::LocalDataResetConfirm(prompt), cx);
        }
    }

    pub(in crate::ui::shell) fn continue_local_data_reset_confirm(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.dialogs.pending_local_data_reset_confirm.take() else {
            return;
        };

        self.start_dialog_exit(DialogOverlaySnapshot::LocalDataResetConfirm(prompt), cx);
        self.open_local_data_reset_confirmation_popup(window, cx);
    }

    fn open_local_data_reset_confirmation_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let popup = PendingLocalDataResetConfirmationPopupState;
        let stable_key = DialogOverlaySnapshot::LocalDataResetConfirmationPopup(popup).stable_key();
        self.dialogs
            .exiting_dialogs
            .retain(|dialog| dialog.snapshot.stable_key() != stable_key);
        self.dialogs.pending_local_data_reset_confirmation_popup = Some(popup);
        self.clear_local_data_reset_confirmation_input(window, cx);
        self.focus_local_data_reset_confirmation_input(window, cx);
        cx.notify();
    }

    fn dismiss_local_data_reset_confirmation_popup(&mut self, cx: &mut Context<Self>) {
        if let Some(popup) = self
            .dialogs
            .pending_local_data_reset_confirmation_popup
            .take()
        {
            self.start_dialog_exit(
                DialogOverlaySnapshot::LocalDataResetConfirmationPopup(popup),
                cx,
            );
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn close_local_data_reset_confirmation_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.local_data_reset_in_progress {
            return;
        }

        self.clear_local_data_reset_confirmation_input(window, cx);
        self.dismiss_local_data_reset_confirmation_popup(cx);
    }

    pub(in crate::ui::shell) fn submit_local_data_reset_confirmation_popup_action(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.local_data_reset_in_progress {
            return;
        }

        let confirmation = self
            .panel_forms
            .settings
            .local_data_reset_confirmation_input
            .read(cx)
            .value()
            .trim()
            .to_string();

        if confirmation.is_empty() {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.about.reset_local.validation.required"),
                cx,
            );
            return;
        }

        if confirmation != LOCAL_DATA_RESET_CONFIRMATION_TOKEN {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::InvalidInput,
                i18n::string("settings.about.reset_local.validation.mismatch"),
                cx,
            );
            return;
        }

        self.dismiss_local_data_reset_confirmation_popup(cx);
        self.spawn_local_data_reset(window, cx);
    }

    fn clear_local_data_reset_confirmation_input(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        set_input_value(
            &self
                .panel_forms
                .settings
                .local_data_reset_confirmation_input,
            String::new(),
            window,
            cx,
        );
    }

    fn focus_local_data_reset_confirmation_input(&self, window: &mut Window, cx: &mut App) {
        self.panel_forms
            .settings
            .local_data_reset_confirmation_input
            .update(cx, |input, cx| {
                input.focus(window, cx);
            });
    }

    fn spawn_local_data_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.local_data_reset_in_progress {
            return;
        }

        self.local_data_reset_in_progress = true;
        let notification_window = cx.active_window();
        let (session_ids, managed_key_ids, ai_provider_ids) = self.local_vault_secret_ids();
        cx.notify();

        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name("local-data-reset".to_string())
            .spawn(move || {
                let result = SettingsService::reset_local_data(
                    &session_ids,
                    &managed_key_ids,
                    &ai_provider_ids,
                );
                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            self.local_data_reset_in_progress = false;

            let error = anyhow::anyhow!(error).context("failed to spawn local data reset worker");
            self.notify_local_data_reset_failed(window, error, cx);
            return;
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("local data reset task cancelled")))
                })
                .await;

            if let Some(window_handle) = notification_window {
                let result = std::rc::Rc::new(std::cell::RefCell::new(Some(result)));
                let result_for_window = result.clone();
                let this_for_window = this.clone();
                let update_result = window_handle.update(cx, move |_, window, cx| {
                    let Some(result) = result_for_window.borrow_mut().take() else {
                        return;
                    };

                    if let Err(error) = this_for_window.update(cx, move |this, cx| {
                        this.finish_local_data_reset(result, window, cx);
                    }) {
                        log::debug!("failed to apply local data reset result in window: {error:?}");
                    }
                });

                if let Err(error) = update_result {
                    log::debug!("failed to access active window for local data reset: {error:?}");
                    if let Some(result) = result.borrow_mut().take()
                        && let Err(error) = this.update(cx, move |this, cx| {
                            this.finish_local_data_reset_without_window(result, cx);
                        })
                    {
                        log::debug!(
                            "failed to apply local data reset result without window: {error:?}"
                        );
                    }
                }
            } else if let Err(error) = this.update(cx, move |this, cx| {
                this.finish_local_data_reset_without_window(result, cx);
            }) {
                log::debug!(
                    "failed to apply local data reset result without active window: {error:?}"
                );
            }
        })
        .detach();
    }

    fn finish_local_data_reset(
        &mut self,
        result: anyhow::Result<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.local_data_reset_in_progress = false;

        match result {
            Ok(()) => {
                self.rebuild_after_local_data_reset(window, cx);

                let message =
                    i18n::string("settings.about.reset_local.notifications.success.message");
                self.status_message = message.clone();
                window.push_notification(
                    Self::success_notification(
                        i18n::string("settings.about.reset_local.notifications.success.title"),
                        message,
                    ),
                    cx,
                );
                cx.notify();
            }
            Err(error) => {
                self.notify_local_data_reset_failed(window, error, cx);
            }
        }
    }

    fn finish_local_data_reset_without_window(
        &mut self,
        result: anyhow::Result<()>,
        cx: &mut Context<Self>,
    ) {
        self.local_data_reset_in_progress = false;

        match result {
            Ok(()) => {
                let error_message = i18n::string(
                    "settings.about.reset_local.notifications.window_unavailable.message",
                );
                self.status_message = error_message;
            }
            Err(error) => {
                let error_message = error.to_string();
                self.status_message = i18n::string_args(
                    "settings.about.reset_local.notifications.failed.message",
                    &[("error", &error_message)],
                );
            }
        }

        cx.notify();
    }

    fn rebuild_after_local_data_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let runtime = self.services.runtime.clone();
        *self = AppView::new(runtime, window, cx);
    }

    fn notify_local_data_reset_failed(
        &mut self,
        window: &mut Window,
        error: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        let error_message = error.to_string();
        let message = i18n::string_args(
            "settings.about.reset_local.notifications.failed.message",
            &[("error", &error_message)],
        );
        self.status_message = message.clone();
        window.push_notification(
            Self::error_notification(
                i18n::string("settings.about.reset_local.notifications.failed.title"),
                message,
            ),
            cx,
        );
        cx.notify();
    }

    pub(in crate::ui::shell) fn sync_passphrase_action_label(&self) -> String {
        i18n::string(if self.passphrase_is_set() {
            "settings.sync.encryption.passphrase.actions.update"
        } else {
            "settings.sync.encryption.passphrase.actions.set"
        })
    }

    pub(in crate::ui::shell) fn sync_passphrase_clear_action_label(&self) -> String {
        i18n::string("settings.sync.encryption.passphrase.actions.clear")
    }

    fn open_sync_passphrase_clear_confirm_popup(&mut self, cx: &mut Context<Self>) {
        let popup = PendingSyncPassphraseClearConfirmPopupState;
        let stable_key = DialogOverlaySnapshot::SyncPassphraseClearConfirmPopup(popup).stable_key();
        self.dialogs
            .exiting_dialogs
            .retain(|dialog| dialog.snapshot.stable_key() != stable_key);
        self.dialogs.pending_sync_passphrase_clear_confirm_popup = Some(popup);
        cx.notify();
    }

    fn dismiss_sync_passphrase_clear_confirm_popup(&mut self, cx: &mut Context<Self>) {
        if let Some(popup) = self
            .dialogs
            .pending_sync_passphrase_clear_confirm_popup
            .take()
        {
            self.start_dialog_exit(
                DialogOverlaySnapshot::SyncPassphraseClearConfirmPopup(popup),
                cx,
            );
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn close_sync_passphrase_clear_confirm_popup(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }

        self.dismiss_sync_passphrase_clear_confirm_popup(cx);
    }

    pub(in crate::ui::shell) fn submit_sync_passphrase_clear_confirm_popup_action(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }

        self.dismiss_sync_passphrase_clear_confirm_popup(cx);
        self.continue_clear_sync_passphrase_after_confirm(window, cx);
    }

    pub(in crate::ui::shell) fn open_sync_passphrase_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }

        let popup = PendingSyncPassphrasePopupState;
        let stable_key = DialogOverlaySnapshot::SyncPassphrasePopup(popup).stable_key();
        self.dialogs
            .exiting_dialogs
            .retain(|dialog| dialog.snapshot.stable_key() != stable_key);
        self.sync_passphrase_popup = Some(popup);
        self.clear_sync_passphrase_popup_inputs(window, cx);
        self.focus_sync_passphrase_input(window, cx);
        cx.notify();
    }

    fn dismiss_sync_passphrase_popup(&mut self, cx: &mut Context<Self>) {
        if let Some(popup) = self.sync_passphrase_popup.take() {
            self.start_dialog_exit(DialogOverlaySnapshot::SyncPassphrasePopup(popup), cx);
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn close_sync_passphrase_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }

        self.clear_sync_passphrase_popup_inputs(window, cx);
        self.dismiss_sync_passphrase_popup(cx);
    }

    pub(in crate::ui::shell) fn submit_sync_passphrase_popup_action(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }

        let passphrase = self
            .panel_forms
            .settings
            .sync_passphrase_input
            .read(cx)
            .value()
            .to_string();
        let passphrase_confirmation = self
            .panel_forms
            .settings
            .sync_passphrase_confirmation_input
            .read(cx)
            .value()
            .to_string();

        if passphrase.trim().is_empty() {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.sync.encryption.passphrase.required_error.message"),
                cx,
            );
            return;
        }

        if passphrase_confirmation.trim().is_empty() {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string(
                    "settings.sync.encryption.passphrase.confirmation_required_error.message",
                ),
                cx,
            );
            return;
        }

        if passphrase != passphrase_confirmation {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::InvalidInput,
                i18n::string("settings.sync.encryption.passphrase.mismatch_error.message"),
                cx,
            );
            return;
        }

        if self.local_vault_status == LocalVaultStatus::Locked {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::SaveSyncPassphrase(passphrase),
                window,
                cx,
            );
            return;
        }

        self.spawn_sync_passphrase_operation(SyncPassphraseTaskRequest::Save(passphrase), cx);
    }

    pub(in crate::ui::shell) fn clear_sync_passphrase(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }

        self.open_sync_passphrase_clear_confirm_popup(cx);
    }

    fn notify_sync_secret_saved(
        &mut self,
        window: &mut Window,
        field_label: &str,
        cx: &mut Context<Self>,
    ) {
        let message = i18n::string_args(
            "settings.sync.save_feedback.saved_message",
            &[("field", field_label)],
        );
        self.status_message = message.clone();
        window.push_notification(
            Self::success_notification(
                i18n::string("settings.sync.save_feedback.saved_title"),
                message,
            ),
            cx,
        );
        cx.notify();
    }

    fn notify_sync_secret_save_failed(
        &mut self,
        window: &mut Window,
        field_label: &str,
        error: &str,
        cx: &mut Context<Self>,
    ) {
        let message = i18n::string_args(
            "settings.sync.save_feedback.failed_message",
            &[("field", field_label), ("error", error)],
        );
        self.status_message = message.clone();
        window.push_notification(
            Self::error_notification(
                i18n::string("settings.sync.save_feedback.failed_title"),
                message,
            ),
            cx,
        );
        cx.notify();
    }

    fn clear_sync_passphrase_popup_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        set_input_value(
            &self.panel_forms.settings.sync_passphrase_input,
            String::new(),
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.sync_passphrase_confirmation_input,
            String::new(),
            window,
            cx,
        );
        set_input_masked(
            &self.panel_forms.settings.sync_passphrase_input,
            true,
            false,
            window,
            cx,
        );
        set_input_masked(
            &self.panel_forms.settings.sync_passphrase_confirmation_input,
            true,
            false,
            window,
            cx,
        );
        self.secret_visibility
            .set_visible(SecretRevealTarget::SyncPassphrase, false);
        self.secret_visibility
            .set_visible(SecretRevealTarget::SyncPassphraseConfirmation, false);
    }

    fn focus_sync_passphrase_input(&self, window: &mut Window, cx: &mut App) {
        self.panel_forms
            .settings
            .sync_passphrase_input
            .update(cx, |input, cx| {
                input.focus(window, cx);
            });
    }

    fn local_vault_operation_in_progress(&self) -> bool {
        self.local_vault_unlock_in_progress || self.local_vault_disable_in_progress
    }

    pub(in crate::ui::shell) fn sync_passphrase_operation_in_progress(&self) -> bool {
        self.sync.sync_passphrase_operation.is_some()
    }

    pub(in crate::ui::shell) fn sync_secret_save_in_progress(&self) -> bool {
        self.sync.sync_secret_save_operation.is_some()
    }

    pub(in crate::ui::shell) fn ai_provider_save_in_progress(&self) -> bool {
        self.ai_provider_save_in_progress
    }

    pub(in crate::ui::shell) fn web_search_save_in_progress(&self) -> bool {
        self.web_search_save_in_progress
    }

    pub(in crate::ui::shell) fn ai_provider_api_key_load_in_progress_for(
        &self,
        provider_id: &str,
    ) -> bool {
        self.ai_provider_api_key_load_in_progress.as_deref() == Some(provider_id)
    }

    pub(in crate::ui::shell) fn sync_github_token_save_in_progress(&self) -> bool {
        self.sync.sync_secret_save_operation == Some(SyncSecretSaveOperation::GithubToken)
    }

    pub(in crate::ui::shell) fn sync_webdav_password_save_in_progress(&self) -> bool {
        self.sync.sync_secret_save_operation == Some(SyncSecretSaveOperation::WebdavPassword)
    }

    pub(in crate::ui::shell) fn sync_passphrase_save_in_progress(&self) -> bool {
        self.sync.sync_passphrase_operation == Some(SyncPassphraseOperation::Save)
    }

    fn sync_secret_field_label(operation: SyncSecretSaveOperation) -> String {
        match operation {
            SyncSecretSaveOperation::GithubToken => i18n::string("settings.sync.gist.token.label"),
            SyncSecretSaveOperation::WebdavPassword => {
                i18n::string("settings.sync.webdav.password.label")
            }
        }
    }

    fn sync_gist_field_label() -> String {
        i18n::string("settings.sync.gist.gist_id.label")
    }

    fn ai_provider_ids(&self) -> Vec<String> {
        self.settings_store
            .settings()
            .ai_providers
            .iter()
            .map(|provider| provider.id.clone())
            .collect()
    }

    pub(in crate::ui::shell) fn current_ai_provider_kind(&self, cx: &App) -> AiProviderKind {
        self.panel_forms
            .settings
            .ai_provider_kind_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(AiProviderKind::OpenAi)
    }

    pub(in crate::ui::shell) fn current_web_search_kind(&self, cx: &App) -> WebSearchProviderKind {
        self.panel_forms
            .settings
            .web_search_kind_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(WebSearchProviderKind::Tavily)
    }

    pub(in crate::ui::shell) fn selected_ai_provider_id(&self, cx: &App) -> Option<String> {
        self.panel_forms
            .settings
            .ai_provider_select
            .read(cx)
            .selected_value()
            .cloned()
    }

    fn refresh_ai_provider_select(
        &mut self,
        selected_provider_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let options = ai_provider_select_options(self.settings_store.settings());
        let fallback_id = options.first().map(|option| option.value().clone());
        let selected_id = selected_provider_id
            .filter(|id| options.iter().any(|option| option.value().as_str() == *id))
            .map(ToOwned::to_owned)
            .or(fallback_id);

        let current_persisted = self
            .settings_store
            .settings()
            .selected_ai_provider_id
            .clone();
        if selected_id != current_persisted {
            self.settings_store.update(|settings| {
                settings.selected_ai_provider_id = selected_id.clone();
            });
        }

        self.panel_forms
            .settings
            .ai_provider_select
            .update(cx, |select, cx| {
                select.set_items(options, window, cx);
                if let Some(selected_id) = selected_id.as_ref() {
                    select.set_selected_value(selected_id, window, cx);
                } else {
                    select.set_selected_index(None, window, cx);
                }
            });
    }

    fn open_ai_provider_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let popup = PendingAiProviderPopupState;
        let stable_key = DialogOverlaySnapshot::AiProviderPopup(popup).stable_key();
        self.dialogs
            .exiting_dialogs
            .retain(|dialog| dialog.snapshot.stable_key() != stable_key);
        self.ai_provider_popup = Some(popup);
        self.panel_forms
            .settings
            .ai_provider_name_input
            .update(cx, |input, cx| {
                input.focus(window, cx);
            });
        cx.notify();
    }

    fn dismiss_ai_provider_popup(&mut self, cx: &mut Context<Self>) {
        if let Some(popup) = self.ai_provider_popup.take() {
            self.start_dialog_exit(DialogOverlaySnapshot::AiProviderPopup(popup), cx);
        }
    }

    pub(in crate::ui::shell) fn close_ai_provider_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        set_input_value(
            &self.panel_forms.settings.ai_provider_api_key_input,
            "",
            window,
            cx,
        );
        self.secret_visibility.clear_ai_provider_visibility();
        self.dismiss_ai_provider_popup(cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn open_selected_ai_provider_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(provider_id) = self.selected_ai_provider_id(cx) else {
            return;
        };
        self.edit_ai_provider(provider_id, window, cx);
    }

    pub(in crate::ui::shell) fn start_new_ai_provider(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.panel_forms.settings.editing_ai_provider_id = None;
        let kind = AiProviderKind::OpenAi;
        self.panel_forms
            .settings
            .ai_provider_kind_select
            .update(cx, |select, cx| {
                select.set_selected_value(&kind, window, cx);
            });
        set_input_value(
            &self.panel_forms.settings.ai_provider_name_input,
            i18n::string(ai_provider_kind_label_key(kind)),
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.ai_provider_model_input,
            kind.default_model(),
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.ai_provider_base_url_input,
            "",
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.ai_provider_api_key_input,
            "",
            window,
            cx,
        );
        self.secret_visibility.clear_ai_provider_visibility();
        self.open_ai_provider_popup(window, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn edit_ai_provider(
        &mut self,
        provider_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) = self
            .settings_store
            .settings()
            .ai_providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .cloned()
        else {
            return;
        };

        if self.local_vault_status == LocalVaultStatus::Locked && provider.has_api_key {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::OpenAiProvider(provider.id),
                window,
                cx,
            );
            return;
        }

        self.panel_forms.settings.editing_ai_provider_id = Some(provider.id.clone());
        self.panel_forms
            .settings
            .ai_provider_kind_select
            .update(cx, |select, cx| {
                select.set_selected_value(&provider.kind, window, cx);
            });
        set_input_value(
            &self.panel_forms.settings.ai_provider_name_input,
            provider.name.clone(),
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.ai_provider_model_input,
            provider.model.clone(),
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.ai_provider_base_url_input,
            provider.base_url.clone(),
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.ai_provider_api_key_input,
            "",
            window,
            cx,
        );
        self.set_secret_visibility(
            SecretRevealTarget::AiProviderApiKey(provider.id.clone()),
            false,
            false,
            window,
            cx,
        );
        self.open_ai_provider_popup(window, cx);
        if provider.has_api_key {
            self.spawn_ai_provider_api_key_load(provider.id.clone(), cx);
        } else {
            self.ai_provider_api_key_load_in_progress = None;
        }
        cx.notify();
    }

    fn spawn_ai_provider_api_key_load(&mut self, provider_id: String, cx: &mut Context<Self>) {
        let notification_window = cx.active_window();
        let secrets = self.services.secrets.clone();
        let provider_id_for_worker = provider_id.clone();
        let provider_id_for_cancel = provider_id.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);

        self.ai_provider_api_key_load_in_progress = Some(provider_id.clone());
        cx.notify();

        let spawn_result = std::thread::Builder::new()
            .name("ai-provider-api-key-load".to_string())
            .spawn(move || {
                let result = AiProviderApiKeyLoadTaskResult {
                    api_key: secrets.get(&provider_id_for_worker, SecretKind::AiProviderApiKey),
                    provider_id: provider_id_for_worker,
                };
                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            self.finish_ai_provider_api_key_load(
                AiProviderApiKeyLoadTaskResult {
                    provider_id,
                    api_key: Err(
                        anyhow::anyhow!(error).context("failed to spawn AI provider key loader")
                    ),
                },
                cx,
            );
            return;
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv()
                        .unwrap_or_else(|_| AiProviderApiKeyLoadTaskResult {
                            provider_id: provider_id_for_cancel,
                            api_key: Err(anyhow::anyhow!("AI provider key load task cancelled")),
                        })
                })
                .await;

            if let Some(window_handle) = notification_window {
                let result = std::rc::Rc::new(std::cell::RefCell::new(Some(result)));
                let result_for_window = result.clone();
                let this_for_window = this.clone();
                let update_result = window_handle.update(cx, move |_, window, cx| {
                    let Some(result) = result_for_window.borrow_mut().take() else {
                        return;
                    };

                    if let Err(error) = this_for_window.update(cx, move |this, cx| {
                        this.finish_ai_provider_api_key_load_in_window(result, window, cx);
                    }) {
                        log::debug!(
                            "failed to apply AI provider key load result in window: {error:?}"
                        );
                    }
                });

                if let Err(error) = update_result {
                    log::debug!(
                        "failed to access active window for AI provider key load: {error:?}"
                    );
                    if let Some(result) = result.borrow_mut().take()
                        && let Err(error) = this.update(cx, move |this, cx| {
                            this.finish_ai_provider_api_key_load(result, cx);
                        })
                    {
                        log::debug!(
                            "failed to apply AI provider key load result without window: {error:?}"
                        );
                    }
                }
            } else if let Err(error) = this.update(cx, move |this, cx| {
                this.finish_ai_provider_api_key_load(result, cx);
            }) {
                log::debug!(
                    "failed to apply AI provider key load result without active window: {error:?}"
                );
            }
        })
        .detach();
    }

    fn finish_ai_provider_api_key_load_in_window(
        &mut self,
        result: AiProviderApiKeyLoadTaskResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let provider_id = result.provider_id;
        if !self.ai_provider_api_key_load_matches_current_editor(&provider_id) {
            return;
        }

        self.ai_provider_api_key_load_in_progress = None;
        match result.api_key {
            Ok(Some(api_key)) => {
                set_input_value(
                    &self.panel_forms.settings.ai_provider_api_key_input,
                    api_key,
                    window,
                    cx,
                );
            }
            Ok(None) => {
                set_input_value(
                    &self.panel_forms.settings.ai_provider_api_key_input,
                    "",
                    window,
                    cx,
                );
            }
            Err(error) => {
                set_input_value(
                    &self.panel_forms.settings.ai_provider_api_key_input,
                    "",
                    window,
                    cx,
                );
                self.notify_secret_reveal_failed(window, &error, cx);
            }
        }

        cx.notify();
    }

    fn finish_ai_provider_api_key_load(
        &mut self,
        result: AiProviderApiKeyLoadTaskResult,
        cx: &mut Context<Self>,
    ) {
        if !self.ai_provider_api_key_load_matches_current_editor(&result.provider_id) {
            return;
        }

        self.ai_provider_api_key_load_in_progress = None;
        if let Err(error) = result.api_key {
            self.status_message = error.to_string();
        }
        cx.notify();
    }

    fn ai_provider_api_key_load_matches_current_editor(&self, provider_id: &str) -> bool {
        self.ai_provider_popup.is_some()
            && self.panel_forms.settings.editing_ai_provider_id.as_deref() == Some(provider_id)
            && self.ai_provider_api_key_load_in_progress.as_deref() == Some(provider_id)
    }

    pub(in crate::ui::shell) fn submit_ai_provider_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ai_provider_save_in_progress() {
            return;
        }

        let kind = self.current_ai_provider_kind(cx);
        let editing_id = self.panel_forms.settings.editing_ai_provider_id.clone();
        let existing = editing_id.as_ref().and_then(|id| {
            self.settings_store
                .settings()
                .ai_providers
                .iter()
                .find(|provider| provider.id == *id)
                .cloned()
        });
        let api_key = self
            .panel_forms
            .settings
            .ai_provider_api_key_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let name = self
            .panel_forms
            .settings
            .ai_provider_name_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if name.is_empty() {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.ai_providers.validation.name_required"),
                cx,
            );
            return;
        }
        if api_key.is_empty() {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.ai_providers.validation.api_key_required"),
                cx,
            );
            return;
        }
        let existing_has_api_key = existing.as_ref().is_some_and(|p| p.has_api_key);
        let mut provider = existing.unwrap_or_else(|| AiProviderConfig::new(kind));
        provider.kind = kind;
        provider.name = name;
        provider.model = self
            .panel_forms
            .settings
            .ai_provider_model_input
            .read(cx)
            .value()
            .to_string();
        provider.base_url = self
            .panel_forms
            .settings
            .ai_provider_base_url_input
            .read(cx)
            .value()
            .to_string();
        if api_key.is_empty() && !existing_has_api_key {
            // Not an existing provider with a stored key and no new key → keep env var fallback when set
            provider.api_key_env = provider.api_key_env.trim().to_string();
        }
        provider.has_api_key = !api_key.is_empty() || existing_has_api_key;
        provider.sanitize();

        let draft = AiProviderSaveDraft { provider, api_key };
        if self.local_vault_status == LocalVaultStatus::Locked && !draft.api_key.is_empty() {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::SaveAiProvider(draft),
                window,
                cx,
            );
            return;
        }

        self.continue_save_ai_provider_after_unlock(draft, window, cx);
    }

    fn continue_save_ai_provider_after_unlock(
        &mut self,
        draft: AiProviderSaveDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ai_provider_save_in_progress() {
            return;
        }

        if self.local_vault_status == LocalVaultStatus::Locked {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::SaveAiProvider(draft),
                window,
                cx,
            );
            return;
        }

        self.spawn_ai_provider_save_operation(draft, cx);
    }

    fn apply_ai_provider_save_result(
        &mut self,
        task_result: AiProviderSaveTaskResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let provider_id = task_result.provider.id.clone();
        let changed = self.upsert_ai_provider(task_result.provider);

        self.panel_forms.settings.editing_ai_provider_id = Some(provider_id.clone());
        if changed {
            self.refresh_ai_provider_select(Some(&provider_id), window, cx);
        }
        set_input_value(
            &self.panel_forms.settings.ai_provider_api_key_input,
            "",
            window,
            cx,
        );
        self.set_secret_visibility(
            SecretRevealTarget::AiProviderApiKey(provider_id),
            false,
            false,
            window,
            cx,
        );
        let message = i18n::string("settings.ai_providers.notifications.saved_message");
        self.status_message = message.clone();
        window.push_notification(
            Self::success_notification(
                i18n::string("settings.ai_providers.notifications.saved_title"),
                message,
            ),
            cx,
        );
        self.dismiss_ai_provider_popup(cx);
        cx.notify();
    }

    fn finish_ai_provider_save_operation(
        &mut self,
        result: Result<AiProviderSaveTaskResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ai_provider_save_in_progress = false;

        match result {
            Ok(task_result) => {
                self.apply_ai_provider_save_result(task_result, window, cx);
            }
            Err(error) => {
                self.notify_sync_secret_save_failed(
                    window,
                    &i18n::string("settings.ai_providers.api_key.label"),
                    &error.to_string(),
                    cx,
                );
            }
        }

        cx.notify();
    }

    fn finish_ai_provider_save_operation_without_window(
        &mut self,
        result: Result<AiProviderSaveTaskResult>,
        cx: &mut Context<Self>,
    ) {
        self.ai_provider_save_in_progress = false;

        match result {
            Ok(task_result) => {
                let provider_id = task_result.provider.id.clone();
                self.upsert_ai_provider(task_result.provider);
                self.panel_forms.settings.editing_ai_provider_id = Some(provider_id.clone());
                self.secret_visibility
                    .set_visible(SecretRevealTarget::AiProviderApiKey(provider_id), false);
                self.status_message =
                    i18n::string("settings.ai_providers.notifications.saved_message");
                self.dismiss_ai_provider_popup(cx);
            }
            Err(error) => {
                self.status_message = i18n::string_args(
                    "settings.sync.save_feedback.failed_message",
                    &[
                        (
                            "field",
                            &i18n::string("settings.ai_providers.api_key.label"),
                        ),
                        ("error", &error.to_string()),
                    ],
                );
            }
        }

        cx.notify();
    }

    fn upsert_ai_provider(&mut self, provider: AiProviderConfig) -> bool {
        let provider_id = provider.id.clone();
        self.settings_store.update(|settings| {
            if let Some(existing) = settings
                .ai_providers
                .iter_mut()
                .find(|existing| existing.id == provider_id)
            {
                *existing = provider;
            } else {
                settings.ai_providers.push(provider);
            }
        })
    }

    fn spawn_ai_provider_save_operation(
        &mut self,
        draft: AiProviderSaveDraft,
        cx: &mut Context<Self>,
    ) {
        let notification_window = cx.active_window();
        self.ai_provider_save_in_progress = true;
        cx.notify();

        let secrets = self.services.secrets.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name("ai-provider-save".to_string())
            .spawn(move || {
                let AiProviderSaveDraft {
                    mut provider,
                    api_key,
                } = draft;

                let result = if api_key.is_empty() {
                    Ok(AiProviderSaveTaskResult { provider })
                } else {
                    secrets
                        .set(&provider.id, SecretKind::AiProviderApiKey, &api_key)
                        .map(|()| {
                            provider.has_api_key = true;
                            AiProviderSaveTaskResult { provider }
                        })
                };

                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            self.ai_provider_save_in_progress = true;
            self.finish_ai_provider_save_operation_without_window(
                Err(anyhow::anyhow!(error).context("failed to spawn AI provider save worker")),
                cx,
            );
            return;
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("AI provider save task cancelled")))
                })
                .await;

            if let Some(window_handle) = notification_window {
                let result = std::rc::Rc::new(std::cell::RefCell::new(Some(result)));
                let result_for_window = result.clone();
                let this_for_window = this.clone();
                let update_result = window_handle.update(cx, move |_, window, cx| {
                    let Some(result) = result_for_window.borrow_mut().take() else {
                        return;
                    };

                    if let Err(error) = this_for_window.update(cx, move |this, cx| {
                        this.finish_ai_provider_save_operation(result, window, cx);
                    }) {
                        log::debug!("failed to apply AI provider save result in window: {error:?}");
                    }
                });

                if let Err(error) = update_result {
                    log::debug!("failed to access active window for AI provider save: {error:?}");
                    if let Some(result) = result.borrow_mut().take()
                        && let Err(error) = this.update(cx, move |this, cx| {
                            this.finish_ai_provider_save_operation_without_window(result, cx);
                        })
                    {
                        log::debug!(
                            "failed to apply AI provider save result without window: {error:?}"
                        );
                    }
                }
            } else if let Err(error) = this.update(cx, move |this, cx| {
                this.finish_ai_provider_save_operation_without_window(result, cx);
            }) {
                log::debug!(
                    "failed to apply AI provider save result without active window: {error:?}"
                );
            }
        })
        .detach();
    }

    pub(in crate::ui::shell) fn on_web_search_kind_changed(
        &mut self,
        kind: WebSearchProviderKind,
        cx: &mut Context<Self>,
    ) {
        if let Some(window_handle) = cx.active_window() {
            let update_result = window_handle.update(cx, |_, window, cx| {
                set_input_value(
                    &self.panel_forms.settings.web_search_api_key_input,
                    "",
                    window,
                    cx,
                );
                set_input_placeholder(
                    &self.panel_forms.settings.web_search_endpoint_input,
                    web_search_endpoint_placeholder(kind),
                    window,
                    cx,
                );
                set_input_placeholder(
                    &self.panel_forms.settings.web_search_api_key_input,
                    Self::localized_secret_placeholder(
                        self.settings_store.settings().web_search.has_api_key,
                        "settings.web_search.placeholders.api_key",
                    ),
                    window,
                    cx,
                );
            });
            if let Err(error) = update_result {
                log::debug!("failed to update web search form after provider change: {error:?}");
            }
        }
        self.secret_visibility
            .set_visible(SecretRevealTarget::WebSearchApiKey, false);
        self.status_message = i18n::string_args(
            "settings.web_search.status.kind_selected",
            &[(
                "kind",
                &i18n::string(web_search_provider_kind_label_key(kind)),
            )],
        );
        cx.notify();
    }

    pub(in crate::ui::shell) fn submit_web_search_settings_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.web_search_save_in_progress() {
            return;
        }

        let kind = self.current_web_search_kind(cx);
        let api_key = self
            .panel_forms
            .settings
            .web_search_api_key_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let endpoint = self
            .panel_forms
            .settings
            .web_search_endpoint_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let max_results_text = self
            .panel_forms
            .settings
            .web_search_max_results_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let Ok(max_results) = max_results_text.parse::<u32>() else {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::InvalidInput,
                i18n::string_args(
                    "settings.web_search.validation.max_results_range",
                    &[
                        (
                            "min",
                            &miaominal_settings::WEB_SEARCH_MAX_RESULTS_MIN.to_string(),
                        ),
                        (
                            "max",
                            &miaominal_settings::WEB_SEARCH_MAX_RESULTS_MAX.to_string(),
                        ),
                    ],
                ),
                cx,
            );
            return;
        };
        if !(miaominal_settings::WEB_SEARCH_MAX_RESULTS_MIN
            ..=miaominal_settings::WEB_SEARCH_MAX_RESULTS_MAX)
            .contains(&max_results)
        {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::InvalidInput,
                i18n::string_args(
                    "settings.web_search.validation.max_results_range",
                    &[
                        (
                            "min",
                            &miaominal_settings::WEB_SEARCH_MAX_RESULTS_MIN.to_string(),
                        ),
                        (
                            "max",
                            &miaominal_settings::WEB_SEARCH_MAX_RESULTS_MAX.to_string(),
                        ),
                    ],
                ),
                cx,
            );
            return;
        }
        if kind == WebSearchProviderKind::SearXng && endpoint.is_empty() {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.web_search.validation.endpoint_required"),
                cx,
            );
            return;
        }

        let mut config = self.settings_store.settings().web_search.clone();
        config.kind = kind;
        config.endpoint = endpoint;
        config.max_results = max_results;
        config.api_key_env.clear();
        config.has_api_key = config.has_api_key || !api_key.is_empty();
        config.sanitize();

        if config.kind.requires_api_key() && !config.has_api_key {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.web_search.validation.api_key_required"),
                cx,
            );
            return;
        }
        config.enabled = true;

        let draft = WebSearchSaveDraft { config, api_key };
        if self.local_vault_status == LocalVaultStatus::Locked && !draft.api_key.is_empty() {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::SaveWebSearch(draft),
                window,
                cx,
            );
            return;
        }

        self.continue_save_web_search_after_unlock(draft, window, cx);
    }

    fn continue_save_web_search_after_unlock(
        &mut self,
        draft: WebSearchSaveDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.web_search_save_in_progress() {
            return;
        }

        if self.local_vault_status == LocalVaultStatus::Locked {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::SaveWebSearch(draft),
                window,
                cx,
            );
            return;
        }

        self.spawn_web_search_save_operation(draft, cx);
    }

    fn apply_web_search_save_result(
        &mut self,
        task_result: WebSearchSaveTaskResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let config = task_result.config;
        self.settings_store
            .update(|settings| settings.web_search = config.clone());
        set_input_value(
            &self.panel_forms.settings.web_search_api_key_input,
            "",
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.settings.web_search_api_key_input,
            Self::localized_secret_placeholder(
                config.has_api_key,
                "settings.web_search.placeholders.api_key",
            ),
            window,
            cx,
        );
        self.set_secret_visibility(
            SecretRevealTarget::WebSearchApiKey,
            false,
            false,
            window,
            cx,
        );
        let message = i18n::string("settings.web_search.notifications.saved_message");
        self.status_message = message.clone();
        window.push_notification(
            Self::success_notification(
                i18n::string("settings.web_search.notifications.saved_title"),
                message,
            ),
            cx,
        );
        cx.notify();
    }

    fn finish_web_search_save_operation(
        &mut self,
        result: Result<WebSearchSaveTaskResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.web_search_save_in_progress = false;

        match result {
            Ok(task_result) => self.apply_web_search_save_result(task_result, window, cx),
            Err(error) => self.notify_sync_secret_save_failed(
                window,
                &i18n::string("settings.web_search.api_key.label"),
                &error.to_string(),
                cx,
            ),
        }

        cx.notify();
    }

    fn finish_web_search_save_operation_without_window(
        &mut self,
        result: Result<WebSearchSaveTaskResult>,
        cx: &mut Context<Self>,
    ) {
        self.web_search_save_in_progress = false;

        match result {
            Ok(task_result) => {
                self.settings_store
                    .update(|settings| settings.web_search = task_result.config);
                self.secret_visibility
                    .set_visible(SecretRevealTarget::WebSearchApiKey, false);
                self.status_message =
                    i18n::string("settings.web_search.notifications.saved_message");
            }
            Err(error) => {
                self.status_message = i18n::string_args(
                    "settings.sync.save_feedback.failed_message",
                    &[
                        ("field", &i18n::string("settings.web_search.api_key.label")),
                        ("error", &error.to_string()),
                    ],
                );
            }
        }

        cx.notify();
    }

    fn spawn_web_search_save_operation(
        &mut self,
        draft: WebSearchSaveDraft,
        cx: &mut Context<Self>,
    ) {
        let notification_window = cx.active_window();
        self.web_search_save_in_progress = true;
        cx.notify();

        let secrets = self.services.secrets.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name("web-search-save".to_string())
            .spawn(move || {
                let WebSearchSaveDraft {
                    mut config,
                    api_key,
                } = draft;

                let result = if api_key.is_empty() {
                    Ok(WebSearchSaveTaskResult { config })
                } else {
                    secrets
                        .set("web_search", SecretKind::WebSearchApiKey, &api_key)
                        .map(|()| {
                            config.has_api_key = true;
                            WebSearchSaveTaskResult { config }
                        })
                };

                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            self.web_search_save_in_progress = true;
            self.finish_web_search_save_operation_without_window(
                Err(anyhow::anyhow!(error).context("failed to spawn web search save worker")),
                cx,
            );
            return;
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("web search save task cancelled")))
                })
                .await;

            if let Some(window_handle) = notification_window {
                let result = std::rc::Rc::new(std::cell::RefCell::new(Some(result)));
                let result_for_window = result.clone();
                let this_for_window = this.clone();
                let update_result = window_handle.update(cx, move |_, window, cx| {
                    let Some(result) = result_for_window.borrow_mut().take() else {
                        return;
                    };

                    if let Err(error) = this_for_window.update(cx, move |this, cx| {
                        this.finish_web_search_save_operation(result, window, cx);
                    }) {
                        log::debug!("failed to apply web search save result in window: {error:?}");
                    }
                });

                if let Err(error) = update_result {
                    log::debug!("failed to access active window for web search save: {error:?}");
                    if let Some(result) = result.borrow_mut().take()
                        && let Err(error) = this.update(cx, move |this, cx| {
                            this.finish_web_search_save_operation_without_window(result, cx);
                        })
                    {
                        log::debug!(
                            "failed to apply web search save result without window: {error:?}"
                        );
                    }
                }
            } else if let Err(error) = this.update(cx, move |this, cx| {
                this.finish_web_search_save_operation_without_window(result, cx);
            }) {
                log::debug!(
                    "failed to apply web search save result without active window: {error:?}"
                );
            }
        })
        .detach();
    }

    pub(in crate::ui::shell) fn delete_ai_provider(
        &mut self,
        provider_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.local_vault_status == LocalVaultStatus::Locked {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.sync.vault.access_required_error.message"),
                cx,
            );
            return;
        }

        let removed = self.settings_store.update(|settings| {
            settings
                .ai_providers
                .retain(|provider| provider.id != provider_id);
        });
        self.services
            .secrets
            .delete_ai_provider_api_key(&provider_id);
        self.secret_visibility.set_visible(
            SecretRevealTarget::AiProviderApiKey(provider_id.clone()),
            false,
        );
        if self.panel_forms.settings.editing_ai_provider_id.as_deref() == Some(&provider_id) {
            self.panel_forms.settings.editing_ai_provider_id = None;
            set_input_value(
                &self.panel_forms.settings.ai_provider_api_key_input,
                "",
                window,
                cx,
            );
            self.dismiss_ai_provider_popup(cx);
        }
        self.refresh_ai_provider_select(None, window, cx);
        if removed {
            let message = i18n::string("settings.ai_providers.notifications.deleted_message");
            self.status_message = message.clone();
            window.push_notification(
                Self::success_notification(
                    i18n::string("settings.ai_providers.notifications.deleted_title"),
                    message,
                ),
                cx,
            );
        }
        cx.notify();
    }

    fn run_local_vault_unlock_follow_up(
        &mut self,
        follow_up: PendingLocalVaultUnlockAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match follow_up {
            PendingLocalVaultUnlockAction::OpenSession(profile) => {
                self.open_session_tab(*profile, window, cx);
            }
            PendingLocalVaultUnlockAction::OpenSyncNow => {
                self.trigger_sync_now(window, cx);
            }
            PendingLocalVaultUnlockAction::DeployManagedKey => {
                self.deploy_managed_key(window, cx);
            }
            PendingLocalVaultUnlockAction::SaveProfile => {
                self.continue_save_profile_after_unlock(window, cx);
            }
            PendingLocalVaultUnlockAction::ImportManagedKey => {
                self.continue_import_managed_key_after_unlock(window, cx);
            }
            PendingLocalVaultUnlockAction::SavePortForwardRule => {
                self.continue_save_port_forward_rule_after_unlock(window, cx);
            }
            PendingLocalVaultUnlockAction::SaveSnippet => {
                self.continue_save_snippet_after_unlock(window, cx);
            }
            PendingLocalVaultUnlockAction::SaveSyncGithubToken(token) => {
                self.continue_save_sync_github_token_after_unlock(token, window, cx);
            }
            PendingLocalVaultUnlockAction::SaveSyncWebdavPassword(password) => {
                self.continue_save_sync_webdav_password_after_unlock(password, window, cx);
            }
            PendingLocalVaultUnlockAction::SaveSyncPassphrase(passphrase) => {
                self.continue_save_sync_passphrase_after_unlock(passphrase, window, cx);
            }
            PendingLocalVaultUnlockAction::OpenAiProvider(provider_id) => {
                self.edit_ai_provider(provider_id, window, cx);
            }
            PendingLocalVaultUnlockAction::SaveAiProvider(draft) => {
                self.continue_save_ai_provider_after_unlock(draft, window, cx);
            }
            PendingLocalVaultUnlockAction::SaveWebSearch(draft) => {
                self.continue_save_web_search_after_unlock(draft, window, cx);
            }
            PendingLocalVaultUnlockAction::ClearSyncPassphrase => {
                self.continue_clear_sync_passphrase_after_unlock(window, cx);
            }
            PendingLocalVaultUnlockAction::RevealSecret(target) => {
                self.continue_reveal_secret_after_unlock(target, window, cx);
            }
        }
    }

    fn schedule_local_vault_unlock_follow_up(
        &mut self,
        follow_up: PendingLocalVaultUnlockAction,
        cx: &mut Context<Self>,
    ) {
        let notification_window = cx.active_window();

        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(0))
                .await;

            let Some(window_handle) = notification_window else {
                return;
            };

            let this_for_window = this.clone();
            let update_result = window_handle.update(cx, move |_, window, cx| {
                if let Err(error) = this_for_window.update(cx, move |this, cx| {
                    this.run_local_vault_unlock_follow_up(follow_up, window, cx);
                }) {
                    log::debug!("failed to run local vault follow-up action in window: {error:?}");
                }
            });

            if let Err(error) = update_result {
                log::debug!("failed to access active window for local vault follow-up: {error:?}");
            }
        })
        .detach();
    }

    pub(in crate::ui::shell) fn update_sync_config(
        &mut self,
        update: impl FnOnce(&mut SyncConfig),
    ) -> anyhow::Result<()> {
        self.sync.sync_engine.config_store.update(update)
    }

    pub(in crate::ui::shell) fn submit_sync_github_token_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_secret_save_in_progress() {
            return;
        }

        let token = self
            .panel_forms
            .settings
            .sync_github_token_input
            .read(cx)
            .value()
            .to_string();

        if self.local_vault_status == LocalVaultStatus::Locked {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::SaveSyncGithubToken(token),
                window,
                cx,
            );
            return;
        }

        self.continue_save_sync_github_token_after_unlock(token, window, cx);
    }

    pub(in crate::ui::shell) fn submit_sync_github_gist_id_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let gist_id_input = self
            .panel_forms
            .settings
            .sync_github_gist_id_input
            .read(cx)
            .value()
            .to_string();
        let gist_id = normalize_github_gist_id(&gist_id_input);
        let field_label = Self::sync_gist_field_label();

        if let Err(error) = self.update_sync_config(|config| {
            config.gist_id = (!gist_id.is_empty()).then_some(gist_id.clone());
        }) {
            self.notify_sync_secret_save_failed(window, &field_label, &error.to_string(), cx);
            return;
        }

        set_input_value(
            &self.panel_forms.settings.sync_github_gist_id_input,
            gist_id.clone(),
            window,
            cx,
        );

        if self.sync.sync_engine.config_store.config.provider == SyncProvider::GithubGist
            && self.sync.sync_engine.config_store.config.gist_enabled
        {
            if gist_id.is_empty() {
                self.sync.sync_status = SyncStatus::RemoteBindingRequired {
                    provider: SyncProvider::GithubGist,
                };
            } else if matches!(
                self.sync.sync_status,
                SyncStatus::RemoteBindingRequired {
                    provider: SyncProvider::GithubGist,
                }
            ) {
                self.sync.sync_status = SyncStatus::Idle;
            }
        }

        self.notify_sync_secret_saved(window, &field_label, cx);
    }

    fn continue_save_sync_github_token_after_unlock(
        &mut self,
        token: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_secret_save_in_progress() {
            return;
        }

        if self.local_vault_status == LocalVaultStatus::Locked {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::SaveSyncGithubToken(token),
                window,
                cx,
            );
            return;
        }

        self.spawn_sync_secret_save_operation(SyncSecretSaveTaskRequest::GithubToken(token), cx);
    }

    pub(in crate::ui::shell) fn submit_sync_webdav_password_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_secret_save_in_progress() {
            return;
        }

        let password = self
            .panel_forms
            .settings
            .sync_webdav_password_input
            .read(cx)
            .value()
            .to_string();

        if self.local_vault_status == LocalVaultStatus::Locked {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::SaveSyncWebdavPassword(password),
                window,
                cx,
            );
            return;
        }

        self.continue_save_sync_webdav_password_after_unlock(password, window, cx);
    }

    fn continue_save_sync_webdav_password_after_unlock(
        &mut self,
        password: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_secret_save_in_progress() {
            return;
        }

        if self.local_vault_status == LocalVaultStatus::Locked {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::SaveSyncWebdavPassword(password),
                window,
                cx,
            );
            return;
        }

        self.spawn_sync_secret_save_operation(
            SyncSecretSaveTaskRequest::WebdavPassword(password),
            cx,
        );
    }

    fn continue_save_sync_passphrase_after_unlock(
        &mut self,
        passphrase: String,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }

        self.spawn_sync_passphrase_operation(SyncPassphraseTaskRequest::Save(passphrase), cx);
    }

    fn continue_clear_sync_passphrase_after_confirm(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }

        if self.local_vault_status == LocalVaultStatus::Locked {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::ClearSyncPassphrase,
                window,
                cx,
            );
            return;
        }

        self.spawn_sync_passphrase_operation(SyncPassphraseTaskRequest::Clear, cx);
    }

    fn continue_clear_sync_passphrase_after_unlock(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }

        self.spawn_sync_passphrase_operation(SyncPassphraseTaskRequest::Clear, cx);
    }

    fn spawn_sync_secret_save_operation(
        &mut self,
        request: SyncSecretSaveTaskRequest,
        cx: &mut Context<Self>,
    ) {
        let operation = request.operation();
        let worker_name = request.worker_name().to_string();
        let cancelled_message = request.cancelled_message().to_string();
        let notification_window = cx.active_window();

        self.sync.sync_secret_save_operation = Some(operation);
        cx.notify();

        let mut sync_engine = self.sync.sync_engine.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name(worker_name)
            .spawn(move || {
                let result = match request {
                    SyncSecretSaveTaskRequest::GithubToken(token) => {
                        SettingsService::persist_sync_github_token(&mut sync_engine, token.as_str())
                            .map(|()| SyncSecretSaveTaskResult {
                                operation: SyncSecretSaveOperation::GithubToken,
                                updated_config: sync_engine.config_store.config.clone(),
                            })
                    }
                    SyncSecretSaveTaskRequest::WebdavPassword(password) => {
                        SettingsService::persist_sync_webdav_password(
                            &mut sync_engine,
                            password.as_str(),
                        )
                        .map(|()| SyncSecretSaveTaskResult {
                            operation: SyncSecretSaveOperation::WebdavPassword,
                            updated_config: sync_engine.config_store.config.clone(),
                        })
                    }
                };

                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            self.finish_sync_secret_save_spawn_error(operation, anyhow::anyhow!(error), cx);
            return;
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!(cancelled_message)))
                })
                .await;

            if let Some(window_handle) = notification_window {
                let result = std::rc::Rc::new(std::cell::RefCell::new(Some(result)));
                let result_for_window = result.clone();
                let this_for_window = this.clone();
                let update_result = window_handle.update(cx, move |_, window, cx| {
                    let Some(result) = result_for_window.borrow_mut().take() else {
                        return;
                    };

                    if let Err(error) = this_for_window.update(cx, move |this, cx| {
                        this.finish_sync_secret_save_operation(result, window, cx);
                    }) {
                        log::debug!("failed to apply sync secret save result in window: {error:?}");
                    }
                });

                if let Err(error) = update_result {
                    log::debug!("failed to access active window for sync secret save: {error:?}");
                    if let Some(result) = result.borrow_mut().take()
                        && let Err(error) = this.update(cx, move |this, cx| {
                            this.finish_sync_secret_save_operation_without_window(result, cx);
                        })
                    {
                        log::debug!(
                            "failed to apply sync secret save result without window: {error:?}"
                        );
                    }
                }
            } else if let Err(error) = this.update(cx, move |this, cx| {
                this.finish_sync_secret_save_operation_without_window(result, cx);
            }) {
                log::debug!(
                    "failed to apply sync secret save result without active window: {error:?}"
                );
            }
        })
        .detach();
    }

    fn finish_sync_secret_save_spawn_error(
        &mut self,
        operation: SyncSecretSaveOperation,
        error: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        self.sync.sync_secret_save_operation = Some(operation);
        self.finish_sync_secret_save_operation_without_window(
            Err(error.context("failed to spawn sync secret save worker")),
            cx,
        );
    }

    fn finish_sync_secret_save_operation(
        &mut self,
        result: Result<SyncSecretSaveTaskResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let operation = self
            .sync
            .sync_secret_save_operation
            .unwrap_or(SyncSecretSaveOperation::GithubToken);
        self.sync.sync_secret_save_operation = None;

        match result {
            Ok(task_result) => {
                let SyncSecretSaveTaskResult {
                    operation,
                    updated_config,
                } = task_result;
                self.sync.sync_engine.config_store.config = updated_config;
                self.refresh_sync_secret_placeholders(window, cx);
                let field_label = Self::sync_secret_field_label(operation);
                self.notify_sync_secret_saved(window, &field_label, cx);
            }
            Err(error) => {
                let field_label = Self::sync_secret_field_label(operation);
                self.notify_sync_secret_save_failed(window, &field_label, &error.to_string(), cx);
            }
        }
    }

    fn finish_sync_secret_save_operation_without_window(
        &mut self,
        result: Result<SyncSecretSaveTaskResult>,
        cx: &mut Context<Self>,
    ) {
        let operation = self
            .sync
            .sync_secret_save_operation
            .unwrap_or(SyncSecretSaveOperation::GithubToken);
        self.sync.sync_secret_save_operation = None;

        match result {
            Ok(task_result) => {
                let SyncSecretSaveTaskResult {
                    operation,
                    updated_config,
                } = task_result;
                self.sync.sync_engine.config_store.config = updated_config;
                let field_label = Self::sync_secret_field_label(operation);
                self.status_message = i18n::string_args(
                    "settings.sync.save_feedback.saved_message",
                    &[("field", &field_label)],
                );
            }
            Err(error) => {
                let field_label = Self::sync_secret_field_label(operation);
                self.status_message = i18n::string_args(
                    "settings.sync.save_feedback.failed_message",
                    &[("field", &field_label), ("error", &error.to_string())],
                );
            }
        }

        cx.notify();
    }

    fn spawn_sync_passphrase_operation(
        &mut self,
        request: SyncPassphraseTaskRequest,
        cx: &mut Context<Self>,
    ) {
        let operation = request.operation();
        let worker_name = request.worker_name().to_string();
        let cancelled_message = request.cancelled_message().to_string();
        let notification_window = cx.active_window();

        self.sync.sync_passphrase_operation = Some(operation);
        cx.notify();

        let mut sync_engine = self.sync.sync_engine.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name(worker_name)
            .spawn(move || {
                let result = match request {
                    SyncPassphraseTaskRequest::Save(passphrase) => {
                        SettingsService::persist_sync_passphrase(
                            &mut sync_engine,
                            Some(passphrase.as_str()),
                        )
                        .map(|configured| SyncPassphraseTaskResult {
                            operation: SyncPassphraseOperation::Save,
                            configured,
                            updated_config: sync_engine.config_store.config.clone(),
                        })
                    }
                    SyncPassphraseTaskRequest::Clear => {
                        SettingsService::persist_sync_passphrase(&mut sync_engine, None).map(
                            |configured| SyncPassphraseTaskResult {
                                operation: SyncPassphraseOperation::Clear,
                                configured,
                                updated_config: sync_engine.config_store.config.clone(),
                            },
                        )
                    }
                };

                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            self.finish_sync_passphrase_spawn_error(operation, anyhow::anyhow!(error), cx);
            return;
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!(cancelled_message)))
                })
                .await;

            if let Some(window_handle) = notification_window {
                let result = std::rc::Rc::new(std::cell::RefCell::new(Some(result)));
                let result_for_window = result.clone();
                let this_for_window = this.clone();
                let update_result = window_handle.update(cx, move |_, window, cx| {
                    let Some(result) = result_for_window.borrow_mut().take() else {
                        return;
                    };

                    if let Err(error) = this_for_window.update(cx, move |this, cx| {
                        this.finish_sync_passphrase_operation(result, window, cx);
                    }) {
                        log::debug!("failed to apply sync passphrase result in window: {error:?}");
                    }
                });

                if let Err(error) = update_result {
                    log::debug!("failed to access active window for sync passphrase: {error:?}");
                    if let Some(result) = result.borrow_mut().take()
                        && let Err(error) = this.update(cx, move |this, cx| {
                            this.finish_sync_passphrase_operation_without_window(result, cx);
                        })
                    {
                        log::debug!(
                            "failed to apply sync passphrase result without window: {error:?}"
                        );
                    }
                }
            } else if let Err(error) = this.update(cx, move |this, cx| {
                this.finish_sync_passphrase_operation_without_window(result, cx);
            }) {
                log::debug!(
                    "failed to apply sync passphrase result without active window: {error:?}"
                );
            }
        })
        .detach();
    }

    fn finish_sync_passphrase_spawn_error(
        &mut self,
        operation: SyncPassphraseOperation,
        error: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        self.sync.sync_passphrase_operation = Some(operation);
        self.finish_sync_passphrase_operation_without_window(
            Err(error.context("failed to spawn sync passphrase persistence worker")),
            cx,
        );
    }

    fn finish_sync_passphrase_operation(
        &mut self,
        result: Result<SyncPassphraseTaskResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let operation = self
            .sync
            .sync_passphrase_operation
            .unwrap_or(SyncPassphraseOperation::Save);
        self.sync.sync_passphrase_operation = None;

        match result {
            Ok(task_result) => {
                let SyncPassphraseTaskResult {
                    operation,
                    configured,
                    updated_config,
                } = task_result;
                self.sync.sync_engine.config_store.config = updated_config;
                self.set_sync_passphrase_configured_state(configured);
                self.clear_sync_passphrase_popup_inputs(window, cx);
                self.dismiss_sync_passphrase_popup(cx);

                match operation {
                    SyncPassphraseOperation::Save => {
                        let field_label = i18n::string("settings.sync.encryption.passphrase.label");
                        self.notify_sync_secret_saved(window, &field_label, cx);
                    }
                    SyncPassphraseOperation::Clear => {
                        let message = i18n::string(
                            "settings.sync.encryption.passphrase.notifications.cleared_message",
                        );
                        self.status_message = message.clone();
                        window.push_notification(
                            Self::success_notification(
                                i18n::string(
                                    "settings.sync.encryption.passphrase.notifications.cleared_title",
                                ),
                                message,
                            ),
                            cx,
                        );
                        cx.notify();
                    }
                }
            }
            Err(error) => match operation {
                SyncPassphraseOperation::Save => {
                    let field_label = i18n::string("settings.sync.encryption.passphrase.label");
                    self.notify_sync_secret_save_failed(
                        window,
                        &field_label,
                        &error.to_string(),
                        cx,
                    );
                }
                SyncPassphraseOperation::Clear => {
                    let message = i18n::string_args(
                        "settings.sync.encryption.passphrase.notifications.clear_failed_message",
                        &[("error", &error.to_string())],
                    );
                    self.status_message = message.clone();
                    window.push_notification(
                        Self::error_notification(
                            i18n::string(
                                "settings.sync.encryption.passphrase.notifications.clear_failed_title",
                            ),
                            message,
                        ),
                        cx,
                    );
                    cx.notify();
                }
            },
        }
    }

    fn finish_sync_passphrase_operation_without_window(
        &mut self,
        result: Result<SyncPassphraseTaskResult>,
        cx: &mut Context<Self>,
    ) {
        let operation = self
            .sync
            .sync_passphrase_operation
            .unwrap_or(SyncPassphraseOperation::Save);
        self.sync.sync_passphrase_operation = None;

        match result {
            Ok(task_result) => {
                let SyncPassphraseTaskResult {
                    operation,
                    configured,
                    updated_config,
                } = task_result;
                self.sync.sync_engine.config_store.config = updated_config;
                self.set_sync_passphrase_configured_state(configured);
                self.dismiss_sync_passphrase_popup(cx);
                self.status_message = match operation {
                    SyncPassphraseOperation::Save => i18n::string_args(
                        "settings.sync.save_feedback.saved_message",
                        &[(
                            "field",
                            &i18n::string("settings.sync.encryption.passphrase.label"),
                        )],
                    ),
                    SyncPassphraseOperation::Clear => i18n::string(
                        "settings.sync.encryption.passphrase.notifications.cleared_message",
                    ),
                };
            }
            Err(error) => {
                self.status_message = match operation {
                    SyncPassphraseOperation::Save => i18n::string_args(
                        "settings.sync.save_feedback.failed_message",
                        &[
                            (
                                "field",
                                &i18n::string("settings.sync.encryption.passphrase.label"),
                            ),
                            ("error", &error.to_string()),
                        ],
                    ),
                    SyncPassphraseOperation::Clear => i18n::string_args(
                        "settings.sync.encryption.passphrase.notifications.clear_failed_message",
                        &[("error", &error.to_string())],
                    ),
                };
            }
        }

        cx.notify();
    }

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

    fn invalidate_terminal_metrics(&mut self) {
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

    pub(in crate::ui::shell) fn set_terminal_right_click_behavior(
        &mut self,
        behavior: TerminalRightClickBehavior,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .settings_store
            .update(|settings| settings.terminal_right_click_behavior = behavior);
        if changed {
            self.status_message = match behavior {
                TerminalRightClickBehavior::ContextMenu => {
                    i18n::string("status.right_click_context_menu")
                }
                TerminalRightClickBehavior::CopySelectionOrPaste => {
                    i18n::string("status.right_click_copy_paste")
                }
            };
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn set_sync_provider(
        &mut self,
        provider: SyncProvider,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = SettingsService::set_sync_provider(&mut self.sync.sync_engine, provider)
        {
            log::warn!("failed to persist sync config: {error:?}");
            return;
        }

        cx.notify();
    }

    pub(in crate::ui::shell) fn local_vault_status_label(&self) -> String {
        i18n::string(match self.local_vault_status {
            LocalVaultStatus::Disabled => "settings.sync.vault.state.disabled",
            LocalVaultStatus::Locked => "settings.sync.vault.state.locked",
            LocalVaultStatus::Unlocked => "settings.sync.vault.state.unlocked",
        })
    }

    pub(in crate::ui::shell) fn local_vault_requires_passphrase(&self) -> bool {
        !matches!(self.local_vault_status, LocalVaultStatus::Unlocked)
    }

    pub(in crate::ui::shell) fn local_vault_primary_action_label(&self) -> String {
        i18n::string(match self.local_vault_status {
            LocalVaultStatus::Disabled => "settings.sync.vault.actions.enable",
            LocalVaultStatus::Locked => "settings.sync.vault.actions.unlock",
            LocalVaultStatus::Unlocked => "settings.sync.vault.actions.lock",
        })
    }

    pub(in crate::ui::shell) fn local_vault_can_disable(&self) -> bool {
        self.local_vault_status == LocalVaultStatus::Unlocked
            && !self.local_vault_operation_in_progress()
    }

    pub(in crate::ui::shell) fn local_vault_can_change_passphrase(&self) -> bool {
        self.local_vault_status == LocalVaultStatus::Unlocked
            && self.local_vault_session_passphrase.is_some()
            && !self.local_vault_operation_in_progress()
    }

    pub(in crate::ui::shell) fn local_vault_disable_action_label(&self) -> String {
        i18n::string("settings.sync.vault.actions.disable")
    }

    pub(in crate::ui::shell) fn local_vault_change_action_label(&self) -> String {
        i18n::string("settings.sync.vault.actions.change_passphrase")
    }

    pub(in crate::ui::shell) fn local_vault_passphrase_popup_title(
        &self,
        mode: LocalVaultPassphrasePopupMode,
    ) -> String {
        match mode {
            LocalVaultPassphrasePopupMode::PrimaryAction => self.local_vault_primary_action_label(),
            LocalVaultPassphrasePopupMode::ChangePassphrase => {
                self.local_vault_change_action_label()
            }
        }
    }

    pub(in crate::ui::shell) fn open_local_vault_passphrase_popup(
        &mut self,
        mode: LocalVaultPassphrasePopupMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.local_vault_operation_in_progress() {
            return;
        }

        let stable_key = DialogOverlaySnapshot::LocalVaultPassphrasePopup(mode).stable_key();
        self.dialogs
            .exiting_dialogs
            .retain(|dialog| dialog.snapshot.stable_key() != stable_key);
        self.local_vault_passphrase_popup = Some(mode);
        self.local_vault_unlock_in_progress = false;
        self.clear_local_vault_passphrase_input(window, cx);
        self.focus_local_vault_passphrase_input(window, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn open_local_vault_passphrase_popup_in_active_window(
        &mut self,
        mode: LocalVaultPassphrasePopupMode,
        cx: &mut Context<Self>,
    ) {
        if self.local_vault_operation_in_progress() {
            return;
        }

        let stable_key = DialogOverlaySnapshot::LocalVaultPassphrasePopup(mode).stable_key();
        self.dialogs
            .exiting_dialogs
            .retain(|dialog| dialog.snapshot.stable_key() != stable_key);
        self.local_vault_passphrase_popup = Some(mode);
        self.local_vault_unlock_in_progress = false;

        if let Some(window_handle) = cx.active_window() {
            let input = self
                .panel_forms
                .settings
                .local_vault_passphrase_input
                .clone();
            let confirmation_input = self
                .panel_forms
                .settings
                .local_vault_passphrase_confirmation_input
                .clone();
            if let Err(error) = window_handle.update(cx, move |_, window, cx| {
                input.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                    input.set_masked(true, window, cx);
                    input.focus(window, cx);
                });
                confirmation_input.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                    input.set_masked(true, window, cx);
                });
            }) {
                log::debug!(
                    "failed to reset local vault passphrase input before opening popup: {error:?}"
                );
            }
        }

        self.secret_visibility
            .set_visible(SecretRevealTarget::LocalVaultPassphrase, false);
        self.secret_visibility
            .set_visible(SecretRevealTarget::LocalVaultPassphraseConfirmation, false);

        cx.notify();
    }

    fn dismiss_local_vault_passphrase_popup(&mut self, cx: &mut Context<Self>) {
        if let Some(mode) = self.local_vault_passphrase_popup.take() {
            self.start_dialog_exit(DialogOverlaySnapshot::LocalVaultPassphrasePopup(mode), cx);
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn close_local_vault_passphrase_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.local_vault_operation_in_progress() {
            return;
        }

        self.local_vault_unlock_in_progress = false;
        self.pending_local_vault_unlock_action = None;
        self.clear_local_vault_passphrase_input(window, cx);
        self.dismiss_local_vault_passphrase_popup(cx);
    }

    pub(in crate::ui::shell) fn submit_local_vault_passphrase_popup_action(
        &mut self,
        mode: LocalVaultPassphrasePopupMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match mode {
            LocalVaultPassphrasePopupMode::PrimaryAction => {
                self.submit_local_vault_primary_action(window, cx);
            }
            LocalVaultPassphrasePopupMode::ChangePassphrase => {
                self.submit_local_vault_change_passphrase_action(window, cx);
            }
        }
    }

    pub(in crate::ui::shell) fn local_vault_secondary_action_label(&self) -> String {
        i18n::string("settings.sync.vault.actions.lock")
    }

    pub(in crate::ui::shell) fn submit_local_vault_primary_action(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.local_vault_operation_in_progress() {
            return;
        }

        let passphrase = self
            .panel_forms
            .settings
            .local_vault_passphrase_input
            .read(cx)
            .value()
            .trim()
            .to_string();

        match self.local_vault_status {
            LocalVaultStatus::Disabled => {
                let passphrase_confirmation = self
                    .panel_forms
                    .settings
                    .local_vault_passphrase_confirmation_input
                    .read(cx)
                    .value()
                    .trim()
                    .to_string();

                if passphrase.is_empty() {
                    self.notify_validation_failure_in_window(
                        window,
                        ValidationNotificationKind::RequiredInputMissing,
                        i18n::string("settings.sync.vault.passphrase_required_error.message"),
                        cx,
                    );
                    return;
                }

                if passphrase_confirmation.is_empty() {
                    self.notify_validation_failure_in_window(
                        window,
                        ValidationNotificationKind::RequiredInputMissing,
                        i18n::string(
                            "settings.sync.vault.passphrase_confirmation_required_error.message",
                        ),
                        cx,
                    );
                    return;
                }

                if passphrase != passphrase_confirmation {
                    self.notify_validation_failure_in_window(
                        window,
                        ValidationNotificationKind::InvalidInput,
                        i18n::string("settings.sync.vault.passphrase_mismatch_error.message"),
                        cx,
                    );
                    return;
                }

                self.spawn_local_vault_enable(passphrase, cx);
            }
            LocalVaultStatus::Locked => {
                if passphrase.is_empty() {
                    self.notify_validation_failure_in_window(
                        window,
                        ValidationNotificationKind::RequiredInputMissing,
                        i18n::string("settings.sync.vault.passphrase_required_error.message"),
                        cx,
                    );
                    return;
                }

                self.spawn_local_vault_unlock(passphrase, cx);
            }
            LocalVaultStatus::Unlocked => {
                if let Err(error) = self.lock_local_vault(window, cx) {
                    self.notify_local_vault_error(
                        window,
                        &self.local_vault_primary_action_label(),
                        error,
                        cx,
                    );
                }
            }
        }
    }

    fn spawn_local_vault_unlock(&mut self, passphrase: String, cx: &mut Context<Self>) {
        self.local_vault_unlock_in_progress = true;
        let notification_window = cx.active_window();
        cx.notify();

        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name("local-vault-unlock".to_string())
            .spawn(move || {
                let result = SettingsService::unlock_local_vault(passphrase).map(|transition| {
                    let sync_secret_inputs = Self::load_sync_secret_inputs(&transition.sync_engine);

                    LocalVaultUnlockResult {
                        transition,
                        sync_secret_inputs,
                    }
                });
                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            self.local_vault_unlock_in_progress = false;

            let error = anyhow::anyhow!(error).context("failed to spawn local vault unlock worker");
            let error_message = error.to_string();
            let action = self.local_vault_primary_action_label();
            let message = i18n::string_args(
                "settings.sync.vault.notifications.failed_message",
                &[("action", &action), ("error", &error_message)],
            );

            log::warn!("{error:?}");
            self.status_message = message.clone();

            if let Some(window_handle) = notification_window.as_ref()
                && let Err(update_error) = window_handle.update(cx, move |_, window, cx| {
                    window.push_notification(
                        Self::error_notification(
                            i18n::string("settings.sync.vault.notifications.failed_title"),
                            message,
                        ),
                        cx,
                    );
                })
            {
                log::debug!(
                    "failed to access active window for local vault unlock spawn error: {update_error:?}"
                );
            }

            cx.notify();
            return;
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv().unwrap_or_else(|_| {
                        Err(anyhow::anyhow!("local vault unlock task cancelled"))
                    })
                })
                .await;

            if let Some(window_handle) = notification_window {
                let result = std::rc::Rc::new(std::cell::RefCell::new(Some(result)));
                let result_for_window = result.clone();
                let this_for_window = this.clone();
                let update_result = window_handle.update(cx, move |_, window, cx| {
                    let Some(result) = result_for_window.borrow_mut().take() else {
                        return;
                    };

                    if let Err(error) = this_for_window.update(cx, move |this, cx| {
                        this.finish_local_vault_unlock(result, window, cx);
                    }) {
                        log::debug!(
                            "failed to apply local vault unlock result in window: {error:?}"
                        );
                    }
                });

                if let Err(error) = update_result {
                    log::debug!("failed to access active window for local vault unlock: {error:?}");
                    if let Some(result) = result.borrow_mut().take()
                        && let Err(error) = this.update(cx, move |this, cx| {
                            this.finish_local_vault_unlock_without_window(result, cx);
                        })
                    {
                        log::debug!(
                            "failed to apply local vault unlock result without window: {error:?}"
                        );
                    }
                }
            } else {
                if let Err(error) = this.update(cx, move |this, cx| {
                    this.finish_local_vault_unlock_without_window(result, cx);
                }) {
                    log::debug!(
                        "failed to apply local vault unlock result without active window: {error:?}"
                    );
                }
            }
        })
        .detach();
    }

    fn finish_local_vault_unlock(
        &mut self,
        result: Result<LocalVaultUnlockResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.local_vault_unlock_in_progress = false;

        match result {
            Ok(unlock_result) => {
                let LocalVaultUnlockResult {
                    transition,
                    sync_secret_inputs,
                } = unlock_result;

                self.apply_local_vault_transition(transition, cx);
                self.set_sync_passphrase_configured(&sync_secret_inputs.sync_passphrase);
                self.apply_sync_secret_inputs(sync_secret_inputs, window, cx);
                self.clear_local_vault_passphrase_input(window, cx);
                self.dismiss_local_vault_passphrase_popup(cx);

                let message = i18n::string("settings.sync.vault.notifications.unlocked_message");
                let follow_up = self.pending_local_vault_unlock_action.take();

                if follow_up.is_none() {
                    self.status_message = message.clone();
                }

                window.push_notification(
                    Self::success_notification(
                        i18n::string("settings.sync.vault.notifications.unlocked_title"),
                        message,
                    ),
                    cx,
                );

                cx.notify();

                if let Some(follow_up) = follow_up {
                    self.schedule_local_vault_unlock_follow_up(follow_up, cx);
                }
            }
            Err(error) => {
                self.notify_local_vault_error(
                    window,
                    &self.local_vault_primary_action_label(),
                    error,
                    cx,
                );
            }
        }
    }

    fn finish_local_vault_unlock_without_window(
        &mut self,
        result: Result<LocalVaultUnlockResult>,
        cx: &mut Context<Self>,
    ) {
        self.local_vault_unlock_in_progress = false;

        match result {
            Ok(unlock_result) => {
                let LocalVaultUnlockResult {
                    transition,
                    sync_secret_inputs,
                } = unlock_result;

                self.apply_local_vault_transition(transition, cx);
                self.set_sync_passphrase_configured(&sync_secret_inputs.sync_passphrase);
                self.pending_local_vault_unlock_action = None;
                self.dismiss_local_vault_passphrase_popup(cx);
                self.status_message =
                    i18n::string("settings.sync.vault.notifications.unlocked_message");
            }
            Err(error) => {
                let error_message = error.to_string();
                let action = self.local_vault_primary_action_label();
                self.status_message = i18n::string_args(
                    "settings.sync.vault.notifications.failed_message",
                    &[("action", &action), ("error", &error_message)],
                );
            }
        }

        cx.notify();
    }

    pub(in crate::ui::shell) fn submit_local_vault_lock_action(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.local_vault_operation_in_progress() {
            return;
        }

        if let Err(error) = self.lock_local_vault(window, cx) {
            self.notify_local_vault_error(
                window,
                &self.local_vault_secondary_action_label(),
                error,
                cx,
            );
        }
    }

    pub(in crate::ui::shell) fn submit_local_vault_disable_action(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.local_vault_operation_in_progress() {
            return;
        }

        if self.local_vault_status != LocalVaultStatus::Unlocked {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.sync.vault.disable_requires_unlock_error.message"),
                cx,
            );
            return;
        }

        self.dialogs.pending_local_vault_disable_confirm =
            Some(PendingLocalVaultDisableConfirmState);
        cx.notify();
    }

    pub(in crate::ui::shell) fn cancel_local_vault_disable_confirm(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if let Some(prompt) = self.dialogs.pending_local_vault_disable_confirm.take() {
            self.start_dialog_exit(DialogOverlaySnapshot::LocalVaultDisableConfirm(prompt), cx);
        }
    }

    pub(in crate::ui::shell) fn confirm_local_vault_disable(&mut self, cx: &mut Context<Self>) {
        let Some(prompt) = self.dialogs.pending_local_vault_disable_confirm.take() else {
            return;
        };

        self.start_dialog_exit(DialogOverlaySnapshot::LocalVaultDisableConfirm(prompt), cx);
        self.spawn_local_vault_disable(cx);
    }

    pub(in crate::ui::shell) fn submit_local_vault_change_passphrase_action(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.local_vault_operation_in_progress() {
            return;
        }

        let passphrase = self
            .panel_forms
            .settings
            .local_vault_passphrase_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let passphrase_confirmation = self
            .panel_forms
            .settings
            .local_vault_passphrase_confirmation_input
            .read(cx)
            .value()
            .trim()
            .to_string();

        if !self.local_vault_can_change_passphrase() {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.sync.vault.change_requires_unlock_error.message"),
                cx,
            );
            return;
        }

        if passphrase.is_empty() {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.sync.vault.passphrase_required_error.message"),
                cx,
            );
            return;
        }

        if passphrase_confirmation.is_empty() {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.sync.vault.passphrase_confirmation_required_error.message"),
                cx,
            );
            return;
        }

        if passphrase != passphrase_confirmation {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::InvalidInput,
                i18n::string("settings.sync.vault.passphrase_mismatch_error.message"),
                cx,
            );
            return;
        }

        let Some(current_passphrase) = self.local_vault_session_passphrase.clone() else {
            return;
        };

        self.spawn_local_vault_change_passphrase(current_passphrase, passphrase, cx);
    }

    fn spawn_local_vault_disable(&mut self, cx: &mut Context<Self>) {
        self.local_vault_disable_in_progress = true;
        let notification_window = cx.active_window();
        cx.notify();

        let previous_secrets = self.services.secrets.clone();
        let previous_sync_engine = self.sync.sync_engine.clone();
        let (session_ids, managed_key_ids, ai_provider_ids) = self.local_vault_secret_ids();

        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name("local-vault-disable".to_string())
            .spawn(move || {
                let result = SettingsService::prepare_vault_disable(
                    &previous_secrets,
                    &previous_sync_engine,
                    &session_ids,
                    &managed_key_ids,
                    &ai_provider_ids,
                );
                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            self.local_vault_disable_in_progress = false;

            let error =
                anyhow::anyhow!(error).context("failed to spawn local vault disable worker");
            let error_message = error.to_string();
            let action = self.local_vault_disable_action_label();
            let message = i18n::string_args(
                "settings.sync.vault.notifications.failed_message",
                &[("action", &action), ("error", &error_message)],
            );

            log::warn!("{error:?}");
            self.status_message = message.clone();

            if let Some(window_handle) = notification_window.as_ref()
                && let Err(update_error) = window_handle.update(cx, move |_, window, cx| {
                    window.push_notification(
                        Self::error_notification(
                            i18n::string("settings.sync.vault.notifications.failed_title"),
                            message,
                        ),
                        cx,
                    );
                })
            {
                log::debug!(
                    "failed to access active window for local vault disable spawn error: {update_error:?}"
                );
            }

            cx.notify();
            return;
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv().unwrap_or_else(|_| {
                        Err(anyhow::anyhow!("local vault disable task cancelled"))
                    })
                })
                .await;

            if let Some(window_handle) = notification_window {
                let result = std::rc::Rc::new(std::cell::RefCell::new(Some(result)));
                let result_for_window = result.clone();
                let this_for_window = this.clone();
                let update_result = window_handle.update(cx, move |_, window, cx| {
                    let Some(result) = result_for_window.borrow_mut().take() else {
                        return;
                    };

                    if let Err(error) = this_for_window.update(cx, move |this, cx| {
                        this.finish_local_vault_disable(result, window, cx);
                    }) {
                        log::debug!(
                            "failed to apply local vault disable result in window: {error:?}"
                        );
                    }
                });

                if let Err(error) = update_result {
                    log::debug!(
                        "failed to access active window for local vault disable: {error:?}"
                    );
                    if let Some(result) = result.borrow_mut().take()
                        && let Err(error) = this.update(cx, move |this, cx| {
                            this.finish_local_vault_disable_without_window(result, cx);
                        })
                    {
                        log::debug!(
                            "failed to apply local vault disable result without window: {error:?}"
                        );
                    }
                }
            } else if let Err(error) = this.update(cx, move |this, cx| {
                this.finish_local_vault_disable_without_window(result, cx);
            }) {
                log::debug!(
                    "failed to apply local vault disable result without active window: {error:?}"
                );
            }
        })
        .detach();
    }

    fn spawn_local_vault_enable(&mut self, passphrase: String, cx: &mut Context<Self>) {
        self.local_vault_unlock_in_progress = true;
        let notification_window = cx.active_window();
        cx.notify();

        let previous_secrets = self.services.secrets.clone();
        let previous_sync_engine = self.sync.sync_engine.clone();
        let (session_ids, managed_key_ids, ai_provider_ids) = self.local_vault_secret_ids();

        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name("local-vault-enable".to_string())
            .spawn(move || {
                let result = SettingsService::prepare_vault_enable(
                    &passphrase,
                    session_ids.clone(),
                    managed_key_ids.clone(),
                    ai_provider_ids.clone(),
                    previous_secrets,
                    previous_sync_engine,
                )
                .map(|(vault_secrets, vault_sync_engine)| {
                    let sync_secret_inputs = Self::load_sync_secret_inputs(&vault_sync_engine);
                    LocalVaultEnableResult {
                        passphrase,
                        vault_secrets,
                        vault_sync_engine,
                        sync_secret_inputs,
                        session_ids,
                        managed_key_ids,
                        ai_provider_ids,
                    }
                });
                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            self.local_vault_unlock_in_progress = false;

            let error = anyhow::anyhow!(error).context("failed to spawn local vault enable worker");
            let error_message = error.to_string();
            let action = self.local_vault_primary_action_label();
            let message = i18n::string_args(
                "settings.sync.vault.notifications.failed_message",
                &[("action", &action), ("error", &error_message)],
            );

            log::warn!("{error:?}");
            self.status_message = message.clone();

            if let Some(window_handle) = notification_window.as_ref()
                && let Err(update_error) = window_handle.update(cx, move |_, window, cx| {
                    window.push_notification(
                        Self::error_notification(
                            i18n::string("settings.sync.vault.notifications.failed_title"),
                            message,
                        ),
                        cx,
                    );
                })
            {
                log::debug!(
                    "failed to access active window for local vault enable spawn error: {update_error:?}"
                );
            }

            cx.notify();
            return;
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv().unwrap_or_else(|_| {
                        Err(anyhow::anyhow!("local vault enable task cancelled"))
                    })
                })
                .await;

            if let Some(window_handle) = notification_window {
                let result = std::rc::Rc::new(std::cell::RefCell::new(Some(result)));
                let result_for_window = result.clone();
                let this_for_window = this.clone();
                let update_result = window_handle.update(cx, move |_, window, cx| {
                    let Some(result) = result_for_window.borrow_mut().take() else {
                        return;
                    };

                    if let Err(error) = this_for_window.update(cx, move |this, cx| {
                        this.finish_local_vault_enable(result, window, cx);
                    }) {
                        log::debug!(
                            "failed to apply local vault enable result in window: {error:?}"
                        );
                    }
                });

                if let Err(error) = update_result {
                    log::debug!("failed to access active window for local vault enable: {error:?}");
                    if let Some(result) = result.borrow_mut().take()
                        && let Err(error) = this.update(cx, move |this, cx| {
                            this.finish_local_vault_enable_without_window(result, cx);
                        })
                    {
                        log::debug!(
                            "failed to apply local vault enable result without window: {error:?}"
                        );
                    }
                }
            } else {
                if let Err(error) = this.update(cx, move |this, cx| {
                    this.finish_local_vault_enable_without_window(result, cx);
                }) {
                    log::debug!(
                        "failed to apply local vault enable result without active window: {error:?}"
                    );
                }
            }
        })
        .detach();
    }

    fn finish_local_vault_enable(
        &mut self,
        result: Result<LocalVaultEnableResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.local_vault_unlock_in_progress = false;

        match result {
            Ok(enable_result) => {
                let LocalVaultEnableResult {
                    passphrase,
                    vault_secrets,
                    vault_sync_engine,
                    sync_secret_inputs,
                    session_ids,
                    managed_key_ids,
                    ai_provider_ids,
                } = enable_result;

                let previous_secrets = self.services.secrets.clone();
                let previous_sync_engine = self.sync.sync_engine.clone();

                match SettingsService::apply_vault_enable(
                    passphrase,
                    vault_secrets,
                    vault_sync_engine,
                    &mut self.settings_store,
                ) {
                    Ok(transition) => {
                        self.apply_local_vault_transition(transition, cx);
                        self.set_sync_passphrase_configured(&sync_secret_inputs.sync_passphrase);
                        self.apply_sync_secret_inputs(sync_secret_inputs, window, cx);
                        self.clear_local_vault_passphrase_input(window, cx);
                        self.dismiss_local_vault_passphrase_popup(cx);
                        SettingsService::delete_migrated_keyring_secrets(
                            &session_ids,
                            &managed_key_ids,
                            &ai_provider_ids,
                            &previous_secrets,
                            &previous_sync_engine,
                        );
                        let message =
                            i18n::string("settings.sync.vault.notifications.enabled_message");
                        self.status_message = message.clone();
                        window.push_notification(
                            Self::success_notification(
                                i18n::string("settings.sync.vault.notifications.enabled_title"),
                                message,
                            ),
                            cx,
                        );
                        cx.notify();
                    }
                    Err(error) => {
                        let action = self.local_vault_primary_action_label();
                        self.notify_local_vault_error(window, &action, error, cx);
                    }
                }
            }
            Err(error) => {
                let action = self.local_vault_primary_action_label();
                self.notify_local_vault_error(window, &action, error, cx);
            }
        }
    }

    fn finish_local_vault_enable_without_window(
        &mut self,
        result: Result<LocalVaultEnableResult>,
        cx: &mut Context<Self>,
    ) {
        self.local_vault_unlock_in_progress = false;

        match result {
            Ok(enable_result) => {
                let LocalVaultEnableResult {
                    passphrase,
                    vault_secrets,
                    vault_sync_engine,
                    sync_secret_inputs,
                    session_ids,
                    managed_key_ids,
                    ai_provider_ids,
                } = enable_result;

                let previous_secrets = self.services.secrets.clone();
                let previous_sync_engine = self.sync.sync_engine.clone();

                match SettingsService::apply_vault_enable(
                    passphrase,
                    vault_secrets,
                    vault_sync_engine,
                    &mut self.settings_store,
                ) {
                    Ok(transition) => {
                        self.apply_local_vault_transition(transition, cx);
                        self.set_sync_passphrase_configured(&sync_secret_inputs.sync_passphrase);
                        self.dismiss_local_vault_passphrase_popup(cx);
                        SettingsService::delete_migrated_keyring_secrets(
                            &session_ids,
                            &managed_key_ids,
                            &ai_provider_ids,
                            &previous_secrets,
                            &previous_sync_engine,
                        );
                        self.status_message =
                            i18n::string("settings.sync.vault.notifications.enabled_message");
                    }
                    Err(error) => {
                        let error_message = error.to_string();
                        let action = self.local_vault_primary_action_label();
                        self.status_message = i18n::string_args(
                            "settings.sync.vault.notifications.failed_message",
                            &[("action", &action), ("error", &error_message)],
                        );
                    }
                }
            }
            Err(error) => {
                let error_message = error.to_string();
                let action = self.local_vault_primary_action_label();
                self.status_message = i18n::string_args(
                    "settings.sync.vault.notifications.failed_message",
                    &[("action", &action), ("error", &error_message)],
                );
            }
        }

        cx.notify();
    }

    fn lock_local_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        self.prepare_host_password_for_lock(window, cx);
        let transition = SettingsService::local_vault_lock_transition(&self.settings_store);
        self.apply_local_vault_transition(transition, cx);
        self.refresh_sync_secret_inputs(window, cx);
        self.hide_storage_backed_secret_visibility(window, cx);
        self.clear_local_vault_passphrase_input(window, cx);

        let message = i18n::string("settings.sync.vault.notifications.locked_message");
        self.status_message = message.clone();
        window.push_notification(
            Self::success_notification(
                i18n::string("settings.sync.vault.notifications.locked_title"),
                message,
            ),
            cx,
        );
        cx.notify();
        Ok(())
    }

    fn spawn_local_vault_change_passphrase(
        &mut self,
        current_passphrase: String,
        new_passphrase: String,
        cx: &mut Context<Self>,
    ) {
        self.local_vault_unlock_in_progress = true;
        let notification_window = cx.active_window();
        cx.notify();

        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name("local-vault-change-passphrase".to_string())
            .spawn(move || {
                let result = SettingsService::change_local_vault_passphrase(
                    &current_passphrase,
                    new_passphrase,
                )
                .map(|outcome| {
                    let sync_secret_inputs = match &outcome {
                        LocalVaultPassphraseChangeOutcome::Reopened(transition) => {
                            Self::load_sync_secret_inputs(&transition.sync_engine)
                        }
                        LocalVaultPassphraseChangeOutcome::Locked { .. } => {
                            LocalVaultSyncSecretInputs {
                                github_token: String::new(),
                                webdav_password: String::new(),
                                sync_passphrase: String::new(),
                            }
                        }
                    };
                    LocalVaultChangePassphraseResult {
                        outcome,
                        sync_secret_inputs,
                    }
                });
                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            self.local_vault_unlock_in_progress = false;

            let error = anyhow::anyhow!(error)
                .context("failed to spawn local vault change passphrase worker");
            let error_message = error.to_string();
            let action = self.local_vault_change_action_label();
            let message = i18n::string_args(
                "settings.sync.vault.notifications.failed_message",
                &[("action", &action), ("error", &error_message)],
            );

            log::warn!("{error:?}");
            self.status_message = message.clone();

            if let Some(window_handle) = notification_window.as_ref()
                && let Err(update_error) = window_handle.update(cx, move |_, window, cx| {
                    window.push_notification(
                        Self::error_notification(
                            i18n::string("settings.sync.vault.notifications.failed_title"),
                            message,
                        ),
                        cx,
                    );
                })
            {
                log::debug!(
                    "failed to access active window for local vault change passphrase spawn error: {update_error:?}"
                );
            }

            cx.notify();
            return;
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv().unwrap_or_else(|_| {
                        Err(anyhow::anyhow!("local vault change passphrase task cancelled"))
                    })
                })
                .await;

            if let Some(window_handle) = notification_window {
                let result = std::rc::Rc::new(std::cell::RefCell::new(Some(result)));
                let result_for_window = result.clone();
                let this_for_window = this.clone();
                let update_result = window_handle.update(cx, move |_, window, cx| {
                    let Some(result) = result_for_window.borrow_mut().take() else {
                        return;
                    };

                    if let Err(error) = this_for_window.update(cx, move |this, cx| {
                        this.finish_local_vault_change_passphrase(result, window, cx);
                    }) {
                        log::debug!(
                            "failed to apply local vault change passphrase result in window: {error:?}"
                        );
                    }
                });

                if let Err(error) = update_result {
                    log::debug!(
                        "failed to access active window for local vault change passphrase: {error:?}"
                    );
                    if let Some(result) = result.borrow_mut().take()
                        && let Err(error) = this.update(cx, move |this, cx| {
                            this.finish_local_vault_change_passphrase_without_window(result, cx);
                        })
                    {
                        log::debug!(
                            "failed to apply local vault change passphrase result without window: {error:?}"
                        );
                    }
                }
            } else {
                if let Err(error) = this.update(cx, move |this, cx| {
                    this.finish_local_vault_change_passphrase_without_window(result, cx);
                }) {
                    log::debug!(
                        "failed to apply local vault change passphrase result without active window: {error:?}"
                    );
                }
            }
        })
        .detach();
    }

    fn finish_local_vault_change_passphrase(
        &mut self,
        result: Result<LocalVaultChangePassphraseResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.local_vault_unlock_in_progress = false;

        match result {
            Ok(change_result) => {
                let LocalVaultChangePassphraseResult {
                    outcome,
                    sync_secret_inputs,
                } = change_result;

                match outcome {
                    LocalVaultPassphraseChangeOutcome::Reopened(transition) => {
                        self.apply_local_vault_transition(transition, cx);
                        self.set_sync_passphrase_configured(&sync_secret_inputs.sync_passphrase);
                        self.apply_sync_secret_inputs(sync_secret_inputs, window, cx);
                        self.clear_local_vault_passphrase_input(window, cx);
                        self.dismiss_local_vault_passphrase_popup(cx);
                        let message = i18n::string(
                            "settings.sync.vault.notifications.passphrase_changed_message",
                        );
                        self.status_message = message.clone();
                        window.push_notification(
                            Self::success_notification(
                                i18n::string(
                                    "settings.sync.vault.notifications.passphrase_changed_title",
                                ),
                                message,
                            ),
                            cx,
                        );
                        cx.notify();
                    }
                    LocalVaultPassphraseChangeOutcome::Locked { transition, error } => {
                        self.apply_local_vault_transition(transition, cx);
                        self.apply_sync_secret_inputs(sync_secret_inputs, window, cx);
                        self.clear_local_vault_passphrase_input(window, cx);
                        let action = self.local_vault_change_action_label();
                        self.notify_local_vault_error(window, &action, error, cx);
                    }
                }
            }
            Err(error) => {
                let action = self.local_vault_change_action_label();
                self.notify_local_vault_error(window, &action, error, cx);
            }
        }
    }

    fn finish_local_vault_change_passphrase_without_window(
        &mut self,
        result: Result<LocalVaultChangePassphraseResult>,
        cx: &mut Context<Self>,
    ) {
        self.local_vault_unlock_in_progress = false;

        match result {
            Ok(change_result) => {
                let LocalVaultChangePassphraseResult {
                    outcome,
                    sync_secret_inputs,
                } = change_result;

                match outcome {
                    LocalVaultPassphraseChangeOutcome::Reopened(transition) => {
                        self.apply_local_vault_transition(transition, cx);
                        self.set_sync_passphrase_configured(&sync_secret_inputs.sync_passphrase);
                        self.dismiss_local_vault_passphrase_popup(cx);
                        self.status_message = i18n::string(
                            "settings.sync.vault.notifications.passphrase_changed_message",
                        );
                    }
                    LocalVaultPassphraseChangeOutcome::Locked { transition, error } => {
                        self.apply_local_vault_transition(transition, cx);
                        let error_message = error.to_string();
                        let action = self.local_vault_change_action_label();
                        self.status_message = i18n::string_args(
                            "settings.sync.vault.notifications.failed_message",
                            &[("action", &action), ("error", &error_message)],
                        );
                    }
                }
            }
            Err(error) => {
                let error_message = error.to_string();
                let action = self.local_vault_change_action_label();
                self.status_message = i18n::string_args(
                    "settings.sync.vault.notifications.failed_message",
                    &[("action", &action), ("error", &error_message)],
                );
            }
        }

        cx.notify();
    }

    fn finish_local_vault_disable(
        &mut self,
        result: Result<LocalVaultTransition>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.local_vault_disable_in_progress = false;

        match result {
            Ok(transition) => {
                if let Err(error) = self.disable_local_vault(transition, window, cx) {
                    let action = self.local_vault_disable_action_label();
                    self.notify_local_vault_error(window, &action, error, cx);
                }
            }
            Err(error) => {
                let action = self.local_vault_disable_action_label();
                self.notify_local_vault_error(window, &action, error, cx);
            }
        }
    }

    fn finish_local_vault_disable_without_window(
        &mut self,
        result: Result<LocalVaultTransition>,
        cx: &mut Context<Self>,
    ) {
        self.local_vault_disable_in_progress = false;

        match result {
            Ok(transition) => {
                match SettingsService::apply_vault_disable(&mut self.settings_store) {
                    Ok(()) => {
                        self.apply_local_vault_transition(transition, cx);

                        if let Err(error) = SettingsService::erase_vault_file() {
                            log::warn!(
                                "failed to erase local vault file after disabling vault: {error:?}"
                            );
                        }

                        self.status_message =
                            i18n::string("settings.sync.vault.notifications.disabled_message");
                    }
                    Err(error) => {
                        let error_message = error.to_string();
                        let action = self.local_vault_disable_action_label();
                        self.status_message = i18n::string_args(
                            "settings.sync.vault.notifications.failed_message",
                            &[("action", &action), ("error", &error_message)],
                        );
                    }
                }
            }
            Err(error) => {
                let error_message = error.to_string();
                let action = self.local_vault_disable_action_label();
                self.status_message = i18n::string_args(
                    "settings.sync.vault.notifications.failed_message",
                    &[("action", &action), ("error", &error_message)],
                );
            }
        }

        cx.notify();
    }

    fn disable_local_vault(
        &mut self,
        transition: LocalVaultTransition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        SettingsService::apply_vault_disable(&mut self.settings_store)?;
        self.apply_local_vault_transition(transition, cx);
        self.refresh_sync_secret_inputs(window, cx);
        self.clear_local_vault_passphrase_input(window, cx);

        if let Err(error) = SettingsService::erase_vault_file() {
            log::warn!("failed to erase local vault file after disabling vault: {error:?}");
        }

        let message = i18n::string("settings.sync.vault.notifications.disabled_message");
        self.status_message = message.clone();
        window.push_notification(
            Self::success_notification(
                i18n::string("settings.sync.vault.notifications.disabled_title"),
                message,
            ),
            cx,
        );
        cx.notify();
        Ok(())
    }

    fn local_vault_secret_ids(&self) -> (Vec<String>, Vec<String>, Vec<String>) {
        (
            self.data
                .sessions
                .iter()
                .map(|session| session.id.clone())
                .collect(),
            self.data
                .managed_keys
                .iter()
                .map(|key| key.id.clone())
                .collect(),
            self.ai_provider_ids(),
        )
    }

    fn apply_local_vault_transition(
        &mut self,
        transition: LocalVaultTransition,
        cx: &mut Context<Self>,
    ) {
        self.services.secrets = transition.secrets;
        self.sync.sync_engine = transition.sync_engine;
        self.local_vault_status = match transition.mode {
            LocalVaultMode::Disabled => LocalVaultStatus::Disabled,
            LocalVaultMode::Locked => LocalVaultStatus::Locked,
            LocalVaultMode::Unlocked => LocalVaultStatus::Unlocked,
        };
        self.local_vault_session_passphrase = transition.session_passphrase;
        self.sync_local_vault_auto_lock_task(cx);
    }

    fn set_sync_passphrase_configured(&mut self, _passphrase: &str) {
        self.sync.sync_passphrase_configured =
            self.sync.sync_engine.config_store.config.has_passphrase;
    }

    fn set_sync_passphrase_configured_state(&mut self, configured: bool) {
        self.sync.sync_passphrase_configured = configured;
    }

    fn sync_local_vault_auto_lock_task(&mut self, cx: &mut Context<Self>) {
        self.local_vault_auto_lock_task = None;

        if self.local_vault_status != LocalVaultStatus::Unlocked {
            return;
        }

        let Some(duration) = self
            .settings_store
            .settings()
            .local_vault_auto_lock_duration
            .duration()
        else {
            return;
        };

        let notification_window = cx.active_window();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(duration).await;

            if let Some(window_handle) = notification_window {
                let this_for_window = this.clone();
                let update_result = window_handle.update(cx, move |_, window, cx| {
                    if let Err(error) = this_for_window.update(cx, move |this, cx| {
                        this.finish_local_vault_auto_lock(window, cx);
                    }) {
                        log::debug!("failed to apply local vault auto-lock in window: {error:?}");
                    }
                });

                if let Err(error) = update_result {
                    log::debug!(
                        "failed to access active window for local vault auto-lock: {error:?}"
                    );
                    if let Err(error) = this.update(cx, move |this, cx| {
                        this.finish_local_vault_auto_lock_without_window(cx);
                    }) {
                        log::debug!(
                            "failed to apply local vault auto-lock without window: {error:?}"
                        );
                    }
                }
            } else if let Err(error) = this.update(cx, move |this, cx| {
                this.finish_local_vault_auto_lock_without_window(cx);
            }) {
                log::debug!(
                    "failed to apply local vault auto-lock without active window: {error:?}"
                );
            }
        });

        self.local_vault_auto_lock_task = Some(task);
    }

    fn finish_local_vault_auto_lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.local_vault_status != LocalVaultStatus::Unlocked
            || self
                .settings_store
                .settings()
                .local_vault_auto_lock_duration
                .duration()
                .is_none()
        {
            self.local_vault_auto_lock_task = None;
            return;
        }

        self.prepare_host_password_for_lock(window, cx);
        let transition = SettingsService::local_vault_lock_transition(&self.settings_store);
        self.apply_local_vault_transition(transition, cx);
        self.refresh_sync_secret_inputs(window, cx);
        self.hide_storage_backed_secret_visibility(window, cx);
        self.clear_local_vault_passphrase_input(window, cx);

        let message = i18n::string("settings.sync.vault.notifications.locked_message");
        self.status_message = message.clone();
        window.push_notification(
            Self::success_notification(
                i18n::string("settings.sync.vault.notifications.locked_title"),
                message,
            ),
            cx,
        );
        cx.notify();
    }

    fn finish_local_vault_auto_lock_without_window(&mut self, cx: &mut Context<Self>) {
        if self.local_vault_status != LocalVaultStatus::Unlocked
            || self
                .settings_store
                .settings()
                .local_vault_auto_lock_duration
                .duration()
                .is_none()
        {
            self.local_vault_auto_lock_task = None;
            return;
        }

        let transition = SettingsService::local_vault_lock_transition(&self.settings_store);
        self.apply_local_vault_transition(transition, cx);
        self.secret_visibility
            .set_visible(SecretRevealTarget::SyncGithubToken, false);
        self.secret_visibility
            .set_visible(SecretRevealTarget::SyncWebdavPassword, false);
        self.secret_visibility
            .set_visible(SecretRevealTarget::HostPassword, false);
        self.status_message = i18n::string("settings.sync.vault.notifications.locked_message");
        cx.notify();
    }

    fn refresh_sync_secret_inputs(&self, window: &mut Window, cx: &mut Context<Self>) {
        let sync_secret_inputs = Self::load_sync_secret_inputs(&self.sync.sync_engine);

        self.apply_sync_secret_inputs(sync_secret_inputs, window, cx);
    }

    fn load_sync_secret_inputs(sync_engine: &SyncEngine) -> LocalVaultSyncSecretInputs {
        let sync_secrets = sync_engine
            .config_store
            .get_secrets()
            .unwrap_or_else(|error| {
                log::warn!("failed to refresh sync secret inputs: {error:?}");
                Default::default()
            });

        LocalVaultSyncSecretInputs {
            github_token: sync_secrets.github_token.unwrap_or_default(),
            webdav_password: sync_secrets.webdav_password.unwrap_or_default(),
            sync_passphrase: sync_secrets.passphrase.unwrap_or_default(),
        }
    }

    fn apply_sync_secret_inputs(
        &self,
        sync_secret_inputs: LocalVaultSyncSecretInputs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        set_input_value(
            &self.panel_forms.settings.sync_github_token_input,
            sync_secret_inputs.github_token,
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.sync_webdav_password_input,
            sync_secret_inputs.webdav_password,
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.sync_passphrase_input,
            sync_secret_inputs.sync_passphrase,
            window,
            cx,
        );
        self.refresh_sync_secret_placeholders(window, cx);
    }

    fn refresh_sync_secret_placeholders(&self, window: &mut Window, cx: &mut Context<Self>) {
        set_input_placeholder(
            &self.panel_forms.settings.sync_github_token_input,
            Self::localized_secret_placeholder(
                self.sync.sync_engine.config_store.config.has_github_token,
                "settings.sync.placeholders.github_token",
            ),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.settings.sync_webdav_password_input,
            Self::localized_secret_placeholder(
                self.sync
                    .sync_engine
                    .config_store
                    .config
                    .has_webdav_password,
                "settings.sync.placeholders.webdav_password",
            ),
            window,
            cx,
        );
    }

    fn clear_local_vault_passphrase_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        set_input_value(
            &self.panel_forms.settings.local_vault_passphrase_input,
            String::new(),
            window,
            cx,
        );
        set_input_value(
            &self
                .panel_forms
                .settings
                .local_vault_passphrase_confirmation_input,
            String::new(),
            window,
            cx,
        );
        set_input_masked(
            &self.panel_forms.settings.local_vault_passphrase_input,
            true,
            false,
            window,
            cx,
        );
        set_input_masked(
            &self
                .panel_forms
                .settings
                .local_vault_passphrase_confirmation_input,
            true,
            false,
            window,
            cx,
        );
        self.secret_visibility
            .set_visible(SecretRevealTarget::LocalVaultPassphrase, false);
        self.secret_visibility
            .set_visible(SecretRevealTarget::LocalVaultPassphraseConfirmation, false);
    }

    fn focus_local_vault_passphrase_input(&self, window: &mut Window, cx: &mut App) {
        self.panel_forms
            .settings
            .local_vault_passphrase_input
            .update(cx, |input, cx| {
                input.focus(window, cx);
            });
    }

    fn notify_local_vault_error(
        &mut self,
        window: &mut Window,
        action: &str,
        error: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        let error_message = error.to_string();
        let message = i18n::string_args(
            "settings.sync.vault.notifications.failed_message",
            &[("action", action), ("error", &error_message)],
        );
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

    pub(in crate::ui::shell) fn passphrase_is_set(&self) -> bool {
        self.sync.sync_passphrase_configured
    }

    pub(in crate::ui::shell) fn notify_passphrase_required_in_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.notify_validation_failure_in_window(
            window,
            ValidationNotificationKind::RequiredInputMissing,
            i18n::string("settings.sync.passphrase_required_error.message"),
            cx,
        );
    }

    pub(in crate::ui::shell) fn notify_sync_toggle_update_failed_in_window(
        &mut self,
        window: &mut Window,
        provider_name: String,
        error: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        let error = error.into();
        let message = i18n::string_args(
            "settings.sync.toggle_failed_error.message",
            &[("provider", &provider_name), ("error", &error)],
        );
        let notification = Self::error_notification(
            i18n::string("settings.sync.toggle_failed_error.title"),
            message.clone(),
        );

        self.status_message = message;
        window.push_notification(notification, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_terminal_shift_right_click_context_menu(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .settings_store
            .update(|settings| settings.terminal_shift_right_click_context_menu = enabled);
        if changed {
            self.status_message = if enabled {
                i18n::string("status.shift_right_click_enabled")
            } else {
                i18n::string("status.shift_right_click_disabled")
            };
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn set_monitor_history_duration(
        &mut self,
        duration: MonitorHistoryDuration,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .settings_store
            .update(|settings| settings.monitor_history_duration = duration);
        if changed {
            self.status_message = i18n::string("status.monitor_history_duration_changed");
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn set_local_vault_auto_lock_duration(
        &mut self,
        duration: LocalVaultAutoLockDuration,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .settings_store
            .update(|settings| settings.local_vault_auto_lock_duration = duration);
        if changed {
            self.sync_local_vault_auto_lock_task(cx);
            self.status_message = i18n::string("status.local_vault_auto_lock_duration_changed");
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn set_last_tab_close_behavior(
        &mut self,
        behavior: LastTabCloseBehavior,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .settings_store
            .update(|settings| settings.last_tab_close_behavior = behavior);
        if changed {
            self.status_message = match behavior {
                LastTabCloseBehavior::ExitApplication => {
                    i18n::string("status.last_tab_close_behavior_exit")
                }
                LastTabCloseBehavior::OpenNewHomeTab => {
                    i18n::string("status.last_tab_close_behavior_open_home")
                }
            };
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn persist_sftp_browser_hidden_columns(
        &mut self,
        side: SftpBrowserSide,
        hidden_columns: Vec<usize>,
        _cx: &mut Context<Self>,
    ) {
        let changed = match side {
            SftpBrowserSide::Local => self
                .settings_store
                .update(|settings| settings.local_sftp_hidden_columns = hidden_columns),
            SftpBrowserSide::Remote => self
                .settings_store
                .update(|settings| settings.remote_sftp_hidden_columns = hidden_columns),
        };

        let _ = changed;
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_github_gist_id;

    #[test]
    fn normalize_github_gist_id_extracts_id_from_url() {
        assert_eq!(
            normalize_github_gist_id(
                "  https://gist.github.com/example/abc123def456?file=sync.json  "
            ),
            "abc123def456"
        );
        assert_eq!(normalize_github_gist_id("abc123def456"), "abc123def456");
        assert_eq!(normalize_github_gist_id("   "), "");
    }
}
