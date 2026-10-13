use super::*;
use crate::ui::components::forms::set_input_masked;
use gpui_kit::App;
use miaominal_secrets::SecretKind;
use miaominal_settings::{AiProviderConfig, AiProviderKind};

struct AiProviderSaveTaskResult {
    provider: AiProviderConfig,
    editor_revision: u64,
    changed: bool,
}

struct AiProviderApiKeyLoadTaskResult {
    editor_revision: u64,
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
    pub(in crate::ui) fn ai_provider_ids(&self) -> Vec<String> {
        self.settings_store
            .settings()
            .ai_providers
            .iter()
            .map(|provider| provider.id.clone())
            .collect()
    }

    pub(in crate::ui) fn selected_ai_provider_id(&self, cx: &App) -> Option<String> {
        self.forms
            .ai_provider_select
            .read(cx)
            .selected_value()
            .cloned()
    }

    pub(in crate::ui) fn start_new_ai_provider(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PendingAiProviderPopupState {
        self.renew_popup_forms("ai-provider", window, cx);
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

    pub(in crate::ui) fn edit_ai_provider(
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

        self.renew_popup_forms("ai-provider", window, cx);
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
        self.ai_provider_editor_revision = self.ai_provider_editor_revision.wrapping_add(1);
        self.ai_provider_api_key_load_tasks.clear();
        self.ai_provider_api_key_load_in_progress = None;
        let popup = PendingAiProviderPopupState;
        self.overlay_revisions.begin("ai-provider");
        self.ai_provider_popup = Some(popup);
        self.forms
            .ai_provider_name_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
        popup
    }

    pub(in crate::ui) fn close_ai_provider_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<PendingAiProviderPopupState> {
        self.ai_provider_editor_revision = self.ai_provider_editor_revision.wrapping_add(1);
        self.ai_provider_api_key_load_tasks.clear();
        self.ai_provider_api_key_load_in_progress = None;
        set_input_value(&self.forms.ai_provider_api_key_input, "", window, cx);
        self.clear_ai_provider_api_key_visibility(window, cx);
        let popup = self.ai_provider_popup.take();
        if let Some(popup) = popup {
            cx.emit(AppCommand::OverlayDismissed(self.overlay_snapshot(
                SettingsOverlay::AiProviderPopup(popup),
                cx.entity(),
                cx,
            )));
            cx.notify();
        }
        popup
    }

    pub(in crate::ui) fn submit_ai_provider_save(
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

        if self.local_vault_status == LocalVaultStatus::Locked && !draft.api_key().is_empty() {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                SettingsDeferredCommand::SaveAiProvider(draft),
            )));
            return;
        }

        self.continue_save_ai_provider_after_unlock(draft, cx);
    }

    pub(in crate::ui) fn continue_save_ai_provider_after_unlock(
        &mut self,
        draft: AiProviderSaveDraft,
        cx: &mut Context<Self>,
    ) {
        if self.ai_provider_save_in_progress
            || draft.editor_revision() != self.ai_provider_editor_revision
            || self.ai_provider_popup.is_none()
        {
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
        if self.editing_ai_provider_id.is_some() && existing.is_none() {
            return Err(ValidationFailure::invalid(i18n::string(
                "workspace.panel.agent.provider_missing",
            )));
        }
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

        let draft = Self::validate_ai_provider_save_draft(
            existing,
            kind,
            name,
            model,
            base_url,
            api_key,
            &temperature,
            &max_tokens,
            &context_window,
        )?;
        Ok(draft.with_editor_revision(self.ai_provider_editor_revision))
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

        let requires_existing_target = existing.is_some();
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

        Ok(AiProviderSaveDraft::new(
            provider,
            api_key,
            requires_existing_target,
        ))
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

    fn apply_ai_provider_save_result(
        &mut self,
        task_result: AiProviderSaveTaskResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let provider_id = task_result.provider.id.clone();
        let matches_editor = task_result.editor_revision == self.ai_provider_editor_revision
            && self.ai_provider_popup.is_some();
        // The file lock serializes writes. Reload its current committed state
        // so a later setting change is never replaced by this worker's clone.
        if let Err(error) = self.settings_store.reload_from_disk() {
            self.notify_ai_provider_save_failed(window, &error, cx);
            return;
        }
        if task_result.changed {
            let selected_id = if matches_editor {
                Some(provider_id.clone())
            } else {
                self.settings_store
                    .settings()
                    .selected_ai_provider_id
                    .clone()
            };
            self.refresh_ai_provider_select(selected_id.as_deref(), window, cx);
        }
        if matches_editor {
            self.editing_ai_provider_id = Some(provider_id.clone());
            set_input_value(&self.forms.ai_provider_api_key_input, "", window, cx);
            self.set_ai_provider_api_key_visibility(&provider_id, false, false, window, cx);
            self.dismiss_ai_provider_popup(cx);
        }

        let message = i18n::string("settings.ai_providers.notifications.saved_message");
        crate::ui::components::notifications::push_app_notification(
            window,
            success_notification(
                i18n::string("settings.ai_providers.notifications.saved_title"),
                message.clone(),
            ),
            cx,
        );
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    fn finish_ai_provider_save_operation(
        &mut self,
        result: anyhow::Result<AiProviderSaveTaskResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        self.ai_provider_save_in_progress = false;

        let message = match result {
            Ok(task_result) => {
                let provider_id = task_result.provider.id.clone();
                if let Err(error) = self.settings_store.reload_from_disk() {
                    cx.emit(AppCommand::Feedback(Self::ai_provider_save_failed_message(
                        &error,
                    )));
                    cx.notify();
                    return;
                }
                if task_result.editor_revision == self.ai_provider_editor_revision
                    && self.ai_provider_popup.is_some()
                {
                    self.editing_ai_provider_id = Some(provider_id.clone());
                    self.secret_visibility
                        .set_visible(SecretRevealTarget::AiProviderApiKey(provider_id), false);
                    self.dismiss_ai_provider_popup(cx);
                }
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
        let mut settings_store = self.settings_store.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name("ai-provider-save".to_string())
            .spawn(move || {
                let (mut provider, api_key, editor_revision, requires_existing_target) =
                    draft.into_parts();
                let _sync_guard = miaominal_secrets::lock_sync_data();
                let changed = (|| {
                    settings_store.reload_from_disk()?;
                    if requires_existing_target
                        && !settings_store
                            .settings()
                            .ai_providers
                            .iter()
                            .any(|current| current.id == provider.id)
                    {
                        anyhow::bail!(
                            "AI provider was deleted while its editor was waiting for vault unlock"
                        );
                    }
                    SettingsService::persist_ai_provider(
                        &mut settings_store,
                        &secrets,
                        &mut provider,
                        (!api_key.is_empty()).then_some(api_key.as_str()),
                    )
                })();
                let result = changed.map(|changed| AiProviderSaveTaskResult {
                    provider,
                    editor_revision,
                    changed,
                });
                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            self.finish_ai_provider_save_operation_without_window(
                Err(anyhow::anyhow!(error).context("failed to spawn AI provider save worker")),
                cx,
            );
            return;
        }

        let persistence = self.persistence.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("AI provider save task cancelled")))
                })
                .await;

            cx.update(|cx| persistence.published(cx));

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

    fn spawn_ai_provider_api_key_load(&mut self, provider_id: String, cx: &mut Context<Self>) {
        let editor_revision = self.ai_provider_editor_revision;
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
                    editor_revision,
                    api_key: secrets.get(&provider_id_for_worker, SecretKind::AiProviderApiKey),
                    provider_id: provider_id_for_worker,
                };
                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            self.finish_ai_provider_api_key_load(
                None,
                AiProviderApiKeyLoadTaskResult {
                    editor_revision,
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
                            editor_revision,
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
        if !self.ai_provider_api_key_load_matches_current_editor(
            result.editor_revision,
            &result.provider_id,
        ) {
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
        if !self.ai_provider_api_key_load_matches_current_editor(
            result.editor_revision,
            &result.provider_id,
        ) {
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

    fn ai_provider_api_key_load_matches_current_editor(
        &self,
        editor_revision: u64,
        provider_id: &str,
    ) -> bool {
        editor_revision == self.ai_provider_editor_revision
            && self.ai_provider_popup.is_some()
            && self.editing_ai_provider_id.as_deref() == Some(provider_id)
            && self.ai_provider_api_key_load_in_progress.as_deref() == Some(provider_id)
    }

    pub(in crate::ui) fn delete_ai_provider(
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
            crate::ui::components::notifications::push_app_notification(
                window,
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
        self.ai_provider_editor_revision = self.ai_provider_editor_revision.wrapping_add(1);
        self.ai_provider_api_key_load_tasks.clear();
        self.ai_provider_api_key_load_in_progress = None;
        if let Some(popup) = self.ai_provider_popup.take() {
            cx.emit(AppCommand::OverlayDismissed(self.overlay_snapshot(
                SettingsOverlay::AiProviderPopup(popup),
                cx.entity(),
                cx,
            )));
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
        crate::ui::components::notifications::push_app_notification(
            window,
            validation_notification(failure.kind, message.clone()),
            cx,
        );
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
        crate::ui::components::notifications::push_app_notification(
            window,
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
        crate::ui::components::notifications::push_app_notification(
            window,
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
    use super::*;
    use crate::ui::contracts::catalog::CatalogReader;
    use crate::ui::contracts::overlay::OverlaySnapshot;
    use crate::ui::contracts::proxies::{ProxyCommitOperation, ProxyCommitter};
    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt as _;
    use miaominal_core::profile::SessionProfile;
    use miaominal_secrets::{CredentialStore, LockedCredentialBackend};
    use miaominal_settings::SshBridgeConfig;
    use miaominal_ssh::SshBridgeEndpoint;
    use miaominal_storage::{BridgeAuditLog, BridgeSecuritySettingsStore, KnownHostsStore};
    use miaominal_sync::SyncConfigStore;
    use std::rc::Rc;

    struct EmptyCatalog;
    impl CatalogReader for EmptyCatalog {
        fn profiles(&self) -> Vec<SessionProfile> {
            Vec::new()
        }
        fn profile(&self, _: &str) -> Option<SessionProfile> {
            None
        }
        fn proxies(&self) -> Vec<ProxyProfile> {
            Vec::new()
        }
    }
    struct UnusedProxyCommitter;
    impl ProxyCommitter for UnusedProxyCommitter {
        fn commit(
            &self,
            _: &mut App,
            _: ProxyCommitOperation,
        ) -> anyhow::Result<Vec<ProxyProfile>> {
            anyhow::bail!("the AI editor must not write proxy data")
        }
    }

    fn test_settings_args(
        root: &std::path::Path,
        runtime: &tokio::runtime::Runtime,
        settings_store: SettingsStore,
        persistence: SettingsPersistencePort,
    ) -> SettingsControllerArgs {
        let bridge = SshBridgeService::new_with_stores(
            runtime.handle().clone(),
            SshBridgeEndpoint::derive(root).unwrap(),
            SshBridgeEndpoint::instance_id(root).unwrap(),
            root.join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(root.join("known_hosts")),
            BridgeSecuritySettingsStore::open(&root.join("settings.toml"))
                .map_err(|error| format!("{error:#}")),
            BridgeAuditLog::open(&root.join("bridge_audit.log"))
                .map_err(|error| format!("{error:#}")),
        );
        SettingsControllerArgs {
            runtime: runtime.handle().clone(),
            proxy_store: None,
            proxies: Vec::new(),
            settings_store,
            secrets: SecretStore::new_locked_vault(),
            sync_engine: SyncEngine {
                config_store: SyncConfigStore::with_credentials(
                    root.join("sync.toml"),
                    SyncConfig::default(),
                    CredentialStore::with_backend("settings-ui-test", LockedCredentialBackend),
                ),
                last_capability_report: None,
            },
            local_vault_status: LocalVaultStatus::Disabled,
            open_ssh_integration_service: OpenSshIntegrationService::new(
                bridge.clone(),
                root.join("ssh"),
                SshBridgeEndpoint::instance_id(root).unwrap(),
            ),
            ssh_bridge_service: bridge,
            sync_executor: None,
            auto_sync: miaominal_services::AutoSyncSnapshot {
                capability: Default::default(),
                capability_notice_id: None,
                updated_config: None,
                revision: 0,
                enabled: false,
                phase: miaominal_services::AutoSyncPhase::Disabled,
                message: None,
                last_result: None,
                last_result_id: None,
                dirty: false,
                retry_at_unix: None,
                intervention: None,
            },
            proxy_commit: ProxyCommitPort::new(UnusedProxyCommitter),
            auto_sync_control: AutoSyncControlPort::new(|_, _, _| false, |_| {}),
            persistence,
        }
    }

    #[gpui_kit::test]
    async fn durable_save_publishes_latest_settings_after_its_window_and_owner_close(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let settings_path = directory.path().join("settings.toml");
        let mut store = SettingsStore::load_with_path(settings_path.clone()).unwrap();
        let mut initial = store.settings().clone();
        initial.mark_current_onboarding_completed();
        store.replace(initial).unwrap();
        let published = cx.new(|_| store.clone());
        let publication_count = Rc::new(std::cell::Cell::new(0));
        let persistence = SettingsPersistencePort::new({
            let published = published.downgrade();
            let publication_count = publication_count.clone();
            move |cx| {
                published.upgrade().unwrap().update(cx, |store, cx| {
                    store.reload_from_disk().unwrap();
                    publication_count.set(publication_count.get() + 1);
                    cx.notify();
                });
            }
        });
        let args = test_settings_args(directory.path(), &runtime, store.clone(), persistence);
        let mut provider = AiProviderConfig::new(AiProviderKind::OpenAi);
        provider.id = "after-window-close".into();
        provider.name = "Saved after closing".into();
        let draft = AiProviderSaveDraft::new(provider, String::new(), false);

        // Prevent the durable worker from committing until the submitting window and owner
        // are dropped. Release the guard before running the test executor's receiver task.
        let sync_guard = miaominal_secrets::lock_sync_data();
        let closed_owner = cx.update(|cx| {
            let mut owner = None;
            let (handle, harness) =
                gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
                    let settings = cx.new(|cx| {
                        SettingsController::new(
                            args,
                            CatalogQueryPort::new(Rc::new(EmptyCatalog)),
                            window,
                            cx,
                        )
                    });
                    settings.update(cx, |settings, cx| {
                        settings.spawn_ai_provider_save_operation(draft, cx);
                    });
                    owner = Some(settings.downgrade());
                    cx.new(|_| AiEditorHarness {
                        owner: settings,
                        retained: None,
                    })
                })
                .unwrap();
            handle
                .update(cx, |_, window, _| window.remove_window())
                .unwrap();
            drop(harness);
            owner.unwrap()
        });
        assert!(closed_owner.upgrade().is_none());
        assert!(store.update(|settings| settings.recent_connections_count = 3));
        drop(sync_guard);

        cx.condition(&published, |store, _| {
            store
                .settings()
                .ai_providers
                .iter()
                .any(|provider| provider.id == "after-window-close")
        })
        .await;
        published.read_with(cx, |store, _| {
            assert_eq!(store.settings().recent_connections_count, 3);
        });
        assert_eq!(publication_count.get(), 1);
        assert!(closed_owner.upgrade().is_none());
    }

    struct AiEditorHarness {
        owner: Entity<SettingsController>,
        retained: Option<OverlaySnapshot>,
    }
    impl Render for AiEditorHarness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().children(
                self.retained
                    .as_ref()
                    .map(|surface| surface.render(None, 700., cx)),
            )
        }
    }

    #[gpui_kit::test]
    fn delayed_api_key_load_and_retained_cancel_cannot_touch_replacement_editor(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let mut settings_store = SettingsStore::load_with_path(root.join("settings.toml")).unwrap();
        let mut settings = settings_store.settings().clone();
        settings.mark_current_onboarding_completed();
        settings.ai_providers = ["alpha", "bravo"]
            .into_iter()
            .map(|id| {
                let mut provider = AiProviderConfig::new(AiProviderKind::OpenAi);
                provider.id = id.into();
                provider.name = id.into();
                provider
            })
            .collect();
        settings_store.replace(settings).unwrap();
        let bridge = SshBridgeService::new_with_stores(
            runtime.handle().clone(),
            SshBridgeEndpoint::derive(root).unwrap(),
            SshBridgeEndpoint::instance_id(root).unwrap(),
            root.join("bridge_known_hosts"),
            SshBridgeConfig::default(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(root.join("known_hosts")),
            BridgeSecuritySettingsStore::open(&root.join("settings.toml"))
                .map_err(|error| format!("{error:#}")),
            BridgeAuditLog::open(&root.join("bridge_audit.log"))
                .map_err(|error| format!("{error:#}")),
        );
        let sync_engine = SyncEngine {
            config_store: SyncConfigStore::with_credentials(
                root.join("sync.toml"),
                SyncConfig::default(),
                CredentialStore::with_backend("settings-ui-test", LockedCredentialBackend),
            ),
            last_capability_report: None,
        };
        let args = SettingsControllerArgs {
            runtime: runtime.handle().clone(),
            proxy_store: None,
            proxies: Vec::new(),
            settings_store,
            secrets: SecretStore::new_locked_vault(),
            sync_engine,
            local_vault_status: LocalVaultStatus::Disabled,
            open_ssh_integration_service: OpenSshIntegrationService::new(
                bridge.clone(),
                root.join("ssh"),
                SshBridgeEndpoint::instance_id(root).unwrap(),
            ),
            ssh_bridge_service: bridge,
            sync_executor: None,
            auto_sync: miaominal_services::AutoSyncSnapshot {
                capability: Default::default(),
                capability_notice_id: None,
                updated_config: None,
                revision: 0,
                enabled: false,
                phase: miaominal_services::AutoSyncPhase::Disabled,
                message: None,
                last_result: None,
                last_result_id: None,
                dirty: false,
                retry_at_unix: None,
                intervention: None,
            },
            proxy_commit: ProxyCommitPort::new(UnusedProxyCommitter),
            auto_sync_control: AutoSyncControlPort::new(|_, _, _| false, |_| {}),
            persistence: SettingsPersistencePort::new(|_| {}),
        };
        let (view, cx) = cx.add_window_view(move |window, cx| {
            let owner = cx.new(|cx| {
                SettingsController::new(
                    args,
                    CatalogQueryPort::new(Rc::new(EmptyCatalog)),
                    window,
                    cx,
                )
            });
            owner.update(cx, |owner, cx| {
                owner.edit_ai_provider("alpha".into(), window, cx).unwrap();
                set_input_value(
                    &owner.forms.ai_provider_api_key_input,
                    "alpha draft",
                    window,
                    cx,
                );
            });
            let retained = owner.read(cx).overlay_snapshots(owner.clone(), cx).pop();
            AiEditorHarness { owner, retained }
        });
        let owner = view.read_with(cx, |view, _| view.owner.clone());
        let (old_revision, old_input) = owner.read_with(cx, |owner, _| {
            (
                owner.ai_provider_editor_revision,
                owner.forms.ai_provider_api_key_input.clone(),
            )
        });
        cx.update(|window, cx| {
            owner.update(cx, |owner, cx| {
                owner.edit_ai_provider("bravo".into(), window, cx).unwrap();
                set_input_value(
                    &owner.forms.ai_provider_api_key_input,
                    "bravo draft",
                    window,
                    cx,
                );
                owner.ai_provider_api_key_load_in_progress = Some("bravo".into());
                owner.finish_ai_provider_api_key_load_in_window(
                    None,
                    AiProviderApiKeyLoadTaskResult {
                        editor_revision: old_revision,
                        provider_id: "alpha".into(),
                        api_key: Ok(Some("late secret".into())),
                    },
                    window,
                    cx,
                );
                assert_eq!(
                    owner.forms.ai_provider_api_key_input.read(cx).value(),
                    "bravo draft"
                );
                assert_eq!(
                    owner.ai_provider_api_key_load_in_progress.as_deref(),
                    Some("bravo")
                );
                assert_eq!(old_input.read(cx).value(), "alpha draft");
                assert_ne!(
                    old_input.entity_id(),
                    owner.forms.ai_provider_api_key_input.entity_id()
                );
            })
        });
        cx.update(|window, cx| {
            window.render_frame(cx);
            window.click("ai-provider-popup-cancel", cx);
            let owner = owner.read(cx);
            assert!(owner.ai_provider_popup.is_some());
            assert_eq!(owner.editing_ai_provider_id.as_deref(), Some("bravo"));
            assert_eq!(
                owner.forms.ai_provider_api_key_input.read(cx).value(),
                "bravo draft"
            );
        });
    }

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
