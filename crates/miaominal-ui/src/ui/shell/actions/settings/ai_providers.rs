use super::*;

fn parse_optional_ai_provider_temperature(value: &str) -> Result<Option<f64>, ValidationFailure> {
    let trimmed_value = value.trim();
    if trimmed_value.is_empty() {
        return Ok(None);
    }

    let temperature = trimmed_value.parse::<f64>().map_err(|_| {
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
    })?;

    if !temperature.is_finite()
        || !(miaominal_settings::AI_PROVIDER_TEMPERATURE_MIN
            ..=miaominal_settings::AI_PROVIDER_TEMPERATURE_MAX)
            .contains(&temperature)
    {
        return Err(ValidationFailure::invalid(i18n::string_args(
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
        )));
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

    let number = trimmed_value.parse::<u64>().map_err(|_| {
        ValidationFailure::invalid(i18n::string_args(
            validation_key,
            &[(
                "min",
                &miaominal_settings::AI_PROVIDER_POSITIVE_INTEGER_MIN.to_string(),
            )],
        ))
    })?;

    if number < miaominal_settings::AI_PROVIDER_POSITIVE_INTEGER_MIN {
        return Err(ValidationFailure::invalid(i18n::string_args(
            validation_key,
            &[(
                "min",
                &miaominal_settings::AI_PROVIDER_POSITIVE_INTEGER_MIN.to_string(),
            )],
        )));
    }

    Ok(Some(number))
}

struct AiProviderSaveTaskResult {
    provider: AiProviderConfig,
}

struct AiProviderApiKeyLoadTaskResult {
    provider_id: String,
    api_key: Result<Option<String>>,
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
    pub(in crate::ui::shell) fn ai_provider_save_in_progress(&self) -> bool {
        self.ai_provider_save_in_progress
    }
    pub(in crate::ui::shell) fn ai_provider_api_key_load_in_progress_for(
        &self,
        provider_id: &str,
    ) -> bool {
        self.ai_provider_api_key_load_in_progress.as_deref() == Some(provider_id)
    }
    pub(super) fn ai_provider_ids(&self) -> Vec<String> {
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
    pub(in crate::ui::shell) fn selected_ai_provider_id(&self, cx: &App) -> Option<String> {
        self.panel_forms
            .settings
            .ai_provider_select
            .read(cx)
            .selected_value()
            .cloned()
    }
    pub(super) fn refresh_ai_provider_select(
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
    pub(super) fn open_ai_provider_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn dismiss_ai_provider_popup(&mut self, cx: &mut Context<Self>) {
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
        set_input_value(
            &self.panel_forms.settings.ai_provider_temperature_input,
            "",
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.ai_provider_max_tokens_input,
            "",
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.ai_provider_context_window_input,
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
        set_input_value(
            &self.panel_forms.settings.ai_provider_temperature_input,
            provider
                .temperature
                .map(|t| t.to_string())
                .unwrap_or_default(),
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.ai_provider_max_tokens_input,
            provider
                .max_tokens
                .map(|t| t.to_string())
                .unwrap_or_default(),
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.ai_provider_context_window_input,
            provider
                .context_window
                .map(|t| t.to_string())
                .unwrap_or_default(),
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
    pub(super) fn spawn_ai_provider_api_key_load(
        &mut self,
        provider_id: String,
        cx: &mut Context<Self>,
    ) {
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
    pub(super) fn ai_provider_api_key_load_matches_current_editor(
        &self,
        provider_id: &str,
    ) -> bool {
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
        let temperature_value = self
            .panel_forms
            .settings
            .ai_provider_temperature_input
            .read(cx)
            .value();
        let temperature = match parse_optional_ai_provider_temperature(&temperature_value) {
            Ok(temperature) => temperature,
            Err(validation_error) => {
                self.notify_validation_failure_in_window(
                    window,
                    validation_error.kind,
                    validation_error.message,
                    cx,
                );
                return;
            }
        };
        provider.temperature = temperature;
        let max_tokens_value = self
            .panel_forms
            .settings
            .ai_provider_max_tokens_input
            .read(cx)
            .value();
        let max_tokens = match parse_optional_ai_provider_positive_u64(
            &max_tokens_value,
            "settings.ai_providers.validation.max_tokens_range",
        ) {
            Ok(max_tokens) => max_tokens,
            Err(validation_error) => {
                self.notify_validation_failure_in_window(
                    window,
                    validation_error.kind,
                    validation_error.message,
                    cx,
                );
                return;
            }
        };
        provider.max_tokens = max_tokens;
        let context_window_value = self
            .panel_forms
            .settings
            .ai_provider_context_window_input
            .read(cx)
            .value();
        let context_window = match parse_optional_ai_provider_positive_u64(
            &context_window_value,
            "settings.ai_providers.validation.context_window_range",
        ) {
            Ok(context_window) => context_window,
            Err(validation_error) => {
                self.notify_validation_failure_in_window(
                    window,
                    validation_error.kind,
                    validation_error.message,
                    cx,
                );
                return;
            }
        };
        provider.context_window = context_window;
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
    pub(super) fn continue_save_ai_provider_after_unlock(
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
    pub(super) fn upsert_ai_provider(&mut self, provider: AiProviderConfig) -> bool {
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
    pub(super) fn spawn_ai_provider_save_operation(
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
