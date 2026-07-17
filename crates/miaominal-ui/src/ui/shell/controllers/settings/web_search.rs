use super::*;
use crate::ui::shell::support::set_input_masked;
use crate::ui::shell::{
    DeferredAppCommand, DialogOverlaySnapshot, SettingsDeferredCommand, ValidationFailure,
    error_notification, success_notification, validation_notification,
};
use gpui::App;
use gpui_component::WindowExt as _;
use miaominal_secrets::SecretKind;
use miaominal_settings::{WebSearchConfig, WebSearchProviderKind};

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct WebSearchSaveDraft {
    pub(in crate::ui::shell) config: WebSearchConfig,
    pub(in crate::ui::shell) api_key: String,
}

struct WebSearchSaveTaskResult {
    config: WebSearchConfig,
}

impl SettingsController {
    pub(in crate::ui::shell) fn set_web_search_enabled(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if self
            .settings_store
            .update(|settings| settings.web_search.enabled = enabled)
        {
            let message = if enabled {
                i18n::string("settings.web_search.status.enabled")
            } else {
                i18n::string("settings.web_search.status.disabled")
            };
            cx.emit(AppCommand::Feedback(message));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn open_web_search_config_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PendingWebSearchConfigPopupState> {
        let config = self.settings_store.settings().web_search.clone();

        if self.local_vault_status == LocalVaultStatus::Locked && config.has_api_key {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                SettingsDeferredCommand::OpenWebSearchConfig,
            )));
            return None;
        }

        self.prepare_web_search_config_popup_inputs(&config, window, cx);
        let popup = PendingWebSearchConfigPopupState;
        self.web_search_config_popup = Some(popup);
        self.forms
            .web_search_api_key_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
        Some(popup)
    }

    fn prepare_web_search_config_popup_inputs(
        &mut self,
        config: &WebSearchConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.forms.web_search_kind_select.update(cx, |select, cx| {
            select.set_selected_value(&config.kind, window, cx);
        });

        let api_key = if config.has_api_key && self.local_vault_status != LocalVaultStatus::Locked {
            match self.secrets.get("web_search", SecretKind::WebSearchApiKey) {
                Ok(api_key) => api_key.unwrap_or_default(),
                Err(error) => {
                    self.notify_web_search_secret_load_failed(window, &error, cx);
                    String::new()
                }
            }
        } else {
            String::new()
        };

        set_input_value(&self.forms.web_search_api_key_input, api_key, window, cx);
        set_input_value(
            &self.forms.web_search_endpoint_input,
            config.endpoint.clone(),
            window,
            cx,
        );
        set_input_value(
            &self.forms.web_search_max_results_input,
            config.max_results.to_string(),
            window,
            cx,
        );
        set_input_placeholder(
            &self.forms.web_search_endpoint_input,
            web_search_endpoint_placeholder(config.kind),
            window,
            cx,
        );
        set_input_placeholder(
            &self.forms.web_search_api_key_input,
            Self::web_search_api_key_placeholder(config.has_api_key),
            window,
            cx,
        );
        self.set_web_search_api_key_visibility(false, false, window, cx);
    }

    pub(in crate::ui::shell) fn close_web_search_config_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PendingWebSearchConfigPopupState> {
        if self.web_search_save_in_progress {
            return None;
        }

        set_input_value(&self.forms.web_search_api_key_input, "", window, cx);
        self.set_web_search_api_key_visibility(false, false, window, cx);
        let popup = self.web_search_config_popup.take();
        if let Some(popup) = popup {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::WebSearchConfigPopup(popup),
            ));
            cx.notify();
        }
        popup
    }

    pub(in crate::ui::shell) fn submit_web_search_settings_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.web_search_save_in_progress {
            return;
        }

        let draft = match self.web_search_save_draft(cx) {
            Ok(draft) => draft,
            Err(failure) => {
                self.notify_web_search_validation_failure(failure, window, cx);
                return;
            }
        };

        if self.local_vault_status == LocalVaultStatus::Locked && !draft.api_key.is_empty() {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                SettingsDeferredCommand::SaveWebSearch(draft),
            )));
            return;
        }

        self.continue_save_web_search_after_unlock(draft, cx);
    }

    pub(in crate::ui::shell) fn continue_save_web_search_after_unlock(
        &mut self,
        draft: WebSearchSaveDraft,
        cx: &mut Context<Self>,
    ) {
        if self.web_search_save_in_progress {
            return;
        }

        if self.local_vault_status == LocalVaultStatus::Locked {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                SettingsDeferredCommand::SaveWebSearch(draft),
            )));
            return;
        }

        self.spawn_web_search_save_operation(draft, cx);
    }

    fn web_search_save_draft(&self, cx: &App) -> Result<WebSearchSaveDraft, ValidationFailure> {
        let kind = self
            .forms
            .web_search_kind_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(WebSearchProviderKind::Tavily);
        let api_key = self
            .forms
            .web_search_api_key_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let endpoint = self
            .forms
            .web_search_endpoint_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let max_results = self
            .forms
            .web_search_max_results_input
            .read(cx)
            .value()
            .trim()
            .to_string();

        Self::validate_web_search_save_draft(
            self.settings_store.settings().web_search.clone(),
            kind,
            api_key,
            endpoint,
            &max_results,
        )
    }

    fn validate_web_search_save_draft(
        mut config: WebSearchConfig,
        kind: WebSearchProviderKind,
        api_key: String,
        endpoint: String,
        max_results_text: &str,
    ) -> Result<WebSearchSaveDraft, ValidationFailure> {
        let max_results = max_results_text.parse::<u32>().map_err(|_| {
            ValidationFailure::invalid(Self::web_search_max_results_range_message())
        })?;
        if !(miaominal_settings::WEB_SEARCH_MAX_RESULTS_MIN
            ..=miaominal_settings::WEB_SEARCH_MAX_RESULTS_MAX)
            .contains(&max_results)
        {
            return Err(ValidationFailure::invalid(
                Self::web_search_max_results_range_message(),
            ));
        }
        if kind == WebSearchProviderKind::SearXng && endpoint.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "settings.web_search.validation.endpoint_required",
            )));
        }

        config.kind = kind;
        config.endpoint = endpoint;
        config.max_results = max_results;
        config.api_key_env.clear();
        config.has_api_key = config.has_api_key || !api_key.is_empty();
        config.sanitize();

        if config.kind.requires_api_key() && !config.has_api_key {
            return Err(ValidationFailure::required(i18n::string(
                "settings.web_search.validation.api_key_required",
            )));
        }
        config.enabled = true;

        Ok(WebSearchSaveDraft { config, api_key })
    }

    fn web_search_max_results_range_message() -> String {
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
        )
    }

    fn web_search_api_key_placeholder(has_api_key: bool) -> String {
        if has_api_key {
            i18n::string("placeholders.saved.keep_existing")
        } else {
            i18n::string("settings.web_search.placeholders.api_key")
        }
    }

    fn set_web_search_api_key_visibility(
        &mut self,
        visible: bool,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.secret_visibility
            .set_visible(SecretRevealTarget::WebSearchApiKey, visible);
        set_input_masked(
            &self.forms.web_search_api_key_input,
            !visible,
            focus,
            window,
            cx,
        );
    }

    fn notify_web_search_validation_failure(
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

    fn notify_web_search_secret_load_failed(
        &mut self,
        window: &mut Window,
        error: &anyhow::Error,
        cx: &mut Context<Self>,
    ) {
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
    }

    fn notify_web_search_save_failed(
        &mut self,
        window: &mut Window,
        error: &anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        let message = i18n::string_args(
            "settings.sync.save_feedback.failed_message",
            &[
                ("field", &i18n::string("settings.web_search.api_key.label")),
                ("error", &error.to_string()),
            ],
        );
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

    fn apply_web_search_save_result(
        &mut self,
        task_result: WebSearchSaveTaskResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let config = task_result.config;
        self.settings_store
            .update(|settings| settings.web_search = config.clone());
        set_input_value(&self.forms.web_search_api_key_input, "", window, cx);
        set_input_placeholder(
            &self.forms.web_search_api_key_input,
            Self::web_search_api_key_placeholder(config.has_api_key),
            window,
            cx,
        );
        self.set_web_search_api_key_visibility(false, false, window, cx);
        if let Some(popup) = self.web_search_config_popup.take() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::WebSearchConfigPopup(popup),
            ));
        }

        let message = i18n::string("settings.web_search.notifications.saved_message");
        window.push_notification(
            success_notification(
                i18n::string("settings.web_search.notifications.saved_title"),
                message.clone(),
            ),
            cx,
        );
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    fn finish_web_search_save_operation(
        &mut self,
        result: anyhow::Result<WebSearchSaveTaskResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.web_search_save_task = None;
        self.web_search_save_in_progress = false;

        match result {
            Ok(task_result) => self.apply_web_search_save_result(task_result, window, cx),
            Err(error) => self.notify_web_search_save_failed(window, &error, cx),
        }

        cx.notify();
    }

    fn finish_web_search_save_operation_without_window(
        &mut self,
        result: anyhow::Result<WebSearchSaveTaskResult>,
        cx: &mut Context<Self>,
    ) {
        self.web_search_save_task = None;
        self.web_search_save_in_progress = false;

        let message = match result {
            Ok(task_result) => {
                self.settings_store
                    .update(|settings| settings.web_search = task_result.config);
                self.secret_visibility
                    .set_visible(SecretRevealTarget::WebSearchApiKey, false);
                i18n::string("settings.web_search.notifications.saved_message")
            }
            Err(error) => i18n::string_args(
                "settings.sync.save_feedback.failed_message",
                &[
                    ("field", &i18n::string("settings.web_search.api_key.label")),
                    ("error", &error.to_string()),
                ],
            ),
        };
        cx.emit(AppCommand::Feedback(message));
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

        let secrets = self.secrets.clone();
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
            self.finish_web_search_save_operation_without_window(
                Err(anyhow::anyhow!(error).context("failed to spawn web search save worker")),
                cx,
            );
            return;
        }

        self.web_search_save_task = Some(cx.spawn(async move |this, cx| {
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
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_search_save_accepts_existing_api_key() {
        let config = WebSearchConfig {
            has_api_key: true,
            ..WebSearchConfig::default()
        };

        let draft = SettingsController::validate_web_search_save_draft(
            config,
            WebSearchProviderKind::Tavily,
            String::new(),
            String::new(),
            "5",
        )
        .expect("stored API key should satisfy provider requirement");

        assert!(draft.config.enabled);
        assert!(draft.config.has_api_key);
        assert!(draft.api_key.is_empty());
    }

    #[test]
    fn web_search_save_requires_searxng_endpoint() {
        let error = SettingsController::validate_web_search_save_draft(
            WebSearchConfig::default(),
            WebSearchProviderKind::SearXng,
            String::new(),
            String::new(),
            "5",
        )
        .expect_err("SearXNG should require an endpoint");

        assert_eq!(
            error.kind,
            crate::ui::shell::ValidationNotificationKind::RequiredInputMissing
        );
    }

    #[test]
    fn web_search_save_rejects_out_of_range_max_results() {
        let error = SettingsController::validate_web_search_save_draft(
            WebSearchConfig::default(),
            WebSearchProviderKind::Tavily,
            "secret".to_string(),
            String::new(),
            &(miaominal_settings::WEB_SEARCH_MAX_RESULTS_MAX + 1).to_string(),
        )
        .expect_err("max results above the configured limit should fail");

        assert_eq!(
            error.kind,
            crate::ui::shell::ValidationNotificationKind::InvalidInput
        );
    }
}
