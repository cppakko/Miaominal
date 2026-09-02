use super::*;
use crate::ui::application::{
    ApplicationBootstrapSnapshot, ApplicationVaultSnapshot, ApplicationVaultStatus,
    application_state,
};
use crate::ui::i18n;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VaultLockPresentation {
    Silent,
    StatusOnly,
    Toast,
}

fn vault_lock_presentation(automatic_lock: bool, window_active: bool) -> VaultLockPresentation {
    match (automatic_lock, window_active) {
        (true, true) => VaultLockPresentation::Toast,
        (true, false) => VaultLockPresentation::StatusOnly,
        (false, _) => VaultLockPresentation::Silent,
    }
}

impl AppView {
    pub(in crate::ui::shell) fn apply_application_snapshot(
        &mut self,
        snapshot: ApplicationBootstrapSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.applying_application_snapshot = true;
        let snapshot_sync_engine = snapshot.vault.sync_engine.clone();

        if snapshot.generations.catalogs != self.application_generations.catalogs {
            self.controllers
                .session
                .read(cx)
                .replace_profiles_from_application(snapshot.profiles);
            self.controllers
                .session
                .read(cx)
                .replace_proxies_from_application(snapshot.proxies.clone());
            self.controllers
                .session
                .read(cx)
                .replace_snippets(snapshot.snippets);
            self.controllers
                .session
                .read(cx)
                .replace_known_hosts_entries(snapshot.known_hosts_entries);
            self.controllers.settings.update(cx, |controller, cx| {
                controller.replace_proxies(snapshot.proxies, window, cx);
            });
            self.controllers.keychain.update(cx, |controller, cx| {
                controller.replace_managed_keys(snapshot.managed_keys, cx);
            });
            let managed_key_options = ManagedKeySelectItem::sorted_items(
                self.controllers.keychain.read(cx).managed_keys(),
            );
            self.controllers.session.update(cx, |controller, cx| {
                controller.refresh_entry_proxy_select(window, cx);
                controller.sync_managed_key_select_in_active_window(managed_key_options, None, cx);
            });
            self.application_generations.catalogs = snapshot.generations.catalogs;
        }

        if snapshot.generations.settings != self.application_generations.settings {
            let previous_settings = self.controllers.settings.read(cx).settings().clone();
            let next_settings = snapshot.settings_store.settings();
            let language_changed = previous_settings.language != next_settings.language;
            let auto_collect_changed = previous_settings.auto_collect_session_monitoring
                != next_settings.auto_collect_session_monitoring;
            let sftp_columns_changed = previous_settings.local_sftp_hidden_columns
                != next_settings.local_sftp_hidden_columns
                || previous_settings.remote_sftp_hidden_columns
                    != next_settings.remote_sftp_hidden_columns;
            let next_auto_collect = next_settings.auto_collect_session_monitoring;
            let next_local_hidden_columns = next_settings.local_sftp_hidden_columns.clone();
            let next_remote_hidden_columns = next_settings.remote_sftp_hidden_columns.clone();
            self.controllers.settings.update(cx, |controller, cx| {
                controller.replace_application_settings(
                    snapshot.settings_store,
                    snapshot_sync_engine.clone(),
                    window,
                    cx,
                );
            });
            if language_changed {
                crate::ui::i18n::set_language(
                    self.controllers.settings.read(cx).settings().language,
                );
                self.refresh_localized_placeholders(window, cx);
            }
            if auto_collect_changed {
                self.controllers.session.update(cx, |controller, cx| {
                    controller.apply_auto_collect_monitoring_preference(next_auto_collect, cx);
                });
            }
            if sftp_columns_changed {
                self.controllers.sftp.update(cx, |controller, cx| {
                    controller.apply_hidden_columns(
                        next_local_hidden_columns,
                        next_remote_hidden_columns,
                        cx,
                    );
                });
            }
            miaominal_settings::sync_component_theme(cx);
            crate::ui::sync_system_tray(cx);
            self.application_generations.settings = snapshot.generations.settings;
        }

        if snapshot.generations.vault != self.application_generations.vault {
            let vault_locked = snapshot.vault.status == ApplicationVaultStatus::Locked;
            let lock_presentation =
                vault_lock_presentation(snapshot.vault.automatic_lock, window.is_window_active());
            if vault_locked {
                // Reconcile visible inputs while this window still owns the unlocked secret store.
                self.controllers.session.update(cx, |controller, cx| {
                    controller.prepare_host_password_for_lock(window, cx);
                });
            }
            self.apply_application_vault_snapshot(snapshot.vault, cx);
            if vault_locked {
                self.controllers.session.update(cx, |controller, cx| {
                    controller.set_host_password_visibility(false, false, window, cx);
                });
                let message =
                    self.controllers.settings.update(
                        cx,
                        |controller, cx| match lock_presentation {
                            VaultLockPresentation::Toast => {
                                Some(controller.finish_local_vault_lock(window, cx))
                            }
                            VaultLockPresentation::StatusOnly | VaultLockPresentation::Silent => {
                                controller.apply_application_vault_lock(window, cx);
                                (lock_presentation == VaultLockPresentation::StatusOnly).then(
                                    || {
                                        i18n::string(
                                            "settings.sync.vault.notifications.locked_message",
                                        )
                                    },
                                )
                            }
                        },
                    );
                if let Some(message) = message {
                    self.shell.status_message = message;
                }
            }
            self.application_generations.vault = snapshot.generations.vault;
        }

        if snapshot.generations.chat != self.application_generations.chat {
            self.controllers.agent.update(cx, |controller, cx| {
                controller.replace_chat_state(snapshot.chat_service, snapshot.chat_sessions, cx);
            });
            if let Some(message) = snapshot.chat_status_message {
                self.shell.status_message = message;
            }
            self.application_generations.chat = snapshot.generations.chat;
        }

        if snapshot.generations.port_forwards != self.application_generations.port_forwards {
            if window.is_window_active() {
                self.ensure_port_forward_prompt_views(&snapshot.port_forwards, cx);
            }
            let updates = self
                .controllers
                .session
                .read(cx)
                .apply_port_forward_manager_snapshot(
                    &snapshot.port_forwards,
                    window.is_window_active(),
                );
            for (tab_id, status, notification) in updates {
                if let Some(mut tab) = self.workspace.tabs.get_mut(tab_id) {
                    tab.status = status;
                }
                if let Some(notification) = notification {
                    crate::ui::shell::session::push_session_notification(
                        notification.tone,
                        notification.title,
                        notification.message,
                        notification.id,
                        window,
                        cx,
                    );
                }
            }
            self.application_generations.port_forwards = snapshot.generations.port_forwards;
        }

        if snapshot.generations.bridge != self.application_generations.bridge {
            self.controllers.settings.update(cx, |controller, cx| {
                controller.apply_application_bridge_snapshot(
                    snapshot.bridge_status,
                    snapshot.bridge_sync_result,
                    snapshot.bridge_security,
                    window.is_window_active()
                        && crate::ui::bridge_security_platform::is_app_foreground(),
                    window,
                    cx,
                );
            });
            self.application_generations.bridge = snapshot.generations.bridge;
        }

        if snapshot.generations.auto_sync != self.application_generations.auto_sync {
            self.controllers.settings.update(cx, |controller, cx| {
                controller.apply_auto_sync_snapshot(
                    snapshot.auto_sync.clone(),
                    snapshot_sync_engine,
                    cx,
                );
            });
            self.application_generations.auto_sync = snapshot.generations.auto_sync;
        }

        self.applying_application_snapshot = false;
        cx.notify();
    }

    pub(in crate::ui::shell) fn refresh_application_runtime_for_window(
        &mut self,
        active: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snapshot = application_state(cx).read(cx).snapshot();
        if active {
            self.ensure_port_forward_prompt_views(&snapshot.port_forwards, cx);
        }
        let updates = self
            .controllers
            .session
            .read(cx)
            .apply_port_forward_manager_snapshot(&snapshot.port_forwards, active);
        for (tab_id, status, notification) in updates {
            if let Some(mut tab) = self.workspace.tabs.get_mut(tab_id) {
                tab.status = status;
            }
            if let Some(notification) = notification {
                crate::ui::shell::session::push_session_notification(
                    notification.tone,
                    notification.title,
                    notification.message,
                    notification.id,
                    window,
                    cx,
                );
            }
        }
        self.controllers.settings.update(cx, |controller, cx| {
            controller.apply_application_bridge_snapshot(
                snapshot.bridge_status,
                snapshot.bridge_sync_result,
                snapshot.bridge_security,
                active && crate::ui::bridge_security_platform::is_app_foreground(),
                window,
                cx,
            );
        });
        cx.notify();
    }

    fn ensure_port_forward_prompt_views(
        &mut self,
        snapshot: &miaominal_services::PortForwardManagerSnapshot,
        cx: &mut Context<Self>,
    ) {
        let pending = snapshot
            .sessions
            .iter()
            .filter(|runtime| runtime.prompt.is_some())
            .filter(|runtime| {
                !self
                    .controllers
                    .session
                    .read(cx)
                    .has_port_forward_rule_session(&runtime.key.profile_id, &runtime.key.rule_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        for runtime in pending {
            let tab_id = self.workspace.allocate_tab_id();
            let Some((tab, session)) = self
                .controllers
                .session
                .read(cx)
                .build_port_forward_status_tab(tab_id, &runtime)
            else {
                continue;
            };
            self.insert_session_tab(tab, session, cx);
        }
    }

    pub(in crate::ui::shell) fn apply_application_vault_snapshot(
        &mut self,
        snapshot: ApplicationVaultSnapshot,
        cx: &mut Context<Self>,
    ) {
        let local_vault_status = match snapshot.status {
            ApplicationVaultStatus::Disabled => LocalVaultStatus::Disabled,
            ApplicationVaultStatus::Locked => LocalVaultStatus::Locked,
            ApplicationVaultStatus::Unlocked => LocalVaultStatus::Unlocked,
        };
        self.controllers.settings.update(cx, |controller, _| {
            controller.replace_sync_engine(snapshot.sync_engine);
            controller.set_local_vault_status(local_vault_status);
            controller.set_local_vault_session_passphrase(snapshot.session_passphrase);
        });
        self.controllers.broadcast_credentials_changed(
            snapshot.secrets,
            snapshot.agent_service,
            local_vault_status,
            cx,
        );
    }

    pub(in crate::ui::shell) fn publish_session_application_state(&self, cx: &mut Context<Self>) {
        if self.applying_application_snapshot {
            return;
        }
        let (profiles, proxies, snippets, known_hosts) = {
            let controller = self.controllers.session.read(cx);
            (
                controller.profiles().clone(),
                controller.proxies().clone(),
                controller.snippets().clone(),
                controller.known_hosts_entries().clone(),
            )
        };

        application_state(cx).update(cx, |application, cx| {
            application.publish_catalogs(profiles, proxies, snippets, known_hosts, cx);
        });
    }

    pub(in crate::ui::shell) fn publish_settings_application_state(&self, cx: &mut Context<Self>) {
        if self.applying_application_snapshot {
            return;
        }
        let (settings_store, proxies, sync_engine) = {
            let controller = self.controllers.settings.read(cx);
            (
                controller.settings_store(),
                controller.proxies().to_vec(),
                controller.sync_engine().clone(),
            )
        };

        application_state(cx).update(cx, |application, cx| {
            application.publish_settings(settings_store, proxies, sync_engine, cx);
        });
    }

    pub(in crate::ui::shell) fn publish_managed_keys_application_state(
        &self,
        cx: &mut Context<Self>,
    ) {
        if self.applying_application_snapshot {
            return;
        }
        let managed_keys = self.controllers.keychain.read(cx).managed_keys().to_vec();
        application_state(cx).update(cx, |application, cx| {
            application.publish_managed_keys(managed_keys, cx);
        });
    }

    pub(in crate::ui::shell) fn publish_chat_application_state(&self, cx: &mut Context<Self>) {
        if self.applying_application_snapshot {
            return;
        }
        let chat_sessions = self.controllers.agent.read(cx).chat_sessions().to_vec();
        application_state(cx).update(cx, |application, cx| {
            application.publish_chat_sessions(chat_sessions, cx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_vault_lock_notifies_only_the_active_window() {
        assert_eq!(
            vault_lock_presentation(true, true),
            VaultLockPresentation::Toast
        );
        assert_eq!(
            vault_lock_presentation(true, false),
            VaultLockPresentation::StatusOnly
        );
        assert_eq!(
            vault_lock_presentation(false, true),
            VaultLockPresentation::Silent
        );
    }
}
