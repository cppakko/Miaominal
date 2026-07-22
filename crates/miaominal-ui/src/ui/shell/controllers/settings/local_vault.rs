use super::*;
use crate::ui::shell::support::set_input_masked;
use crate::ui::shell::{
    DialogOverlaySnapshot, ValidationNotificationKind, error_notification, success_notification,
    validation_notification,
};
use gpui_component::WindowExt as _;
use miaominal_secrets::{MAX_VAULT_PASSPHRASE_BYTES, ProtectedPassphrase};
use zeroize::Zeroizing;

fn local_vault_passphrase_too_long(passphrase: &str) -> bool {
    passphrase.len() > MAX_VAULT_PASSPHRASE_BYTES
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) enum LocalVaultActionRequest {
    Enable {
        passphrase: ProtectedPassphrase,
    },
    Unlock {
        passphrase: ProtectedPassphrase,
    },
    Lock,
    Disable,
    ChangePassphrase {
        current_passphrase: ProtectedPassphrase,
        new_passphrase: ProtectedPassphrase,
    },
}

pub(in crate::ui::shell) struct LocalVaultUnlockResult {
    pub(in crate::ui::shell) transition: LocalVaultTransition,
    pub(in crate::ui::shell) sync_secret_inputs: LocalVaultSyncSecretInputs,
}

pub(in crate::ui::shell) struct LocalVaultEnableResult {
    pub(in crate::ui::shell) passphrase: ProtectedPassphrase,
    pub(in crate::ui::shell) vault_secrets: SecretStore,
    pub(in crate::ui::shell) vault_sync_engine: SyncEngine,
    pub(in crate::ui::shell) sync_secret_inputs: LocalVaultSyncSecretInputs,
    pub(in crate::ui::shell) session_ids: Vec<String>,
    pub(in crate::ui::shell) managed_key_ids: Vec<String>,
    pub(in crate::ui::shell) ai_provider_ids: Vec<String>,
}

pub(in crate::ui::shell) struct LocalVaultChangePassphraseResult {
    pub(in crate::ui::shell) outcome: LocalVaultPassphraseChangeOutcome,
    pub(in crate::ui::shell) sync_secret_inputs: LocalVaultSyncSecretInputs,
}

pub(in crate::ui::shell) enum LocalVaultOperationResult {
    Unlock(anyhow::Result<LocalVaultUnlockResult>),
    Enable(anyhow::Result<LocalVaultEnableResult>),
    Disable(anyhow::Result<LocalVaultTransition>),
    ChangePassphrase(anyhow::Result<LocalVaultChangePassphraseResult>),
    AutoLock,
}

#[derive(Clone, Copy)]
enum LocalVaultOperationKind {
    Unlock,
    Enable,
    Disable,
    ChangePassphrase,
}

impl LocalVaultOperationKind {
    fn cancelled_message(self) -> &'static str {
        match self {
            Self::Unlock => "local vault unlock task cancelled",
            Self::Enable => "local vault enable task cancelled",
            Self::Disable => "local vault disable task cancelled",
            Self::ChangePassphrase => "local vault change passphrase task cancelled",
        }
    }

    fn failure(self, error: anyhow::Error) -> LocalVaultOperationResult {
        match self {
            Self::Unlock => LocalVaultOperationResult::Unlock(Err(error)),
            Self::Enable => LocalVaultOperationResult::Enable(Err(error)),
            Self::Disable => LocalVaultOperationResult::Disable(Err(error)),
            Self::ChangePassphrase => LocalVaultOperationResult::ChangePassphrase(Err(error)),
        }
    }
}

impl SettingsController {
    pub(in crate::ui::shell) fn local_vault_operation_in_progress(&self) -> bool {
        self.local_vault_unlock_in_progress || self.local_vault_disable_in_progress
    }

    pub(in crate::ui::shell) fn local_vault_status_label(&self) -> String {
        i18n::string(match self.local_vault_status {
            LocalVaultStatus::Disabled => "settings.sync.vault.state.disabled",
            LocalVaultStatus::Locked => "settings.sync.vault.state.locked",
            LocalVaultStatus::Unlocked => "settings.sync.vault.state.unlocked",
        })
    }

    pub(in crate::ui::shell) fn local_vault_requires_passphrase(&self) -> bool {
        self.local_vault_status != LocalVaultStatus::Unlocked
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
    ) -> bool {
        if self.local_vault_operation_in_progress() {
            return false;
        }

        self.local_vault_passphrase_popup = Some(mode);
        self.local_vault_unlock_in_progress = false;
        self.clear_local_vault_passphrase_input(window, cx);
        self.focus_local_vault_passphrase_input(mode, window, cx);
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn open_local_vault_passphrase_popup_in_active_window(
        &mut self,
        mode: LocalVaultPassphrasePopupMode,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.local_vault_operation_in_progress() {
            return false;
        }

        self.local_vault_passphrase_popup = Some(mode);
        self.local_vault_unlock_in_progress = false;

        if let Some(window_handle) = cx.active_window() {
            let current_input = self.forms.local_vault_current_passphrase_input.clone();
            let input = self.forms.local_vault_passphrase_input.clone();
            let confirmation_input = self.forms.local_vault_passphrase_confirmation_input.clone();
            if let Err(error) = window_handle.update(cx, move |_, window, cx| {
                current_input.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                    input.set_masked(true, window, cx);
                    if mode == LocalVaultPassphrasePopupMode::ChangePassphrase {
                        input.focus(window, cx);
                    }
                });
                input.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                    input.set_masked(true, window, cx);
                    if mode != LocalVaultPassphrasePopupMode::ChangePassphrase {
                        input.focus(window, cx);
                    }
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
            .set_visible(SecretRevealTarget::LocalVaultCurrentPassphrase, false);
        self.secret_visibility
            .set_visible(SecretRevealTarget::LocalVaultPassphrase, false);
        self.secret_visibility
            .set_visible(SecretRevealTarget::LocalVaultPassphraseConfirmation, false);
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn close_local_vault_passphrase_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.local_vault_operation_in_progress() {
            return false;
        }

        self.local_vault_unlock_in_progress = false;
        self.clear_local_vault_passphrase_input(window, cx);
        self.dismiss_local_vault_passphrase_popup(cx);
        true
    }

    pub(in crate::ui::shell) fn open_local_vault_disable_confirm(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.local_vault_operation_in_progress() {
            return false;
        }
        if self.local_vault_status != LocalVaultStatus::Unlocked {
            self.notify_local_vault_validation_failure(
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.sync.vault.disable_requires_unlock_error.message"),
                window,
                cx,
            );
            return false;
        }

        self.local_vault_disable_confirm = Some(PendingLocalVaultDisableConfirmState);
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn cancel_local_vault_disable_confirm(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if let Some(prompt) = self.local_vault_disable_confirm.take() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::LocalVaultDisableConfirm(prompt),
            ));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn confirm_local_vault_disable(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(prompt) = self.local_vault_disable_confirm.take() else {
            return false;
        };
        cx.emit(AppCommand::OverlayDismissed(
            DialogOverlaySnapshot::LocalVaultDisableConfirm(prompt),
        ));
        cx.emit(AppCommand::VaultActionRequested(
            LocalVaultActionRequest::Disable,
        ));
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn submit_local_vault_passphrase_popup_action(
        &mut self,
        mode: LocalVaultPassphrasePopupMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<LocalVaultActionRequest> {
        let request = match mode {
            LocalVaultPassphrasePopupMode::PrimaryAction => {
                self.submit_local_vault_primary_action(window, cx)
            }
            LocalVaultPassphrasePopupMode::ChangePassphrase => {
                self.submit_local_vault_change_passphrase_action(window, cx)
            }
        };
        if let Some(request) = request.as_ref() {
            cx.emit(AppCommand::VaultActionRequested(request.clone()));
        }
        request
    }

    fn submit_local_vault_primary_action(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<LocalVaultActionRequest> {
        if self.local_vault_operation_in_progress() {
            return None;
        }

        let passphrase = Zeroizing::new(
            self.forms
                .local_vault_passphrase_input
                .read(cx)
                .value()
                .trim()
                .to_string(),
        );

        match self.local_vault_status {
            LocalVaultStatus::Disabled => {
                let passphrase_confirmation = Zeroizing::new(
                    self.forms
                        .local_vault_passphrase_confirmation_input
                        .read(cx)
                        .value()
                        .trim()
                        .to_string(),
                );

                if passphrase.is_empty() {
                    self.notify_local_vault_validation_failure(
                        ValidationNotificationKind::RequiredInputMissing,
                        i18n::string("settings.sync.vault.passphrase_required_error.message"),
                        window,
                        cx,
                    );
                    return None;
                }
                if passphrase_confirmation.is_empty() {
                    self.notify_local_vault_validation_failure(
                        ValidationNotificationKind::RequiredInputMissing,
                        i18n::string(
                            "settings.sync.vault.passphrase_confirmation_required_error.message",
                        ),
                        window,
                        cx,
                    );
                    return None;
                }
                if passphrase != passphrase_confirmation {
                    self.notify_local_vault_validation_failure(
                        ValidationNotificationKind::InvalidInput,
                        i18n::string("settings.sync.vault.passphrase_mismatch_error.message"),
                        window,
                        cx,
                    );
                    return None;
                }

                let passphrase =
                    self.protect_local_vault_passphrase(passphrase.as_str(), window, cx)?;
                self.clear_local_vault_passphrase_input(window, cx);
                Some(LocalVaultActionRequest::Enable { passphrase })
            }
            LocalVaultStatus::Locked => {
                if passphrase.is_empty() {
                    self.notify_local_vault_validation_failure(
                        ValidationNotificationKind::RequiredInputMissing,
                        i18n::string("settings.sync.vault.passphrase_required_error.message"),
                        window,
                        cx,
                    );
                    return None;
                }
                let passphrase =
                    self.protect_local_vault_passphrase(passphrase.as_str(), window, cx)?;
                self.clear_local_vault_passphrase_input(window, cx);
                Some(LocalVaultActionRequest::Unlock { passphrase })
            }
            LocalVaultStatus::Unlocked => Some(LocalVaultActionRequest::Lock),
        }
    }

    fn submit_local_vault_change_passphrase_action(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<LocalVaultActionRequest> {
        if self.local_vault_operation_in_progress() {
            return None;
        }

        let current_passphrase = Zeroizing::new(
            self.forms
                .local_vault_current_passphrase_input
                .read(cx)
                .value()
                .trim()
                .to_string(),
        );
        let passphrase = Zeroizing::new(
            self.forms
                .local_vault_passphrase_input
                .read(cx)
                .value()
                .trim()
                .to_string(),
        );
        let passphrase_confirmation = Zeroizing::new(
            self.forms
                .local_vault_passphrase_confirmation_input
                .read(cx)
                .value()
                .trim()
                .to_string(),
        );

        if !self.local_vault_can_change_passphrase() {
            self.notify_local_vault_validation_failure(
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.sync.vault.change_requires_unlock_error.message"),
                window,
                cx,
            );
            return None;
        }
        if current_passphrase.is_empty() {
            self.notify_local_vault_validation_failure(
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.sync.vault.current_passphrase_required_error.message"),
                window,
                cx,
            );
            return None;
        }
        if passphrase.is_empty() {
            self.notify_local_vault_validation_failure(
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.sync.vault.new_passphrase_required_error.message"),
                window,
                cx,
            );
            return None;
        }
        if passphrase_confirmation.is_empty() {
            self.notify_local_vault_validation_failure(
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string(
                    "settings.sync.vault.new_passphrase_confirmation_required_error.message",
                ),
                window,
                cx,
            );
            return None;
        }
        if passphrase != passphrase_confirmation {
            self.notify_local_vault_validation_failure(
                ValidationNotificationKind::InvalidInput,
                i18n::string("settings.sync.vault.passphrase_mismatch_error.message"),
                window,
                cx,
            );
            return None;
        }

        let current_passphrase =
            self.protect_local_vault_passphrase(current_passphrase.as_str(), window, cx)?;
        let new_passphrase =
            self.protect_local_vault_passphrase(passphrase.as_str(), window, cx)?;
        Some(LocalVaultActionRequest::ChangePassphrase {
            current_passphrase,
            new_passphrase,
        })
    }

    fn protect_local_vault_passphrase(
        &mut self,
        passphrase: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<ProtectedPassphrase> {
        if local_vault_passphrase_too_long(passphrase) {
            self.notify_local_vault_validation_failure(
                ValidationNotificationKind::InvalidInput,
                i18n::string("settings.sync.vault.passphrase_too_long_error.message"),
                window,
                cx,
            );
            return None;
        }

        match ProtectedPassphrase::try_from_string(passphrase.to_string()) {
            Ok(passphrase) => Some(passphrase),
            Err(error) => {
                log::warn!("failed to protect local vault passphrase: {error:#}");
                self.notify_local_vault_validation_failure(
                    ValidationNotificationKind::InvalidInput,
                    i18n::string("settings.sync.vault.secure_memory_unavailable_error.message"),
                    window,
                    cx,
                );
                None
            }
        }
    }

    pub(in crate::ui::shell) fn clear_local_vault_passphrase_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        set_input_value(
            &self.forms.local_vault_current_passphrase_input,
            String::new(),
            window,
            cx,
        );
        set_input_value(
            &self.forms.local_vault_passphrase_input,
            String::new(),
            window,
            cx,
        );
        set_input_value(
            &self.forms.local_vault_passphrase_confirmation_input,
            String::new(),
            window,
            cx,
        );
        set_input_masked(
            &self.forms.local_vault_current_passphrase_input,
            true,
            false,
            window,
            cx,
        );
        set_input_masked(
            &self.forms.local_vault_passphrase_input,
            true,
            false,
            window,
            cx,
        );
        set_input_masked(
            &self.forms.local_vault_passphrase_confirmation_input,
            true,
            false,
            window,
            cx,
        );
        self.secret_visibility
            .set_visible(SecretRevealTarget::LocalVaultCurrentPassphrase, false);
        self.secret_visibility
            .set_visible(SecretRevealTarget::LocalVaultPassphrase, false);
        self.secret_visibility
            .set_visible(SecretRevealTarget::LocalVaultPassphraseConfirmation, false);
    }

    fn focus_local_vault_passphrase_input(
        &self,
        mode: LocalVaultPassphrasePopupMode,
        window: &mut Window,
        cx: &mut gpui::App,
    ) {
        let input = if mode == LocalVaultPassphrasePopupMode::ChangePassphrase {
            &self.forms.local_vault_current_passphrase_input
        } else {
            &self.forms.local_vault_passphrase_input
        };
        input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub(in crate::ui::shell) fn dismiss_local_vault_passphrase_popup(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if let Some(mode) = self.local_vault_passphrase_popup.take() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::LocalVaultPassphrasePopup(mode),
            ));
            cx.notify();
        }
    }

    fn notify_local_vault_validation_failure(
        &mut self,
        kind: ValidationNotificationKind,
        message: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.push_notification(validation_notification(kind, message.clone()), cx);
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    pub(in crate::ui::shell) fn finish_local_vault_unlock(
        &mut self,
        sync_secret_inputs: LocalVaultSyncSecretInputs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        self.local_vault_unlock_in_progress = false;
        self.set_sync_passphrase_configured_from_inputs(&sync_secret_inputs.sync_passphrase, cx);
        self.apply_sync_secret_inputs(sync_secret_inputs, window, cx);
        self.clear_local_vault_passphrase_input(window, cx);
        self.dismiss_local_vault_passphrase_popup(cx);
        self.notify_local_vault_success(
            "settings.sync.vault.notifications.unlocked_title",
            "settings.sync.vault.notifications.unlocked_message",
            window,
            cx,
        )
    }

    pub(in crate::ui::shell) fn finish_local_vault_unlock_without_window(
        &mut self,
        sync_secret_inputs: &LocalVaultSyncSecretInputs,
        cx: &mut Context<Self>,
    ) -> String {
        self.local_vault_unlock_in_progress = false;
        self.set_sync_passphrase_configured_from_inputs(&sync_secret_inputs.sync_passphrase, cx);
        self.dismiss_local_vault_passphrase_popup(cx);
        i18n::string("settings.sync.vault.notifications.unlocked_message")
    }

    pub(in crate::ui::shell) fn finish_local_vault_enable(
        &mut self,
        sync_secret_inputs: LocalVaultSyncSecretInputs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        self.local_vault_unlock_in_progress = false;
        self.set_sync_passphrase_configured_from_inputs(&sync_secret_inputs.sync_passphrase, cx);
        self.apply_sync_secret_inputs(sync_secret_inputs, window, cx);
        self.clear_local_vault_passphrase_input(window, cx);
        self.dismiss_local_vault_passphrase_popup(cx);
        self.notify_local_vault_success(
            "settings.sync.vault.notifications.enabled_title",
            "settings.sync.vault.notifications.enabled_message",
            window,
            cx,
        )
    }

    pub(in crate::ui::shell) fn finish_local_vault_enable_without_window(
        &mut self,
        sync_secret_inputs: &LocalVaultSyncSecretInputs,
        cx: &mut Context<Self>,
    ) -> String {
        self.local_vault_unlock_in_progress = false;
        self.set_sync_passphrase_configured_from_inputs(&sync_secret_inputs.sync_passphrase, cx);
        self.dismiss_local_vault_passphrase_popup(cx);
        i18n::string("settings.sync.vault.notifications.enabled_message")
    }

    pub(in crate::ui::shell) fn finish_local_vault_change_passphrase(
        &mut self,
        sync_secret_inputs: LocalVaultSyncSecretInputs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        self.local_vault_unlock_in_progress = false;
        self.set_sync_passphrase_configured_from_inputs(&sync_secret_inputs.sync_passphrase, cx);
        self.apply_sync_secret_inputs(sync_secret_inputs, window, cx);
        self.clear_local_vault_passphrase_input(window, cx);
        self.dismiss_local_vault_passphrase_popup(cx);
        self.notify_local_vault_success(
            "settings.sync.vault.notifications.passphrase_changed_title",
            "settings.sync.vault.notifications.passphrase_changed_message",
            window,
            cx,
        )
    }

    pub(in crate::ui::shell) fn finish_local_vault_change_passphrase_without_window(
        &mut self,
        sync_secret_inputs: &LocalVaultSyncSecretInputs,
        cx: &mut Context<Self>,
    ) -> String {
        self.local_vault_unlock_in_progress = false;
        self.set_sync_passphrase_configured_from_inputs(&sync_secret_inputs.sync_passphrase, cx);
        self.dismiss_local_vault_passphrase_popup(cx);
        i18n::string("settings.sync.vault.notifications.passphrase_changed_message")
    }

    pub(in crate::ui::shell) fn finish_local_vault_change_passphrase_locked(
        &mut self,
        sync_secret_inputs: LocalVaultSyncSecretInputs,
        action: &str,
        error: &anyhow::Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        self.local_vault_unlock_in_progress = false;
        self.apply_sync_secret_inputs(sync_secret_inputs, window, cx);
        self.clear_local_vault_passphrase_input(window, cx);
        self.notify_local_vault_error(action, error, window, cx)
    }

    pub(in crate::ui::shell) fn finish_local_vault_disable(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        self.local_vault_disable_in_progress = false;
        self.refresh_sync_secret_inputs(window, cx);
        self.clear_local_vault_passphrase_input(window, cx);
        self.notify_local_vault_success(
            "settings.sync.vault.notifications.disabled_title",
            "settings.sync.vault.notifications.disabled_message",
            window,
            cx,
        )
    }

    pub(in crate::ui::shell) fn finish_local_vault_disable_without_window(&mut self) -> String {
        self.local_vault_disable_in_progress = false;
        i18n::string("settings.sync.vault.notifications.disabled_message")
    }

    pub(in crate::ui::shell) fn finish_local_vault_lock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        self.refresh_sync_secret_inputs(window, cx);
        self.hide_storage_backed_secret_visibility(window, cx);
        self.clear_local_vault_passphrase_input(window, cx);
        self.notify_local_vault_success(
            "settings.sync.vault.notifications.locked_title",
            "settings.sync.vault.notifications.locked_message",
            window,
            cx,
        )
    }

    pub(in crate::ui::shell) fn finish_local_vault_lock_without_window(&mut self) -> String {
        self.secret_visibility
            .set_visible(SecretRevealTarget::SyncGithubToken, false);
        self.secret_visibility
            .set_visible(SecretRevealTarget::SyncWebdavPassword, false);
        i18n::string("settings.sync.vault.notifications.locked_message")
    }

    pub(in crate::ui::shell) fn finish_local_vault_error(
        &mut self,
        action: &str,
        error: &anyhow::Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        self.local_vault_unlock_in_progress = false;
        self.notify_local_vault_error(action, error, window, cx)
    }

    pub(in crate::ui::shell) fn finish_local_vault_change_passphrase_error(
        &mut self,
        action: &str,
        error: &anyhow::Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        self.local_vault_unlock_in_progress = false;
        let message = Self::local_vault_change_passphrase_error_message(action, error);
        window.push_notification(
            error_notification(
                i18n::string("settings.sync.vault.notifications.failed_title"),
                message.clone(),
            ),
            cx,
        );
        cx.notify();
        message
    }

    pub(in crate::ui::shell) fn finish_local_vault_disable_error(
        &mut self,
        action: &str,
        error: &anyhow::Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        self.local_vault_disable_in_progress = false;
        self.notify_local_vault_error(action, error, window, cx)
    }

    pub(in crate::ui::shell) fn finish_local_vault_error_without_window(
        &mut self,
        action: &str,
        error: &anyhow::Error,
    ) -> String {
        self.local_vault_unlock_in_progress = false;
        Self::local_vault_error_message(action, error)
    }

    pub(in crate::ui::shell) fn finish_local_vault_change_passphrase_error_without_window(
        &mut self,
        action: &str,
        error: &anyhow::Error,
    ) -> String {
        self.local_vault_unlock_in_progress = false;
        Self::local_vault_change_passphrase_error_message(action, error)
    }

    pub(in crate::ui::shell) fn finish_local_vault_disable_error_without_window(
        &mut self,
        action: &str,
        error: &anyhow::Error,
    ) -> String {
        self.local_vault_disable_in_progress = false;
        Self::local_vault_error_message(action, error)
    }

    pub(in crate::ui::shell) fn local_vault_error_message(
        action: &str,
        error: &anyhow::Error,
    ) -> String {
        let error_message = if error.chain().any(|cause| {
            cause
                .to_string()
                .contains("local vault session has been revoked")
        }) {
            i18n::string("settings.sync.vault.session_revoked_error.message")
        } else if error.chain().any(|cause| {
            let message = cause.to_string();
            message.contains("protected memory")
                || message.contains("protected-memory")
                || message.contains("unseal local vault passphrase")
                || message.contains("unseal local vault cache key")
                || message.contains("unseal derived key")
        }) {
            i18n::string("settings.sync.vault.secure_memory_unavailable_error.message")
        } else {
            error.to_string()
        };
        i18n::string_args(
            "settings.sync.vault.notifications.failed_message",
            &[("action", action), ("error", &error_message)],
        )
    }

    fn local_vault_change_passphrase_error_message(action: &str, error: &anyhow::Error) -> String {
        if error
            .chain()
            .any(|cause| cause.to_string().contains("vault decryption failed"))
        {
            i18n::string("settings.sync.vault.current_passphrase_incorrect_error.message")
        } else {
            Self::local_vault_error_message(action, error)
        }
    }

    fn notify_local_vault_error(
        &mut self,
        action: &str,
        error: &anyhow::Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        let message = Self::local_vault_error_message(action, error);
        window.push_notification(
            error_notification(
                i18n::string("settings.sync.vault.notifications.failed_title"),
                message.clone(),
            ),
            cx,
        );
        cx.notify();
        message
    }

    fn notify_local_vault_success(
        &mut self,
        title_key: &str,
        message_key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        let message = i18n::string(message_key);
        window.push_notification(
            success_notification(i18n::string(title_key), message.clone()),
            cx,
        );
        cx.notify();
        message
    }

    fn hide_storage_backed_secret_visibility(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    }

    pub(in crate::ui::shell) fn start_vault_unlock(
        &mut self,
        passphrase: ProtectedPassphrase,
        cx: &mut Context<Self>,
    ) {
        self.local_vault_unlock_in_progress = true;
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
                tx.send(LocalVaultOperationResult::Unlock(result)).ok();
            });

        self.await_local_vault_operation(LocalVaultOperationKind::Unlock, rx, spawn_result, cx);
    }

    pub(in crate::ui::shell) fn start_vault_enable(
        &mut self,
        passphrase: ProtectedPassphrase,
        session_ids: Vec<String>,
        managed_key_ids: Vec<String>,
        ai_provider_ids: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        self.local_vault_unlock_in_progress = true;
        cx.notify();

        let previous_secrets = self.secrets.clone();
        let previous_sync_engine = self.sync.sync_engine.clone();
        let worker_session_ids = session_ids.clone();
        let worker_managed_key_ids = managed_key_ids.clone();
        let worker_ai_provider_ids = ai_provider_ids.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name("local-vault-enable".to_string())
            .spawn(move || {
                let result = SettingsService::prepare_vault_enable(
                    &passphrase,
                    worker_session_ids,
                    worker_managed_key_ids,
                    worker_ai_provider_ids,
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
                tx.send(LocalVaultOperationResult::Enable(result)).ok();
            });

        self.await_local_vault_operation(LocalVaultOperationKind::Enable, rx, spawn_result, cx);
    }

    pub(in crate::ui::shell) fn start_vault_disable(
        &mut self,
        session_ids: Vec<String>,
        managed_key_ids: Vec<String>,
        ai_provider_ids: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        self.local_vault_disable_in_progress = true;
        cx.notify();

        let previous_secrets = self.secrets.clone();
        let previous_sync_engine = self.sync.sync_engine.clone();
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
                tx.send(LocalVaultOperationResult::Disable(result)).ok();
            });

        self.await_local_vault_operation(LocalVaultOperationKind::Disable, rx, spawn_result, cx);
    }

    pub(in crate::ui::shell) fn start_vault_change_passphrase(
        &mut self,
        current_passphrase: ProtectedPassphrase,
        new_passphrase: ProtectedPassphrase,
        cx: &mut Context<Self>,
    ) {
        self.local_vault_unlock_in_progress = true;
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
                tx.send(LocalVaultOperationResult::ChangePassphrase(result))
                    .ok();
            });

        self.await_local_vault_operation(
            LocalVaultOperationKind::ChangePassphrase,
            rx,
            spawn_result,
            cx,
        );
    }

    fn await_local_vault_operation(
        &mut self,
        kind: LocalVaultOperationKind,
        rx: std::sync::mpsc::Receiver<LocalVaultOperationResult>,
        spawn_result: std::io::Result<std::thread::JoinHandle<()>>,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = spawn_result {
            let result =
                kind.failure(anyhow::anyhow!(error).context("failed to spawn local vault worker"));
            self.publish_local_vault_operation_after_yield(result, cx);
            return;
        }

        self.local_vault_operation_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv()
                        .unwrap_or_else(|_| kind.failure(anyhow::anyhow!(kind.cancelled_message())))
                })
                .await;

            if let Err(error) = this.update(cx, |this, cx| {
                this.publish_local_vault_operation(result, cx);
            }) {
                log::debug!("failed to publish local vault operation result: {error:?}");
            }
        }));
    }

    fn publish_local_vault_operation_after_yield(
        &mut self,
        result: LocalVaultOperationResult,
        cx: &mut Context<Self>,
    ) {
        self.local_vault_operation_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(0))
                .await;
            if let Err(error) = this.update(cx, |this, cx| {
                this.publish_local_vault_operation(result, cx);
            }) {
                log::debug!("failed to publish local vault spawn error: {error:?}");
            }
        }));
    }

    fn publish_local_vault_operation(
        &mut self,
        result: LocalVaultOperationResult,
        cx: &mut Context<Self>,
    ) {
        self.local_vault_operation_task = None;
        self.local_vault_operation_results.push_back(result);
        cx.emit(AppCommand::CredentialsChanged);
        cx.notify();
    }

    pub(in crate::ui::shell) fn take_local_vault_operation_result(
        &mut self,
    ) -> Option<LocalVaultOperationResult> {
        self.local_vault_operation_results.pop_front()
    }

    pub(in crate::ui::shell) fn apply_vault_enable(
        &mut self,
        passphrase: ProtectedPassphrase,
        vault_secrets: SecretStore,
        vault_sync_engine: SyncEngine,
    ) -> anyhow::Result<LocalVaultTransition> {
        SettingsService::apply_vault_enable(
            passphrase,
            vault_secrets,
            vault_sync_engine,
            &mut self.settings_store,
        )
    }

    pub(in crate::ui::shell) fn apply_vault_disable(&mut self) -> anyhow::Result<()> {
        SettingsService::apply_vault_disable(&mut self.settings_store)
    }

    pub(in crate::ui::shell) fn delete_migrated_keyring_secrets(
        session_ids: &[String],
        managed_key_ids: &[String],
        ai_provider_ids: &[String],
        source_secrets: &SecretStore,
        source_sync_engine: &SyncEngine,
    ) {
        SettingsService::delete_migrated_keyring_secrets(
            session_ids,
            managed_key_ids,
            ai_provider_ids,
            source_secrets,
            source_sync_engine,
        );
    }

    pub(in crate::ui::shell) fn erase_vault_file() -> anyhow::Result<()> {
        SettingsService::erase_vault_file()
    }

    pub(in crate::ui::shell) fn local_vault_lock_transition(&self) -> LocalVaultTransition {
        SettingsService::local_vault_lock_transition(&self.settings_store)
    }

    pub(in crate::ui::shell) fn sync_local_vault_auto_lock_task(&mut self, cx: &mut Context<Self>) {
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

        self.local_vault_auto_lock_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(duration).await;
            if let Err(error) = this.update(cx, |this, cx| {
                this.local_vault_auto_lock_task = None;
                if this.local_vault_status != LocalVaultStatus::Unlocked
                    || this
                        .settings_store
                        .settings()
                        .local_vault_auto_lock_duration
                        .duration()
                        .is_none()
                {
                    return;
                }

                this.local_vault_operation_results
                    .push_back(LocalVaultOperationResult::AutoLock);
                cx.emit(AppCommand::CredentialsChanged);
                cx.notify();
            }) {
                log::debug!("failed to publish local vault auto-lock result: {error:?}");
            }
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::{SettingsController, local_vault_passphrase_too_long};
    use miaominal_secrets::MAX_VAULT_PASSPHRASE_BYTES;
    use miaominal_settings::AppLanguage;

    #[test]
    fn local_vault_passphrase_limit_uses_utf8_bytes() {
        assert!(!local_vault_passphrase_too_long(
            &"a".repeat(MAX_VAULT_PASSPHRASE_BYTES)
        ));
        assert!(local_vault_passphrase_too_long(
            &"a".repeat(MAX_VAULT_PASSPHRASE_BYTES + 1)
        ));
        assert!(local_vault_passphrase_too_long(
            &"猫".repeat(MAX_VAULT_PASSPHRASE_BYTES / "猫".len() + 1)
        ));
    }

    #[test]
    fn revoked_vault_session_uses_reunlock_guidance() {
        crate::ui::i18n::set_language(AppLanguage::English);
        let error = anyhow::anyhow!("local vault session has been revoked");

        let message = SettingsController::local_vault_error_message("Unlock", &error);

        assert!(message.contains("Unlock it again to continue."));
    }

    #[test]
    fn protected_memory_failures_use_safe_unlock_guidance() {
        crate::ui::i18n::set_language(AppLanguage::English);
        let error = anyhow::anyhow!("failed to unseal derived key: memory protection failed");

        let message = SettingsController::local_vault_error_message("Unlock", &error);

        assert!(message.contains("cannot be unlocked safely"));
    }

    #[test]
    fn cache_key_unseal_failures_use_safe_unlock_guidance() {
        crate::ui::i18n::set_language(AppLanguage::English);
        let error = anyhow::anyhow!("failed to unseal local vault cache key: protection failed");

        let message = SettingsController::local_vault_error_message("Unlock", &error);

        assert!(message.contains("cannot be unlocked safely"));
    }

    #[test]
    fn change_passphrase_decryption_failure_reports_incorrect_current_passphrase() {
        crate::ui::i18n::set_language(AppLanguage::English);
        let error = anyhow::anyhow!("vault decryption failed: authentication failure");

        let message = SettingsController::local_vault_change_passphrase_error_message(
            "Update secrets vault passphrase",
            &error,
        );

        assert_eq!(
            message,
            "The current local secrets vault passphrase is incorrect."
        );
    }
}
