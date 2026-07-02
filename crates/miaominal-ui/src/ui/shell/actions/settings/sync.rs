use super::*;

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

pub(super) struct LocalVaultSyncSecretInputs {
    pub(super) github_token: String,
    pub(super) webdav_password: String,
    pub(super) sync_passphrase: String,
}
enum SyncSecretSaveTaskRequest {
    GithubToken(String),
    WebdavPassword(String),
}

struct SyncSecretSaveTaskResult {
    operation: SyncSecretSaveOperation,
    updated_config: SyncConfig,
}

impl SyncSecretSaveTaskRequest {
    fn operation(&self) -> SyncSecretSaveOperation {
        match self {
            Self::GithubToken(_) => SyncSecretSaveOperation::GithubToken,
            Self::WebdavPassword(_) => SyncSecretSaveOperation::WebdavPassword,
        }
    }

    fn worker_name(&self) -> &'static str {
        match self {
            Self::GithubToken(_) => "sync-github-token-save",
            Self::WebdavPassword(_) => "sync-webdav-password-save",
        }
    }

    fn cancelled_message(&self) -> &'static str {
        match self {
            Self::GithubToken(_) => "sync GitHub token save task cancelled",
            Self::WebdavPassword(_) => "sync WebDAV password save task cancelled",
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

impl AppView {
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
    pub(super) fn open_sync_passphrase_clear_confirm_popup(&mut self, cx: &mut Context<Self>) {
        let popup = PendingSyncPassphraseClearConfirmPopupState;
        let stable_key = DialogOverlaySnapshot::SyncPassphraseClearConfirmPopup(popup).stable_key();
        self.dialogs
            .exiting_dialogs
            .retain(|dialog| dialog.snapshot.stable_key() != stable_key);
        self.dialogs.pending_sync_passphrase_clear_confirm_popup = Some(popup);
        cx.notify();
    }
    pub(super) fn dismiss_sync_passphrase_clear_confirm_popup(&mut self, cx: &mut Context<Self>) {
        if let Some(popup) = self
            .dialogs
            .pending_sync_passphrase_clear_confirm_popup
            .take()
        {
            self.start_dialog_exit(
                DialogOverlaySnapshot::SyncPassphraseClearConfirmPopup(popup),
                cx,
            );
            cx.notify();
        }
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }

        self.dismiss_sync_passphrase_clear_confirm_popup(cx);
        self.continue_clear_sync_passphrase_after_confirm(window, cx);
    }
    pub(in crate::ui::shell) fn open_sync_passphrase_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }

        let popup = PendingSyncPassphrasePopupState;
        let stable_key = DialogOverlaySnapshot::SyncPassphrasePopup(popup).stable_key();
        self.dialogs
            .exiting_dialogs
            .retain(|dialog| dialog.snapshot.stable_key() != stable_key);
        self.sync_passphrase_popup = Some(popup);
        self.clear_sync_passphrase_popup_inputs(window, cx);
        self.focus_sync_passphrase_input(window, cx);
        cx.notify();
    }
    pub(super) fn dismiss_sync_passphrase_popup(&mut self, cx: &mut Context<Self>) {
        if let Some(popup) = self.sync_passphrase_popup.take() {
            self.start_dialog_exit(DialogOverlaySnapshot::SyncPassphrasePopup(popup), cx);
            cx.notify();
        }
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
            .panel_forms
            .settings
            .sync_passphrase_input
            .read(cx)
            .value()
            .to_string();
        let passphrase_confirmation = self
            .panel_forms
            .settings
            .sync_passphrase_confirmation_input
            .read(cx)
            .value()
            .to_string();

        if passphrase.trim().is_empty() {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("settings.sync.encryption.passphrase.required_error.message"),
                cx,
            );
            return;
        }

        if passphrase_confirmation.trim().is_empty() {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string(
                    "settings.sync.encryption.passphrase.confirmation_required_error.message",
                ),
                cx,
            );
            return;
        }

        if passphrase != passphrase_confirmation {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::InvalidInput,
                i18n::string("settings.sync.encryption.passphrase.mismatch_error.message"),
                cx,
            );
            return;
        }

        if self.local_vault_status == LocalVaultStatus::Locked {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::SaveSyncPassphrase(passphrase),
                window,
                cx,
            );
            return;
        }

        self.spawn_sync_passphrase_operation(SyncPassphraseTaskRequest::Save(passphrase), cx);
    }
    pub(in crate::ui::shell) fn clear_sync_passphrase(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }

        self.open_sync_passphrase_clear_confirm_popup(cx);
    }
    pub(super) fn notify_sync_secret_saved(
        &mut self,
        window: &mut Window,
        field_label: &str,
        cx: &mut Context<Self>,
    ) {
        let message = i18n::string_args(
            "settings.sync.save_feedback.saved_message",
            &[("field", field_label)],
        );
        self.status_message = message.clone();
        window.push_notification(
            Self::success_notification(
                i18n::string("settings.sync.save_feedback.saved_title"),
                message,
            ),
            cx,
        );
        cx.notify();
    }
    pub(super) fn notify_sync_secret_save_failed(
        &mut self,
        window: &mut Window,
        field_label: &str,
        error: &str,
        cx: &mut Context<Self>,
    ) {
        let message = i18n::string_args(
            "settings.sync.save_feedback.failed_message",
            &[("field", field_label), ("error", error)],
        );
        self.status_message = message.clone();
        window.push_notification(
            Self::error_notification(
                i18n::string("settings.sync.save_feedback.failed_title"),
                message,
            ),
            cx,
        );
        cx.notify();
    }
    pub(super) fn clear_sync_passphrase_popup_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        set_input_value(
            &self.panel_forms.settings.sync_passphrase_input,
            String::new(),
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.sync_passphrase_confirmation_input,
            String::new(),
            window,
            cx,
        );
        set_input_masked(
            &self.panel_forms.settings.sync_passphrase_input,
            true,
            false,
            window,
            cx,
        );
        set_input_masked(
            &self.panel_forms.settings.sync_passphrase_confirmation_input,
            true,
            false,
            window,
            cx,
        );
        self.secret_visibility
            .set_visible(SecretRevealTarget::SyncPassphrase, false);
        self.secret_visibility
            .set_visible(SecretRevealTarget::SyncPassphraseConfirmation, false);
    }
    pub(super) fn focus_sync_passphrase_input(&self, window: &mut Window, cx: &mut App) {
        self.panel_forms
            .settings
            .sync_passphrase_input
            .update(cx, |input, cx| {
                input.focus(window, cx);
            });
    }
    pub(in crate::ui::shell) fn sync_passphrase_operation_in_progress(&self) -> bool {
        self.sync.sync_passphrase_operation.is_some()
    }
    pub(in crate::ui::shell) fn sync_secret_save_in_progress(&self) -> bool {
        self.sync.sync_secret_save_operation.is_some()
    }
    pub(in crate::ui::shell) fn sync_github_token_save_in_progress(&self) -> bool {
        self.sync.sync_secret_save_operation == Some(SyncSecretSaveOperation::GithubToken)
    }
    pub(in crate::ui::shell) fn sync_webdav_password_save_in_progress(&self) -> bool {
        self.sync.sync_secret_save_operation == Some(SyncSecretSaveOperation::WebdavPassword)
    }
    pub(in crate::ui::shell) fn sync_passphrase_save_in_progress(&self) -> bool {
        self.sync.sync_passphrase_operation == Some(SyncPassphraseOperation::Save)
    }
    pub(super) fn sync_secret_field_label(operation: SyncSecretSaveOperation) -> String {
        match operation {
            SyncSecretSaveOperation::GithubToken => i18n::string("settings.sync.gist.token.label"),
            SyncSecretSaveOperation::WebdavPassword => {
                i18n::string("settings.sync.webdav.password.label")
            }
        }
    }
    pub(super) fn sync_gist_field_label() -> String {
        i18n::string("settings.sync.gist.gist_id.label")
    }
    pub(in crate::ui::shell) fn update_sync_config(
        &mut self,
        update: impl FnOnce(&mut SyncConfig),
    ) -> anyhow::Result<()> {
        self.sync.sync_engine.config_store.update(update)
    }
    pub(in crate::ui::shell) fn submit_sync_github_token_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_secret_save_in_progress() {
            return;
        }

        let token = self
            .panel_forms
            .settings
            .sync_github_token_input
            .read(cx)
            .value()
            .to_string();

        if self.local_vault_status == LocalVaultStatus::Locked {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::SaveSyncGithubToken(token),
                window,
                cx,
            );
            return;
        }

        self.continue_save_sync_github_token_after_unlock(token, window, cx);
    }
    pub(in crate::ui::shell) fn submit_sync_github_gist_id_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let gist_id_input = self
            .panel_forms
            .settings
            .sync_github_gist_id_input
            .read(cx)
            .value()
            .to_string();
        let gist_id = normalize_github_gist_id(&gist_id_input);
        let field_label = Self::sync_gist_field_label();

        if let Err(error) = self.update_sync_config(|config| {
            config.gist_id = (!gist_id.is_empty()).then_some(gist_id.clone());
        }) {
            self.notify_sync_secret_save_failed(window, &field_label, &error.to_string(), cx);
            return;
        }

        set_input_value(
            &self.panel_forms.settings.sync_github_gist_id_input,
            gist_id.clone(),
            window,
            cx,
        );

        if self.sync.sync_engine.config_store.config.provider == SyncProvider::GithubGist
            && self.sync.sync_engine.config_store.config.gist_enabled
        {
            if gist_id.is_empty() {
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

        self.notify_sync_secret_saved(window, &field_label, cx);
    }
    pub(super) fn continue_save_sync_github_token_after_unlock(
        &mut self,
        token: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_secret_save_in_progress() {
            return;
        }

        if self.local_vault_status == LocalVaultStatus::Locked {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::SaveSyncGithubToken(token),
                window,
                cx,
            );
            return;
        }

        self.spawn_sync_secret_save_operation(SyncSecretSaveTaskRequest::GithubToken(token), cx);
    }
    pub(in crate::ui::shell) fn submit_sync_webdav_password_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_secret_save_in_progress() {
            return;
        }

        let password = self
            .panel_forms
            .settings
            .sync_webdav_password_input
            .read(cx)
            .value()
            .to_string();

        if self.local_vault_status == LocalVaultStatus::Locked {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::SaveSyncWebdavPassword(password),
                window,
                cx,
            );
            return;
        }

        self.continue_save_sync_webdav_password_after_unlock(password, window, cx);
    }
    pub(super) fn continue_save_sync_webdav_password_after_unlock(
        &mut self,
        password: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_secret_save_in_progress() {
            return;
        }

        if self.local_vault_status == LocalVaultStatus::Locked {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::SaveSyncWebdavPassword(password),
                window,
                cx,
            );
            return;
        }

        self.spawn_sync_secret_save_operation(
            SyncSecretSaveTaskRequest::WebdavPassword(password),
            cx,
        );
    }
    pub(super) fn continue_save_sync_passphrase_after_unlock(
        &mut self,
        passphrase: String,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }

        self.spawn_sync_passphrase_operation(SyncPassphraseTaskRequest::Save(passphrase), cx);
    }
    pub(super) fn continue_clear_sync_passphrase_after_confirm(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }

        if self.local_vault_status == LocalVaultStatus::Locked {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::ClearSyncPassphrase,
                window,
                cx,
            );
            return;
        }

        self.spawn_sync_passphrase_operation(SyncPassphraseTaskRequest::Clear, cx);
    }
    pub(super) fn continue_clear_sync_passphrase_after_unlock(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sync_passphrase_operation_in_progress() {
            return;
        }

        self.spawn_sync_passphrase_operation(SyncPassphraseTaskRequest::Clear, cx);
    }
    fn spawn_sync_secret_save_operation(
        &mut self,
        request: SyncSecretSaveTaskRequest,
        cx: &mut Context<Self>,
    ) {
        let operation = request.operation();
        let worker_name = request.worker_name().to_string();
        let cancelled_message = request.cancelled_message().to_string();
        let notification_window = cx.active_window();

        self.sync.sync_secret_save_operation = Some(operation);
        cx.notify();

        let mut sync_engine = self.sync.sync_engine.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name(worker_name)
            .spawn(move || {
                let result = match request {
                    SyncSecretSaveTaskRequest::GithubToken(token) => {
                        SettingsService::persist_sync_github_token(&mut sync_engine, token.as_str())
                            .map(|()| SyncSecretSaveTaskResult {
                                operation: SyncSecretSaveOperation::GithubToken,
                                updated_config: sync_engine.config_store.config.clone(),
                            })
                    }
                    SyncSecretSaveTaskRequest::WebdavPassword(password) => {
                        SettingsService::persist_sync_webdav_password(
                            &mut sync_engine,
                            password.as_str(),
                        )
                        .map(|()| SyncSecretSaveTaskResult {
                            operation: SyncSecretSaveOperation::WebdavPassword,
                            updated_config: sync_engine.config_store.config.clone(),
                        })
                    }
                };

                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            self.finish_sync_secret_save_spawn_error(operation, anyhow::anyhow!(error), cx);
            return;
        }

        cx.spawn(async move |this, cx| {
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
                        this.finish_sync_secret_save_operation(result, window, cx);
                    }) {
                        log::debug!("failed to apply sync secret save result in window: {error:?}");
                    }
                });

                if let Err(error) = update_result {
                    log::debug!("failed to access active window for sync secret save: {error:?}");
                    if let Some(result) = result.borrow_mut().take()
                        && let Err(error) = this.update(cx, move |this, cx| {
                            this.finish_sync_secret_save_operation_without_window(result, cx);
                        })
                    {
                        log::debug!(
                            "failed to apply sync secret save result without window: {error:?}"
                        );
                    }
                }
            } else if let Err(error) = this.update(cx, move |this, cx| {
                this.finish_sync_secret_save_operation_without_window(result, cx);
            }) {
                log::debug!(
                    "failed to apply sync secret save result without active window: {error:?}"
                );
            }
        })
        .detach();
    }
    pub(super) fn finish_sync_secret_save_spawn_error(
        &mut self,
        operation: SyncSecretSaveOperation,
        error: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        self.sync.sync_secret_save_operation = Some(operation);
        self.finish_sync_secret_save_operation_without_window(
            Err(error.context("failed to spawn sync secret save worker")),
            cx,
        );
    }
    fn finish_sync_secret_save_operation(
        &mut self,
        result: Result<SyncSecretSaveTaskResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let operation = self
            .sync
            .sync_secret_save_operation
            .unwrap_or(SyncSecretSaveOperation::GithubToken);
        self.sync.sync_secret_save_operation = None;

        match result {
            Ok(task_result) => {
                let SyncSecretSaveTaskResult {
                    operation,
                    updated_config,
                } = task_result;
                self.sync.sync_engine.config_store.config = updated_config;
                self.refresh_sync_secret_placeholders(window, cx);
                let field_label = Self::sync_secret_field_label(operation);
                self.notify_sync_secret_saved(window, &field_label, cx);
            }
            Err(error) => {
                let field_label = Self::sync_secret_field_label(operation);
                self.notify_sync_secret_save_failed(window, &field_label, &error.to_string(), cx);
            }
        }
    }
    fn finish_sync_secret_save_operation_without_window(
        &mut self,
        result: Result<SyncSecretSaveTaskResult>,
        cx: &mut Context<Self>,
    ) {
        let operation = self
            .sync
            .sync_secret_save_operation
            .unwrap_or(SyncSecretSaveOperation::GithubToken);
        self.sync.sync_secret_save_operation = None;

        match result {
            Ok(task_result) => {
                let SyncSecretSaveTaskResult {
                    operation,
                    updated_config,
                } = task_result;
                self.sync.sync_engine.config_store.config = updated_config;
                let field_label = Self::sync_secret_field_label(operation);
                self.status_message = i18n::string_args(
                    "settings.sync.save_feedback.saved_message",
                    &[("field", &field_label)],
                );
            }
            Err(error) => {
                let field_label = Self::sync_secret_field_label(operation);
                self.status_message = i18n::string_args(
                    "settings.sync.save_feedback.failed_message",
                    &[("field", &field_label), ("error", &error.to_string())],
                );
            }
        }

        cx.notify();
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
            self.finish_sync_passphrase_spawn_error(operation, anyhow::anyhow!(error), cx);
            return;
        }

        cx.spawn(async move |this, cx| {
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
        })
        .detach();
    }
    pub(super) fn finish_sync_passphrase_spawn_error(
        &mut self,
        operation: SyncPassphraseOperation,
        error: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        self.sync.sync_passphrase_operation = Some(operation);
        self.finish_sync_passphrase_operation_without_window(
            Err(error.context("failed to spawn sync passphrase persistence worker")),
            cx,
        );
    }
    fn finish_sync_passphrase_operation(
        &mut self,
        result: Result<SyncPassphraseTaskResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let operation = self
            .sync
            .sync_passphrase_operation
            .unwrap_or(SyncPassphraseOperation::Save);
        self.sync.sync_passphrase_operation = None;

        match result {
            Ok(task_result) => {
                let SyncPassphraseTaskResult {
                    operation,
                    configured,
                    updated_config,
                } = task_result;
                self.sync.sync_engine.config_store.config = updated_config;
                self.set_sync_passphrase_configured_state(configured);
                self.clear_sync_passphrase_popup_inputs(window, cx);
                self.dismiss_sync_passphrase_popup(cx);

                match operation {
                    SyncPassphraseOperation::Save => {
                        let field_label = i18n::string("settings.sync.encryption.passphrase.label");
                        self.notify_sync_secret_saved(window, &field_label, cx);
                    }
                    SyncPassphraseOperation::Clear => {
                        let message = i18n::string(
                            "settings.sync.encryption.passphrase.notifications.cleared_message",
                        );
                        self.status_message = message.clone();
                        window.push_notification(
                            Self::success_notification(
                                i18n::string(
                                    "settings.sync.encryption.passphrase.notifications.cleared_title",
                                ),
                                message,
                            ),
                            cx,
                        );
                        cx.notify();
                    }
                }
            }
            Err(error) => match operation {
                SyncPassphraseOperation::Save => {
                    let field_label = i18n::string("settings.sync.encryption.passphrase.label");
                    self.notify_sync_secret_save_failed(
                        window,
                        &field_label,
                        &error.to_string(),
                        cx,
                    );
                }
                SyncPassphraseOperation::Clear => {
                    let message = i18n::string_args(
                        "settings.sync.encryption.passphrase.notifications.clear_failed_message",
                        &[("error", &error.to_string())],
                    );
                    self.status_message = message.clone();
                    window.push_notification(
                        Self::error_notification(
                            i18n::string(
                                "settings.sync.encryption.passphrase.notifications.clear_failed_title",
                            ),
                            message,
                        ),
                        cx,
                    );
                    cx.notify();
                }
            },
        }
    }
    fn finish_sync_passphrase_operation_without_window(
        &mut self,
        result: Result<SyncPassphraseTaskResult>,
        cx: &mut Context<Self>,
    ) {
        let operation = self
            .sync
            .sync_passphrase_operation
            .unwrap_or(SyncPassphraseOperation::Save);
        self.sync.sync_passphrase_operation = None;

        match result {
            Ok(task_result) => {
                let SyncPassphraseTaskResult {
                    operation,
                    configured,
                    updated_config,
                } = task_result;
                self.sync.sync_engine.config_store.config = updated_config;
                self.set_sync_passphrase_configured_state(configured);
                self.dismiss_sync_passphrase_popup(cx);
                self.status_message = match operation {
                    SyncPassphraseOperation::Save => i18n::string_args(
                        "settings.sync.save_feedback.saved_message",
                        &[(
                            "field",
                            &i18n::string("settings.sync.encryption.passphrase.label"),
                        )],
                    ),
                    SyncPassphraseOperation::Clear => i18n::string(
                        "settings.sync.encryption.passphrase.notifications.cleared_message",
                    ),
                };
            }
            Err(error) => {
                self.status_message = match operation {
                    SyncPassphraseOperation::Save => i18n::string_args(
                        "settings.sync.save_feedback.failed_message",
                        &[
                            (
                                "field",
                                &i18n::string("settings.sync.encryption.passphrase.label"),
                            ),
                            ("error", &error.to_string()),
                        ],
                    ),
                    SyncPassphraseOperation::Clear => i18n::string_args(
                        "settings.sync.encryption.passphrase.notifications.clear_failed_message",
                        &[("error", &error.to_string())],
                    ),
                };
            }
        }

        cx.notify();
    }
    pub(in crate::ui::shell) fn set_sync_provider(
        &mut self,
        provider: SyncProvider,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = SettingsService::set_sync_provider(&mut self.sync.sync_engine, provider)
        {
            log::warn!("failed to persist sync config: {error:?}");
            return;
        }

        cx.notify();
    }
    pub(super) fn set_sync_passphrase_configured(&mut self, _passphrase: &str) {
        self.sync.sync_passphrase_configured =
            self.sync.sync_engine.config_store.config.has_passphrase;
    }
    pub(super) fn set_sync_passphrase_configured_state(&mut self, configured: bool) {
        self.sync.sync_passphrase_configured = configured;
    }
    pub(super) fn refresh_sync_secret_inputs(&self, window: &mut Window, cx: &mut Context<Self>) {
        let sync_secret_inputs = Self::load_sync_secret_inputs(&self.sync.sync_engine);

        self.apply_sync_secret_inputs(sync_secret_inputs, window, cx);
    }
    pub(super) fn load_sync_secret_inputs(sync_engine: &SyncEngine) -> LocalVaultSyncSecretInputs {
        let sync_secrets = sync_engine
            .config_store
            .get_secrets()
            .unwrap_or_else(|error| {
                log::warn!("failed to refresh sync secret inputs: {error:?}");
                Default::default()
            });

        LocalVaultSyncSecretInputs {
            github_token: sync_secrets.github_token.unwrap_or_default(),
            webdav_password: sync_secrets.webdav_password.unwrap_or_default(),
            sync_passphrase: sync_secrets.passphrase.unwrap_or_default(),
        }
    }
    pub(super) fn apply_sync_secret_inputs(
        &self,
        sync_secret_inputs: LocalVaultSyncSecretInputs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        set_input_value(
            &self.panel_forms.settings.sync_github_token_input,
            sync_secret_inputs.github_token,
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.sync_webdav_password_input,
            sync_secret_inputs.webdav_password,
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.settings.sync_passphrase_input,
            sync_secret_inputs.sync_passphrase,
            window,
            cx,
        );
        self.refresh_sync_secret_placeholders(window, cx);
    }
    pub(super) fn refresh_sync_secret_placeholders(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        set_input_placeholder(
            &self.panel_forms.settings.sync_github_token_input,
            Self::localized_secret_placeholder(
                self.sync.sync_engine.config_store.config.has_github_token,
                "settings.sync.placeholders.github_token",
            ),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.settings.sync_webdav_password_input,
            Self::localized_secret_placeholder(
                self.sync
                    .sync_engine
                    .config_store
                    .config
                    .has_webdav_password,
                "settings.sync.placeholders.webdav_password",
            ),
            window,
            cx,
        );
    }
    pub(in crate::ui::shell) fn passphrase_is_set(&self) -> bool {
        self.sync.sync_passphrase_configured
    }
    pub(in crate::ui::shell) fn notify_passphrase_required_in_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.notify_validation_failure_in_window(
            window,
            ValidationNotificationKind::RequiredInputMissing,
            i18n::string("settings.sync.passphrase_required_error.message"),
            cx,
        );
    }
    pub(in crate::ui::shell) fn notify_sync_toggle_update_failed_in_window(
        &mut self,
        window: &mut Window,
        provider_name: String,
        error: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        let error = error.into();
        let message = i18n::string_args(
            "settings.sync.toggle_failed_error.message",
            &[("provider", &provider_name), ("error", &error)],
        );
        let notification = Self::error_notification(
            i18n::string("settings.sync.toggle_failed_error.title"),
            message.clone(),
        );

        self.status_message = message;
        window.push_notification(notification, cx);
        cx.notify();
    }
}
#[cfg(test)]
mod tests {
    use super::normalize_github_gist_id;

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
}
