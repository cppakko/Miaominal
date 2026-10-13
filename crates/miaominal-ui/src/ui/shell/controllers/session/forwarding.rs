use gpui_kit::Context;

use super::*;

pub(in crate::ui::shell) struct PortForwardSessionStart {
    pub(in crate::ui::shell) tab: TabState,
    pub(in crate::ui::shell) feedback: String,
}

fn port_forward_failure_notification(
    tab_id: TabId,
    rule: &str,
    error: &str,
) -> SessionNotificationRequest {
    SessionNotificationRequest {
        tone: SessionNotificationTone::Error,
        title: i18n::string("forwarding.notifications.connection_failed_title"),
        message: i18n::string_args(
            "forwarding.notifications.connection_failed_message",
            &[("rule", rule), ("error", error)],
        ),
        id: format!("port-forward-failure-{tab_id}"),
    }
}

fn should_notify_port_forward_failure(
    previous_state: &SessionConnectionState,
    runtime_state: &PortForwardRuntimeState,
) -> bool {
    matches!(runtime_state, PortForwardRuntimeState::Failed(_))
        && !matches!(previous_state, SessionConnectionState::Failed { .. })
}

impl SessionController {
    pub(in crate::ui::shell) fn profile_requires_local_vault_unlock(
        &self,
        profile: &SessionProfile,
    ) -> bool {
        if profile.is_local() || self.services.local_vault_status != LocalVaultStatus::Locked {
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
                let error = error.to_string();
                crate::ui::shell::session::publish_session_notification(
                    port_forward_failure_notification(
                        tab_id,
                        &Self::rule_summary_label(&rule),
                        &error,
                    ),
                    cx,
                );
                cx.emit(AppCommand::Feedback(i18n::string_args(
                    "session.messages.port_forwarding_failed_for",
                    &[("title", &profile.name), ("error", &error)],
                )));
                cx.notify();
                return None;
            }
        };
        rule.enabled = true;
        // ApplicationState reconciles and persists the manager's semantic runtime
        // snapshot. Starting a resource must not independently publish a stale catalog.
        let commands = self
            .port_forward_manager()
            .command_sender(profile_id, rule_id);
        let (mut tab, session) = Self::build_port_forwarding_tab(tab_id, &profile, &rule, commands);
        tab.status = runtime_snapshot.status_message.clone();
        self.insert_tab(tab_id, session);
        // The forwarding worker can fail before the shell registers this background tab. Consume
        // that first snapshot here so its notification is not lost after the state becomes Failed.
        let startup_updates =
            self.apply_port_forward_manager_snapshot(&self.port_forward_manager().snapshot(), true);
        for (updated_tab_id, status, notification) in startup_updates {
            if updated_tab_id == tab_id {
                tab.status = status;
            }
            if let Some(notification) = notification {
                crate::ui::shell::session::publish_session_notification(notification, cx);
            }
        }
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

    pub(in crate::ui::shell) fn apply_port_forward_manager_snapshot(
        &self,
        snapshot: &PortForwardManagerSnapshot,
        allow_prompt: bool,
    ) -> Vec<(TabId, String, Option<SessionNotificationRequest>)> {
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
                session.pending_host_key_request_id = 0;
                session.pending_keyboard_interactive = None;
                session.pending_keyboard_interactive_request_id = 0;
                session.set_connection_state(SessionConnectionState::Disconnected);
                updates.push((*tab_id, i18n::string("session.status.disconnected"), None));
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
            let failure_notification = match &runtime.state {
                PortForwardRuntimeState::Failed(error)
                    if should_notify_port_forward_failure(
                        &session.connection_state,
                        &runtime.state,
                    ) =>
                {
                    Some(port_forward_failure_notification(
                        *tab_id,
                        &Self::rule_summary_label(&runtime.rule),
                        error,
                    ))
                }
                _ => None,
            };
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
                Some(PortForwardPrompt::HostKey(request)) => {
                    session.pending_host_key_request_id = request.request_id;
                    session.pending_host_key = Some(request.prompt.clone());
                    session.pending_keyboard_interactive = None;
                    session.pending_keyboard_interactive_request_id = 0;
                }
                Some(PortForwardPrompt::KeyboardInteractive(request)) => {
                    session.pending_host_key = None;
                    session.pending_host_key_request_id = 0;
                    session.pending_keyboard_interactive_request_id = request.request_id;
                    session.pending_keyboard_interactive = Some(request.challenge.clone());
                }
                None => {
                    session.pending_host_key = None;
                    session.pending_host_key_request_id = 0;
                    session.pending_keyboard_interactive = None;
                    session.pending_keyboard_interactive_request_id = 0;
                }
            }
            updates.push((
                *tab_id,
                runtime.status_message.clone(),
                failure_notification,
            ));
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

struct SessionForwardingRuntime(WeakEntity<SessionController>);
impl crate::ui::contracts::ForwardingRuntime for SessionForwardingRuntime {
    fn has_rule_session(&self, profile_id: &str, rule_id: &str, cx: &App) -> bool {
        self.0.upgrade().is_some_and(|owner| {
            owner
                .read(cx)
                .has_port_forward_rule_session(profile_id, rule_id)
        })
    }
    fn online_session_count_for_profile(&self, profile_id: &str, cx: &App) -> usize {
        self.0.upgrade().map_or(0, |owner| {
            owner.read(cx).online_session_count_for_profile(profile_id)
        })
    }
    fn sync_rules(&self, profile_id: &str, rules: &[PortForwardRule], cx: &mut App) -> usize {
        self.0.upgrade().map_or(0, |owner| {
            owner
                .read(cx)
                .sync_port_forward_rules_for_profile(profile_id, rules)
        })
    }
    fn retire_rule_session(&self, profile_id: &str, rule_id: &str, cx: &mut App) -> bool {
        self.0.upgrade().is_some_and(|owner| {
            owner.update(cx, |controller, cx| {
                if let Some((tab_id, _)) =
                    controller.retire_port_forward_connection(profile_id, rule_id)
                {
                    cx.emit(AppCommand::CloseTab(tab_id));
                    cx.notify();
                    true
                } else {
                    false
                }
            })
        })
    }
}
impl SessionController {
    pub(in crate::ui::shell) fn forwarding_port(
        owner: &Entity<Self>,
    ) -> crate::ui::contracts::ForwardingRuntimePort {
        crate::ui::contracts::ForwardingRuntimePort::new(Rc::new(SessionForwardingRuntime(
            owner.downgrade(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_forward_failure_notification_is_emitted_only_on_transition_to_failed() {
        let failed = PortForwardRuntimeState::Failed("connection refused".into());

        assert!(should_notify_port_forward_failure(
            &SessionConnectionState::Connecting,
            &failed,
        ));
        assert!(!should_notify_port_forward_failure(
            &SessionConnectionState::Failed {
                error: "connection refused".into(),
                status: Some(SessionFailureStatus::Closed),
            },
            &failed,
        ));
        assert!(!should_notify_port_forward_failure(
            &SessionConnectionState::Connecting,
            &PortForwardRuntimeState::Running,
        ));
    }
}
