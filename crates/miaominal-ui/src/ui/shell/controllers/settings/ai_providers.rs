use super::*;
use crate::ui::shell::support::set_input_masked;
use crate::ui::shell::{
    DeferredAppCommand, DialogOverlaySnapshot, SettingsDeferredCommand, ValidationFailure,
    ai_provider_select_options, error_notification, success_notification, validation_notification,
};
use gpui::App;
use gpui_component::WindowExt as _;
use miaominal_secrets::SecretKind;
use miaominal_settings::{AiProviderConfig, AiProviderKind};

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct AiProviderSaveDraft {
    pub(in crate::ui::shell) provider: AiProviderConfig,
    pub(in crate::ui::shell) api_key: String,
}

struct AiProviderSaveTaskResult {
    provider: AiProviderConfig,
}

struct AiProviderApiKeyLoadTaskResult {
    provider_id: String,
    api_key: anyhow::Result<Option<String>>,
}

fn parse_optional_ai_provider_temperature(value: &str) -> Result<Option<f64>, ValidationFailure> {
    let trimmed_value = value.trim();
    if trimmed_value.is_empty() {
        return Ok(None);
    }

    let range_error = || {
        ValidationFailure::invalid(i18n::string_args(
            "settings.ai_providers.validation.temperature_range",
            &[
                (
                    "min",
                    &miaominal_settings::AI_PROVIDER_TEMPERATURE_MIN.to_string(),
                ),
                (
                    "max",
                    &miaominal_settings::AI_PROVIDER_TEMPERATURE_MAX.to_string(),
                ),
            ],
        ))
    };
    let temperature = trimmed_value.parse::<f64>().map_err(|_| range_error())?;

    if !temperature.is_finite()
        || !(miaominal_settings::AI_PROVIDER_TEMPERATURE_MIN
            ..=miaominal_settings::AI_PROVIDER_TEMPERATURE_MAX)
            .contains(&temperature)
    {
        return Err(range_error());
    }

    Ok(Some(temperature))
}

fn parse_optional_ai_provider_positive_u64(
    value: &str,
    validation_key: &'static str,
) -> Result<Option<u64>, ValidationFailure> {
    let trimmed_value = value.trim();
    if trimmed_value.is_empty() {
        return Ok(None);
    }

    let range_error = || {
        ValidationFailure::invalid(i18n::string_args(
            validation_key,
            &[(
                "min",
                &miaominal_settings::AI_PROVIDER_POSITIVE_INTEGER_MIN.to_string(),
            )],
        ))
    };
    let number = trimmed_value.parse::<u64>().map_err(|_| range_error())?;

    if number < miaominal_settings::AI_PROVIDER_POSITIVE_INTEGER_MIN {
        return Err(range_error());
    }

    Ok(Some(number))
}

impl SettingsController {
    pub(in crate::ui::shell) fn ai_provider_ids(&self) -> Vec<String> {
        self.settings_store
            .settings()
            .ai_providers
            .iter()
            .map(|provider| provider.id.clone())
            .collect()
    }

    pub(in crate::ui::shell) fn selected_ai_provider_id(&self, cx: &App) -> Option<String> {
        self.forms
            .ai_provider_select
            .read(cx)
            .selected_value()
            .cloned()
    }

    pub(in crate::ui::shell) fn ai_provider_api_key_load_in_progress_for(
        &self,
        provider_id: &str,
    ) -> bool {
        self.ai_provider_api_key_load_in_progress.as_deref() == Some(provider_id)
    }

    pub(in crate::ui::shell) fn start_new_ai_provider(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PendingAiProviderPopupState {
        self.editing_ai_provider_id = None;
        let kind = AiProviderKind::OpenAi;
        self.forms.ai_provider_kind_select.update(cx, |select, cx| {
            select.set_selected_value(&kind, window, cx);
        });
        set_input_value(
            &self.forms.ai_provider_name_input,
            i18n::string(ai_provider_kind_label_key(kind)),
            window,
            cx,
        );
        set_input_value(
            &self.forms.ai_provider_model_input,
            kind.default_model(),
            window,
            cx,
        );
        set_input_value(&self.forms.ai_provider_base_url_input, "", window, cx);
        set_input_value(&self.forms.ai_provider_api_key_input, "", window, cx);
        set_input_value(&self.forms.ai_provider_temperature_input, "", window, cx);
        set_input_value(&self.forms.ai_provider_max_tokens_input, "", window, cx);
        set_input_value(&self.forms.ai_provider_context_window_input, "", window, cx);
        self.clear_ai_provider_api_key_visibility(window, cx);
        self.open_ai_provider_popup(window, cx)
    }

    pub(in crate::ui::shell) fn edit_ai_provider(
        &mut self,
        provider_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PendingAiProviderPopupState> {
        let provider = self
            .settings_store
            .settings()
            .ai_providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .cloned()?;

        if self.local_vault_status == LocalVaultStatus::Locked && provider.has_api_key {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                SettingsDeferredCommand::OpenAiProvider(provider.id),
            )));
            return None;
        }

        self.editing_ai_provider_id = Some(provider.id.clone());
        self.forms.ai_provider_kind_select.update(cx, |select, cx| {
            select.set_selected_value(&provider.kind, window, cx);
        });
        set_input_value(
            &self.forms.ai_provider_name_input,
            provider.name.clone(),
            window,
            cx,
        );
        set_input_value(
            &self.forms.ai_provider_model_input,
            provider.model.clone(),
            window,
            cx,
        );
        set_input_value(
            &self.forms.ai_provider_base_url_input,
            provider.base_url.clone(),
            window,
            cx,
        );
        set_input_value(&self.forms.ai_provider_api_key_input, "", window, cx);
        set_input_value(
            &self.forms.ai_provider_temperature_input,
            provider
                .temperature
                .map(|temperature| temperature.to_string())
                .unwrap_or_default(),
            window,
            cx,
        );
        set_input_value(
            &self.forms.ai_provider_max_tokens_input,
            provider
                .max_tokens
                .map(|max_tokens| max_tokens.to_string())
                .unwrap_or_default(),
            window,
            cx,
        );
        set_input_value(
            &self.forms.ai_provider_context_window_input,
            provider
                .context_window
                .map(|context_window| context_window.to_string())
                .unwrap_or_default(),
            window,
            cx,
        );
        self.set_ai_provider_api_key_visibility(&provider.id, false, false, window, cx);

        let popup = self.open_ai_provider_popup(window, cx);
        if provider.has_api_key {
            self.spawn_ai_provider_api_key_load(provider.id, cx);
        } else {
            self.ai_provider_api_key_load_in_progress = None;
        }
        cx.notify();
        Some(popup)
    }

    fn open_ai_provider_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PendingAiProviderPopupState {
        let popup = PendingAiProviderPopupState;
        self.ai_provider_popup = Some(popup);
        self.forms
            .ai_provider_name_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
        popup
    }

    pub(in crate::ui::shell) fn close_ai_provider_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PendingAiProviderPopupState> {
        set_input_value(&self.forms.ai_provider_api_key_input, "", window, cx);
        self.clear_ai_provider_api_key_visibility(window, cx);
        let popup = self.ai_provider_popup.take();
        if let Some(popup) = popup {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::AiProviderPopup(popup),
            ));
            cx.notify();
        }
        popup
    }

    pub(in crate::ui::shell) fn submit_ai_provider_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ai_provider_save_in_progress {
            return;
        }

        let draft = match self.ai_provider_save_draft(cx) {
            Ok(draft) => draft,
            Err(failure) => {
                self.notify_ai_provider_validation_failure(failure, window, cx);
                return;
            }
        };

        if self.local_vault_status == LocalVaultStatus::Locked && !draft.api_key.is_empty() {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                SettingsDeferredCommand::SaveAiProvider(draft),
            )));
            return;
        }

        self.continue_save_ai_provider_after_unlock(draft, cx);
    }

    pub(in crate::ui::shell) fn continue_save_ai_provider_after_unlock(
        &mut self,
        draft: AiProviderSaveDraft,
        cx: &mut Context<Self>,
    ) {
        if self.ai_provider_save_in_progress {
            return;
        }

        if self.local_vault_status == LocalVaultStatus::Locked {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                SettingsDeferredCommand::SaveAiProvider(draft),
            )));
            return;
        }

        self.spawn_ai_provider_save_operation(draft, cx);
    }

    fn ai_provider_save_draft(&self, cx: &App) -> Result<AiProviderSaveDraft, ValidationFailure> {
        let kind = self
            .forms
            .ai_provider_kind_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(AiProviderKind::OpenAi);
        let existing = self.editing_ai_provider_id.as_ref().and_then(|id| {
            self.settings_store
                .settings()
                .ai_providers
                .iter()
                .find(|provider| provider.id == *id)
                .cloned()
        });
        let api_key = self
            .forms
            .ai_provider_api_key_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let name = self
            .forms
            .ai_provider_name_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let model = self
            .forms
            .ai_provider_model_input
            .read(cx)
            .value()
            .to_string();
        let base_url = self
            .forms
            .ai_provider_base_url_input
            .read(cx)
            .value()
            .to_string();
        let temperature = self
            .forms
            .ai_provider_temperature_input
            .read(cx)
            .value()
            .to_string();
        let max_tokens = self
            .forms
            .ai_provider_max_tokens_input
            .read(cx)
            .value()
            .to_string();
        let context_window = self
            .forms
            .ai_provider_context_window_input
            .read(cx)
            .value()
            .to_string();

        Self::validate_ai_provider_save_draft(
            existing,
            kind,
            name,
            model,
            base_url,
            api_key,
            &temperature,
            &max_tokens,
            &context_window,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_ai_provider_save_draft(
        existing: Option<AiProviderConfig>,
        kind: AiProviderKind,
        name: String,
        model: String,
        base_url: String,
        api_key: String,
        temperature: &str,
        max_tokens: &str,
        context_window: &str,
    ) -> Result<AiProviderSaveDraft, ValidationFailure> {
        if name.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "settings.ai_providers.validation.name_required",
            )));
        }
        if api_key.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "settings.ai_providers.validation.api_key_required",
            )));
        }

        let existing_has_api_key = existing
            .as_ref()
            .is_some_and(|provider| provider.has_api_key);
        let mut provider = existing.unwrap_or_else(|| AiProviderConfig::new(kind));
        provider.kind = kind;
        provider.name = name;
        provider.model = model;
        provider.base_url = base_url;
        provider.temperature = parse_optional_ai_provider_temperature(temperature)?;
        provider.max_tokens = parse_optional_ai_provider_positive_u64(
            max_tokens,
            "settings.ai_providers.validation.max_tokens_range",
        )?;
        provider.context_window = parse_optional_ai_provider_positive_u64(
            context_window,
            "settings.ai_providers.validation.context_window_range",
        )?;
        if api_key.is_empty() && !existing_has_api_key {
            provider.api_key_env = provider.api_key_env.trim().to_string();
        }
        provider.has_api_key = !api_key.is_empty() || existing_has_api_key;
        provider.sanitize();

        Ok(AiProviderSaveDraft { provider, api_key })
    }

    fn refresh_ai_provider_select(
        &mut self,
        selected_provider_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = self.settings_store.settings().clone();
        let options = ai_provider_select_options(&settings);
        let fallback_id = options.first().map(|option| option.value().clone());
        let selected_id = selected_provider_id
            .filter(|id| options.iter().any(|option| option.value().as_str() == *id))
            .map(ToOwned::to_owned)
            .or(fallback_id);

        if selected_id != settings.selected_ai_provider_id {
            self.settings_store.update(|settings| {
                settings.selected_ai_provider_id = selected_id.clone();
            });
        }

        self.forms.ai_provider_select.update(cx, |select, cx| {
            select.set_items(options, window, cx);
            if let Some(selected_id) = selected_id.as_ref() {
                select.set_selected_value(selected_id, window, cx);
            } else {
                select.set_selected_index(None, window, cx);
            }
        });
    }

    fn upsert_ai_provider(&mut self, provider: AiProviderConfig, cx: &mut Context<Self>) -> bool {
        let provider_id = provider.id.clone();
        let changed = self.settings_store.update(|settings| {
            if let Some(existing) = settings
                .ai_providers
                .iter_mut()
                .find(|existing| existing.id == provider_id)
            {
                *existing = provider;
            } else {
                settings.ai_providers.push(provider);
            }
        });
        if changed {
            cx.notify();
        }
        changed
    }

    fn apply_ai_provider_save_result(
        &mut self,
        task_result: AiProviderSaveTaskResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let provider_id = task_result.provider.id.clone();
        let changed = self.upsert_ai_provider(task_result.provider, cx);
        self.editing_ai_provider_id = Some(provider_id.clone());
        if changed {
            self.refresh_ai_provider_select(Some(&provider_id), window, cx);
        }
        set_input_value(&self.forms.ai_provider_api_key_input, "", window, cx);
        self.set_ai_provider_api_key_visibility(&provider_id, false, false, window, cx);

        let message = i18n::string("settings.ai_providers.notifications.saved_message");
        window.push_notification(
            success_notification(
                i18n::string("settings.ai_providers.notifications.saved_title"),
                message.clone(),
            ),
            cx,
        );
        cx.emit(AppCommand::Feedback(message));
        self.dismiss_ai_provider_popup(cx);
        cx.notify();
    }

    fn finish_ai_provider_save_operation(
        &mut self,
        result: anyhow::Result<AiProviderSaveTaskResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ai_provider_save_task = None;
        self.ai_provider_save_in_progress = false;
        match result {
            Ok(task_result) => self.apply_ai_provider_save_result(task_result, window, cx),
            Err(error) => self.notify_ai_provider_save_failed(window, &error, cx),
        }
        cx.notify();
    }

    fn finish_ai_provider_save_operation_without_window(
        &mut self,
        result: anyhow::Result<AiProviderSaveTaskResult>,
        cx: &mut Context<Self>,
    ) {
        self.ai_provider_save_task = None;
        self.ai_provider_save_in_progress = false;

        let message = match result {
            Ok(task_result) => {
                let provider_id = task_result.provider.id.clone();
                self.upsert_ai_provider(task_result.provider, cx);
                self.editing_ai_provider_id = Some(provider_id.clone());
                self.secret_visibility
                    .set_visible(SecretRevealTarget::AiProviderApiKey(provider_id), false);
                self.dismiss_ai_provider_popup(cx);
                i18n::string("settings.ai_providers.notifications.saved_message")
            }
            Err(error) => Self::ai_provider_save_failed_message(&error),
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    fn spawn_ai_provider_save_operation(
        &mut self,
        draft: AiProviderSaveDraft,
        cx: &mut Context<Self>,
    ) {
        let notification_window = cx.active_window();
        self.ai_provider_save_in_progress = true;
        cx.notify();

        let secrets = self.secrets.clone();
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
            self.finish_ai_provider_save_operation_without_window(
                Err(anyhow::anyhow!(error).context("failed to spawn AI provider save worker")),
                cx,
            );
            return;
        }

        self.ai_provider_save_task = Some(cx.spawn(async move |this, cx| {
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
        }));
    }

    fn spawn_ai_provider_api_key_load(&mut self, provider_id: String, cx: &mut Context<Self>) {
        let notification_window = cx.active_window();
        let secrets = self.secrets.clone();
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
                None,
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

        let task_id = self.next_ai_provider_api_key_load_task_id;
        self.next_ai_provider_api_key_load_task_id =
            self.next_ai_provider_api_key_load_task_id.wrapping_add(1);
        let task = cx.spawn(async move |this, cx| {
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
                        this.finish_ai_provider_api_key_load_in_window(
                            Some(task_id),
                            result,
                            window,
                            cx,
                        );
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
                            this.finish_ai_provider_api_key_load(Some(task_id), result, cx);
                        })
                    {
                        log::debug!(
                            "failed to apply AI provider key load result without window: {error:?}"
                        );
                    }
                }
            } else if let Err(error) = this.update(cx, move |this, cx| {
                this.finish_ai_provider_api_key_load(Some(task_id), result, cx);
            }) {
                log::debug!(
                    "failed to apply AI provider key load result without active window: {error:?}"
                );
            }
        });
        self.ai_provider_api_key_load_tasks.insert(task_id, task);
    }

    fn finish_ai_provider_api_key_load_in_window(
        &mut self,
        task_id: Option<u64>,
        result: AiProviderApiKeyLoadTaskResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remove_ai_provider_api_key_load_task(task_id);
        if !self.ai_provider_api_key_load_matches_current_editor(&result.provider_id) {
            return;
        }

        self.ai_provider_api_key_load_in_progress = None;
        match result.api_key {
            Ok(Some(api_key)) => {
                set_input_value(&self.forms.ai_provider_api_key_input, api_key, window, cx);
            }
            Ok(None) => {
                set_input_value(&self.forms.ai_provider_api_key_input, "", window, cx);
            }
            Err(error) => {
                set_input_value(&self.forms.ai_provider_api_key_input, "", window, cx);
                self.notify_ai_provider_secret_load_failed(window, &error, cx);
            }
        }
        cx.notify();
    }

    fn finish_ai_provider_api_key_load(
        &mut self,
        task_id: Option<u64>,
        result: AiProviderApiKeyLoadTaskResult,
        cx: &mut Context<Self>,
    ) {
        self.remove_ai_provider_api_key_load_task(task_id);
        if !self.ai_provider_api_key_load_matches_current_editor(&result.provider_id) {
            return;
        }

        self.ai_provider_api_key_load_in_progress = None;
        if let Err(error) = result.api_key {
            cx.emit(AppCommand::Feedback(error.to_string()));
        }
        cx.notify();
    }

    fn remove_ai_provider_api_key_load_task(&mut self, task_id: Option<u64>) {
        if let Some(task_id) = task_id {
            self.ai_provider_api_key_load_tasks.remove(&task_id);
        }
    }

    fn ai_provider_api_key_load_matches_current_editor(&self, provider_id: &str) -> bool {
        self.ai_provider_popup.is_some()
            && self.editing_ai_provider_id.as_deref() == Some(provider_id)
            && self.ai_provider_api_key_load_in_progress.as_deref() == Some(provider_id)
    }

    pub(in crate::ui::shell) fn delete_ai_provider(
        &mut self,
        provider_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.local_vault_status == LocalVaultStatus::Locked {
            self.notify_ai_provider_validation_failure(
                ValidationFailure::required(i18n::string(
                    "settings.sync.vault.access_required_error.message",
                )),
                window,
                cx,
            );
            return;
        }

        let removed = self.settings_store.update(|settings| {
            settings
                .ai_providers
                .retain(|provider| provider.id != provider_id);
        });
        self.secrets.delete_ai_provider_api_key(&provider_id);
        self.secret_visibility.set_visible(
            SecretRevealTarget::AiProviderApiKey(provider_id.clone()),
            false,
        );
        if self.editing_ai_provider_id.as_deref() == Some(provider_id.as_str()) {
            self.editing_ai_provider_id = None;
            set_input_value(&self.forms.ai_provider_api_key_input, "", window, cx);
            self.dismiss_ai_provider_popup(cx);
        }
        self.refresh_ai_provider_select(None, window, cx);

        if removed {
            let message = i18n::string("settings.ai_providers.notifications.deleted_message");
            window.push_notification(
                success_notification(
                    i18n::string("settings.ai_providers.notifications.deleted_title"),
                    message.clone(),
                ),
                cx,
            );
            cx.emit(AppCommand::Feedback(message));
        }
        cx.notify();
    }

    fn dismiss_ai_provider_popup(&mut self, cx: &mut Context<Self>) {
        if let Some(popup) = self.ai_provider_popup.take() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::AiProviderPopup(popup),
            ));
        }
    }

    fn clear_ai_provider_api_key_visibility(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.secret_visibility.clear_ai_provider_visibility();
        set_input_masked(
            &self.forms.ai_provider_api_key_input,
            true,
            false,
            window,
            cx,
        );
    }

    fn set_ai_provider_api_key_visibility(
        &mut self,
        provider_id: &str,
        visible: bool,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.secret_visibility.set_visible(
            SecretRevealTarget::AiProviderApiKey(provider_id.to_string()),
            visible,
        );
        set_input_masked(
            &self.forms.ai_provider_api_key_input,
            !visible,
            focus,
            window,
            cx,
        );
    }

    fn notify_ai_provider_validation_failure(
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

    fn notify_ai_provider_secret_load_failed(
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

    fn ai_provider_save_failed_message(error: &anyhow::Error) -> String {
        i18n::string_args(
            "settings.sync.save_feedback.failed_message",
            &[
                (
                    "field",
                    &i18n::string("settings.ai_providers.api_key.label"),
                ),
                ("error", &error.to_string()),
            ],
        )
    }

    fn notify_ai_provider_save_failed(
        &mut self,
        window: &mut Window,
        error: &anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        let message = Self::ai_provider_save_failed_message(error);
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
}

#[cfg(test)]
mod tests {
    use super::{parse_optional_ai_provider_positive_u64, parse_optional_ai_provider_temperature};

    #[test]
    fn parse_optional_ai_provider_temperature_rejects_out_of_range_values() {
        assert!(parse_optional_ai_provider_temperature("-0.1").is_err());
        assert!(parse_optional_ai_provider_temperature("2.1").is_err());
        assert_eq!(
            parse_optional_ai_provider_temperature("0.7").unwrap(),
            Some(0.7)
        );
        assert_eq!(parse_optional_ai_provider_temperature("   ").unwrap(), None);
    }

    #[test]
    fn parse_optional_ai_provider_positive_u64_rejects_zero() {
        assert!(
            parse_optional_ai_provider_positive_u64(
                "0",
                "settings.ai_providers.validation.max_tokens_range",
            )
            .is_err()
        );
        assert_eq!(
            parse_optional_ai_provider_positive_u64(
                "42",
                "settings.ai_providers.validation.max_tokens_range",
            )
            .unwrap(),
            Some(42)
        );
        assert_eq!(
            parse_optional_ai_provider_positive_u64(
                "   ",
                "settings.ai_providers.validation.max_tokens_range",
            )
            .unwrap(),
            None
        );
    }
}
