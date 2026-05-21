use super::super::*;
use crate::ui::i18n;
use gpui_component::WindowExt as _;
use miaominal_core::keychain::ManagedKeySource;
use miaominal_services::KeychainService;
use std::fs;
use std::path::{Path, PathBuf};

fn keychain_managed_key_noun(count: usize) -> String {
    i18n::string(if count == 1 {
        "keychain.messages.managed_key_noun_one"
    } else {
        "keychain.messages.managed_key_noun_other"
    })
}

fn keychain_agent_identity_noun(count: usize) -> String {
    i18n::string(if count == 1 {
        "keychain.messages.agent_identity_noun_one"
    } else {
        "keychain.messages.agent_identity_noun_other"
    })
}

fn keychain_deploy_exec_command(
    template: &str,
    location: &str,
    filename: &str,
    public_key: &str,
) -> String {
    KeychainService::deploy_command(template, location, filename, public_key)
}

struct ManagedKeyImportAfterUnlockRequest {
    import_name: String,
    source: ManagedKeySource,
    import_path: Option<String>,
    private_key_material: Option<String>,
    public_key_material: Option<String>,
    passphrase: Option<String>,
}

struct ManagedKeyImportAfterUnlockResult {
    record: ManagedKeyRecord,
    updated_keys: Vec<ManagedKeyRecord>,
}

impl AppView {
    fn persist_managed_keys_after_user_change(
        &mut self,
        service: &KeychainService,
        _cx: &mut Context<Self>,
    ) -> Result<()> {
        service.persist_keys(&self.data.managed_keys)?;
        Ok(())
    }

    fn persist_keychain_changes_after_user_change(
        &mut self,
        service: &KeychainService,
        sessions_changed: bool,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        service.persist_keys(&self.data.managed_keys)?;

        if sessions_changed && self.services.session_store.is_some() {
            self.persist_sessions_after_user_change(cx)?;
        }

        Ok(())
    }

    fn keychain_service(&self) -> Option<KeychainService> {
        self.services.keychain_store.clone().map(|store| {
            KeychainService::new(
                self.services.runtime.clone(),
                store,
                self.services.secrets.clone(),
                self.services.known_hosts.clone(),
            )
        })
    }

    fn keychain_deploy_profile_items(sessions: &[SessionProfile]) -> Vec<ForwardProfileSelectItem> {
        let mut items = sessions
            .iter()
            .filter(|profile| Self::keychain_profile_supports_deploy(profile))
            .map(ForwardProfileSelectItem::new)
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            left.title()
                .as_ref()
                .to_ascii_lowercase()
                .cmp(&right.title().as_ref().to_ascii_lowercase())
                .then_with(|| left.value().cmp(right.value()))
        });

        items
    }

    pub(in crate::ui::shell) fn keychain_profile_supports_deploy(profile: &SessionProfile) -> bool {
        KeychainService::profile_supports_deploy(profile)
    }

    pub(in crate::ui::shell) fn keychain_deploy_profile_options(
        sessions: &[SessionProfile],
    ) -> SearchableVec<ForwardProfileSelectItem> {
        SearchableVec::new(Self::keychain_deploy_profile_items(sessions))
    }

    pub(in crate::ui::shell) fn sync_keychain_deploy_profile_select(
        &mut self,
        selected_profile_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = Self::keychain_deploy_profile_items(&self.data.sessions);
        let selected_profile_id = selected_profile_id.map(str::to_string).or_else(|| {
            self.panel_forms
                .keychain
                .deploy_profile_select
                .read(cx)
                .selected_value()
                .cloned()
        });
        let has_selected_profile =
            selected_profile_id
                .as_ref()
                .is_some_and(|selected_profile_id| {
                    items.iter().any(|item| item.value() == selected_profile_id)
                });
        let options = SearchableVec::new(items);

        self.panel_forms
            .keychain
            .deploy_profile_select
            .update(cx, |select, cx| {
                select.set_items(options, window, cx);
                if has_selected_profile {
                    if let Some(selected_profile_id) = selected_profile_id.as_ref() {
                        select.set_selected_value(selected_profile_id, window, cx);
                    }
                } else {
                    select.set_selected_index(None, window, cx);
                }
            });
    }

    pub(in crate::ui::shell) fn keychain_selected_deploy_key(&self) -> Option<&ManagedKeyRecord> {
        self.keychain_deploy_key_id.as_deref().and_then(|key_id| {
            self.data
                .managed_keys
                .iter()
                .find(|key| key.id.as_str() == key_id)
        })
    }

    pub(in crate::ui::shell) fn open_keychain_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_keychain_inputs(window, cx);
        self.keychain_editor_mode = KeychainEditorMode::Import;
        self.editors.keychain_editor_open = true;
        self.status_message = i18n::string("keychain.messages.preparing_new_managed_key");
        cx.notify();
    }

    pub(in crate::ui::shell) fn open_keychain_deploy_editor(
        &mut self,
        key_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((target_key_id, summary)) = self
            .data
            .managed_keys
            .iter()
            .find(|candidate| candidate.id == key_id)
            .map(|key| (key.id.clone(), key.summary()))
        else {
            self.status_message = i18n::string("keychain.messages.not_found");
            cx.notify();
            return;
        };

        self.clear_keychain_inputs(window, cx);
        self.keychain_editor_mode = KeychainEditorMode::Deploy;
        self.keychain_deploy_key_id = Some(target_key_id);
        self.editors.keychain_editor_open = true;
        self.status_message = i18n::string_args(
            "keychain.messages.preparing_deploy",
            &[("summary", &summary)],
        );
        cx.notify();
    }

    pub(in crate::ui::shell) fn close_keychain_editor(&mut self, cx: &mut Context<Self>) {
        if !self.editors.keychain_editor_open {
            return;
        }

        self.editors.keychain_editor_open = false;
        self.keychain_editor_mode = KeychainEditorMode::Import;
        self.keychain_deploy_key_id = None;
        self.status_message = i18n::string("keychain.messages.closed_sidebar");
        cx.notify();
    }

    pub(in crate::ui::shell) fn clear_keychain_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.keychain_editor_draft_source = None;
        self.keychain_editor_mode = KeychainEditorMode::Import;
        self.keychain_deploy_key_id = None;
        set_input_value(&self.panel_forms.keychain.name_input, "", window, cx);
        set_input_value(&self.panel_forms.keychain.import_path_input, "", window, cx);
        set_input_value(
            &self.panel_forms.keychain.import_private_key_input,
            "",
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.keychain.import_public_key_input,
            "",
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.keychain.import_passphrase_input,
            "",
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.keychain.deploy_location_input,
            KEYCHAIN_DEPLOY_DEFAULT_LOCATION,
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.keychain.deploy_filename_input,
            KEYCHAIN_DEPLOY_DEFAULT_FILENAME,
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.keychain.deploy_command_input,
            KEYCHAIN_DEPLOY_DEFAULT_COMMAND,
            window,
            cx,
        );
        self.sync_keychain_deploy_profile_select(None, window, cx);
    }

    pub(in crate::ui::shell) fn set_managed_key_import_file_path(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.keychain_editor_draft_source = Some(ManagedKeySource::Imported);
        set_input_value(
            &self.panel_forms.keychain.import_path_input,
            path.display().to_string(),
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.keychain.import_private_key_input,
            "",
            window,
            cx,
        );
        let path = path.display().to_string();
        self.status_message =
            i18n::string_args("keychain.messages.selected_import_file", &[("path", &path)]);
        cx.notify();
    }

    pub(in crate::ui::shell) fn refresh_keychain_data(&mut self, cx: &mut Context<Self>) {
        let selected_managed_key_id = self
            .host_editor_forms
            .managed_key_select
            .read(cx)
            .selected_value()
            .cloned();
        if let Some(service) = self.keychain_service() {
            match service.refresh_data() {
                Ok(snapshot) => {
                    self.data.managed_keys = snapshot.managed_keys;
                    self.data.agent_identities = snapshot.agent_identities;
                    if let Some(error) = snapshot.agent_scan_error {
                        let managed_count = self.data.managed_keys.len().to_string();
                        let managed_keys_label =
                            keychain_managed_key_noun(self.data.managed_keys.len());
                        self.status_message = i18n::string_args(
                            "keychain.messages.loaded_agent_scan_failed",
                            &[
                                ("managed_count", &managed_count),
                                ("managed_keys_label", &managed_keys_label),
                                ("error", &error),
                            ],
                        );
                    } else {
                        let managed_count = self.data.managed_keys.len().to_string();
                        let managed_keys_label =
                            keychain_managed_key_noun(self.data.managed_keys.len());
                        let agent_count = self.data.agent_identities.len().to_string();
                        let agent_identities_label =
                            keychain_agent_identity_noun(self.data.agent_identities.len());
                        self.status_message = i18n::string_args(
                            "keychain.messages.loaded",
                            &[
                                ("managed_count", &managed_count),
                                ("managed_keys_label", &managed_keys_label),
                                ("agent_count", &agent_count),
                                ("agent_identities_label", &agent_identities_label),
                            ],
                        );
                    }
                }
                Err(error) => {
                    self.data.managed_keys.clear();
                    self.data.agent_identities.clear();
                    let error = error.to_string();
                    self.status_message =
                        i18n::string_args("keychain.messages.load_failed", &[("error", &error)]);
                    cx.notify();
                    return;
                }
            }
        } else {
            self.data.managed_keys.clear();
            match self
                .services
                .runtime
                .block_on(miaominal_ssh::list_local_agent_identities())
            {
                Ok(identities) => {
                    self.data.agent_identities = identities;
                    let agent_count = self.data.agent_identities.len().to_string();
                    let agent_identities_label =
                        keychain_agent_identity_noun(self.data.agent_identities.len());
                    self.status_message = i18n::string_args(
                        "keychain.messages.loaded",
                        &[
                            ("managed_count", "0"),
                            ("managed_keys_label", &keychain_managed_key_noun(0)),
                            ("agent_count", &agent_count),
                            ("agent_identities_label", &agent_identities_label),
                        ],
                    );
                }
                Err(error) => {
                    self.data.agent_identities.clear();
                    self.status_message = i18n::string_args(
                        "keychain.messages.loaded_agent_scan_failed",
                        &[
                            ("managed_count", "0"),
                            ("managed_keys_label", &keychain_managed_key_noun(0)),
                            ("error", &error.to_string()),
                        ],
                    );
                }
            }
        }

        self.sync_managed_key_select_in_active_window(selected_managed_key_id.as_deref(), cx);

        cx.notify();
    }

    pub(in crate::ui::shell) fn request_managed_key_delete(
        &mut self,
        key_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.data.managed_keys.iter().find(|key| key.id == key_id) else {
            self.status_message = i18n::string("keychain.messages.not_found");
            cx.notify();
            return;
        };

        self.dialogs.pending_managed_key_delete = Some(PendingManagedKeyDeleteState {
            key_id: key.id.clone(),
            key_name: key.name.clone(),
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn confirm_managed_key_delete(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.dialogs.pending_managed_key_delete.take() else {
            return;
        };

        self.start_dialog_exit(DialogOverlaySnapshot::ManagedKeyDelete(pending.clone()), cx);

        self.delete_managed_key(&pending.key_id, window, cx);
    }

    pub(in crate::ui::shell) fn cancel_managed_key_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.dialogs.pending_managed_key_delete.take() {
            self.start_dialog_exit(DialogOverlaySnapshot::ManagedKeyDelete(pending), cx);
        }
    }

    pub(in crate::ui::shell) fn generate_managed_key(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let generated_material = self
            .keychain_service()
            .map_or_else(KeychainService::generate_ed25519_material, |service| {
                service.generate_material()
            });

        match generated_material {
            Ok((private_key_material, public_key_material)) => {
                self.keychain_editor_draft_source = Some(ManagedKeySource::Generated);
                set_input_value(&self.panel_forms.keychain.import_path_input, "", window, cx);
                set_input_value(
                    &self.panel_forms.keychain.import_private_key_input,
                    &private_key_material,
                    window,
                    cx,
                );
                set_input_value(
                    &self.panel_forms.keychain.import_public_key_input,
                    &public_key_material,
                    window,
                    cx,
                );
                set_input_value(
                    &self.panel_forms.keychain.import_passphrase_input,
                    "",
                    window,
                    cx,
                );
                self.status_message = i18n::string("keychain.messages.generated");
            }
            Err(error) => {
                let error = error.to_string();
                self.status_message =
                    i18n::string_args("keychain.messages.generation_failed", &[("error", &error)]);
            }
        }

        cx.notify();
    }

    pub(in crate::ui::shell) fn import_managed_key(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(service) = self.keychain_service() else {
            self.status_message = i18n::string("keychain.messages.storage_unavailable");
            cx.notify();
            return;
        };

        let import_path = self
            .panel_forms
            .keychain
            .import_path_input
            .read(cx)
            .value()
            .to_string();
        let import_private_key = self
            .panel_forms
            .keychain
            .import_private_key_input
            .read(cx)
            .value()
            .to_string();
        let import_public_key = self
            .panel_forms
            .keychain
            .import_public_key_input
            .read(cx)
            .value()
            .to_string();
        let passphrase = self
            .panel_forms
            .keychain
            .import_passphrase_input
            .read(cx)
            .value()
            .to_string();
        let import_path = import_path.trim().to_string();
        let has_pasted_private_key = !import_private_key.trim().is_empty();
        if import_path.is_empty() && !has_pasted_private_key {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("keychain.messages.import_private_key_required"),
                cx,
            );
            return;
        }

        let import_name = self
            .panel_forms
            .keychain
            .name_input
            .read(cx)
            .value()
            .to_string();
        let default_import_name = i18n::string("keychain.messages.imported_key_default_name");
        let import_name = if import_name.trim().is_empty() {
            if has_pasted_private_key {
                default_import_name.clone()
            } else {
                Path::new(&import_path)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or(default_import_name.as_str())
                    .to_string()
            }
        } else {
            import_name.trim().to_string()
        };

        let private_key_material = if has_pasted_private_key {
            import_private_key
        } else {
            match fs::read_to_string(&import_path) {
                Ok(content) => content,
                Err(error) => {
                    let error = error.to_string();
                    let message =
                        i18n::string_args("keychain.messages.import_failed", &[("error", &error)]);
                    self.notify_validation_failure_in_window(
                        window,
                        ValidationNotificationKind::InvalidInput,
                        message,
                        cx,
                    );
                    return;
                }
            }
        };

        let passphrase = (!passphrase.trim().is_empty()).then_some(passphrase.trim());
        let public_key_material =
            (!import_public_key.trim().is_empty()).then_some(import_public_key.trim());
        let source = self
            .keychain_editor_draft_source
            .unwrap_or(ManagedKeySource::Imported);

        if self.local_vault_status == LocalVaultStatus::Locked {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::ImportManagedKey,
                window,
                cx,
            );
            return;
        }

        match service.import_key(
            &self.data.managed_keys,
            import_name,
            source,
            &private_key_material,
            public_key_material,
            passphrase,
        ) {
            Ok(imported) => {
                self.data.managed_keys.push(imported.record.clone());
                if let Err(error) = self.persist_managed_keys_after_user_change(&service, cx) {
                    self.data
                        .managed_keys
                        .retain(|key| key.id != imported.record.id);
                    self.services
                        .secrets
                        .delete_managed_key(&imported.record.id);
                    let error = error.to_string();
                    let message =
                        i18n::string_args("keychain.messages.import_failed", &[("error", &error)]);
                    self.notify_validation_failure_in_window(
                        window,
                        ValidationNotificationKind::InvalidInput,
                        message,
                        cx,
                    );
                    return;
                } else {
                    self.clear_keychain_inputs(window, cx);
                    self.sync_managed_key_select(None, window, cx);
                    self.editors.keychain_editor_open = false;
                    let summary = imported.record.summary();
                    self.status_message =
                        i18n::string_args("keychain.messages.imported", &[("summary", &summary)]);
                }
            }
            Err(error) => {
                let error = error.to_string();
                let message =
                    i18n::string_args("keychain.messages.import_failed", &[("error", &error)]);
                self.notify_validation_failure_in_window(
                    window,
                    ValidationNotificationKind::InvalidInput,
                    message,
                    cx,
                );
                return;
            }
        }

        cx.notify();
    }

    fn build_managed_key_import_after_unlock_request(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<ManagedKeyImportAfterUnlockRequest> {
        let import_path = self
            .panel_forms
            .keychain
            .import_path_input
            .read(cx)
            .value()
            .to_string();
        let import_private_key = self
            .panel_forms
            .keychain
            .import_private_key_input
            .read(cx)
            .value()
            .to_string();
        let import_public_key = self
            .panel_forms
            .keychain
            .import_public_key_input
            .read(cx)
            .value()
            .to_string();
        let passphrase = self
            .panel_forms
            .keychain
            .import_passphrase_input
            .read(cx)
            .value()
            .to_string();
        let import_path = import_path.trim().to_string();
        let has_pasted_private_key = !import_private_key.trim().is_empty();
        if import_path.is_empty() && !has_pasted_private_key {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("keychain.messages.import_private_key_required"),
                cx,
            );
            return None;
        }

        let import_name = self
            .panel_forms
            .keychain
            .name_input
            .read(cx)
            .value()
            .to_string();
        let default_import_name = i18n::string("keychain.messages.imported_key_default_name");
        let import_name = if import_name.trim().is_empty() {
            if has_pasted_private_key {
                default_import_name.clone()
            } else {
                Path::new(&import_path)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or(default_import_name.as_str())
                    .to_string()
            }
        } else {
            import_name.trim().to_string()
        };

        let source = self
            .keychain_editor_draft_source
            .unwrap_or(ManagedKeySource::Imported);

        Some(ManagedKeyImportAfterUnlockRequest {
            import_name,
            source,
            import_path: (!import_path.is_empty()).then_some(import_path),
            private_key_material: has_pasted_private_key.then_some(import_private_key),
            public_key_material: (!import_public_key.trim().is_empty())
                .then(|| import_public_key.trim().to_string()),
            passphrase: (!passphrase.trim().is_empty()).then(|| passphrase.trim().to_string()),
        })
    }

    pub(in crate::ui::shell) fn continue_import_managed_key_after_unlock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(service) = self.keychain_service() else {
            self.status_message = i18n::string("keychain.messages.storage_unavailable");
            cx.notify();
            return;
        };

        let Some(request) = self.build_managed_key_import_after_unlock_request(window, cx) else {
            return;
        };

        let existing_keys = self.data.managed_keys.clone();
        let secrets = self.services.secrets.clone();
        let notification_window = cx.active_window();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name("post-unlock-key-import".to_string())
            .spawn(move || {
                let result = (|| -> Result<ManagedKeyImportAfterUnlockResult> {
                    let private_key_material =
                        if let Some(private_key_material) = request.private_key_material.as_ref() {
                            private_key_material.clone()
                        } else if let Some(import_path) = request.import_path.as_ref() {
                            fs::read_to_string(import_path)?
                        } else {
                            String::new()
                        };

                    let imported = service.import_key(
                        &existing_keys,
                        request.import_name,
                        request.source,
                        &private_key_material,
                        request.public_key_material.as_deref(),
                        request.passphrase.as_deref(),
                    )?;

                    let mut updated_keys = existing_keys.clone();
                    updated_keys.push(imported.record.clone());

                    if let Err(error) = service.persist_keys(&updated_keys) {
                        secrets.delete_managed_key(&imported.record.id);
                        return Err(error);
                    }

                    Ok(ManagedKeyImportAfterUnlockResult {
                        record: imported.record,
                        updated_keys,
                    })
                })();

                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            let message = i18n::string_args(
                "keychain.messages.import_failed",
                &[("error", &error.to_string())],
            );
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::InvalidInput,
                message,
                cx,
            );
            return;
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv().unwrap_or_else(|_| {
                        Err(anyhow!("post-unlock managed key import task cancelled"))
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

                    if let Err(error) = this_for_window.update(cx, move |this, cx| match result {
                        Ok(result) => {
                            this.data.managed_keys = result.updated_keys;
                            this.clear_keychain_inputs(window, cx);
                            this.sync_managed_key_select(None, window, cx);
                            this.editors.keychain_editor_open = false;
                            let summary = result.record.summary();
                            this.status_message =
                                i18n::string_args("keychain.messages.imported", &[("summary", &summary)]);
                            cx.notify();
                        }
                        Err(error) => {
                            let message = i18n::string_args(
                                "keychain.messages.import_failed",
                                &[("error", &error.to_string())],
                            );
                            this.notify_validation_failure_in_window(
                                window,
                                ValidationNotificationKind::InvalidInput,
                                message,
                                cx,
                            );
                        }
                    }) {
                        log::debug!(
                            "failed to apply post-unlock managed key import in window: {error:?}"
                        );
                    }
                });

                if let Err(error) = update_result {
                    log::debug!(
                        "failed to access active window for post-unlock managed key import: {error:?}"
                    );
                }
            }
        })
        .detach();
    }

    pub(in crate::ui::shell) fn deploy_managed_key(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((key_summary, key_public_key)) = self
            .keychain_selected_deploy_key()
            .map(|key| (key.summary(), key.public_key.trim().to_string()))
        else {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("keychain.deploy.key_missing"),
                cx,
            );
            return;
        };

        if key_public_key.is_empty() {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::InvalidInput,
                i18n::string("keychain.messages.deploy_public_key_missing"),
                cx,
            );
            return;
        }

        let Some(selected_profile_id) = self
            .panel_forms
            .keychain
            .deploy_profile_select
            .read(cx)
            .selected_value()
            .cloned()
        else {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("keychain.messages.deploy_profile_required"),
                cx,
            );
            return;
        };

        let location = self
            .panel_forms
            .keychain
            .deploy_location_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if location.is_empty() {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("keychain.messages.deploy_location_required"),
                cx,
            );
            return;
        }

        let filename = self
            .panel_forms
            .keychain
            .deploy_filename_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if filename.is_empty() {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("keychain.messages.deploy_filename_required"),
                cx,
            );
            return;
        }

        let command_template = self
            .panel_forms
            .keychain
            .deploy_command_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let command_template = if command_template.is_empty() {
            KEYCHAIN_DEPLOY_DEFAULT_COMMAND.to_string()
        } else {
            command_template
        };

        let Some(profile) = self
            .data
            .sessions
            .iter()
            .find(|profile| profile.id == selected_profile_id)
            .cloned()
        else {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::InvalidInput,
                i18n::string("keychain.messages.deploy_profile_not_found"),
                cx,
            );
            return;
        };

        if !Self::keychain_profile_supports_deploy(&profile) {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::InvalidInput,
                i18n::string("keychain.messages.deploy_profile_unsupported"),
                cx,
            );
            return;
        }

        if self.profile_requires_local_vault_unlock(&profile) {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::DeployManagedKey,
                window,
                cx,
            );
            return;
        }

        let profile_label = profile.connection_label();
        let command =
            keychain_deploy_exec_command(&command_template, &location, &filename, &key_public_key);
        let all_profiles = self.data.sessions.clone();
        let runtime = self.services.runtime.clone();
        let Some(service) = self.keychain_service() else {
            self.status_message = i18n::string("keychain.messages.storage_unavailable");
            cx.notify();
            return;
        };
        let start_summary = key_summary.clone();
        let start_profile = profile_label.clone();
        self.keychain_deploy_in_progress = true;
        self.status_message = i18n::string_args(
            "keychain.messages.deploy_started",
            &[("summary", &start_summary), ("profile", &start_profile)],
        );
        cx.notify();

        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name(format!("key-deploy-{}", selected_profile_id))
            .spawn(move || {
                let outcome =
                    runtime.block_on(service.execute_deploy(profile, all_profiles, command));
                tx.send(outcome).ok();
            })
            .expect("failed to spawn key deploy thread");

        let success_summary = key_summary.clone();
        let success_profile = profile_label.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv()
                        .unwrap_or_else(|_| Err(anyhow!("key deployment task cancelled")))
                })
                .await;

            this.update(cx, |this, cx| {
                this.keychain_deploy_in_progress = false;
                match result {
                    Ok(_) => {
                        let message = i18n::string_args(
                            "keychain.messages.deployed",
                            &[("summary", &success_summary), ("profile", &success_profile)],
                        );
                        let notification = Self::success_notification(
                            i18n::string("keychain.notifications.deploy_succeeded_title"),
                            message.clone(),
                        );
                        this.status_message = message;
                        this.with_active_window(cx, move |window, cx| {
                            window.push_notification(notification, cx);
                        });
                    }
                    Err(error) => {
                        let error = error.to_string();
                        let message = i18n::string_args(
                            "keychain.messages.deploy_failed",
                            &[
                                ("summary", &success_summary),
                                ("profile", &success_profile),
                                ("error", &error),
                            ],
                        );
                        let notification = Self::error_notification(
                            i18n::string("keychain.notifications.deploy_failed_title"),
                            message.clone(),
                        );
                        this.status_message = message;
                        this.with_active_window(cx, move |window, cx| {
                            window.push_notification(notification, cx);
                        });
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(in crate::ui::shell) fn delete_managed_key(
        &mut self,
        key_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(service) = self.keychain_service() else {
            self.status_message = i18n::string("keychain.messages.storage_unavailable");
            cx.notify();
            return;
        };

        let Some(outcome) =
            service.delete_key(&mut self.data.managed_keys, &mut self.data.sessions, key_id)
        else {
            self.status_message = i18n::string("keychain.messages.not_found");
            cx.notify();
            return;
        };

        if self.keychain_deploy_key_id.as_deref() == Some(outcome.removed.id.as_str()) {
            self.keychain_deploy_key_id = None;
            self.keychain_editor_mode = KeychainEditorMode::Import;
            self.editors.keychain_editor_open = false;
        }

        if let Err(error) = self.persist_keychain_changes_after_user_change(
            &service,
            !outcome.cleared_profile_ids.is_empty(),
            cx,
        ) {
            let error = error.to_string();
            self.status_message = if !outcome.cleared_profile_ids.is_empty()
                && self.services.session_store.is_some()
            {
                i18n::string_args(
                    "keychain.messages.removed_locally_session_save_failed",
                    &[("error", &error)],
                )
            } else {
                i18n::string_args(
                    "keychain.messages.removed_locally_save_failed",
                    &[("error", &error)],
                )
            };
            cx.notify();
            return;
        }

        if self
            .host_editor_forms
            .managed_key_select
            .read(cx)
            .selected_value()
            .is_some_and(|selected| selected == &outcome.removed.id)
        {
            self.sync_managed_key_select(None, window, cx);
            if self.host_editor_forms.editing_auth_method == AuthMethod::ManagedKey {
                self.host_editor_forms.editing_auth_method = AuthMethod::Password;
            }
        } else {
            self.sync_managed_key_select(None, window, cx);
        }

        let summary = outcome.removed.summary();
        self.status_message =
            i18n::string_args("keychain.messages.removed", &[("summary", &summary)]);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deploy_exec_command_uses_positional_arguments() {
        let command = KeychainService::deploy_command(
            "echo $1/$2/$3",
            ".ssh",
            "authorized_keys",
            "ssh-ed25519 AAAA",
        );

        assert_eq!(
            command,
            "sh -lc 'echo $1/$2/$3' gpui-keychain-deploy '.ssh' 'authorized_keys' 'ssh-ed25519 AAAA'"
        );
    }

    #[test]
    fn deploy_exec_command_escapes_single_quotes() {
        let command =
            KeychainService::deploy_command("echo '$3'", "/tmp/o'clock", "keys", "ssh 'key'");

        assert!(command.contains("'echo '\"'\"'$3'\"'\"''"));
        assert!(command.contains("'/tmp/o'\"'\"'clock'"));
        assert!(command.contains("'ssh '\"'\"'key'\"'\"''"));
    }
}
