use gpui::Context;

use super::*;

pub(in crate::ui::shell) struct PortForwardSessionStart {
    pub(in crate::ui::shell) tab: TabState,
    pub(in crate::ui::shell) feedback: String,
}

impl SessionController {
    pub(in crate::ui::shell) fn profile_requires_local_vault_unlock(
        &self,
        profile: &SessionProfile,
    ) -> bool {
        if self.services.local_vault_status != LocalVaultStatus::Locked {
            return false;
        }

        match profile.effective_auth_method() {
            AuthMethod::Password => profile.password.is_empty() && profile.has_stored_password,
            AuthMethod::KeyFile => profile.passphrase.is_empty() && profile.has_stored_passphrase,
            AuthMethod::ManagedKey => !profile.managed_key_id.trim().is_empty(),
            AuthMethod::Agent | AuthMethod::KeyboardInteractive => false,
        }
    }

    fn port_forward_rule(
        &self,
        profile_id: &str,
        rule_id: &str,
    ) -> Option<(SessionProfile, PortForwardRule)> {
        let (profile_index, rule_index) = self.port_forward_rule_indices(profile_id, rule_id)?;
        let profiles = self.profiles.borrow();
        let profile = profiles.get(profile_index)?.clone();
        let rule = profile.port_forwarding_rules.get(rule_index)?.clone();
        Some((profile, rule))
    }

    fn port_forward_manager(&self) -> &PortForwardManager {
        self.services
            .port_forward_manager
            .as_ref()
            .expect("port-forward manager is available in the application")
    }

    fn retire_port_forward_connection(
        &self,
        profile_id: &str,
        rule_id: &str,
    ) -> Option<(TabId, String)> {
        let tab_id = self.port_forward_rule_session_id(profile_id, rule_id)?;
        let title = self
            .ports
            .borrow()
            .snapshot
            .sessions
            .get(&tab_id)
            .map(|session| session.title.clone())
            .unwrap_or_else(|| {
                i18n::string_args(
                    "forwarding.messages.fallback_tab_title",
                    &[("profile_id", profile_id)],
                )
            });
        if let Some(mut session) = self.tab_mut(tab_id) {
            session.commands.take();
        }
        self.remove_tab(tab_id);
        Some((tab_id, title))
    }

    pub(in crate::ui::shell) fn confirm_port_forward_rule_removal(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.take_pending_port_forward_rule_delete() else {
            return;
        };

        cx.emit(AppCommand::OverlayDismissed(
            DialogOverlaySnapshot::PortForwardRuleDelete(pending.clone()),
        ));
        if self
            .port_forward_rule_indices(&pending.profile_id, &pending.rule_id)
            .is_none()
        {
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "forwarding.messages.already_removed",
                &[
                    ("rule", pending.rule_label.as_str()),
                    ("profile", pending.profile_label.as_str()),
                ],
            )));
            cx.notify();
            return;
        }

        self.remove_port_forward_rule(&pending.profile_id, &pending.rule_id, cx);
    }

    fn remove_port_forward_rule(&self, profile_id: &str, rule_id: &str, cx: &mut Context<Self>) {
        let Some((profile_index, rule_index)) = self.port_forward_rule_indices(profile_id, rule_id)
        else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.rule_not_found",
            )));
            cx.notify();
            return;
        };

        let editor_state = self.editor_state();
        if editor_state.port_forward_editor_profile_id.as_deref() == Some(profile_id)
            && editor_state.port_forward_editor_rule_id.as_deref() == Some(rule_id)
        {
            self.clear_port_forward_editor();
        }

        self.port_forward_manager().stop(profile_id, rule_id);
        let retired = self.retire_port_forward_connection(profile_id, rule_id);
        let (profile_name, rule_label) = {
            let mut profiles = self.profiles.borrow_mut();
            let profile = &mut profiles[profile_index];
            let removed_rule = profile.port_forwarding_rules.remove(rule_index);
            (
                profile.name.clone(),
                Self::rule_summary_label(&removed_rule),
            )
        };
        let synced_sessions = self.sync_current_port_forward_rules_for_profile(profile_id);
        let synced_suffix = Self::synced_sessions_suffix(synced_sessions);
        let disconnect_suffix = Self::forwarding_closed_connection_suffix(retired.is_some());
        let message = match self.persist_profiles() {
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
        if let Some((tab_id, _)) = retired {
            cx.emit(AppCommand::CloseTab(tab_id));
        }
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_port_forward_rule_enabled(
        &self,
        profile_id: &str,
        rule_id: &str,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if enabled {
            self.connect_port_forward_rule(profile_id, rule_id, cx);
            return;
        }

        let Some((profile_name, rule_label)) =
            self.update_port_forward_rule_enabled_state(profile_id, rule_id, false)
        else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.rule_not_found",
            )));
            cx.notify();
            return;
        };

        self.port_forward_manager().stop(profile_id, rule_id);
        let retired = self.retire_port_forward_connection(profile_id, rule_id);
        let synced_sessions = self.sync_current_port_forward_rules_for_profile(profile_id);
        let synced_suffix = Self::synced_sessions_suffix(synced_sessions);
        let persisted = self.persist_profiles();
        let message = if retired.is_some() {
            let stopped_suffix = Self::forwarding_stopped_tunnel_suffix(true);
            match persisted {
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
            }
        } else {
            match persisted {
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
            }
        };
        if let Some((tab_id, _)) = retired {
            cx.emit(AppCommand::CloseTab(tab_id));
        }
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    pub(in crate::ui::shell) fn connect_port_forward_rule(
        &self,
        profile_id: &str,
        rule_id: &str,
        cx: &mut Context<Self>,
    ) {
        if self.has_port_forward_rule_session(profile_id, rule_id) {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.already_connected",
            )));
            cx.notify();
            return;
        }
        let Some((profile, _)) = self.port_forward_rule(profile_id, rule_id) else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.rule_not_found",
            )));
            cx.notify();
            return;
        };
        if self.profile_requires_local_vault_unlock(&profile) {
            cx.emit(AppCommand::vault_unlock_prompt());
            return;
        }

        cx.emit(AppCommand::OpenTab(TabOpenRequest::PortForwarding {
            profile_id: profile_id.to_string(),
            rule_id: rule_id.to_string(),
        }));
    }

    pub(in crate::ui::shell) fn start_port_forward_session(
        &self,
        tab_id: TabId,
        profile_id: &str,
        rule_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<PortForwardSessionStart> {
        if self.has_port_forward_rule_session(profile_id, rule_id) {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.already_connected",
            )));
            return None;
        }
        let Some((profile, mut rule)) = self.port_forward_rule(profile_id, rule_id) else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.rule_not_found",
            )));
            return None;
        };
        if self.profile_requires_local_vault_unlock(&profile) {
            cx.emit(AppCommand::vault_unlock_prompt());
            return None;
        }

        let runtime_snapshot = match self.port_forward_manager().start(profile_id, rule_id) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                log::warn!("failed to start global port-forward session: {error:?}");
                cx.emit(AppCommand::Feedback(i18n::string_args(
                    "session.messages.port_forwarding_failed_for",
                    &[("title", &profile.name), ("error", &error.to_string())],
                )));
                cx.notify();
                return None;
            }
        };
        rule.enabled = true;
        let _ = self.update_port_forward_rule_enabled_state(profile_id, rule_id, true);
        if let Err(error) = self.persist_profiles() {
            log::warn!("failed to persist connected port-forward rule state: {error:?}");
        }
        let commands = self
            .port_forward_manager()
            .command_sender(profile_id, rule_id);
        let (mut tab, session) = Self::build_port_forwarding_tab(tab_id, &profile, &rule, commands);
        tab.status = runtime_snapshot.status_message.clone();
        self.insert_tab(tab_id, session);
        let _ =
            self.apply_port_forward_manager_snapshot(&self.port_forward_manager().snapshot(), true);
        let synced_sessions = self.sync_current_port_forward_rules_for_profile(&profile.id);
        let rule_label = Self::rule_summary_label(&rule);
        let synced_suffix = Self::synced_sessions_suffix(synced_sessions);
        let feedback = i18n::string_args(
            "forwarding.messages.connecting",
            &[("rule", &rule_label), ("synced_suffix", &synced_suffix)],
        );
        cx.notify();
        Some(PortForwardSessionStart { tab, feedback })
    }

    pub(in crate::ui::shell) fn disconnect_port_forward_rule(
        &self,
        profile_id: &str,
        rule_id: &str,
        cx: &mut Context<Self>,
    ) {
        let _ = self.update_port_forward_rule_enabled_state(profile_id, rule_id, false);
        let stopped = self.port_forward_manager().stop(profile_id, rule_id);
        let retired = self.retire_port_forward_connection(profile_id, rule_id);
        if !stopped && retired.is_none() {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.not_connected",
            )));
            cx.notify();
            return;
        }
        let title = retired
            .as_ref()
            .map(|(_, title)| title.clone())
            .unwrap_or_else(|| {
                self.port_forward_rule(profile_id, rule_id)
                    .map(|(_, rule)| Self::rule_summary_label(&rule))
                    .unwrap_or_else(|| profile_id.to_string())
            });

        let synced_sessions = self.sync_current_port_forward_rules_for_profile(profile_id);
        let synced_suffix = Self::synced_sessions_suffix(synced_sessions);
        if let Err(error) = self.persist_profiles() {
            log::warn!("failed to persist disconnected port-forward rule state: {error:?}");
        }
        if let Some((tab_id, _)) = retired {
            cx.emit(AppCommand::CloseTab(tab_id));
        }
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "forwarding.messages.disconnected",
            &[("title", &title), ("synced_suffix", &synced_suffix)],
        )));
        cx.notify();
    }

    pub(in crate::ui::shell) fn apply_port_forward_manager_snapshot(
        &self,
        snapshot: &PortForwardManagerSnapshot,
        allow_prompt: bool,
    ) -> Vec<(TabId, String)> {
        let manager = self.port_forward_manager();
        let mut updates = Vec::new();
        for (tab_id, session) in self.tabs.borrow_mut().iter_mut() {
            if session.purpose != SessionPurpose::PortForwarding {
                continue;
            }
            let Some(rule_id) = session.port_forward_rule_id.as_deref() else {
                continue;
            };
            let Some(runtime) = snapshot.session(&session.profile_id, rule_id) else {
                session.commands = None;
                session.pending_host_key = None;
                session.pending_keyboard_interactive = None;
                session.set_connection_state(SessionConnectionState::Disconnected);
                updates.push((*tab_id, i18n::string("session.status.disconnected")));
                continue;
            };
            if runtime.log.len() < session.port_forward_log_len {
                session.port_forward_log_len = 0;
            }
            for message in runtime.log.iter().skip(session.port_forward_log_len) {
                session.terminal.push_text(&format!(
                    "{} {message}\r\n",
                    i18n::string("session.terminal.forward_prefix")
                ));
            }
            session.port_forward_log_len = runtime.log.len();
            session.port_forward_revision = runtime.revision;
            session.commands = manager.command_sender(&session.profile_id, rule_id);
            session.set_connection_state(match &runtime.state {
                PortForwardRuntimeState::Starting | PortForwardRuntimeState::Stopping => {
                    SessionConnectionState::Connecting
                }
                PortForwardRuntimeState::Running => SessionConnectionState::Ready,
                PortForwardRuntimeState::Reconnecting { error, attempt, .. } => {
                    SessionConnectionState::Reconnecting {
                        error: error.clone(),
                        attempt: *attempt,
                    }
                }
                PortForwardRuntimeState::Stopped => SessionConnectionState::Disconnected,
                PortForwardRuntimeState::Failed(error) => SessionConnectionState::Failed {
                    error: error.clone(),
                    status: Some(SessionFailureStatus::Closed),
                },
            });
            match runtime.prompt.as_ref().filter(|_| allow_prompt) {
                Some(PortForwardPrompt::HostKey(prompt)) => {
                    session.pending_host_key = Some(prompt.clone());
                    session.pending_keyboard_interactive = None;
                }
                Some(PortForwardPrompt::KeyboardInteractive(challenge)) => {
                    session.pending_host_key = None;
                    session.pending_keyboard_interactive = Some(challenge.clone());
                }
                None => {
                    session.pending_host_key = None;
                    session.pending_keyboard_interactive = None;
                }
            }
            updates.push((*tab_id, runtime.status_message.clone()));
        }
        updates
    }

    pub(in crate::ui::shell) fn build_port_forward_status_tab(
        &self,
        tab_id: TabId,
        runtime: &miaominal_services::PortForwardRuntimeSnapshot,
    ) -> Option<(TabState, SessionTabState)> {
        let profile = self
            .profiles
            .borrow()
            .iter()
            .find(|profile| profile.id == runtime.key.profile_id)
            .cloned()?;
        let commands = self
            .port_forward_manager()
            .command_sender(&runtime.key.profile_id, &runtime.key.rule_id);
        let (mut tab, session) =
            Self::build_port_forwarding_tab(tab_id, &profile, &runtime.rule, commands);
        tab.status = runtime.status_message.clone();
        Some((tab, session))
    }
}
