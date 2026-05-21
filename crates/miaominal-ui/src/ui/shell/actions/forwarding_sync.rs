use super::super::*;
use crate::ui::i18n;

impl AppView {
    pub(in crate::ui::shell) fn localized_port_forward_kind_label(kind: PortForwardKind) -> String {
        match kind {
            PortForwardKind::Local => i18n::string("forwarding.editor.local"),
            PortForwardKind::Remote => i18n::string("forwarding.editor.remote"),
        }
    }

    pub(in crate::ui::shell) fn rule_summary_label(rule: &PortForwardRule) -> String {
        let label = rule.label.trim();
        if !label.is_empty() {
            return label.to_string();
        }

        format!(
            "{} {}:{} -> {}:{}",
            Self::localized_port_forward_kind_label(rule.kind),
            rule.listen_host,
            rule.listen_port,
            rule.target_host,
            rule.target_port
        )
    }

    pub(in crate::ui::shell) fn forward_profile_options(
        sessions: &[SessionProfile],
    ) -> SearchableVec<ForwardProfileSelectItem> {
        SearchableVec::new(
            sessions
                .iter()
                .map(ForwardProfileSelectItem::new)
                .collect::<Vec<_>>(),
        )
    }

    pub(in crate::ui::shell) fn sync_port_forward_profile_select(
        &mut self,
        selected_profile_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let options = Self::forward_profile_options(&self.data.sessions);
        let selected_profile_id = selected_profile_id.map(str::to_string);

        self.panel_forms
            .forwarding
            .profile_select
            .update(cx, |select, cx| {
                select.set_items(options, window, cx);
                if let Some(selected_profile_id) = selected_profile_id.as_ref() {
                    select.set_selected_value(selected_profile_id, window, cx);
                } else {
                    select.set_selected_index(None, window, cx);
                }
            });
    }

    pub(in crate::ui::shell) fn select_port_forward_editor_profile(
        &mut self,
        profile_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(profile_id) = profile_id else {
            self.editors.port_forward_editor_profile_id = None;
            self.status_message = i18n::string("forwarding.messages.choose_profile_before_adding");
            cx.notify();
            return;
        };

        let Some(profile_index) = self
            .data
            .sessions
            .iter()
            .position(|profile| profile.id == profile_id)
        else {
            self.editors.port_forward_editor_profile_id = None;
            self.status_message = i18n::string("forwarding.messages.profile_not_found");
            cx.notify();
            return;
        };

        let profile = &self.data.sessions[profile_index];
        self.data.selected_profile = Some(profile_index);
        self.editors.port_forward_editor_profile_id = Some(profile.id.clone());
        if self.editors.port_forward_editor_rule_id.is_none() {
            self.status_message = i18n::string_args(
                "forwarding.messages.creating_rule_for_profile",
                &[("profile", &profile.name)],
            );
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn port_forward_rule_indices(
        &self,
        profile_id: &str,
        rule_id: &str,
    ) -> Option<(usize, usize)> {
        self.data
            .sessions
            .iter()
            .enumerate()
            .find_map(|(profile_index, profile)| {
                (profile.id == profile_id)
                    .then_some(profile)
                    .and_then(|profile| {
                        profile
                            .port_forwarding_rules
                            .iter()
                            .position(|rule| rule.id == rule_id)
                            .map(|rule_index| (profile_index, rule_index))
                    })
            })
    }

    fn port_forward_connection_tab_index(&self, profile_id: &str, rule_id: &str) -> Option<usize> {
        self.workspace_state
            .tabs
            .iter()
            .enumerate()
            .find_map(|(index, tab)| {
                tab.as_session().and_then(|session| {
                    (session.profile_id == profile_id
                        && session.port_forward_rule_id.as_deref() == Some(rule_id)
                        && session.purpose == SessionPurpose::PortForwarding
                        && session.commands.is_some())
                    .then_some(index)
                })
            })
    }

    fn port_forward_rule_session(
        &self,
        profile_id: &str,
        rule_id: &str,
    ) -> Option<&SessionTabState> {
        self.workspace_state.tabs.iter().find_map(|tab| {
            tab.as_session().filter(|session| {
                session.profile_id == profile_id
                    && session.port_forward_rule_id.as_deref() == Some(rule_id)
                    && session.purpose == SessionPurpose::PortForwarding
                    && session.commands.is_some()
            })
        })
    }

    pub(in crate::ui::shell) fn has_port_forward_rule_session(
        &self,
        profile_id: &str,
        rule_id: &str,
    ) -> bool {
        self.port_forward_rule_session(profile_id, rule_id)
            .is_some()
    }

    pub(in crate::ui::shell) fn has_port_forward_rule_connection(
        &self,
        profile_id: &str,
        rule_id: &str,
    ) -> bool {
        self.port_forward_rule_session(profile_id, rule_id)
            .is_some_and(|session| {
                matches!(session.connection_state, SessionConnectionState::Ready)
            })
    }

    pub(in crate::ui::shell) fn is_port_forward_rule_connecting(
        &self,
        profile_id: &str,
        rule_id: &str,
    ) -> bool {
        self.port_forward_rule_session(profile_id, rule_id)
            .is_some_and(|session| {
                matches!(
                    session.connection_state,
                    SessionConnectionState::Connecting
                        | SessionConnectionState::Reconnecting { .. }
                )
            })
    }

    pub(in crate::ui::shell) fn update_port_forward_rule_enabled_state(
        &mut self,
        profile_id: &str,
        rule_id: &str,
        enabled: bool,
    ) -> Option<(String, String)> {
        let (profile_index, rule_index) = self.port_forward_rule_indices(profile_id, rule_id)?;
        let profile = self.data.sessions.get_mut(profile_index)?;
        let rule = profile.port_forwarding_rules.get_mut(rule_index)?;
        rule.enabled = enabled;
        Some((profile.name.clone(), Self::rule_summary_label(rule)))
    }

    pub(in crate::ui::shell) fn remove_port_forward_connection_tab(
        &mut self,
        profile_id: &str,
        rule_id: &str,
    ) -> Option<String> {
        let index = self.port_forward_connection_tab_index(profile_id, rule_id)?;
        let title = self
            .workspace_state
            .tabs
            .get(index)
            .map(|tab| tab.title.clone())
            .unwrap_or_else(|| {
                i18n::string_args(
                    "forwarding.messages.fallback_tab_title",
                    &[("profile_id", profile_id)],
                )
            });
        let tab = self.workspace_state.tabs.remove(index);
        if let TabKind::Session(session) = tab.kind
            && let Some(commands) = session.commands
            && let Err(error) = commands.close()
        {
            log::debug!("failed to close forwarding session cleanly: {error:?}");
        }
        self.remap_all_tab_indices_after_removal(&[index]);
        Some(title)
    }

    pub(in crate::ui::shell) fn connect_port_forward_rule(
        &mut self,
        profile_id: &str,
        rule_id: &str,
        cx: &mut Context<Self>,
    ) {
        if self.has_port_forward_rule_session(profile_id, rule_id) {
            self.status_message = i18n::string("forwarding.messages.already_connected");
            cx.notify();
            return;
        }

        let Some((profile, mut rule)) = self
            .port_forward_rule_indices(profile_id, rule_id)
            .and_then(|(profile_index, rule_index)| {
                self.data.sessions.get(profile_index).and_then(|profile| {
                    profile
                        .port_forwarding_rules
                        .get(rule_index)
                        .cloned()
                        .map(|rule| (profile.clone(), rule))
                })
            })
        else {
            self.status_message = i18n::string("forwarding.messages.rule_not_found");
            cx.notify();
            return;
        };

        rule.enabled = true;
        if self.profile_requires_local_vault_unlock(&profile) {
            self.prompt_local_vault_unlock(cx);
            return;
        }

        let mut connection_profile = profile.clone();
        connection_profile.port_forwarding_rules = vec![rule.clone()];
        let connection = miaominal_ssh::start_port_forward_session(
            &self.services.runtime,
            connection_profile,
            self.data.sessions.clone(),
            self.services.secrets.clone(),
            self.services.known_hosts.clone(),
        );
        let tab_id = {
            let next_id = self.workspace_state.next_tab_id;
            self.workspace_state.next_tab_id += 1;
            next_id
        };

        self.workspace_state
            .tabs
            .push(TabState::new_port_forwarding(
                tab_id,
                &profile,
                &rule,
                connection.commands,
            ));
        let synced_sessions = self.sync_port_forward_rules_for_profile(&profile.id);
        self.spawn_session_event_loop(tab_id, connection.events, cx);
        let rule_label = Self::rule_summary_label(&rule);
        let synced_suffix = self.synced_sessions_suffix(synced_sessions);
        self.status_message = i18n::string_args(
            "forwarding.messages.connecting",
            &[("rule", &rule_label), ("synced_suffix", &synced_suffix)],
        );
        cx.notify();
    }

    pub(in crate::ui::shell) fn disconnect_port_forward_rule(
        &mut self,
        profile_id: &str,
        rule_id: &str,
        cx: &mut Context<Self>,
    ) {
        let _ = self.update_port_forward_rule_enabled_state(profile_id, rule_id, false);
        let Some(title) = self.remove_port_forward_connection_tab(profile_id, rule_id) else {
            self.status_message = i18n::string("forwarding.messages.not_connected");
            cx.notify();
            return;
        };

        let synced_sessions = self.sync_port_forward_rules_for_profile(profile_id);
        let synced_suffix = self.synced_sessions_suffix(synced_sessions);
        if let Err(error) = self.persist_sessions_after_user_change(cx) {
            log::warn!("failed to persist disconnected port-forward rule state: {error:?}");
        }
        self.status_message = i18n::string_args(
            "forwarding.messages.disconnected",
            &[("title", &title), ("synced_suffix", &synced_suffix)],
        );
        cx.notify();
    }

    pub(in crate::ui::shell) fn online_session_count_for_profile(&self, profile_id: &str) -> usize {
        self.workspace_state
            .tabs
            .iter()
            .filter_map(TabState::as_session)
            .filter(|session| session.profile_id == profile_id && session.commands.is_some())
            .count()
    }

    pub(in crate::ui::shell) fn next_port_forward_rule_id(
        &self,
        profile: &SessionProfile,
    ) -> String {
        let mut next = profile.port_forwarding_rules.len() + 1;
        loop {
            let candidate = format!("pf-{next}");
            if profile
                .port_forwarding_rules
                .iter()
                .all(|rule| rule.id != candidate)
            {
                return candidate;
            }
            next += 1;
        }
    }

    pub(in crate::ui::shell) fn sync_port_forward_rules_for_profile(
        &mut self,
        profile_id: &str,
    ) -> usize {
        let Some(rules) = self
            .data
            .sessions
            .iter()
            .find(|profile| profile.id == profile_id)
            .map(|profile| profile.port_forwarding_rules.clone())
        else {
            return 0;
        };
        let dedicated_rule_ids: HashSet<_> = self
            .workspace_state
            .tabs
            .iter()
            .filter_map(TabState::as_session)
            .filter(|session| {
                session.profile_id == profile_id
                    && session.purpose == SessionPurpose::PortForwarding
                    && session.commands.is_some()
            })
            .filter_map(|session| session.port_forward_rule_id.clone())
            .collect();

        let mut synced_sessions = 0;
        for tab in &mut self.workspace_state.tabs {
            let Some(session) = tab.as_session_mut() else {
                continue;
            };
            if session.profile_id != profile_id {
                continue;
            }
            let Some(commands) = session.commands.as_ref() else {
                continue;
            };
            let rules_for_session = if session.purpose == SessionPurpose::PortForwarding {
                session
                    .port_forward_rule_id
                    .as_deref()
                    .map(|rule_id| {
                        rules
                            .iter()
                            .filter(|rule| rule.id == rule_id)
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                rules
                    .iter()
                    .filter(|rule| !dedicated_rule_ids.contains(&rule.id))
                    .cloned()
                    .collect()
            };
            if commands.sync_port_forward_rules(rules_for_session).is_ok() {
                synced_sessions += 1;
            }
        }

        synced_sessions
    }

    pub(in crate::ui::shell) fn synced_sessions_suffix(&self, count: usize) -> String {
        if count == 0 {
            String::new()
        } else {
            let count = count.to_string();
            if count == "1" {
                i18n::string_args(
                    "forwarding.messages.synced_suffix_one",
                    &[("count", &count)],
                )
            } else {
                i18n::string_args(
                    "forwarding.messages.synced_suffix_other",
                    &[("count", &count)],
                )
            }
        }
    }

    pub(in crate::ui::shell) fn forwarding_closed_connection_suffix(&self, closed: bool) -> String {
        if closed {
            i18n::string("forwarding.messages.closed_connection_suffix")
        } else {
            String::new()
        }
    }

    pub(in crate::ui::shell) fn forwarding_stopped_tunnel_suffix(&self, stopped: bool) -> String {
        if stopped {
            i18n::string("forwarding.messages.stopped_tunnel_suffix")
        } else {
            String::new()
        }
    }
}
