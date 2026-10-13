use super::*;
use gpui_kit::App;

const LOCAL_DATA_RESET_CONFIRMATION_TOKEN: &str = "RESET";

impl SettingsController {
    pub(in crate::ui) fn cancel_local_data_reset_confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(prompt) = self.local_data_reset_confirm.take() {
            cx.emit(AppCommand::OverlayDismissed(self.overlay_snapshot(
                SettingsOverlay::LocalDataResetConfirm(prompt),
                cx.entity(),
                cx,
            )));
            cx.notify();
        }
    }

    pub(in crate::ui) fn continue_local_data_reset_confirm(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.local_data_reset_confirm.take() else {
            return;
        };

        cx.emit(AppCommand::OverlayDismissed(self.overlay_snapshot(
            SettingsOverlay::LocalDataResetConfirm(prompt),
            cx.entity(),
            cx,
        )));
        self.overlay_revisions
            .begin("local-data-reset-confirmation");
        self.renew_popup_forms("local-data-reset-confirmation", window, cx);
        self.local_data_reset_confirmation_popup =
            Some(PendingLocalDataResetConfirmationPopupState);
        self.clear_local_data_reset_confirmation_input(window, cx);
        self.focus_local_data_reset_confirmation_input(window, cx);
        cx.notify();
    }

    fn dismiss_local_data_reset_confirmation_popup(&mut self, cx: &mut Context<Self>) {
        if let Some(popup) = self.local_data_reset_confirmation_popup.take() {
            cx.emit(AppCommand::OverlayDismissed(self.overlay_snapshot(
                SettingsOverlay::LocalDataResetConfirmationPopup(popup),
                cx.entity(),
                cx,
            )));
            cx.notify();
        }
    }

    pub(in crate::ui) fn close_local_data_reset_confirmation_popup(
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

    pub(in crate::ui) fn submit_local_data_reset_confirmation_popup_action(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.local_data_reset_in_progress {
            return;
        }

        let confirmation = self.local_data_reset_confirmation(cx);
        let validation = if confirmation.is_empty() {
            Some((
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.about.reset_local.validation.required"),
            ))
        } else if confirmation != LOCAL_DATA_RESET_CONFIRMATION_TOKEN {
            Some((
                ValidationNotificationKind::InvalidInput,
                i18n::string("settings.about.reset_local.validation.mismatch"),
            ))
        } else {
            None
        };

        if let Some((kind, message)) = validation {
            crate::ui::components::notifications::push_app_notification(
                window,
                validation_notification(kind, message.clone()),
                cx,
            );
            cx.emit(AppCommand::Feedback(message));
            cx.notify();
            return;
        }

        self.dismiss_local_data_reset_confirmation_popup(cx);
        cx.emit(AppCommand::LocalDataResetRequested);
    }

    pub(in crate::ui) fn local_data_reset_confirmation(&self, cx: &App) -> String {
        self.forms
            .local_data_reset_confirmation_input
            .read(cx)
            .value()
            .trim()
            .to_string()
    }

    pub(in crate::ui) fn clear_local_data_reset_confirmation_input(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        set_input_value(
            &self.forms.local_data_reset_confirmation_input,
            String::new(),
            window,
            cx,
        );
    }

    pub(in crate::ui) fn focus_local_data_reset_confirmation_input(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.forms
            .local_data_reset_confirmation_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub(in crate::ui) fn start_local_data_reset(
        &mut self,
        session_ids: Vec<String>,
        managed_key_ids: Vec<String>,
        ai_provider_ids: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.local_data_reset_in_progress {
            return;
        }

        self.local_data_reset_in_progress = true;
        cx.notify();

        let proxy_ids = self
            .proxies
            .iter()
            .map(|proxy| proxy.id.clone())
            .collect::<Vec<_>>();

        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name("local-data-reset".to_string())
            .spawn(move || {
                let result = SettingsService::reset_local_data(
                    &session_ids,
                    &managed_key_ids,
                    &ai_provider_ids,
                    &proxy_ids,
                );
                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            self.local_data_reset_in_progress = false;
            cx.emit(AppCommand::ChatHistoryReopenRequested);
            let error = anyhow::anyhow!(error).context("failed to spawn local data reset worker");
            log::warn!("local data reset could not start: {error:#}");
            let message = i18n::string("settings.about.reset_local.notifications.failed.message");
            let notification = error_notification(
                i18n::string("settings.about.reset_local.notifications.failed.title"),
                message.clone(),
            );
            crate::ui::components::notifications::push_app_notification(window, notification, cx);
            cx.emit(AppCommand::Feedback(message));
            cx.notify();
            return;
        }

        let notification_window = cx.active_window();
        self.local_data_reset_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("local data reset task cancelled")))
                })
                .await;

            if let Err(error) = this.update(cx, move |this, cx| {
                this.local_data_reset_task = None;
                this.local_data_reset_in_progress = false;

                match result {
                    Ok(()) => cx.emit(AppCommand::RebuildApplication),
                    Err(error) => {
                        log::warn!("local data reset failed: {error:#}");
                        cx.emit(AppCommand::ChatHistoryReopenRequested);
                        let message = i18n::string(
                            "settings.about.reset_local.notifications.failed.message",
                        );
                        let notification = error_notification(
                            i18n::string(
                                "settings.about.reset_local.notifications.failed.title",
                            ),
                            message.clone(),
                        );
                        if let Some(window_handle) = notification_window
                            && let Err(update_error) =
                                window_handle.update(cx, move |_, window, cx| {
                                    crate::ui::components::notifications::push_app_notification(window, notification, cx);
                                })
                        {
                            log::debug!(
                                "failed to access active window for local data reset failure: {update_error:?}"
                            );
                        }
                        cx.emit(AppCommand::Feedback(message));
                    }
                }
                cx.notify();
            }) {
                log::debug!("failed to publish local data reset result: {error:?}");
            }
        }));
    }
}
