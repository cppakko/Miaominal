use super::sync::LocalVaultSyncSecretInputs;
use super::*;

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

impl AppView {
    pub(super) fn local_vault_operation_in_progress(&self) -> bool {
        self.local_vault_unlock_in_progress || self.local_vault_disable_in_progress
    }
    pub(super) fn run_local_vault_unlock_follow_up(
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
            PendingLocalVaultUnlockAction::SaveSyncPassphrase(passphrase) => {
                self.continue_save_sync_passphrase_after_unlock(passphrase, window, cx);
            }
            PendingLocalVaultUnlockAction::OpenSyncProviderConfig(provider) => {
                self.open_sync_provider_config_popup(provider, window, cx);
            }
            PendingLocalVaultUnlockAction::SaveSyncProviderConfig(draft) => {
                self.continue_save_sync_provider_config_after_unlock(draft, window, cx);
            }
            PendingLocalVaultUnlockAction::OpenAiProvider(provider_id) => {
                self.edit_ai_provider(provider_id, window, cx);
            }
            PendingLocalVaultUnlockAction::SaveAiProvider(draft) => {
                self.continue_save_ai_provider_after_unlock(draft, window, cx);
            }
            PendingLocalVaultUnlockAction::OpenWebSearchConfig => {
                self.open_web_search_config_popup(window, cx);
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
    pub(super) fn schedule_local_vault_unlock_follow_up(
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
    pub(super) fn dismiss_local_vault_passphrase_popup(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn spawn_local_vault_unlock(&mut self, passphrase: String, cx: &mut Context<Self>) {
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
    pub(super) fn spawn_local_vault_disable(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn spawn_local_vault_enable(&mut self, passphrase: String, cx: &mut Context<Self>) {
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
    pub(super) fn lock_local_vault(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
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
    pub(super) fn spawn_local_vault_change_passphrase(
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
    pub(super) fn finish_local_vault_disable(
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
    pub(super) fn finish_local_vault_disable_without_window(
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
    pub(super) fn disable_local_vault(
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
    pub(super) fn local_vault_secret_ids(&self) -> (Vec<String>, Vec<String>, Vec<String>) {
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
    pub(super) fn apply_local_vault_transition(
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
    pub(super) fn sync_local_vault_auto_lock_task(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn finish_local_vault_auto_lock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    pub(super) fn finish_local_vault_auto_lock_without_window(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn clear_local_vault_passphrase_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    pub(super) fn focus_local_vault_passphrase_input(&self, window: &mut Window, cx: &mut App) {
        self.panel_forms
            .settings
            .local_vault_passphrase_input
            .update(cx, |input, cx| {
                input.focus(window, cx);
            });
    }
    pub(super) fn notify_local_vault_error(
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
}
