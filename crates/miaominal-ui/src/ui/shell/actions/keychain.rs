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

fn managed_key_name_exists(keys: &[ManagedKeyRecord], candidate: &str) -> bool {
    keys.iter()
        .any(|key| key.name.trim().eq_ignore_ascii_case(candidate.trim()))
}

fn unique_managed_key_name(keys: &[ManagedKeyRecord], base: &str, always_numbered: bool) -> String {
    let base = base.trim();
    if !always_numbered && !managed_key_name_exists(keys, base) {
        return base.to_string();
    }

    let mut suffix = if always_numbered { 1 } else { 2 };
    loop {
        let candidate = format!("{base} {suffix}");
        if !managed_key_name_exists(keys, &candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

fn resolve_managed_key_import_name(
    keys: &[ManagedKeyRecord],
    requested_name: &str,
    import_path: &str,
    has_pasted_private_key: bool,
    source: ManagedKeySource,
    imported_default_name: &str,
    generated_default_name: &str,
) -> String {
    let requested_name = requested_name.trim();
    if !requested_name.is_empty() {
        return requested_name.to_string();
    }

    if !has_pasted_private_key {
        let file_name = Path::new(import_path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::trim)
            .filter(|stem| !stem.is_empty());
        if let Some(file_name) = file_name {
            return unique_managed_key_name(keys, file_name, false);
        }
    }

    let default_name = match source {
        ManagedKeySource::Generated => generated_default_name,
        ManagedKeySource::Imported => imported_default_name,
    };
    unique_managed_key_name(keys, default_name, true)
}

fn apply_managed_key_rename(
    keys: &mut [ManagedKeyRecord],
    key_id: &str,
    new_name: &str,
) -> Option<(usize, String)> {
    let index = keys.iter().position(|key| key.id == key_id)?;
    let old_name = std::mem::replace(&mut keys[index].name, new_name.to_string());
    Some((index, old_name))
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

impl KeychainController {
    fn persist_managed_keys_after_user_change(
        &mut self,
        service: &KeychainService,
        _cx: &mut Context<Self>,
    ) -> Result<()> {
        service.persist_keys(&self.managed_keys)?;
        Ok(())
    }

    fn keychain_service(&self) -> Option<KeychainService> {
        self.keychain_store.clone().map(|store| {
            KeychainService::new(
                self.runtime.clone(),
                store,
                self.secrets.clone(),
                self.known_hosts.clone(),
            )
        })
    }

    fn profile_requires_local_vault_unlock(&self, profile: &SessionProfile) -> bool {
        if self.local_vault_status != LocalVaultStatus::Locked {
            return false;
        }

        match profile.effective_auth_method() {
            AuthMethod::Password => profile.password.is_empty() && profile.has_stored_password,
            AuthMethod::KeyFile => profile.passphrase.is_empty() && profile.has_stored_passphrase,
            AuthMethod::ManagedKey => !profile.managed_key_id.trim().is_empty(),
            AuthMethod::Agent | AuthMethod::KeyboardInteractive => false,
        }
    }

    fn notify_validation_failure_in_window(
        &mut self,
        window: &mut Window,
        kind: ValidationNotificationKind,
        message: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        let message = message.into();
        self.status_message = message.clone();
        window.push_notification(validation_notification(kind, message), cx);
        cx.notify();
    }

    fn with_active_window(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut Window, &mut App) + 'static,
    ) {
        let Some(window_handle) = cx.active_window() else {
            return;
        };
        let _ = window_handle.update(cx, move |_, window, cx| {
            update(window, cx);
        });
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
        let profiles = self.session_query.profiles();
        let items = Self::keychain_deploy_profile_items(&profiles);
        let selected_profile_id = selected_profile_id.map(str::to_string).or_else(|| {
            self.forms
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

        self.forms.deploy_profile_select.update(cx, |select, cx| {
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
        self.deploy_key_id.as_deref().and_then(|key_id| {
            self.managed_keys
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
        self.editor_mode = KeychainEditorMode::Import;
        self.editor_open = true;
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
        self.editor_mode = KeychainEditorMode::Deploy;
        self.deploy_key_id = Some(target_key_id);
        self.editor_open = true;
        self.status_message = i18n::string_args(
            "keychain.messages.preparing_deploy",
            &[("summary", &summary)],
        );
        cx.notify();
    }

    pub(in crate::ui::shell) fn close_keychain_editor(&mut self, cx: &mut Context<Self>) {
        if !self.editor_open {
            return;
        }

        self.editor_open = false;
        self.editor_mode = KeychainEditorMode::Import;
        self.deploy_key_id = None;
        self.status_message = i18n::string("keychain.messages.closed_sidebar");
        cx.notify();
    }

    pub(in crate::ui::shell) fn clear_keychain_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor_draft_source = None;
        self.editor_mode = KeychainEditorMode::Import;
        self.deploy_key_id = None;
        set_input_value(&self.forms.name_input, "", window, cx);
        set_input_value(&self.forms.import_path_input, "", window, cx);
        set_input_value(&self.forms.import_private_key_input, "", window, cx);
        set_input_value(&self.forms.import_public_key_input, "", window, cx);
        set_input_value(&self.forms.import_passphrase_input, "", window, cx);
        set_input_value(
            &self.forms.deploy_location_input,
            KEYCHAIN_DEPLOY_DEFAULT_LOCATION,
            window,
            cx,
        );
        set_input_value(
            &self.forms.deploy_filename_input,
            KEYCHAIN_DEPLOY_DEFAULT_FILENAME,
            window,
            cx,
        );
        set_input_value(
            &self.forms.deploy_command_input,
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
        self.editor_draft_source = Some(ManagedKeySource::Imported);
        set_input_value(
            &self.forms.import_path_input,
            path.display().to_string(),
            window,
            cx,
        );
        set_input_value(&self.forms.import_private_key_input, "", window, cx);
        let path = path.display().to_string();
        self.status_message =
            i18n::string_args("keychain.messages.selected_import_file", &[("path", &path)]);
        cx.notify();
    }

    pub(in crate::ui::shell) fn refresh_keychain_data(&mut self, cx: &mut Context<Self>) {
        if let Some(service) = self.keychain_service() {
            match service.refresh_data() {
                Ok(snapshot) => {
                    self.managed_keys = snapshot.managed_keys;
                    self.agent_identities = snapshot.agent_identities;
                    if let Some(error) = snapshot.agent_scan_error {
                        let managed_count = self.managed_keys.len().to_string();
                        let managed_keys_label = keychain_managed_key_noun(self.managed_keys.len());
                        self.status_message = i18n::string_args(
                            "keychain.messages.loaded_agent_scan_failed",
                            &[
                                ("managed_count", &managed_count),
                                ("managed_keys_label", &managed_keys_label),
                                ("error", &error),
                            ],
                        );
                    } else {
                        let managed_count = self.managed_keys.len().to_string();
                        let managed_keys_label = keychain_managed_key_noun(self.managed_keys.len());
                        let agent_count = self.agent_identities.len().to_string();
                        let agent_identities_label =
                            keychain_agent_identity_noun(self.agent_identities.len());
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
                    self.managed_keys.clear();
                    self.agent_identities.clear();
                    let error = error.to_string();
                    self.status_message =
                        i18n::string_args("keychain.messages.load_failed", &[("error", &error)]);
                    cx.emit(AppCommand::ManagedKeysChanged(ManagedKeysChange::Reloaded));
                    cx.notify();
                    return;
                }
            }
        } else {
            self.managed_keys.clear();
            match self
                .runtime
                .block_on(miaominal_ssh::list_local_agent_identities())
            {
                Ok(identities) => {
                    self.agent_identities = identities;
                    let agent_count = self.agent_identities.len().to_string();
                    let agent_identities_label =
                        keychain_agent_identity_noun(self.agent_identities.len());
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
                    self.agent_identities.clear();
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

        cx.emit(AppCommand::ManagedKeysChanged(ManagedKeysChange::Reloaded));
        cx.notify();
    }

    pub(in crate::ui::shell) fn request_managed_key_delete(
        &mut self,
        key_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.managed_keys.iter().find(|key| key.id == key_id) else {
            self.status_message = i18n::string("keychain.messages.not_found");
            cx.notify();
            return;
        };

        self.pending_managed_key_delete = Some(PendingManagedKeyDeleteState {
            key_id: key.id.clone(),
            key_name: key.name.clone(),
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn request_managed_key_rename(
        &mut self,
        key_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.managed_keys.iter().find(|key| key.id == key_id) else {
            self.status_message = i18n::string("keychain.messages.not_found");
            cx.notify();
            return;
        };

        let current_name = key.name.clone();
        set_input_value(&self.forms.rename_name_input, &current_name, window, cx);
        self.forms
            .rename_name_input
            .update(cx, |input, cx| input.focus(window, cx));
        self.pending_managed_key_rename = Some(PendingManagedKeyRenameState {
            key_id: key.id.clone(),
            current_name,
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn cancel_managed_key_rename(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.pending_managed_key_rename.take() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::ManagedKeyRename(pending),
            ));
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn confirm_managed_key_rename(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending_managed_key_rename.clone() else {
            return;
        };
        let new_name = self
            .forms
            .rename_name_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if new_name.is_empty() {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("keychain.messages.rename_name_required"),
                cx,
            );
            return;
        }

        let Some(index) = self
            .managed_keys
            .iter()
            .position(|key| key.id == pending.key_id)
        else {
            self.pending_managed_key_rename = None;
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::ManagedKeyRename(pending),
            ));
            self.status_message = i18n::string("keychain.messages.not_found");
            cx.notify();
            return;
        };

        let old_name = self.managed_keys[index].name.clone();
        if old_name == new_name {
            self.pending_managed_key_rename = None;
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::ManagedKeyRename(pending),
            ));
            self.status_message = i18n::string("keychain.messages.rename_unchanged");
            cx.notify();
            return;
        }

        let Some(service) = self.keychain_service() else {
            self.status_message = i18n::string("keychain.messages.storage_unavailable");
            cx.notify();
            return;
        };

        let Some((index, old_name)) =
            apply_managed_key_rename(&mut self.managed_keys, &pending.key_id, &new_name)
        else {
            self.status_message = i18n::string("keychain.messages.not_found");
            cx.notify();
            return;
        };
        if let Err(error) = self.persist_managed_keys_after_user_change(&service, cx) {
            self.managed_keys[index].name = old_name;
            let message = i18n::string_args(
                "keychain.messages.rename_failed",
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

        self.pending_managed_key_rename = None;
        cx.emit(AppCommand::OverlayDismissed(
            DialogOverlaySnapshot::ManagedKeyRename(pending),
        ));
        self.status_message =
            i18n::string_args("keychain.messages.renamed", &[("name", &new_name)]);
        cx.emit(AppCommand::ManagedKeysChanged(ManagedKeysChange::Reloaded));
        cx.notify();
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
                self.editor_draft_source = Some(ManagedKeySource::Generated);
                set_input_value(&self.forms.import_path_input, "", window, cx);
                set_input_value(
                    &self.forms.import_private_key_input,
                    &private_key_material,
                    window,
                    cx,
                );
                set_input_value(
                    &self.forms.import_public_key_input,
                    &public_key_material,
                    window,
                    cx,
                );
                set_input_value(&self.forms.import_passphrase_input, "", window, cx);
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

        let import_path = self.forms.import_path_input.read(cx).value().to_string();
        let import_private_key = self
            .forms
            .import_private_key_input
            .read(cx)
            .value()
            .to_string();
        let import_public_key = self
            .forms
            .import_public_key_input
            .read(cx)
            .value()
            .to_string();
        let passphrase = self
            .forms
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

        let source = self
            .editor_draft_source
            .unwrap_or(ManagedKeySource::Imported);
        let import_name = self.forms.name_input.read(cx).value().to_string();
        let default_import_name = i18n::string("keychain.messages.imported_key_default_name");
        let default_generated_name = i18n::string("keychain.messages.generated_key_default_name");
        let import_name = resolve_managed_key_import_name(
            &self.managed_keys,
            &import_name,
            &import_path,
            has_pasted_private_key,
            source,
            &default_import_name,
            &default_generated_name,
        );

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
        if self.local_vault_status == LocalVaultStatus::Locked {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Keychain(
                KeychainDeferredCommand::ImportManagedKey,
            )));
            return;
        }

        match service.import_key(
            &self.managed_keys,
            import_name,
            source,
            &private_key_material,
            public_key_material,
            passphrase,
        ) {
            Ok(imported) => {
                self.managed_keys.push(imported.record.clone());
                if let Err(error) = self.persist_managed_keys_after_user_change(&service, cx) {
                    self.managed_keys.retain(|key| key.id != imported.record.id);
                    self.secrets.delete_managed_key(&imported.record.id);
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
                    self.editor_open = false;
                    let summary = imported.record.summary();
                    self.status_message =
                        i18n::string_args("keychain.messages.imported", &[("summary", &summary)]);
                    cx.emit(AppCommand::ManagedKeysChanged(ManagedKeysChange::Added));
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
        let import_path = self.forms.import_path_input.read(cx).value().to_string();
        let import_private_key = self
            .forms
            .import_private_key_input
            .read(cx)
            .value()
            .to_string();
        let import_public_key = self
            .forms
            .import_public_key_input
            .read(cx)
            .value()
            .to_string();
        let passphrase = self
            .forms
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

        let source = self
            .editor_draft_source
            .unwrap_or(ManagedKeySource::Imported);
        let import_name = self.forms.name_input.read(cx).value().to_string();
        let default_import_name = i18n::string("keychain.messages.imported_key_default_name");
        let default_generated_name = i18n::string("keychain.messages.generated_key_default_name");
        let import_name = resolve_managed_key_import_name(
            &self.managed_keys,
            &import_name,
            &import_path,
            has_pasted_private_key,
            source,
            &default_import_name,
            &default_generated_name,
        );

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

        let existing_keys = self.managed_keys.clone();
        let secrets = self.secrets.clone();
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

        self.import_task = Some(cx.spawn(async move |this, cx| {
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
                            this.managed_keys = result.updated_keys;
                            this.clear_keychain_inputs(window, cx);
                            this.editor_open = false;
                            let summary = result.record.summary();
                            this.status_message =
                                i18n::string_args("keychain.messages.imported", &[("summary", &summary)]);
                            cx.emit(AppCommand::ManagedKeysChanged(ManagedKeysChange::Added));
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
        }));
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
            .forms
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
            .forms
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
            .forms
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
            .forms
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

        let profiles = self.session_query.profiles();
        let Some(profile) = profiles
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
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Keychain(
                KeychainDeferredCommand::DeployManagedKey,
            )));
            return;
        }

        let profile_label = profile.connection_label();
        let command =
            keychain_deploy_exec_command(&command_template, &location, &filename, &key_public_key);
        let all_profiles = profiles;
        let runtime = self.runtime.clone();
        let Some(service) = self.keychain_service() else {
            self.status_message = i18n::string("keychain.messages.storage_unavailable");
            cx.notify();
            return;
        };
        let start_summary = key_summary.clone();
        let start_profile = profile_label.clone();
        self.deploy_in_progress = true;
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
        self.deploy_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv()
                        .unwrap_or_else(|_| Err(anyhow!("key deployment task cancelled")))
                })
                .await;

            this.update(cx, |this, cx| {
                this.deploy_in_progress = false;
                match result {
                    Ok(_) => {
                        let message = i18n::string_args(
                            "keychain.messages.deployed",
                            &[("summary", &success_summary), ("profile", &success_profile)],
                        );
                        let notification = success_notification(
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
                        let notification = error_notification(
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
        }));
    }

    pub(in crate::ui::shell) fn delete_managed_key(
        &mut self,
        key_id: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(service) = self.keychain_service() else {
            self.status_message = i18n::string("keychain.messages.storage_unavailable");
            cx.notify();
            return;
        };

        let Some(removed) = service.delete_key_record(&mut self.managed_keys, key_id) else {
            self.status_message = i18n::string("keychain.messages.not_found");
            cx.notify();
            return;
        };
        let removed_id = removed.id.clone();

        if self.deploy_key_id.as_deref() == Some(removed.id.as_str()) {
            self.deploy_key_id = None;
            self.editor_mode = KeychainEditorMode::Import;
            self.editor_open = false;
        }

        if let Err(error) = self.persist_managed_keys_after_user_change(&service, cx) {
            let error = error.to_string();
            self.status_message = i18n::string_args(
                "keychain.messages.removed_locally_save_failed",
                &[("error", &error)],
            );
            cx.emit(AppCommand::ManagedKeysChanged(ManagedKeysChange::Removed {
                key_id: removed_id,
            }));
            cx.notify();
            return;
        }

        let summary = removed.summary();
        self.status_message =
            i18n::string_args("keychain.messages.removed", &[("summary", &summary)]);
        cx.emit(AppCommand::ManagedKeysChanged(ManagedKeysChange::Removed {
            key_id: removed_id,
        }));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn managed_key(name: &str) -> ManagedKeyRecord {
        ManagedKeyRecord {
            id: format!("managed-key-{name}"),
            name: name.to_string(),
            algorithm: "ssh-ed25519".to_string(),
            public_key: "ssh-ed25519 AAAA".to_string(),
            source: ManagedKeySource::Imported,
        }
    }

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

    #[test]
    fn pasted_import_default_name_uses_first_available_number() {
        let keys = vec![managed_key("Imported key 1"), managed_key("Imported key 3")];

        let name = resolve_managed_key_import_name(
            &keys,
            "",
            "",
            true,
            ManagedKeySource::Imported,
            "Imported key",
            "Generated key",
        );

        assert_eq!(name, "Imported key 2");
    }

    #[test]
    fn generated_key_uses_generated_default_name() {
        let keys = vec![managed_key("Generated key 1")];

        let name = resolve_managed_key_import_name(
            &keys,
            "",
            "",
            true,
            ManagedKeySource::Generated,
            "Imported key",
            "Generated key",
        );

        assert_eq!(name, "Generated key 2");
    }

    #[test]
    fn file_import_uses_unique_file_stem() {
        let keys = vec![managed_key("id_ed25519")];

        let name = resolve_managed_key_import_name(
            &keys,
            "",
            "C:/Users/akko/.ssh/id_ed25519",
            false,
            ManagedKeySource::Imported,
            "Imported key",
            "Generated key",
        );

        assert_eq!(name, "id_ed25519 2");
    }

    #[test]
    fn explicit_name_is_trimmed_and_may_duplicate_existing_name() {
        let keys = vec![managed_key("Production")];

        let name = resolve_managed_key_import_name(
            &keys,
            "  Production  ",
            "",
            true,
            ManagedKeySource::Imported,
            "Imported key",
            "Generated key",
        );

        assert_eq!(name, "Production");
    }

    #[test]
    fn renaming_changes_only_the_managed_key_name() {
        let original = managed_key("Production");
        let mut keys = vec![original.clone()];

        let (_, old_name) =
            apply_managed_key_rename(&mut keys, original.id.as_str(), "Production deploy key")
                .expect("managed key should be renamed");

        assert_eq!(old_name, "Production");
        assert_eq!(keys[0].name, "Production deploy key");
        assert_eq!(keys[0].id, original.id);
        assert_eq!(keys[0].algorithm, original.algorithm);
        assert_eq!(keys[0].public_key, original.public_key);
        assert_eq!(keys[0].source, original.source);
    }
}
