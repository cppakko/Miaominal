use super::super::*;
use crate::domain::profile::{DEFAULT_SESSION_CHARSET, ShellType};
use crate::secrets::SecretKind;
use crate::services::ProfileService;
use crate::ui::i18n;
use gpui_component::WindowExt as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProfileInputPurpose {
    Save,
    ConnectionTest,
}

impl ProfileInputPurpose {
    fn requires_name(self) -> bool {
        matches!(self, Self::Save)
    }
}

struct SaveProfileAfterUnlockResult {
    profile: SessionProfile,
    sessions: Vec<SessionProfile>,
    selected_profile: Option<usize>,
}

pub(in crate::ui::shell) fn saved_secret_placeholder(
    has_saved: bool,
    fallback_key: &'static str,
) -> String {
    if has_saved {
        i18n::string("placeholders.saved.keep_existing")
    } else {
        i18n::string(fallback_key)
    }
}

fn parse_tags(tags_text: &str) -> Vec<String> {
    ProfileService::parse_tags(tags_text)
}

impl AppView {
    pub(in crate::ui::shell) fn profile_service(&self) -> ProfileService {
        ProfileService::new(
            self.services.session_store.clone(),
            self.services.secrets.clone(),
        )
    }

    pub(in crate::ui::shell) fn host_editor_auth_method(auth_method: AuthMethod) -> AuthMethod {
        match auth_method {
            AuthMethod::KeyFile => AuthMethod::ManagedKey,
            _ => auth_method,
        }
    }

    fn managed_key_select_items(managed_keys: &[ManagedKeyRecord]) -> Vec<ManagedKeySelectItem> {
        let mut items = managed_keys
            .iter()
            .map(ManagedKeySelectItem::new)
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

    pub(in crate::ui::shell) fn managed_key_options(
        managed_keys: &[ManagedKeyRecord],
    ) -> SearchableVec<ManagedKeySelectItem> {
        SearchableVec::new(Self::managed_key_select_items(managed_keys))
    }

    pub(in crate::ui::shell) fn sync_managed_key_select(
        &mut self,
        selected_key_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let options = Self::managed_key_select_items(&self.data.managed_keys);
        let selected_key_id = selected_key_id.map(str::to_string).or_else(|| {
            self.host_editor_forms
                .managed_key_select
                .read(cx)
                .selected_value()
                .cloned()
        });
        let has_selected_key = selected_key_id.as_ref().is_some_and(|selected_key_id| {
            options.iter().any(|item| item.value() == selected_key_id)
        });

        self.host_editor_forms
            .managed_key_select
            .update(cx, |select, cx| {
                select.set_items(SearchableVec::new(options), window, cx);
                if has_selected_key {
                    if let Some(selected_key_id) = selected_key_id.as_ref() {
                        select.set_selected_value(selected_key_id, window, cx);
                    }
                } else {
                    select.set_selected_index(None, window, cx);
                }
            });
    }

    pub(in crate::ui::shell) fn sync_managed_key_select_in_active_window(
        &mut self,
        selected_key_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let options = Self::managed_key_select_items(&self.data.managed_keys);
        let selected_key_id = selected_key_id.map(str::to_string).or_else(|| {
            self.host_editor_forms
                .managed_key_select
                .read(cx)
                .selected_value()
                .cloned()
        });
        let has_selected_key = selected_key_id.as_ref().is_some_and(|selected_key_id| {
            options.iter().any(|item| item.value() == selected_key_id)
        });
        let managed_key_select = self.host_editor_forms.managed_key_select.clone();

        self.with_active_window(cx, move |window, cx| {
            managed_key_select.update(cx, |select, cx| {
                select.set_items(SearchableVec::new(options), window, cx);
                if has_selected_key {
                    if let Some(selected_key_id) = selected_key_id.as_ref() {
                        select.set_selected_value(selected_key_id, window, cx);
                    }
                } else {
                    select.set_selected_index(None, window, cx);
                }
            });
        });
    }

    pub(in crate::ui::shell) fn collect_available_groups(
        sessions: &[SessionProfile],
    ) -> Vec<String> {
        let mut groups: Vec<_> = sessions
            .iter()
            .filter_map(|profile| {
                let group = profile.group.trim();
                (!group.is_empty()).then(|| group.to_string())
            })
            .collect();

        groups.sort_by_key(|group| group.to_ascii_lowercase());
        groups.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        groups
    }

    pub(in crate::ui::shell) fn sync_group_controls(
        &mut self,
        group: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let group = group.trim();
        let available_groups = Self::collect_available_groups(&self.data.sessions);
        let selected_existing_group = available_groups
            .iter()
            .find(|candidate| candidate.eq_ignore_ascii_case(group))
            .cloned();

        self.host_editor_forms
            .group_select
            .update(cx, |select, cx| {
                select.set_items(SearchableVec::new(available_groups.clone()), window, cx);
                if let Some(existing_group) = selected_existing_group.as_ref() {
                    select.set_selected_value(existing_group, window, cx);
                } else {
                    select.set_selected_index(None, window, cx);
                }
            });

        self.host_editor_forms.creating_new_group =
            !group.is_empty() && selected_existing_group.is_none();
        set_input_value(
            &self.host_editor_forms.group_input,
            if self.host_editor_forms.creating_new_group {
                group.to_string()
            } else {
                String::new()
            },
            window,
            cx,
        );
    }

    pub(in crate::ui::shell) fn begin_new_group(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.host_editor_forms.creating_new_group = true;
        self.host_editor_forms
            .group_select
            .update(cx, |select, cx| {
                select.set_selected_index(None, window, cx);
            });
        set_input_value(&self.host_editor_forms.group_input, "", window, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn group_value(&self, cx: &App) -> String {
        if self.host_editor_forms.creating_new_group {
            self.host_editor_forms
                .group_input
                .read(cx)
                .value()
                .trim()
                .to_string()
        } else {
            self.host_editor_forms
                .group_select
                .read(cx)
                .selected_value()
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string()
        }
    }

    pub(in crate::ui::shell) fn session_charset_value(&self, cx: &App) -> String {
        self.host_editor_forms
            .charset_select
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_else(|| DEFAULT_SESSION_CHARSET.to_string())
            .trim()
            .to_string()
    }

    pub(in crate::ui::shell) fn add_profile(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile = SessionProfile::blank(self.next_profile_id(), self.data.sessions.len() + 1);
        self.data.selected_profile = None;
        self.editors.host_editor_open = true;
        self.editors.host_editor_is_new = true;
        self.populate_inputs(&profile, window, cx);
        self.status_message = i18n::string("profile.messages.new_profile_created");
        cx.notify();
    }

    pub(in crate::ui::shell) fn delete_selected_profile(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.data.selected_profile else {
            self.status_message = i18n::string("profile.messages.select_profile_to_delete");
            cx.notify();
            return;
        };

        let Some(profile) = self.data.sessions.get(index) else {
            self.status_message = i18n::string("profile.messages.select_profile_to_delete");
            cx.notify();
            return;
        };

        self.dialogs.pending_profile_delete = Some(PendingProfileDeleteState {
            profile_id: profile.id.clone(),
            profile_name: profile.name.clone(),
            reload_inputs_after_delete: true,
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn delete_profile_at_index(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = self.data.sessions.get(index) else {
            return;
        };

        self.dialogs.pending_profile_delete = Some(PendingProfileDeleteState {
            profile_id: profile.id.clone(),
            profile_name: profile.name.clone(),
            reload_inputs_after_delete: false,
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn confirm_profile_delete(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.dialogs.pending_profile_delete.take() else {
            return;
        };

        self.start_dialog_exit(DialogOverlaySnapshot::ProfileDelete(pending.clone()), cx);

        let Some(index) = self
            .data
            .sessions
            .iter()
            .position(|profile| profile.id == pending.profile_id)
        else {
            self.status_message = i18n::string_args(
                "profile.messages.already_removed",
                &[("name", &pending.profile_name)],
            );
            cx.notify();
            return;
        };

        self.perform_profile_delete_at_index(index, pending.reload_inputs_after_delete, window, cx);
    }

    pub(in crate::ui::shell) fn cancel_profile_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.dialogs.pending_profile_delete.take() {
            self.start_dialog_exit(DialogOverlaySnapshot::ProfileDelete(pending), cx);
        }
    }

    fn perform_profile_delete_at_index(
        &mut self,
        index: usize,
        reload_inputs_after_delete: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.data.sessions.len() {
            return;
        }

        let deleted_selected_profile = self.data.selected_profile == Some(index);
        let Some(outcome) = self.profile_service().delete_profile(
            &mut self.data.sessions,
            &mut self.data.selected_profile,
            index,
        ) else {
            return;
        };

        if deleted_selected_profile {
            self.editors.host_editor_open = false;
            self.editors.host_editor_is_new = false;
        }

        if let Err(error) = self.persist_sessions_after_user_change(cx) {
            let error = error.to_string();
            self.status_message = i18n::string_args(
                "profile.messages.deleted_local_save_failed",
                &[("name", &outcome.removed.name), ("error", &error)],
            );
        } else {
            self.status_message = i18n::string_args(
                "profile.messages.deleted",
                &[("name", &outcome.removed.name)],
            );
        }

        if reload_inputs_after_delete {
            self.load_selected_profile_into_inputs(window, cx);
        }

        cx.notify();
    }

    pub(in crate::ui::shell) fn duplicate_profile_at_index(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if index >= self.data.sessions.len() {
            return;
        }

        let original_name = self.data.sessions[index].name.clone();
        let duplicate_name = i18n::string_args(
            "profile.messages.duplicate_name",
            &[("name", &original_name)],
        );
        let Some(duplicated) = self.profile_service().duplicate_profile(
            &mut self.data.sessions,
            index,
            duplicate_name,
        ) else {
            return;
        };

        if let Err(error) = self.persist_sessions_after_user_change(cx) {
            let error = error.to_string();
            self.status_message = i18n::string_args(
                "profile.messages.duplicated_local_save_failed",
                &[("name", &duplicated.name), ("error", &error)],
            );
        } else {
            self.status_message = i18n::string_args(
                "profile.messages.duplicated_as",
                &[("name", &duplicated.name)],
            );
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn toggle_profile_favorite(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if index >= self.data.sessions.len() {
            return;
        }

        self.data.sessions[index].is_favorite = !self.data.sessions[index].is_favorite;
        let name = self.data.sessions[index].name.clone();
        let is_fav = self.data.sessions[index].is_favorite;

        if let Err(error) = self.persist_sessions_after_user_change(cx) {
            let error = error.to_string();
            self.status_message = if is_fav {
                i18n::string_args(
                    "profile.messages.starred_local_save_failed",
                    &[("name", &name), ("error", &error)],
                )
            } else {
                i18n::string_args(
                    "profile.messages.unstarred_local_save_failed",
                    &[("name", &name), ("error", &error)],
                )
            };
        } else {
            self.status_message = if is_fav {
                i18n::string_args("profile.messages.added_to_favorites", &[("name", &name)])
            } else {
                i18n::string_args(
                    "profile.messages.removed_from_favorites",
                    &[("name", &name)],
                )
            };
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn save_profile(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile_id = self.current_profile_id();

        match self.read_profile_from_inputs(profile_id, ProfileInputPurpose::Save, cx) {
            Ok(profile) => {
                if self.profile_save_requires_local_vault_unlock(&profile) {
                    self.prompt_local_vault_unlock_for_action(
                        PendingLocalVaultUnlockAction::SaveProfile,
                        window,
                        cx,
                    );
                    return;
                }

                match self.save_prepared_profile(profile, window, cx) {
                    Ok(profile) => {
                        self.editors.host_editor_open = false;
                        self.editors.host_editor_is_new = false;
                        self.status_message = if self.services.session_store.is_some() {
                            i18n::string_args("profile.messages.saved", &[("name", &profile.name)])
                        } else {
                            i18n::string_args(
                                "profile.messages.saved_memory_only",
                                &[("name", &profile.name)],
                            )
                        };
                        cx.notify();
                    }
                    Err(error) => {
                        if let Some(validation) = error.downcast_ref::<ValidationFailure>() {
                            self.notify_validation_failure_in_window(
                                window,
                                validation.kind,
                                validation.message.clone(),
                                cx,
                            );
                        } else {
                            let message = error.to_string();
                            self.status_message = i18n::string_args(
                                "profile.messages.save_failed",
                                &[("message", &message)],
                            );
                            cx.notify();
                        }
                    }
                }
            }
            Err(error) => {
                if let Some(validation) = error.downcast_ref::<ValidationFailure>() {
                    self.notify_validation_failure_in_window(
                        window,
                        validation.kind,
                        validation.message.clone(),
                        cx,
                    );
                } else {
                    let message = error.to_string();
                    self.status_message =
                        i18n::string_args("profile.messages.save_failed", &[("message", &message)]);
                    cx.notify();
                }
            }
        }
    }

    pub(in crate::ui::shell) fn continue_save_profile_after_unlock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile = match self.read_profile_from_inputs(
            self.current_profile_id(),
            ProfileInputPurpose::Save,
            cx,
        ) {
            Ok(profile) => profile,
            Err(error) => {
                if let Some(validation) = error.downcast_ref::<ValidationFailure>() {
                    self.notify_validation_failure_in_window(
                        window,
                        validation.kind,
                        validation.message.clone(),
                        cx,
                    );
                } else {
                    let message = error.to_string();
                    self.status_message =
                        i18n::string_args("profile.messages.save_failed", &[("message", &message)]);
                    cx.notify();
                }
                return;
            }
        };

        let service = self.profile_service();
        let mut sessions = self.data.sessions.clone();
        let mut selected_profile = self.data.selected_profile;

        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name("post-unlock-profile-save".to_string())
            .spawn(move || {
                let result = (|| -> Result<SaveProfileAfterUnlockResult> {
                    service.commit_profile_secrets(&profile)?;
                    service.upsert_profile(&mut sessions, &mut selected_profile, profile.clone());
                    service.persist_sessions(&sessions)?;

                    Ok(SaveProfileAfterUnlockResult {
                        profile,
                        sessions,
                        selected_profile,
                    })
                })();

                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            let message = error.to_string();
            self.status_message =
                i18n::string_args("profile.messages.save_failed", &[("message", &message)]);
            cx.notify();
            return;
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv().unwrap_or_else(|_| {
                        Err(anyhow::anyhow!("post-unlock profile save task cancelled"))
                    })
                })
                .await;

            let _ = this.update(cx, move |this, cx| match result {
                Ok(result) => {
                    this.data.sessions = result.sessions;
                    this.data.selected_profile = result.selected_profile;
                    this.editors.host_editor_open = false;
                    this.editors.host_editor_is_new = false;
                    this.status_message = if this.services.session_store.is_some() {
                        i18n::string_args(
                            "profile.messages.saved",
                            &[("name", &result.profile.name)],
                        )
                    } else {
                        i18n::string_args(
                            "profile.messages.saved_memory_only",
                            &[("name", &result.profile.name)],
                        )
                    };
                    cx.notify();
                }
                Err(error) => {
                    let message = error.to_string();
                    this.status_message =
                        i18n::string_args("profile.messages.save_failed", &[("message", &message)]);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn current_profile_id(&self) -> String {
        self.data
            .selected_profile
            .and_then(|index| {
                self.data
                    .sessions
                    .get(index)
                    .map(|profile| profile.id.clone())
            })
            .unwrap_or_else(|| self.next_profile_id())
    }

    fn profile_save_requires_local_vault_unlock(&self, profile: &SessionProfile) -> bool {
        self.sync_requires_local_vault_unlock()
            || (self.local_vault_status == LocalVaultStatus::Locked
                && (!profile.password.is_empty() || !profile.passphrase.is_empty()))
    }

    fn save_prepared_profile(
        &mut self,
        profile: SessionProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<SessionProfile> {
        self.profile_service().commit_profile_secrets(&profile)?;
        self.upsert_profile(profile.clone());
        self.persist_sessions_after_user_change(cx)?;
        self.load_selected_profile_into_inputs(window, cx);
        Ok(profile)
    }

    pub(in crate::ui::shell) fn test_profile_connection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile_id = self
            .data
            .selected_profile
            .and_then(|index| {
                self.data
                    .sessions
                    .get(index)
                    .map(|profile| profile.id.clone())
            })
            .unwrap_or_else(|| self.next_profile_id());

        match self.read_profile_from_inputs(profile_id, ProfileInputPurpose::ConnectionTest, cx) {
            Ok(profile) => {
                self.start_profile_connection_test(profile, window, cx);
            }
            Err(error) => {
                if let Some(validation) = error.downcast_ref::<ValidationFailure>() {
                    self.notify_validation_failure_in_window(
                        window,
                        validation.kind,
                        validation.message.clone(),
                        cx,
                    );
                } else {
                    let message = error.to_string();
                    self.status_message = i18n::string_args(
                        "profile.messages.test_connection_failed",
                        &[("message", &message)],
                    );
                    self.with_active_window(cx, move |window, cx| {
                        window.push_notification(
                            Self::error_notification(
                                i18n::string("profile.messages.test_connection_failed_title"),
                                message,
                            ),
                            cx,
                        );
                    });
                    cx.notify();
                }
            }
        }
    }

    pub(in crate::ui::shell) fn load_selected_profile_into_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.data.selected_profile
            && let Some(profile) = self.data.sessions.get(index).cloned()
        {
            self.populate_inputs(&profile, window, cx);
            return;
        }

        self.clear_inputs(window, cx);
    }

    pub(in crate::ui::shell) fn populate_inputs(
        &mut self,
        profile: &SessionProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_secret_visibility(SecretRevealTarget::HostPassword, false, false, window, cx);
        set_input_value(
            &self.host_editor_forms.name_input,
            profile.name.clone(),
            window,
            cx,
        );
        self.sync_group_controls(&profile.group, window, cx);
        set_input_value(
            &self.host_editor_forms.tags_input,
            profile.tags.join(", "),
            window,
            cx,
        );
        set_input_value(
            &self.host_editor_forms.host_input,
            profile.host.clone(),
            window,
            cx,
        );
        set_input_value(
            &self.host_editor_forms.port_input,
            profile.port.to_string(),
            window,
            cx,
        );
        set_input_value(
            &self.host_editor_forms.username_input,
            profile.username.clone(),
            window,
            cx,
        );
        set_input_value(&self.host_editor_forms.password_input, "", window, cx);
        set_input_placeholder(
            &self.host_editor_forms.password_input,
            saved_secret_placeholder(
                profile.has_stored_password,
                "placeholders.host_editor.password",
            ),
            window,
            cx,
        );
        set_input_value(
            &self.host_editor_forms.private_key_input,
            profile.private_key_path.clone(),
            window,
            cx,
        );
        self.sync_managed_key_select(Some(&profile.managed_key_id), window, cx);
        set_input_value(
            &self.host_editor_forms.agent_identity_input,
            profile.agent_identity.clone(),
            window,
            cx,
        );
        set_input_value(
            &self.host_editor_forms.certificate_input,
            profile.certificate_path.clone(),
            window,
            cx,
        );
        set_input_value(&self.host_editor_forms.passphrase_input, "", window, cx);
        set_input_placeholder(
            &self.host_editor_forms.passphrase_input,
            saved_secret_placeholder(
                profile.has_stored_passphrase,
                "placeholders.host_editor.key_passphrase",
            ),
            window,
            cx,
        );
        set_input_value(
            &self.host_editor_forms.startup_command_input,
            profile.startup_command.clone(),
            window,
            cx,
        );
        let selected_charset = if profile.charset.trim().is_empty() {
            DEFAULT_SESSION_CHARSET.to_string()
        } else {
            profile.charset.clone()
        };
        self.host_editor_forms
            .charset_select
            .update(cx, |select, cx| {
                select.set_selected_value(&selected_charset, window, cx);
            });
        self.host_editor_forms.proxy_jump_profile_ids = profile.proxy_jump_profile_ids.clone();
        self.host_editor_forms.selected_proxy_jump_hop = None;
        self.sync_proxy_jump_candidate_select(None, window, cx);
        self.host_editor_forms.environment_variable_rows =
            Self::host_editor_environment_variable_rows(&profile.environment_variables, window, cx);
        self.host_editor_forms.shell_type = profile.shell_type;
        self.host_editor_forms.editing_auth_method =
            Self::host_editor_auth_method(profile.effective_auth_method());
        self.host_editor_forms.agent_forwarding_enabled = profile.agent_forwarding;
    }

    pub(in crate::ui::shell) fn clear_inputs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_secret_visibility(SecretRevealTarget::HostPassword, false, false, window, cx);
        set_input_value(&self.host_editor_forms.name_input, "", window, cx);
        self.sync_group_controls("", window, cx);
        set_input_value(&self.host_editor_forms.tags_input, "", window, cx);
        set_input_value(&self.host_editor_forms.host_input, "", window, cx);
        set_input_value(&self.host_editor_forms.port_input, "22", window, cx);
        set_input_value(&self.host_editor_forms.username_input, "", window, cx);
        set_input_value(&self.host_editor_forms.password_input, "", window, cx);
        set_input_placeholder(
            &self.host_editor_forms.password_input,
            i18n::string("placeholders.host_editor.password"),
            window,
            cx,
        );
        set_input_value(&self.host_editor_forms.private_key_input, "", window, cx);
        self.sync_managed_key_select(Some(""), window, cx);
        set_input_value(&self.host_editor_forms.agent_identity_input, "", window, cx);
        set_input_value(&self.host_editor_forms.certificate_input, "", window, cx);
        set_input_value(&self.host_editor_forms.passphrase_input, "", window, cx);
        set_input_placeholder(
            &self.host_editor_forms.passphrase_input,
            i18n::string("placeholders.host_editor.key_passphrase"),
            window,
            cx,
        );
        set_input_value(
            &self.host_editor_forms.startup_command_input,
            "",
            window,
            cx,
        );
        self.host_editor_forms
            .charset_select
            .update(cx, |select, cx| {
                let default_charset = DEFAULT_SESSION_CHARSET.to_string();
                select.set_selected_value(&default_charset, window, cx);
            });
        self.host_editor_forms.proxy_jump_profile_ids.clear();
        self.host_editor_forms.selected_proxy_jump_hop = None;
        self.sync_proxy_jump_candidate_select(None, window, cx);
        self.host_editor_forms.environment_variable_rows =
            Self::host_editor_environment_variable_rows(&[], window, cx);
        self.host_editor_forms.shell_type = ShellType::Posix;
        self.host_editor_forms.editing_auth_method = AuthMethod::Password;
        self.host_editor_forms.agent_forwarding_enabled = false;
    }

    pub(in crate::ui::shell) fn load_selected_profile_password_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let Some(index) = self.data.selected_profile else {
            return Ok(());
        };
        let Some(profile) = self.data.sessions.get(index) else {
            return Ok(());
        };

        if !profile.has_stored_password {
            return Ok(());
        }

        let password = self
            .services
            .secrets
            .get(&profile.id, SecretKind::Password)?
            .unwrap_or_default();

        set_input_value(&self.host_editor_forms.password_input, password, window, cx);
        Ok(())
    }

    pub(in crate::ui::shell) fn set_auth_method(
        &mut self,
        auth_method: AuthMethod,
        cx: &mut Context<Self>,
    ) {
        self.host_editor_forms.editing_auth_method = Self::host_editor_auth_method(auth_method);
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_agent_forwarding_enabled(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.host_editor_forms.agent_forwarding_enabled = enabled;
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_shell_type(
        &mut self,
        shell_type: ShellType,
        cx: &mut Context<Self>,
    ) {
        self.host_editor_forms.shell_type = shell_type;
        cx.notify();
    }

    pub(in crate::ui::shell) fn available_groups(&self) -> Vec<String> {
        Self::collect_available_groups(&self.data.sessions)
    }

    fn read_profile_from_inputs(
        &self,
        profile_id: String,
        purpose: ProfileInputPurpose,
        cx: &App,
    ) -> Result<SessionProfile> {
        let name = self
            .host_editor_forms
            .name_input
            .read(cx)
            .value()
            .to_string();
        let group = self.group_value(cx);
        let tags_text = self
            .host_editor_forms
            .tags_input
            .read(cx)
            .value()
            .to_string();
        let host = self
            .host_editor_forms
            .host_input
            .read(cx)
            .value()
            .to_string();
        let port_text = self
            .host_editor_forms
            .port_input
            .read(cx)
            .value()
            .to_string();
        let username = self
            .host_editor_forms
            .username_input
            .read(cx)
            .value()
            .to_string();
        let password = self
            .host_editor_forms
            .password_input
            .read(cx)
            .value()
            .to_string();
        let private_key_path = self
            .host_editor_forms
            .private_key_input
            .read(cx)
            .value()
            .to_string();
        let managed_key_id = self
            .host_editor_forms
            .managed_key_select
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_default();
        let agent_identity = self
            .host_editor_forms
            .agent_identity_input
            .read(cx)
            .value()
            .to_string();
        let certificate_path = self
            .host_editor_forms
            .certificate_input
            .read(cx)
            .value()
            .to_string();
        let passphrase = self
            .host_editor_forms
            .passphrase_input
            .read(cx)
            .value()
            .to_string();
        let startup_command = self
            .host_editor_forms
            .startup_command_input
            .read(cx)
            .value()
            .to_string();
        let charset = self.session_charset_value(cx);

        let name = name.trim().to_string();
        let group = group.trim().to_string();
        let tags = parse_tags(&tags_text);
        let host = host.trim().to_string();
        let username = username.trim().to_string();
        let private_key_path = private_key_path.trim().to_string();
        let managed_key_id = managed_key_id.trim().to_string();
        let agent_identity = agent_identity.trim().to_string();
        let certificate_path = certificate_path.trim().to_string();
        let startup_command = startup_command.trim().to_string();
        let charset = if charset.is_empty() {
            DEFAULT_SESSION_CHARSET.to_string()
        } else {
            charset
        };
        let environment_variables = self.read_environment_variables(cx)?;
        let proxy_jump_profile_ids = self.read_proxy_jump_profile_ids(&profile_id)?;

        if purpose.requires_name() && name.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.profile.validation.profile_name_required",
            ))
            .into());
        }
        if host.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.profile.validation.host_required",
            ))
            .into());
        }
        if username.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.profile.validation.username_required",
            ))
            .into());
        }

        let name = if name.is_empty() {
            format!("{}@{}", username, host)
        } else {
            name
        };

        let port: u16 = port_text.trim().parse().map_err(|_| {
            anyhow!(ValidationFailure::invalid(i18n::string_args(
                "errors.profile.validation.invalid_port",
                &[("port", &port_text)],
            )))
        })?;

        let existing = self
            .data
            .sessions
            .iter()
            .find(|profile| profile.id == profile_id);
        let prior_password = existing.is_some_and(|p| p.has_stored_password);
        let prior_passphrase = existing.is_some_and(|p| p.has_stored_passphrase);

        let has_password = !password.trim().is_empty();
        let has_passphrase = !passphrase.trim().is_empty();
        let auth_method = Self::host_editor_auth_method(self.host_editor_forms.editing_auth_method);

        match auth_method {
            AuthMethod::Password => {
                if !has_password && !prior_password {
                    return Err(ValidationFailure::required(i18n::string(
                        "errors.profile.validation.password_requires_password",
                    ))
                    .into());
                }
            }
            AuthMethod::KeyFile | AuthMethod::ManagedKey => {
                if managed_key_id.is_empty() {
                    return Err(ValidationFailure::required(i18n::string(
                        "errors.profile.validation.managed_key_requires_id",
                    ))
                    .into());
                }
            }
            AuthMethod::Agent => {
                if agent_identity.is_empty() {
                    return Err(ValidationFailure::required(i18n::string(
                        "errors.profile.validation.ssh_agent_requires_identity",
                    ))
                    .into());
                }
            }
            AuthMethod::KeyboardInteractive => {}
        }

        let private_key_path = if matches!(auth_method, AuthMethod::KeyFile) {
            private_key_path
        } else {
            String::new()
        };
        let passphrase = if matches!(auth_method, AuthMethod::KeyFile) {
            passphrase
        } else {
            String::new()
        };
        let has_stored_passphrase = if matches!(auth_method, AuthMethod::KeyFile) {
            has_passphrase || prior_passphrase
        } else {
            false
        };

        if !certificate_path.is_empty() && matches!(auth_method, AuthMethod::Password) {
            return Err(ValidationFailure::invalid(i18n::string(
                "errors.profile.validation.certificate_requires_key_based_identity",
            ))
            .into());
        }

        Ok(SessionProfile {
            id: profile_id,
            name,
            group,
            tags,
            host,
            port,
            username,
            password,
            auth_method: Some(auth_method),
            private_key_path,
            passphrase,
            managed_key_id,
            agent_identity: agent_identity.clone(),
            agent_identity_label: existing
                .map(|profile| profile.agent_identity_label.clone())
                .filter(|label| !label.trim().is_empty())
                .unwrap_or(agent_identity),
            certificate_path,
            agent_forwarding: self.host_editor_forms.agent_forwarding_enabled,
            startup_command,
            charset,
            environment_variables,
            shell_type: self.host_editor_forms.shell_type,
            proxy_jump_profile_ids,
            has_stored_password: has_password || prior_password,
            has_stored_passphrase,
            port_forwarding_rules: existing
                .map(|profile| profile.port_forwarding_rules.clone())
                .unwrap_or_default(),
            is_favorite: existing.map(|profile| profile.is_favorite).unwrap_or(false),
            last_connected_at: existing.and_then(|profile| profile.last_connected_at),
        })
    }

    pub(in crate::ui::shell) fn upsert_profile(&mut self, profile: SessionProfile) {
        self.profile_service().upsert_profile(
            &mut self.data.sessions,
            &mut self.data.selected_profile,
            profile,
        );
    }

    pub(in crate::ui::shell) fn persist_sessions(&self) -> Result<()> {
        self.profile_service().persist_sessions(&self.data.sessions)
    }

    pub(in crate::ui::shell) fn persist_sessions_after_user_change(
        &mut self,
        _cx: &mut Context<Self>,
    ) -> Result<()> {
        self.persist_sessions()?;
        Ok(())
    }

    pub(in crate::ui::shell) fn next_profile_id(&self) -> String {
        self.profile_service().next_profile_id(&self.data.sessions)
    }

    pub(in crate::ui::shell) fn select_profile(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.data.sessions.len() {
            return;
        }

        self.data.selected_profile = Some(index);
        self.load_selected_profile_into_inputs(window, cx);
        let name = self.data.sessions[index].name.clone();
        self.status_message = i18n::string_args("profile.messages.selected", &[("name", &name)]);
        cx.notify();
    }
}
