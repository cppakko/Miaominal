use super::*;

struct WebSearchSaveTaskResult {
    config: WebSearchConfig,
}

impl AppView {
    pub(in crate::ui::shell) fn web_search_save_in_progress(&self) -> bool {
        self.web_search_save_in_progress
    }
    pub(in crate::ui::shell) fn open_web_search_config_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let config = self.settings_store.settings().web_search.clone();

        if self.local_vault_status == LocalVaultStatus::Locked && config.has_api_key {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::OpenWebSearchConfig,
                window,
                cx,
            );
            return;
        }

        self.prepare_web_search_config_popup_inputs(&config, window, cx);
        let popup = PendingWebSearchConfigPopupState;
        let stable_key = DialogOverlaySnapshot::WebSearchConfigPopup(popup).stable_key();
        self.dialogs
            .exiting_dialogs
            .retain(|dialog| dialog.snapshot.stable_key() != stable_key);
        self.web_search_config_popup = Some(popup);
        self.panel_forms
            .settings
            .web_search_api_key_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
    fn prepare_web_search_config_popup_inputs(
        &mut self,
        config: &WebSearchConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.panel_forms
            .settings
            .web_search_kind_select
            .update(cx, |select, cx| {
                select.set_selected_value(&config.kind, window, cx);
            });

        let api_key = if config.has_api_key && self.local_vault_status != LocalVaultStatus::Locked {
            match self
                .services
                .secrets
                .get("web_search", SecretKind::WebSearchApiKey)
            {
                Ok(api_key) => api_key.unwrap_or_default(),
                Err(error) => {
                    self.notify_secret_reveal_failed(window, &error, cx);
                    String::new()
                }
            }
        } else {
            String::new()
        };

        set_input_value(
            &self.panel_forms.settings.web_search_api_key_input,
            api_key,
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.web_search_endpoint_input,
            config.endpoint.clone(),
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.web_search_max_results_input,
            config.max_results.to_string(),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.settings.web_search_endpoint_input,
            web_search_endpoint_placeholder(config.kind),
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
    }
    pub(super) fn dismiss_web_search_config_popup(&mut self, cx: &mut Context<Self>) {
        if let Some(popup) = self.web_search_config_popup.take() {
            self.start_dialog_exit(DialogOverlaySnapshot::WebSearchConfigPopup(popup), cx);
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn close_web_search_config_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.web_search_save_in_progress() {
            return;
        }

        set_input_value(
            &self.panel_forms.settings.web_search_api_key_input,
            "",
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
        self.dismiss_web_search_config_popup(cx);
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
    pub(super) fn continue_save_web_search_after_unlock(
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
        self.dismiss_web_search_config_popup(cx);
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
    pub(super) fn spawn_web_search_save_operation(
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
}
