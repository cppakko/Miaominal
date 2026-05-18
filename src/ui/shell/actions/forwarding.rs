use super::super::*;
use crate::ui::i18n;

struct PortForwardRuleInputValues {
    profile_id: String,
    profile_index: usize,
    kind: PortForwardKind,
    resolved_label: String,
    listen_host: String,
    listen_port: u16,
    target_host: String,
    target_port: u16,
}

struct SavePortForwardRuleAfterUnlockResult {
    sessions: Vec<SessionProfile>,
    profile_id: String,
    profile_name: String,
    rule: PortForwardRule,
    is_edit: bool,
    persist_error: Option<String>,
}

impl AppView {
    pub(in crate::ui::shell) fn request_port_forward_rule_removal(
        &mut self,
        profile_id: &str,
        rule_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some((profile_index, rule_index)) = self.port_forward_rule_indices(profile_id, rule_id)
        else {
            self.status_message = i18n::string("forwarding.messages.rule_not_found");
            cx.notify();
            return;
        };

        let profile = &self.data.sessions[profile_index];
        let Some(rule) = profile.port_forwarding_rules.get(rule_index) else {
            self.status_message = i18n::string("forwarding.messages.rule_not_found");
            cx.notify();
            return;
        };

        self.dialogs.pending_port_forward_rule_delete = Some(PendingPortForwardRuleDeleteState {
            profile_id: profile_id.to_string(),
            rule_id: rule_id.to_string(),
            profile_label: profile.connection_label(),
            rule_label: Self::rule_summary_label(rule),
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn confirm_port_forward_rule_removal(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.dialogs.pending_port_forward_rule_delete.take() else {
            return;
        };

        self.start_dialog_exit(
            DialogOverlaySnapshot::PortForwardRuleDelete(pending.clone()),
            cx,
        );

        if self
            .port_forward_rule_indices(&pending.profile_id, &pending.rule_id)
            .is_none()
        {
            self.status_message = i18n::string_args(
                "forwarding.messages.already_removed",
                &[
                    ("rule", pending.rule_label.as_str()),
                    ("profile", pending.profile_label.as_str()),
                ],
            );
            cx.notify();
            return;
        }

        self.remove_port_forward_rule(&pending.profile_id, &pending.rule_id, cx);
    }

    pub(in crate::ui::shell) fn cancel_port_forward_rule_removal(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        if let Some(pending) = self.dialogs.pending_port_forward_rule_delete.take() {
            self.start_dialog_exit(DialogOverlaySnapshot::PortForwardRuleDelete(pending), cx);
        }
    }

    pub(in crate::ui::shell) fn duplicate_port_forward_rule(
        &mut self,
        profile_id: &str,
        rule_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some((profile_index, rule_index)) = self.port_forward_rule_indices(profile_id, rule_id)
        else {
            self.status_message = i18n::string("forwarding.messages.rule_not_found");
            cx.notify();
            return;
        };

        let (profile_name, duplicated_rule) = {
            let profile = &self.data.sessions[profile_index];
            let Some(source_rule) = profile.port_forwarding_rules.get(rule_index) else {
                self.status_message = i18n::string("forwarding.messages.rule_not_found");
                cx.notify();
                return;
            };

            let mut duplicated_rule = source_rule.clone();
            duplicated_rule.id = self.next_port_forward_rule_id(profile);
            let source_label = Self::rule_summary_label(source_rule);
            duplicated_rule.label = i18n::string_args(
                "forwarding.messages.duplicate_label",
                &[("label", &source_label)],
            );
            (profile.name.clone(), duplicated_rule)
        };

        self.data.sessions[profile_index]
            .port_forwarding_rules
            .insert(rule_index + 1, duplicated_rule.clone());
        let synced_sessions = self.sync_port_forward_rules_for_profile(profile_id);
        let synced_suffix = self.synced_sessions_suffix(synced_sessions);
        self.status_message = match self.persist_sessions_after_user_change(cx) {
            Ok(()) => i18n::string_args(
                "forwarding.messages.duplicated",
                &[
                    ("rule", &duplicated_rule.label),
                    ("profile", &profile_name),
                    ("synced_suffix", &synced_suffix),
                ],
            ),
            Err(error) => {
                let error = error.to_string();
                i18n::string_args(
                    "forwarding.messages.duplicated_memory_only",
                    &[
                        ("rule", &duplicated_rule.label),
                        ("synced_suffix", &synced_suffix),
                        ("error", &error),
                    ],
                )
            }
        };
        cx.notify();
    }

    pub(in crate::ui::shell) fn remove_port_forward_rule(
        &mut self,
        profile_id: &str,
        rule_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some((profile_index, rule_index)) = self.port_forward_rule_indices(profile_id, rule_id)
        else {
            self.status_message = i18n::string("forwarding.messages.rule_not_found");
            cx.notify();
            return;
        };

        if self.editors.port_forward_editor_profile_id.as_deref() == Some(profile_id)
            && self.editors.port_forward_editor_rule_id.as_deref() == Some(rule_id)
        {
            self.editors.port_forward_editor_open = false;
            self.editors.port_forward_editor_profile_id = None;
            self.editors.port_forward_editor_rule_id = None;
        }

        let disconnected = self.remove_port_forward_connection_tab(profile_id, rule_id);
        let (profile_name, rule_label) = {
            let profile = &mut self.data.sessions[profile_index];
            let removed_rule = profile.port_forwarding_rules.remove(rule_index);
            (
                profile.name.clone(),
                Self::rule_summary_label(&removed_rule),
            )
        };

        let synced_sessions = self.sync_port_forward_rules_for_profile(profile_id);
        let synced_suffix = self.synced_sessions_suffix(synced_sessions);
        let disconnect_suffix = self.forwarding_closed_connection_suffix(disconnected.is_some());
        self.status_message = match self.persist_sessions_after_user_change(cx) {
            Ok(()) => i18n::string_args(
                "forwarding.messages.removed",
                &[
                    ("rule", &rule_label),
                    ("profile", &profile_name),
                    ("disconnect_suffix", &disconnect_suffix),
                    ("synced_suffix", &synced_suffix),
                ],
            ),
            Err(error) => {
                let error = error.to_string();
                i18n::string_args(
                    "forwarding.messages.removed_memory_only",
                    &[
                        ("rule", &rule_label),
                        ("disconnect_suffix", &disconnect_suffix),
                        ("synced_suffix", &synced_suffix),
                        ("error", &error),
                    ],
                )
            }
        };
        cx.notify();
    }

    pub(in crate::ui::shell) fn open_port_forward_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editors.port_forward_editor_open = true;
        self.editors.port_forward_editor_profile_id = None;
        self.editors.port_forward_editor_rule_id = None;
        self.editors.port_forward_kind = PortForwardKind::Local;
        set_input_value(&self.panel_forms.forwarding.label_input, "", window, cx);
        set_input_value(
            &self.panel_forms.forwarding.listen_host_input,
            "",
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.forwarding.listen_port_input,
            "",
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.forwarding.target_host_input,
            "",
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.forwarding.target_port_input,
            "",
            window,
            cx,
        );
        self.sync_port_forward_profile_select(None, window, cx);
        self.status_message = if self.data.sessions.is_empty() {
            i18n::string("forwarding.messages.create_host_profile_before_adding")
        } else {
            i18n::string("forwarding.messages.choose_host_profile_for_new_rule")
        };
        cx.notify();
    }

    pub(in crate::ui::shell) fn edit_port_forward_rule(
        &mut self,
        profile_id: String,
        rule_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((profile_index, rule_index)) =
            self.port_forward_rule_indices(&profile_id, &rule_id)
        else {
            self.status_message = i18n::string("forwarding.messages.rule_not_found");
            cx.notify();
            return;
        };

        let profile = self.data.sessions[profile_index].clone();
        let Some(rule) = profile.port_forwarding_rules.get(rule_index).cloned() else {
            self.status_message = i18n::string("forwarding.messages.rule_not_found");
            cx.notify();
            return;
        };

        self.data.selected_profile = Some(profile_index);
        self.editors.port_forward_editor_open = true;
        self.editors.port_forward_editor_profile_id = Some(profile.id.clone());
        self.editors.port_forward_editor_rule_id = Some(rule.id.clone());
        self.editors.port_forward_kind = rule.kind;
        set_input_value(
            &self.panel_forms.forwarding.label_input,
            rule.label.clone(),
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.forwarding.listen_host_input,
            rule.listen_host.clone(),
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.forwarding.listen_port_input,
            rule.listen_port.to_string(),
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.forwarding.target_host_input,
            rule.target_host.clone(),
            window,
            cx,
        );
        set_input_value(
            &self.panel_forms.forwarding.target_port_input,
            rule.target_port.to_string(),
            window,
            cx,
        );
        self.sync_port_forward_profile_select(Some(profile.id.as_str()), window, cx);
        let rule_label = Self::rule_summary_label(&rule);
        self.status_message =
            i18n::string_args("forwarding.messages.editing_rule", &[("rule", &rule_label)]);
        cx.notify();
    }

    pub(in crate::ui::shell) fn close_port_forward_rule_editor(&mut self, cx: &mut Context<Self>) {
        if !self.editors.port_forward_editor_open {
            return;
        }

        self.editors.port_forward_editor_open = false;
        self.editors.port_forward_editor_profile_id = None;
        self.status_message = if self.editors.port_forward_editor_rule_id.is_some() {
            i18n::string("forwarding.messages.canceled_editing")
        } else {
            i18n::string("forwarding.messages.canceled_new")
        };
        self.editors.port_forward_editor_rule_id = None;
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_port_forward_kind(
        &mut self,
        kind: PortForwardKind,
        cx: &mut Context<Self>,
    ) {
        self.editors.port_forward_kind = kind;
        cx.notify();
    }

    fn read_port_forward_rule_input_values(&self, cx: &App) -> Result<PortForwardRuleInputValues> {
        let kind = self.editors.port_forward_kind;
        let Some(profile_id) = self.editors.port_forward_editor_profile_id.clone() else {
            return Err(ValidationFailure::required(i18n::string(
                "errors.forwarding.validation.host_profile_required",
            ))
            .into());
        };
        let Some(profile_index) = self
            .data
            .sessions
            .iter()
            .position(|profile| profile.id == profile_id)
        else {
            return Err(ValidationFailure::invalid(i18n::string(
                "errors.forwarding.validation.selected_host_profile_missing",
            ))
            .into());
        };

        let label = self
            .panel_forms
            .forwarding
            .label_input
            .read(cx)
            .value()
            .to_string();
        let listen_host = self
            .panel_forms
            .forwarding
            .listen_host_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let listen_port_text = self
            .panel_forms
            .forwarding
            .listen_port_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let target_host = self
            .panel_forms
            .forwarding
            .target_host_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let target_port_text = self
            .panel_forms
            .forwarding
            .target_port_input
            .read(cx)
            .value()
            .trim()
            .to_string();

        if listen_host.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.forwarding.validation.listen_host_required",
            ))
            .into());
        }
        if listen_port_text.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.forwarding.validation.listen_port_required",
            ))
            .into());
        }
        if target_host.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.forwarding.validation.target_host_required",
            ))
            .into());
        }
        if target_port_text.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.forwarding.validation.target_port_required",
            ))
            .into());
        }

        let listen_port: u16 = listen_port_text.trim().parse().map_err(|_| {
            anyhow!(ValidationFailure::invalid(i18n::string_args(
                "errors.forwarding.validation.invalid_listen_port",
                &[("port", &listen_port_text)],
            )))
        })?;
        let target_port: u16 = target_port_text.trim().parse().map_err(|_| {
            anyhow!(ValidationFailure::invalid(i18n::string_args(
                "errors.forwarding.validation.invalid_target_port",
                &[("port", &target_port_text)],
            )))
        })?;

        let resolved_label = {
            let label = label.trim();
            if label.is_empty() {
                format!(
                    "{} {}:{} -> {}:{}",
                    Self::localized_port_forward_kind_label(kind),
                    listen_host,
                    listen_port,
                    target_host,
                    target_port
                )
            } else {
                label.to_string()
            }
        };

        Ok(PortForwardRuleInputValues {
            profile_id,
            profile_index,
            kind,
            resolved_label,
            listen_host,
            listen_port,
            target_host,
            target_port,
        })
    }

    pub(in crate::ui::shell) fn create_port_forward_rule(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let PortForwardRuleInputValues {
            profile_id,
            profile_index,
            kind,
            resolved_label,
            listen_host,
            listen_port,
            target_host,
            target_port,
        } = match self.read_port_forward_rule_input_values(cx) {
            Ok(values) => values,
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
                        "forwarding.messages.save_failed",
                        &[("message", &message)],
                    );
                    cx.notify();
                }
                return;
            }
        };

        if self.sync_requires_local_vault_unlock() {
            self.prompt_local_vault_unlock_for_action(
                PendingLocalVaultUnlockAction::SavePortForwardRule,
                window,
                cx,
            );
            return;
        }

        if let Some(rule_id) = self.editors.port_forward_editor_rule_id.clone() {
            let Some(rule_index) = self.data.sessions[profile_index]
                .port_forwarding_rules
                .iter()
                .position(|rule| rule.id == rule_id)
            else {
                self.editors.port_forward_editor_rule_id = None;
                self.status_message = i18n::string("forwarding.messages.rule_no_longer_exists");
                cx.notify();
                return;
            };

            let existing_enabled =
                self.data.sessions[profile_index].port_forwarding_rules[rule_index].enabled;
            let updated_rule = PortForwardRule {
                id: rule_id,
                label: resolved_label,
                kind,
                listen_host,
                listen_port,
                target_host,
                target_port,
                enabled: existing_enabled,
            };

            let profile_name = {
                let profile = &mut self.data.sessions[profile_index];
                profile.port_forwarding_rules[rule_index] = updated_rule.clone();
                profile.name.clone()
            };
            let synced_sessions = self.sync_port_forward_rules_for_profile(&profile_id);
            let synced_suffix = self.synced_sessions_suffix(synced_sessions);

            self.editors.port_forward_editor_open = false;
            self.editors.port_forward_editor_profile_id = None;
            self.editors.port_forward_editor_rule_id = None;
            self.status_message = match self.persist_sessions_after_user_change(cx) {
                Ok(()) => i18n::string_args(
                    "forwarding.messages.saved",
                    &[
                        ("rule", &updated_rule.label),
                        ("profile", &profile_name),
                        ("synced_suffix", &synced_suffix),
                    ],
                ),
                Err(error) => {
                    let error = error.to_string();
                    i18n::string_args(
                        "forwarding.messages.saved_memory_only",
                        &[
                            ("rule", &updated_rule.label),
                            ("synced_suffix", &synced_suffix),
                            ("error", &error),
                        ],
                    )
                }
            };
            cx.notify();
            return;
        }

        let rule = {
            let profile = &self.data.sessions[profile_index];
            PortForwardRule {
                id: self.next_port_forward_rule_id(profile),
                label: resolved_label,
                kind,
                listen_host,
                listen_port,
                target_host,
                target_port,
                enabled: true,
            }
        };

        let profile_name = {
            let profile = &mut self.data.sessions[profile_index];
            profile.port_forwarding_rules.push(rule.clone());
            profile.name.clone()
        };
        let synced_sessions = self.sync_port_forward_rules_for_profile(&profile_id);
        let synced_suffix = self.synced_sessions_suffix(synced_sessions);

        self.editors.port_forward_editor_open = false;
        self.editors.port_forward_editor_profile_id = None;
        self.editors.port_forward_editor_rule_id = None;
        self.status_message = match self.persist_sessions_after_user_change(cx) {
            Ok(()) => i18n::string_args(
                "forwarding.messages.added",
                &[
                    ("rule", &rule.label),
                    ("profile", &profile_name),
                    ("synced_suffix", &synced_suffix),
                ],
            ),
            Err(error) => {
                let error = error.to_string();
                i18n::string_args(
                    "forwarding.messages.added_memory_only",
                    &[
                        ("rule", &rule.label),
                        ("synced_suffix", &synced_suffix),
                        ("error", &error),
                    ],
                )
            }
        };
        cx.notify();
    }

    pub(in crate::ui::shell) fn continue_save_port_forward_rule_after_unlock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let PortForwardRuleInputValues {
            profile_id,
            profile_index,
            kind,
            resolved_label,
            listen_host,
            listen_port,
            target_host,
            target_port,
        } = match self.read_port_forward_rule_input_values(cx) {
            Ok(values) => values,
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
                        "forwarding.messages.save_failed",
                        &[("message", &message)],
                    );
                    cx.notify();
                }
                return;
            }
        };

        let mut sessions = self.data.sessions.clone();
        let (rule, profile_name, is_edit) =
            if let Some(rule_id) = self.editors.port_forward_editor_rule_id.clone() {
                let Some(rule_index) = sessions[profile_index]
                    .port_forwarding_rules
                    .iter()
                    .position(|rule| rule.id == rule_id)
                else {
                    self.editors.port_forward_editor_rule_id = None;
                    self.status_message = i18n::string("forwarding.messages.rule_no_longer_exists");
                    cx.notify();
                    return;
                };

                let existing_enabled =
                    sessions[profile_index].port_forwarding_rules[rule_index].enabled;
                let updated_rule = PortForwardRule {
                    id: rule_id,
                    label: resolved_label,
                    kind,
                    listen_host,
                    listen_port,
                    target_host,
                    target_port,
                    enabled: existing_enabled,
                };
                let profile_name = sessions[profile_index].name.clone();
                sessions[profile_index].port_forwarding_rules[rule_index] = updated_rule.clone();
                (updated_rule, profile_name, true)
            } else {
                let rule = {
                    let profile = &sessions[profile_index];
                    PortForwardRule {
                        id: self.next_port_forward_rule_id(profile),
                        label: resolved_label,
                        kind,
                        listen_host,
                        listen_port,
                        target_host,
                        target_port,
                        enabled: true,
                    }
                };
                let profile_name = sessions[profile_index].name.clone();
                sessions[profile_index]
                    .port_forwarding_rules
                    .push(rule.clone());
                (rule, profile_name, false)
            };

        let service = self.profile_service();
        let (tx, rx) =
            std::sync::mpsc::sync_channel::<Result<SavePortForwardRuleAfterUnlockResult>>(1);
        let spawn_result = std::thread::Builder::new()
            .name("post-unlock-port-forward-save".to_string())
            .spawn(move || {
                let persist_error = service
                    .persist_sessions(&sessions)
                    .err()
                    .map(|error| error.to_string());
                tx.send(Ok(SavePortForwardRuleAfterUnlockResult {
                    sessions,
                    profile_id,
                    profile_name,
                    rule,
                    is_edit,
                    persist_error,
                }))
                .ok();
            });

        if let Err(error) = spawn_result {
            self.status_message = i18n::string_args(
                "forwarding.messages.save_failed",
                &[("message", &error.to_string())],
            );
            cx.notify();
            return;
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv().unwrap_or_else(|_| {
                        Err(anyhow::anyhow!(
                            "post-unlock port-forward save task cancelled"
                        ))
                    })
                })
                .await;

            let _ = this.update(cx, move |this, cx| match result {
                Ok(result) => {
                    this.data.sessions = result.sessions;
                    let synced_sessions =
                        this.sync_port_forward_rules_for_profile(&result.profile_id);
                    let synced_suffix = this.synced_sessions_suffix(synced_sessions);
                    this.editors.port_forward_editor_open = false;
                    this.editors.port_forward_editor_profile_id = None;
                    this.editors.port_forward_editor_rule_id = None;

                    this.status_message = if let Some(error) = result.persist_error {
                        if result.is_edit {
                            i18n::string_args(
                                "forwarding.messages.saved_memory_only",
                                &[
                                    ("rule", &result.rule.label),
                                    ("synced_suffix", &synced_suffix),
                                    ("error", error.as_str()),
                                ],
                            )
                        } else {
                            i18n::string_args(
                                "forwarding.messages.added_memory_only",
                                &[
                                    ("rule", &result.rule.label),
                                    ("synced_suffix", &synced_suffix),
                                    ("error", error.as_str()),
                                ],
                            )
                        }
                    } else {
                        if result.is_edit {
                            i18n::string_args(
                                "forwarding.messages.saved",
                                &[
                                    ("rule", &result.rule.label),
                                    ("profile", &result.profile_name),
                                    ("synced_suffix", &synced_suffix),
                                ],
                            )
                        } else {
                            i18n::string_args(
                                "forwarding.messages.added",
                                &[
                                    ("rule", &result.rule.label),
                                    ("profile", &result.profile_name),
                                    ("synced_suffix", &synced_suffix),
                                ],
                            )
                        }
                    };
                    cx.notify();
                }
                Err(error) => {
                    this.status_message = i18n::string_args(
                        "forwarding.messages.save_failed",
                        &[("message", &error.to_string())],
                    );
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(in crate::ui::shell) fn set_port_forward_rule_enabled(
        &mut self,
        profile_id: &str,
        rule_id: &str,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if enabled {
            if !self.has_port_forward_rule_session(profile_id, rule_id) {
                self.connect_port_forward_rule(profile_id, rule_id, cx);
                return;
            }

            self.status_message = i18n::string("forwarding.messages.already_connected");
            cx.notify();
            return;
        }

        let Some((profile_name, rule_label)) =
            self.update_port_forward_rule_enabled_state(profile_id, rule_id, false)
        else {
            self.status_message = i18n::string("forwarding.messages.rule_not_found");
            cx.notify();
            return;
        };

        let had_dedicated_connection = self.has_port_forward_rule_session(profile_id, rule_id);
        let title = if had_dedicated_connection {
            self.remove_port_forward_connection_tab(profile_id, rule_id)
        } else {
            None
        };
        let synced_sessions = self.sync_port_forward_rules_for_profile(profile_id);
        let synced_suffix = self.synced_sessions_suffix(synced_sessions);
        let persisted = self.persist_sessions_after_user_change(cx);

        if had_dedicated_connection {
            let stopped_suffix = self.forwarding_stopped_tunnel_suffix(title.is_some());
            self.status_message = match persisted {
                Ok(()) => i18n::string_args(
                    "forwarding.messages.disabled",
                    &[
                        ("rule", &rule_label),
                        ("profile", &profile_name),
                        ("stopped_suffix", &stopped_suffix),
                        ("synced_suffix", &synced_suffix),
                    ],
                ),
                Err(error) => {
                    let error = error.to_string();
                    i18n::string_args(
                        "forwarding.messages.disabled_memory_only",
                        &[
                            ("rule", &rule_label),
                            ("stopped_suffix", &stopped_suffix),
                            ("synced_suffix", &synced_suffix),
                            ("error", &error),
                        ],
                    )
                }
            };
            cx.notify();
            return;
        }

        self.status_message = match persisted {
            Ok(()) => i18n::string_args(
                "forwarding.messages.disabled_plain",
                &[
                    ("rule", &rule_label),
                    ("profile", &profile_name),
                    ("synced_suffix", &synced_suffix),
                ],
            ),
            Err(error) => {
                let error = error.to_string();
                i18n::string_args(
                    "forwarding.messages.updated_memory_only",
                    &[
                        ("rule", &rule_label),
                        ("synced_suffix", &synced_suffix),
                        ("error", &error),
                    ],
                )
            }
        };
        cx.notify();
    }
}
