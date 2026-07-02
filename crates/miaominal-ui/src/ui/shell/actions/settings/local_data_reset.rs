use super::*;

impl AppView {
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
    pub(super) fn open_local_data_reset_confirmation_popup(
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
    pub(super) fn dismiss_local_data_reset_confirmation_popup(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn clear_local_data_reset_confirmation_input(
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
    pub(super) fn focus_local_data_reset_confirmation_input(
        &self,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.panel_forms
            .settings
            .local_data_reset_confirmation_input
            .update(cx, |input, cx| {
                input.focus(window, cx);
            });
    }
    pub(super) fn spawn_local_data_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn finish_local_data_reset(
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
    pub(super) fn finish_local_data_reset_without_window(
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
    pub(super) fn rebuild_after_local_data_reset(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let runtime = self.services.runtime.clone();
        *self = AppView::new(runtime, window, cx);
    }
    pub(super) fn notify_local_data_reset_failed(
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
}
