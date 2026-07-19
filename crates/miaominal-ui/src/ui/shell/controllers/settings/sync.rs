use super::*;
use crate::ui::shell::support::set_input_masked;
use crate::ui::shell::{
    DeferredAppCommand, DialogOverlaySnapshot, SettingsDeferredCommand, ValidationFailure,
    error_notification, success_notification, validation_notification,
};
use gpui::App;
use gpui_component::WindowExt as _;
use gpui_component::notification::Notification;
use miaominal_services::SyncTaskResult;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ManualSyncAction {
    Push,
    ForcePush,
    Pull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManualSyncGate {
    InProgress,
    ProviderRequired,
    VaultUnlockRequired,
    Ready,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) enum SyncProviderConfigSaveDraft {
    GithubGist {
        token: String,
        gist_id: Option<String>,
    },
    WebDav {
        url: String,
        username: String,
        password: String,
    },
}

pub(in crate::ui::shell) struct LocalVaultSyncSecretInputs {
    pub(in crate::ui::shell) github_token: String,
    pub(in crate::ui::shell) webdav_password: String,
    pub(in crate::ui::shell) sync_passphrase: String,
}

enum SyncProviderConfigSaveTaskRequest {
    GithubGist {
        token: String,
        gist_id: Option<String>,
    },
    WebDav {
        url: String,
        username: String,
        password: String,
    },
}

struct SyncProviderConfigSaveTaskResult {
    operation: SyncProviderConfigSaveOperation,
    updated_config: SyncConfig,
}

impl SyncProviderConfigSaveTaskRequest {
    fn operation(&self) -> SyncProviderConfigSaveOperation {
        match self {
            Self::GithubGist { .. } => SyncProviderConfigSaveOperation::GithubGist,
            Self::WebDav { .. } => SyncProviderConfigSaveOperation::WebDav,
        }
    }

    fn worker_name(&self) -> &'static str {
        match self {
            Self::GithubGist { .. } => "sync-gist-config-save",
            Self::WebDav { .. } => "sync-webdav-config-save",
        }
    }

    fn cancelled_message(&self) -> &'static str {
        match self {
            Self::GithubGist { .. } => "sync GitHub Gist config save task cancelled",
            Self::WebDav { .. } => "sync WebDAV config save task cancelled",
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

impl SettingsController {
    fn manual_push_pull_confirm_reason(
        action: ManualSyncAction,
        status: &SyncStatus,
    ) -> Option<SyncPullConfirmReason> {
        if action == ManualSyncAction::Push && matches!(status, SyncStatus::PullRequired { .. }) {
            Some(SyncPullConfirmReason::RemoteNewer)
        } else {
            None
        }
    }

    fn manual_sync_gate(
        sync_in_progress: bool,
        sync_enabled_for_provider: bool,
        sync_requires_local_vault_unlock: bool,
    ) -> ManualSyncGate {
        if sync_in_progress {
            ManualSyncGate::InProgress
        } else if !sync_enabled_for_provider {
            ManualSyncGate::ProviderRequired
        } else if sync_requires_local_vault_unlock {
            ManualSyncGate::VaultUnlockRequired
        } else {
            ManualSyncGate::Ready
        }
    }

    pub(in crate::ui::shell) fn sync_requires_local_vault_unlock(&self) -> bool {
        self.local_vault_status == LocalVaultStatus::Locked
            && self.sync.sync_engine.sync_enabled_for_provider()
    }

    fn show_manual_sync_result(window: &mut Window, status: &SyncStatus, cx: &mut App) -> String {
        let message = crate::ui::shell::support::sync_status_summary(status);
        let notification = match status {
            SyncStatus::Error(_) => Notification::error(message.clone()).title(i18n::string(
                "settings.sync.status.notifications.failed_title",
            )),
            SyncStatus::PullRequired { .. } | SyncStatus::RemoteBindingRequired { .. } => {
                Notification::error(message.clone()).title(i18n::string(
                    "settings.sync.status.notifications.action_required_title",
                ))
            }
            _ => Notification::success(message.clone()).title(i18n::string(
                "settings.sync.status.notifications.succeeded_title",
            )),
        };
        window.push_notification(notification, cx);
        message
    }

    fn show_sync_provider_required_error(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let message = i18n::string("settings.sync.provider_required_error.message");
        self.sync.sync_status = SyncStatus::Error(message.clone());
        window.push_notification(
            error_notification(
                i18n::string("settings.sync.provider_required_error.title"),
                message.clone(),
            ),
            cx,
        );
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    fn execute_manual_sync(
        &mut self,
        action: ManualSyncAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sync_status = self.sync.sync_status.clone();
        let gate = Self::manual_sync_gate(
            matches!(sync_status, SyncStatus::Syncing),
            self.sync.sync_engine.sync_enabled_for_provider(),
            self.sync_requires_local_vault_unlock(),
        );
        match gate {
            ManualSyncGate::InProgress => {
                cx.emit(AppCommand::Feedback(
                    crate::ui::shell::support::sync_status_summary(&sync_status),
                ));
                return;
            }
            ManualSyncGate::ProviderRequired => {
                self.show_sync_provider_required_error(window, cx);
                return;
            }
            ManualSyncGate::VaultUnlockRequired => {
                cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                    SettingsDeferredCommand::ResumeSync,
                )));
                return;
            }
            ManualSyncGate::Ready => {}
        }

        let service = match self.sync_service() {
            Ok(service) => service,
            Err(error) => {
                let status = SyncStatus::Error(error.to_string());
                self.sync.sync_status = status.clone();
                let message = Self::show_manual_sync_result(window, &status, cx);
                cx.emit(AppCommand::Feedback(message));
                cx.notify();
                return;
            }
        };
        let settings_store = self.settings_store.clone();
        let engine = self.sync.sync_engine.clone();
        let runtime = service.runtime().clone();
        let notification_window = cx.active_window();

        self.sync.sync_status = SyncStatus::Syncing;
        cx.notify();

        let (tx, rx) = std::sync::mpsc::sync_channel::<anyhow::Result<SyncTaskResult>>(1);
        runtime.spawn(async move {
            let result = match action {
                ManualSyncAction::Push => service.push(engine, settings_store).await,
                ManualSyncAction::ForcePush => service.push_force(engine, settings_store).await,
                ManualSyncAction::Pull => service.pull(engine, settings_store).await,
            };
            tx.send(result).ok();
        });

        self.sync.active_sync_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("sync task cancelled")))
                })
                .await;

            if let Err(error) = this.update(cx, move |controller, cx| {
                match result {
                    Ok(result) => {
                        let status = result.status;
                        let pull_confirm_reason =
                            Self::manual_push_pull_confirm_reason(action, &status);
                        controller.sync.sync_engine.config_store.config = result.updated_config;
                        controller.sync.sync_status = status.clone();
                        cx.emit(AppCommand::Feedback(
                            crate::ui::shell::support::sync_status_summary(&status),
                        ));

                        if let Some(reason) = pull_confirm_reason {
                            controller.sync_pull_confirm =
                                Some(PendingSyncPullConfirmState { reason });
                        }
                        if let Some(reload) = result.reload {
                            cx.emit(AppCommand::SyncReloaded(Box::new(reload)));
                        }

                        if let Some(window_handle) = notification_window {
                            let gist_id_input = controller.forms.sync_github_gist_id_input.clone();
                            let gist_id = controller
                                .sync
                                .sync_engine
                                .config_store
                                .config
                                .gist_id
                                .clone();
                            let _ = window_handle.update(cx, move |_, window, cx| {
                                set_input_value(
                                    &gist_id_input,
                                    gist_id.unwrap_or_default(),
                                    window,
                                    cx,
                                );
                                Self::show_manual_sync_result(window, &status, cx);
                            });
                        }
                    }
                    Err(error) => {
                        let status = SyncStatus::Error(error.to_string());
                        controller.sync.sync_status = status.clone();
                        cx.emit(AppCommand::Feedback(
                            crate::ui::shell::support::sync_status_summary(&status),
                        ));
                        if let Some(window_handle) = notification_window {
                            let _ = window_handle.update(cx, move |_, window, cx| {
                                Self::show_manual_sync_result(window, &status, cx);
                            });
                        }
                    }
                }
                controller.sync.active_sync_task = None;
                cx.notify();
            }) {
                log::debug!("failed to publish manual sync result: {error:?}");
            }
        }));
    }

    pub(in crate::ui::shell) fn trigger_sync_now(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let status = self.sync.sync_status.clone();
        let gate = Self::manual_sync_gate(
            matches!(status, SyncStatus::Syncing),
            self.sync.sync_engine.sync_enabled_for_provider(),
            self.sync_requires_local_vault_unlock(),
        );
        match gate {
            ManualSyncGate::InProgress => {
                cx.emit(AppCommand::Feedback(
                    crate::ui::shell::support::sync_status_summary(&status),
                ));
            }
            ManualSyncGate::ProviderRequired => self.show_sync_provider_required_error(window, cx),
            ManualSyncGate::VaultUnlockRequired => cx.emit(AppCommand::vault_unlock(
                DeferredAppCommand::Settings(SettingsDeferredCommand::ResumeSync),
            )),
            ManualSyncGate::Ready => {
                self.sync_pull_confirm = None;
                self.sync_direction = Some(PendingSyncDirectionState);
                cx.notify();
            }
        }
    }

    pub(in crate::ui::shell) fn cancel_sync_direction(&mut self, cx: &mut Context<Self>) {
        if let Some(prompt) = self.sync_direction.take() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::SyncDirection(prompt),
            ));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn select_sync_push(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.sync_direction.take() else {
            return;
        };
        cx.emit(AppCommand::OverlayDismissed(
            DialogOverlaySnapshot::SyncDirection(prompt),
        ));
        self.execute_manual_sync(ManualSyncAction::Push, window, cx);
    }

    pub(in crate::ui::shell) fn select_sync_pull(&mut self, cx: &mut Context<Self>) {
        let Some(prompt) = self.sync_direction.take() else {
            return;
        };
        cx.emit(AppCommand::OverlayDismissed(
            DialogOverlaySnapshot::SyncDirection(prompt),
        ));
        self.sync_pull_confirm = Some(PendingSyncPullConfirmState {
            reason: SyncPullConfirmReason::Manual,
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn cancel_sync_pull_confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(prompt) = self.sync_pull_confirm.take() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::SyncPullConfirm(prompt),
            ));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn confirm_sync_pull(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.sync_pull_confirm.take() else {
            return;
        };
        cx.emit(AppCommand::OverlayDismissed(
            DialogOverlaySnapshot::SyncPullConfirm(prompt),
        ));
        self.execute_manual_sync(ManualSyncAction::Pull, window, cx);
    }

    pub(in crate::ui::shell) fn confirm_sync_force_push(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.sync_pull_confirm.take() else {
            return;
        };
        cx.emit(AppCommand::OverlayDismissed(
            DialogOverlaySnapshot::SyncPullConfirm(prompt),
        ));
        self.execute_manual_sync(ManualSyncAction::ForcePush, window, cx);
    }

    pub(in crate::ui::shell) fn select_sync_provider(
        &mut self,
        provider: SyncProvider,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.set_sync_provider(provider) {
            log::warn!("failed to persist sync config: {error:?}");
            return;
        }
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

    pub(in crate::ui::shell) fn sync_passphrase_operation_in_progress(&self) -> bool {
        self.sync.sync_passphrase_operation.is_some()
    }

    pub(in crate::ui::shell) fn sync_provider_config_save_in_progress(&self) -> bool {
        self.sync.sync_provider_config_save_operation.is_some()
    }

    pub(in crate::ui::shell) fn sync_provider_config_save_in_progress_for(
        &self,
        provider: SyncProvider,
    ) -> bool {
        match provider {
            SyncProvider::GithubGist => {
                self.sync.sync_provider_config_save_operation
                    == Some(SyncProviderConfigSaveOperation::GithubGist)
            }
            SyncProvider::WebDav => {
                self.sync.sync_provider_config_save_operation
                    == Some(SyncProviderConfigSaveOperation::WebDav)
            }
            SyncProvider::None => false,
        }
    }

    pub(in crate::ui::shell) fn sync_passphrase_save_in_progress(&self) -> bool {
        self.sync.sync_passphrase_operation == Some(SyncPassphraseOperation::Save)
    }

    pub(in crate::ui::shell) fn passphrase_is_set(&self) -> bool {
        self.sync.sync_passphrase_configured
    }

    pub(in crate::ui::shell) fn open_sync_passphrase_clear_confirm_popup(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<PendingSyncPassphraseClearConfirmPopupState> {
        if self.sync_passphrase_operation_in_progress() {
            return None;
        }
        let popup = PendingSyncPassphraseClearConfirmPopupState;
        self.sync_passphrase_clear_confirm_popup = Some(popup);
        cx.notify();
        Some(popup)
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
        cx: &mut Context<Self>,
    ) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }
        self.dismiss_sync_passphrase_clear_confirm_popup(cx);
        self.continue_clear_sync_passphrase_after_confirm(cx);
    }

    fn dismiss_sync_passphrase_clear_confirm_popup(&mut self, cx: &mut Context<Self>) {
        if let Some(popup) = self.sync_passphrase_clear_confirm_popup.take() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::SyncPassphraseClearConfirmPopup(popup),
            ));
        }
    }

    pub(in crate::ui::shell) fn open_sync_passphrase_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PendingSyncPassphrasePopupState> {
        if self.sync_passphrase_operation_in_progress() {
            return None;
        }

        let popup = PendingSyncPassphrasePopupState;
        self.sync_passphrase_popup = Some(popup);
        self.clear_sync_passphrase_popup_inputs(window, cx);
        self.forms
            .sync_passphrase_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
        Some(popup)
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
            .forms
            .sync_passphrase_input
            .read(cx)
            .value()
            .to_string();
        let confirmation = self
            .forms
            .sync_passphrase_confirmation_input
            .read(cx)
            .value()
            .to_string();
        if let Err(failure) = Self::validate_sync_passphrase(&passphrase, &confirmation) {
            self.notify_sync_validation_failure(failure, window, cx);
            return;
        }

        if self.local_vault_status == LocalVaultStatus::Locked {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                SettingsDeferredCommand::SaveSyncPassphrase(passphrase),
            )));
            return;
        }

        self.spawn_sync_passphrase_operation(SyncPassphraseTaskRequest::Save(passphrase), cx);
    }

    fn validate_sync_passphrase(
        passphrase: &str,
        confirmation: &str,
    ) -> Result<(), ValidationFailure> {
        if passphrase.trim().is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "settings.sync.encryption.passphrase.required_error.message",
            )));
        }
        if confirmation.trim().is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "settings.sync.encryption.passphrase.confirmation_required_error.message",
            )));
        }
        if passphrase != confirmation {
            return Err(ValidationFailure::invalid(i18n::string(
                "settings.sync.encryption.passphrase.mismatch_error.message",
            )));
        }
        Ok(())
    }

    pub(in crate::ui::shell) fn continue_save_sync_passphrase_after_unlock(
        &mut self,
        passphrase: String,
        cx: &mut Context<Self>,
    ) {
        if !self.sync_passphrase_operation_in_progress() {
            self.spawn_sync_passphrase_operation(SyncPassphraseTaskRequest::Save(passphrase), cx);
        }
    }

    fn continue_clear_sync_passphrase_after_confirm(&mut self, cx: &mut Context<Self>) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }
        if self.local_vault_status == LocalVaultStatus::Locked {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                SettingsDeferredCommand::ClearSyncPassphrase,
            )));
            return;
        }
        self.spawn_sync_passphrase_operation(SyncPassphraseTaskRequest::Clear, cx);
    }

    pub(in crate::ui::shell) fn continue_clear_sync_passphrase_after_unlock(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if !self.sync_passphrase_operation_in_progress() {
            self.spawn_sync_passphrase_operation(SyncPassphraseTaskRequest::Clear, cx);
        }
    }

    fn clear_sync_passphrase_popup_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        set_input_value(&self.forms.sync_passphrase_input, "", window, cx);
        set_input_value(
            &self.forms.sync_passphrase_confirmation_input,
            "",
            window,
            cx,
        );
        self.set_sync_secret_visibility(
            SecretRevealTarget::SyncPassphrase,
            false,
            false,
            window,
            cx,
        );
        self.set_sync_secret_visibility(
            SecretRevealTarget::SyncPassphraseConfirmation,
            false,
            false,
            window,
            cx,
        );
    }

    fn dismiss_sync_passphrase_popup(&mut self, cx: &mut Context<Self>) {
        if let Some(popup) = self.sync_passphrase_popup.take() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::SyncPassphrasePopup(popup),
            ));
        }
    }

    pub(in crate::ui::shell) fn open_selected_sync_provider_config_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PendingSyncProviderConfigPopupState> {
        let provider = self
            .forms
            .sync_provider_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(self.sync.sync_engine.config_store.config.provider);
        self.open_sync_provider_config_popup(provider, window, cx)
    }

    pub(in crate::ui::shell) fn open_sync_provider_config_popup(
        &mut self,
        provider: SyncProvider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PendingSyncProviderConfigPopupState> {
        if provider == SyncProvider::None {
            return None;
        }
        if self.local_vault_status == LocalVaultStatus::Locked
            && self.sync_provider_config_has_stored_secret(provider)
        {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                SettingsDeferredCommand::OpenSyncProviderConfig(provider),
            )));
            return None;
        }

        self.prepare_sync_provider_config_popup_inputs(provider, window, cx);
        let popup = PendingSyncProviderConfigPopupState { provider };
        self.sync_provider_config_popup = Some(popup);
        match provider {
            SyncProvider::GithubGist => self
                .forms
                .sync_github_token_input
                .update(cx, |input, cx| input.focus(window, cx)),
            SyncProvider::WebDav => self
                .forms
                .sync_webdav_url_input
                .update(cx, |input, cx| input.focus(window, cx)),
            SyncProvider::None => {}
        }
        cx.notify();
        Some(popup)
    }

    fn sync_provider_config_has_stored_secret(&self, provider: SyncProvider) -> bool {
        let config = &self.sync.sync_engine.config_store.config;
        match provider {
            SyncProvider::GithubGist => config.has_github_token,
            SyncProvider::WebDav => config.has_webdav_password,
            SyncProvider::None => false,
        }
    }

    fn prepare_sync_provider_config_popup_inputs(
        &mut self,
        provider: SyncProvider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_sync_secret_inputs(window, cx);
        let config = self.sync.sync_engine.config_store.config.clone();
        set_input_value(
            &self.forms.sync_github_gist_id_input,
            config.gist_id.unwrap_or_default(),
            window,
            cx,
        );
        set_input_value(
            &self.forms.sync_webdav_url_input,
            config.webdav_url,
            window,
            cx,
        );
        set_input_value(
            &self.forms.sync_webdav_username_input,
            config.webdav_username,
            window,
            cx,
        );
        match provider {
            SyncProvider::GithubGist => self.set_sync_secret_visibility(
                SecretRevealTarget::SyncGithubToken,
                false,
                false,
                window,
                cx,
            ),
            SyncProvider::WebDav => self.set_sync_secret_visibility(
                SecretRevealTarget::SyncWebdavPassword,
                false,
                false,
                window,
                cx,
            ),
            SyncProvider::None => {}
        }
    }

    pub(in crate::ui::shell) fn close_sync_provider_config_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_provider_config_save_in_progress() {
            return;
        }
        set_input_value(&self.forms.sync_github_token_input, "", window, cx);
        set_input_value(&self.forms.sync_webdav_password_input, "", window, cx);
        self.set_sync_secret_visibility(
            SecretRevealTarget::SyncGithubToken,
            false,
            false,
            window,
            cx,
        );
        self.set_sync_secret_visibility(
            SecretRevealTarget::SyncWebdavPassword,
            false,
            false,
            window,
            cx,
        );
        self.dismiss_sync_provider_config_popup(cx);
    }

    pub(in crate::ui::shell) fn submit_sync_provider_config_popup_action(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if self.sync_provider_config_save_in_progress() {
            return;
        }
        let Some(popup) = self.sync_provider_config_popup else {
            return;
        };

        let draft = match popup.provider {
            SyncProvider::GithubGist => {
                let token = self
                    .forms
                    .sync_github_token_input
                    .read(cx)
                    .value()
                    .to_string();
                let gist_id = normalize_github_gist_id(
                    &self.forms.sync_github_gist_id_input.read(cx).value(),
                );
                SyncProviderConfigSaveDraft::GithubGist {
                    token,
                    gist_id: (!gist_id.is_empty()).then_some(gist_id),
                }
            }
            SyncProvider::WebDav => SyncProviderConfigSaveDraft::WebDav {
                url: self
                    .forms
                    .sync_webdav_url_input
                    .read(cx)
                    .value()
                    .to_string(),
                username: self
                    .forms
                    .sync_webdav_username_input
                    .read(cx)
                    .value()
                    .to_string(),
                password: self
                    .forms
                    .sync_webdav_password_input
                    .read(cx)
                    .value()
                    .to_string(),
            },
            SyncProvider::None => return,
        };

        if self.local_vault_status == LocalVaultStatus::Locked {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                SettingsDeferredCommand::SaveSyncProviderConfig(draft),
            )));
            return;
        }
        self.continue_save_sync_provider_config_after_unlock(draft, cx);
    }

    pub(in crate::ui::shell) fn continue_save_sync_provider_config_after_unlock(
        &mut self,
        draft: SyncProviderConfigSaveDraft,
        cx: &mut Context<Self>,
    ) {
        if self.sync_provider_config_save_in_progress() {
            return;
        }
        if self.local_vault_status == LocalVaultStatus::Locked {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                SettingsDeferredCommand::SaveSyncProviderConfig(draft),
            )));
            return;
        }

        let request = match draft {
            SyncProviderConfigSaveDraft::GithubGist { token, gist_id } => {
                SyncProviderConfigSaveTaskRequest::GithubGist { token, gist_id }
            }
            SyncProviderConfigSaveDraft::WebDav {
                url,
                username,
                password,
            } => SyncProviderConfigSaveTaskRequest::WebDav {
                url,
                username,
                password,
            },
        };
        self.spawn_sync_provider_config_save_operation(request, cx);
    }

    fn dismiss_sync_provider_config_popup(&mut self, cx: &mut Context<Self>) {
        if let Some(popup) = self.sync_provider_config_popup.take() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::SyncProviderConfigPopup(popup),
            ));
        }
    }

    fn spawn_sync_provider_config_save_operation(
        &mut self,
        request: SyncProviderConfigSaveTaskRequest,
        cx: &mut Context<Self>,
    ) {
        let operation = request.operation();
        let worker_name = request.worker_name().to_string();
        let cancelled_message = request.cancelled_message().to_string();
        let notification_window = cx.active_window();
        self.sync.sync_provider_config_save_operation = Some(operation);
        cx.notify();

        let mut sync_engine = self.sync.sync_engine.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name(worker_name)
            .spawn(move || {
                let result = match request {
                    SyncProviderConfigSaveTaskRequest::GithubGist { token, gist_id } => {
                        SettingsService::persist_sync_gist_config(
                            &mut sync_engine,
                            token.as_str(),
                            gist_id,
                        )
                        .map(|()| SyncProviderConfigSaveTaskResult {
                            operation: SyncProviderConfigSaveOperation::GithubGist,
                            updated_config: sync_engine.config_store.config.clone(),
                        })
                    }
                    SyncProviderConfigSaveTaskRequest::WebDav {
                        url,
                        username,
                        password,
                    } => SettingsService::persist_sync_webdav_config(
                        &mut sync_engine,
                        url,
                        username,
                        password.as_str(),
                    )
                    .map(|()| SyncProviderConfigSaveTaskResult {
                        operation: SyncProviderConfigSaveOperation::WebDav,
                        updated_config: sync_engine.config_store.config.clone(),
                    }),
                };
                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            self.finish_sync_provider_config_save_operation_without_window(
                Err(anyhow::anyhow!(error)
                    .context("failed to spawn sync provider config save worker")),
                cx,
            );
            return;
        }

        self.sync_provider_config_save_task = Some(cx.spawn(async move |this, cx| {
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
                        this.finish_sync_provider_config_save_operation(result, window, cx);
                    }) {
                        log::debug!(
                            "failed to apply sync provider config save result in window: {error:?}"
                        );
                    }
                });
                if let Err(error) = update_result {
                    log::debug!(
                        "failed to access active window for sync provider config save: {error:?}"
                    );
                    if let Some(result) = result.borrow_mut().take()
                        && let Err(error) = this.update(cx, move |this, cx| {
                            this.finish_sync_provider_config_save_operation_without_window(
                                result, cx,
                            );
                        })
                    {
                        log::debug!(
                            "failed to apply sync provider config save result without window: {error:?}"
                        );
                    }
                }
            } else if let Err(error) = this.update(cx, move |this, cx| {
                this.finish_sync_provider_config_save_operation_without_window(result, cx);
            }) {
                log::debug!(
                    "failed to apply sync provider config save result without active window: {error:?}"
                );
            }
        }));
    }

    fn finish_sync_provider_config_save_operation(
        &mut self,
        result: anyhow::Result<SyncProviderConfigSaveTaskResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fallback_operation = self
            .sync
            .sync_provider_config_save_operation
            .unwrap_or(SyncProviderConfigSaveOperation::GithubGist);
        self.sync_provider_config_save_task = None;
        self.sync.sync_provider_config_save_operation = None;

        match result {
            Ok(task_result) => {
                self.sync.sync_engine.config_store.config = task_result.updated_config;
                self.refresh_sync_secret_placeholders(window, cx);
                self.refresh_sync_provider_config_inputs_after_save(
                    task_result.operation,
                    window,
                    cx,
                );
                let label = Self::sync_provider_config_field_label(task_result.operation);
                self.notify_sync_secret_saved(window, &label, cx);
                self.dismiss_sync_provider_config_popup(cx);
            }
            Err(error) => {
                let label = Self::sync_provider_config_field_label(fallback_operation);
                self.notify_sync_secret_save_failed(window, &label, &error, cx);
            }
        }
        cx.notify();
    }

    fn finish_sync_provider_config_save_operation_without_window(
        &mut self,
        result: anyhow::Result<SyncProviderConfigSaveTaskResult>,
        cx: &mut Context<Self>,
    ) {
        let fallback_operation = self
            .sync
            .sync_provider_config_save_operation
            .unwrap_or(SyncProviderConfigSaveOperation::GithubGist);
        self.sync_provider_config_save_task = None;
        self.sync.sync_provider_config_save_operation = None;

        let message = match result {
            Ok(task_result) => {
                self.sync.sync_engine.config_store.config = task_result.updated_config;
                self.dismiss_sync_provider_config_popup(cx);
                let label = Self::sync_provider_config_field_label(task_result.operation);
                Self::sync_secret_saved_message(&label)
            }
            Err(error) => {
                let label = Self::sync_provider_config_field_label(fallback_operation);
                Self::sync_secret_save_failed_message(&label, &error)
            }
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    fn refresh_sync_provider_config_inputs_after_save(
        &mut self,
        operation: SyncProviderConfigSaveOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let config = self.sync.sync_engine.config_store.config.clone();
        match operation {
            SyncProviderConfigSaveOperation::GithubGist => {
                set_input_value(
                    &self.forms.sync_github_gist_id_input,
                    config.gist_id.clone().unwrap_or_default(),
                    window,
                    cx,
                );
                set_input_value(&self.forms.sync_github_token_input, "", window, cx);
                self.set_sync_secret_visibility(
                    SecretRevealTarget::SyncGithubToken,
                    false,
                    false,
                    window,
                    cx,
                );
                if config.provider == SyncProvider::GithubGist
                    && config
                        .gist_id
                        .as_ref()
                        .map(|value| value.trim().is_empty())
                        .unwrap_or(true)
                {
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
            SyncProviderConfigSaveOperation::WebDav => {
                set_input_value(
                    &self.forms.sync_webdav_url_input,
                    config.webdav_url,
                    window,
                    cx,
                );
                set_input_value(
                    &self.forms.sync_webdav_username_input,
                    config.webdav_username,
                    window,
                    cx,
                );
                set_input_value(&self.forms.sync_webdav_password_input, "", window, cx);
                self.set_sync_secret_visibility(
                    SecretRevealTarget::SyncWebdavPassword,
                    false,
                    false,
                    window,
                    cx,
                );
            }
        }
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
            self.finish_sync_passphrase_operation_without_window(
                Err(anyhow::anyhow!(error)
                    .context("failed to spawn sync passphrase persistence worker")),
                cx,
            );
            return;
        }

        self.sync_passphrase_task = Some(cx.spawn(async move |this, cx| {
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
        }));
    }

    fn finish_sync_passphrase_operation(
        &mut self,
        result: anyhow::Result<SyncPassphraseTaskResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fallback_operation = self
            .sync
            .sync_passphrase_operation
            .unwrap_or(SyncPassphraseOperation::Save);
        self.sync_passphrase_task = None;
        self.sync.sync_passphrase_operation = None;

        match result {
            Ok(task_result) => {
                self.sync.sync_engine.config_store.config = task_result.updated_config;
                self.sync.sync_passphrase_configured = task_result.configured;
                self.clear_sync_passphrase_popup_inputs(window, cx);
                self.dismiss_sync_passphrase_popup(cx);
                match task_result.operation {
                    SyncPassphraseOperation::Save => {
                        let label = i18n::string("settings.sync.encryption.passphrase.label");
                        self.notify_sync_secret_saved(window, &label, cx);
                    }
                    SyncPassphraseOperation::Clear => {
                        let message = i18n::string(
                            "settings.sync.encryption.passphrase.notifications.cleared_message",
                        );
                        window.push_notification(
                            success_notification(
                                i18n::string(
                                    "settings.sync.encryption.passphrase.notifications.cleared_title",
                                ),
                                message.clone(),
                            ),
                            cx,
                        );
                        cx.emit(AppCommand::Feedback(message));
                    }
                }
            }
            Err(error) => match fallback_operation {
                SyncPassphraseOperation::Save => {
                    let label = i18n::string("settings.sync.encryption.passphrase.label");
                    self.notify_sync_secret_save_failed(window, &label, &error, cx);
                }
                SyncPassphraseOperation::Clear => {
                    let message = Self::sync_passphrase_clear_failed_message(&error);
                    window.push_notification(
                        error_notification(
                            i18n::string(
                                "settings.sync.encryption.passphrase.notifications.clear_failed_title",
                            ),
                            message.clone(),
                        ),
                        cx,
                    );
                    cx.emit(AppCommand::Feedback(message));
                }
            },
        }
        cx.notify();
    }

    fn finish_sync_passphrase_operation_without_window(
        &mut self,
        result: anyhow::Result<SyncPassphraseTaskResult>,
        cx: &mut Context<Self>,
    ) {
        let fallback_operation = self
            .sync
            .sync_passphrase_operation
            .unwrap_or(SyncPassphraseOperation::Save);
        self.sync_passphrase_task = None;
        self.sync.sync_passphrase_operation = None;

        let message = match result {
            Ok(task_result) => {
                self.sync.sync_engine.config_store.config = task_result.updated_config;
                self.sync.sync_passphrase_configured = task_result.configured;
                self.dismiss_sync_passphrase_popup(cx);
                match task_result.operation {
                    SyncPassphraseOperation::Save => Self::sync_secret_saved_message(
                        &i18n::string("settings.sync.encryption.passphrase.label"),
                    ),
                    SyncPassphraseOperation::Clear => i18n::string(
                        "settings.sync.encryption.passphrase.notifications.cleared_message",
                    ),
                }
            }
            Err(error) => match fallback_operation {
                SyncPassphraseOperation::Save => Self::sync_secret_save_failed_message(
                    &i18n::string("settings.sync.encryption.passphrase.label"),
                    &error,
                ),
                SyncPassphraseOperation::Clear => {
                    Self::sync_passphrase_clear_failed_message(&error)
                }
            },
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    fn sync_provider_config_field_label(operation: SyncProviderConfigSaveOperation) -> String {
        match operation {
            SyncProviderConfigSaveOperation::GithubGist => {
                i18n::string("settings.sync.providers.gist")
            }
            SyncProviderConfigSaveOperation::WebDav => {
                i18n::string("settings.sync.providers.webdav")
            }
        }
    }

    fn sync_secret_saved_message(field_label: &str) -> String {
        i18n::string_args(
            "settings.sync.save_feedback.saved_message",
            &[("field", field_label)],
        )
    }

    fn sync_secret_save_failed_message(field_label: &str, error: &anyhow::Error) -> String {
        i18n::string_args(
            "settings.sync.save_feedback.failed_message",
            &[("field", field_label), ("error", &error.to_string())],
        )
    }

    fn sync_passphrase_clear_failed_message(error: &anyhow::Error) -> String {
        i18n::string_args(
            "settings.sync.encryption.passphrase.notifications.clear_failed_message",
            &[("error", &error.to_string())],
        )
    }

    fn notify_sync_secret_saved(
        &mut self,
        window: &mut Window,
        field_label: &str,
        cx: &mut Context<Self>,
    ) {
        let message = Self::sync_secret_saved_message(field_label);
        window.push_notification(
            success_notification(
                i18n::string("settings.sync.save_feedback.saved_title"),
                message.clone(),
            ),
            cx,
        );
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    fn notify_sync_secret_save_failed(
        &mut self,
        window: &mut Window,
        field_label: &str,
        error: &anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        let message = Self::sync_secret_save_failed_message(field_label, error);
        window.push_notification(
            error_notification(
                i18n::string("settings.sync.save_feedback.failed_title"),
                message.clone(),
            ),
            cx,
        );
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    fn notify_sync_validation_failure(
        &mut self,
        failure: ValidationFailure,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = failure.message;
        window.push_notification(validation_notification(failure.kind, message.clone()), cx);
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    pub(super) fn set_sync_secret_visibility(
        &mut self,
        target: SecretRevealTarget,
        visible: bool,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = match &target {
            SecretRevealTarget::SyncGithubToken => self.forms.sync_github_token_input.clone(),
            SecretRevealTarget::SyncWebdavPassword => self.forms.sync_webdav_password_input.clone(),
            SecretRevealTarget::SyncPassphrase => self.forms.sync_passphrase_input.clone(),
            SecretRevealTarget::SyncPassphraseConfirmation => {
                self.forms.sync_passphrase_confirmation_input.clone()
            }
            _ => return,
        };
        self.secret_visibility.set_visible(target, visible);
        set_input_masked(&input, !visible, focus, window, cx);
    }

    pub(in crate::ui::shell) fn set_sync_passphrase_configured_from_inputs(
        &mut self,
        _passphrase: &str,
        cx: &mut Context<Self>,
    ) {
        self.sync.sync_passphrase_configured =
            self.sync.sync_engine.config_store.config.has_passphrase;
        cx.notify();
    }

    pub(in crate::ui::shell) fn refresh_sync_secret_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let inputs = Self::load_sync_secret_inputs(&self.sync.sync_engine);
        self.apply_sync_secret_inputs(inputs, window, cx);
    }

    pub(in crate::ui::shell) fn load_sync_secret_inputs(
        sync_engine: &SyncEngine,
    ) -> LocalVaultSyncSecretInputs {
        let secrets = sync_engine
            .config_store
            .get_secrets()
            .unwrap_or_else(|error| {
                log::warn!("failed to refresh sync secret inputs: {error:?}");
                Default::default()
            });
        LocalVaultSyncSecretInputs {
            github_token: secrets.github_token.unwrap_or_default(),
            webdav_password: secrets.webdav_password.unwrap_or_default(),
            sync_passphrase: secrets.passphrase.unwrap_or_default(),
        }
    }

    pub(in crate::ui::shell) fn apply_sync_secret_inputs(
        &mut self,
        inputs: LocalVaultSyncSecretInputs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        set_input_value(
            &self.forms.sync_github_token_input,
            inputs.github_token,
            window,
            cx,
        );
        set_input_value(
            &self.forms.sync_webdav_password_input,
            inputs.webdav_password,
            window,
            cx,
        );
        set_input_value(
            &self.forms.sync_passphrase_input,
            inputs.sync_passphrase,
            window,
            cx,
        );
        self.refresh_sync_secret_placeholders(window, cx);
    }

    fn refresh_sync_secret_placeholders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = &self.sync.sync_engine.config_store.config;
        set_input_placeholder(
            &self.forms.sync_github_token_input,
            Self::sync_secret_placeholder(
                config.has_github_token,
                "settings.sync.placeholders.github_token",
            ),
            window,
            cx,
        );
        set_input_placeholder(
            &self.forms.sync_webdav_password_input,
            Self::sync_secret_placeholder(
                config.has_webdav_password,
                "settings.sync.placeholders.webdav_password",
            ),
            window,
            cx,
        );
    }

    fn sync_secret_placeholder(has_saved: bool, fallback_key: &'static str) -> String {
        if has_saved {
            i18n::string("placeholders.saved.keep_existing")
        } else {
            i18n::string(fallback_key)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SettingsController, normalize_github_gist_id};

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

    #[test]
    fn sync_passphrase_validation_requires_matching_values() {
        assert!(SettingsController::validate_sync_passphrase("", "secret").is_err());
        assert!(SettingsController::validate_sync_passphrase("secret", "").is_err());
        assert!(SettingsController::validate_sync_passphrase("secret", "other").is_err());
        assert!(SettingsController::validate_sync_passphrase("secret", "secret").is_ok());
    }
}
