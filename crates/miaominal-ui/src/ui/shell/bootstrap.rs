use super::*;
use crate::ui::application::{ApplicationBootstrapSnapshot, ApplicationVaultStatus};
use crate::ui::i18n;
use crate::ui::shell::bootstrap_subscriptions::{AppViewSubscriptionsArgs, build_subscriptions};
use gpui_kit::component::WindowExt as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocaleRefreshDomain {
    Session,
    Agent,
    Sftp,
    Settings,
    Keychain,
}

const fn locale_refresh_domains() -> [LocaleRefreshDomain; 5] {
    [
        LocaleRefreshDomain::Session,
        LocaleRefreshDomain::Agent,
        LocaleRefreshDomain::Sftp,
        LocaleRefreshDomain::Settings,
        LocaleRefreshDomain::Keychain,
    ]
}

fn build_keystroke_interceptor(cx: &mut Context<AppView>) -> Subscription {
    let weak = cx.weak_entity();
    cx.intercept_keystrokes(move |event, window, cx| {
        if let Some(view) = weak.upgrade() {
            view.update(cx, |this, cx| {
                this.remember_overlay_trigger_focus(window, cx)
            });
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if !modifiers.control && !modifiers.alt && !modifiers.platform && !modifiers.shift {
            let consumed = weak.upgrade().is_some_and(|view| {
                view.update(cx, |this, cx| {
                    let consumed = this.shell.overlay_state.handle_plain_key(key, window, cx);
                    if consumed {
                        this.sync_window_overlays(window, cx);
                    }
                    consumed
                })
            });
            if consumed {
                cx.stop_propagation();
                return;
            }
        }
        if !modifiers.control && !modifiers.alt && !modifiers.platform && !modifiers.shift {
            let consumed = weak.upgrade().is_some_and(|view| {
                view.update(cx, |this, cx| {
                    this.shell
                        .search
                        .update(cx, |search, cx| search.handle_key(key, window, cx))
                })
            });
            if consumed {
                cx.stop_propagation();
                return;
            }
        }
        if key == "escape"
            && !modifiers.control
            && !modifiers.alt
            && !modifiers.platform
            && !modifiers.shift
        {
            let Some(view) = weak.upgrade() else { return };
            let mut consumed = false;
            view.update(cx, |this, cx| {
                let dismissed_mention = this
                    .controllers
                    .agent
                    .update(cx, |controller, cx| controller.dismiss_at_mention(cx));
                if dismissed_mention {
                    consumed = true;
                } else if this
                    .controllers
                    .settings
                    .read(cx)
                    .recording_binding()
                    .is_some()
                {
                    let settings = this.controllers.settings.clone();
                    settings.update(cx, |controller, cx| {
                        controller.cancel_recording_key_binding(cx);
                    });
                    consumed = true;
                } else if this.controllers.session.read(cx).terminal_search_open() {
                    this.close_terminal_search(window, cx);
                    consumed = true;
                } else if this.workspace.renaming_tab.is_some() {
                    this.cancel_rename_tab(cx);
                    consumed = true;
                } else if this.workspace.active_topbar_tab.is_some_and(|tab_id| {
                    this.workspace
                        .tabs
                        .get(tab_id)
                        .is_some_and(|tab| tab.is_sftp())
                        && this.controllers.sftp.read(cx).has_inline_rename(tab_id)
                }) {
                    let controller = this.controllers.sftp.clone();
                    controller.update(cx, |controller, cx| {
                        controller.cancel_inline_rename(cx);
                    });
                    consumed = true;
                }
            });
            if consumed {
                cx.stop_propagation();
                return;
            }
        }

        let Some(view) = weak.upgrade() else {
            return;
        };
        if view
            .read(cx)
            .controllers
            .settings
            .read(cx)
            .recording_binding()
            .is_some()
        {
            return;
        }

        let mut handled_session_agent_prompt_shortcut = false;
        view.update(cx, |this, cx| {
            let agent = this.controllers.agent.clone();
            let prompt_input = agent.read(cx).prompt_input();
            if window
                .focused_input(cx)
                .is_none_or(|input| input.as_textarea() != Some(&prompt_input))
            {
                return;
            }

            let command_modifier = modifiers.control || modifiers.platform;
            let command_only = command_modifier && !modifiers.alt && !modifiers.shift;
            let plain_key =
                !modifiers.control && !modifiers.alt && !modifiers.platform && !modifiers.shift;

            match key {
                "enter" if plain_key => {
                    this.sync_session_port_snapshot(cx);
                    agent.update(cx, |controller, cx| {
                        controller.submit_session_agent_prompt(window, cx);
                    });
                    handled_session_agent_prompt_shortcut = true;
                }
                "k" if command_only => {
                    agent.update(cx, |controller, cx| {
                        controller.clear_prompt_input(window, cx);
                    });
                    handled_session_agent_prompt_shortcut = true;
                }
                "n" if command_only => {
                    agent.update(cx, |controller, cx| {
                        controller.finish_text_drag(cx);
                        controller.start_new_conversation(window, cx);
                    });
                    handled_session_agent_prompt_shortcut = true;
                }
                "up" if plain_key => {
                    handled_session_agent_prompt_shortcut = agent.update(cx, |controller, cx| {
                        controller.browse_prompt_history(
                            PromptHistoryDirection::Previous,
                            window,
                            cx,
                        )
                    });
                }
                "down" if plain_key => {
                    handled_session_agent_prompt_shortcut = agent.update(cx, |controller, cx| {
                        controller.browse_prompt_history(PromptHistoryDirection::Next, window, cx)
                    });
                }
                _ => {}
            }
        });
        if handled_session_agent_prompt_shortcut {
            cx.stop_propagation();
            return;
        }

        let mut handled_global_shortcut = false;
        view.update(cx, |this, cx| {
            handled_global_shortcut = this.handle_global_shortcut(&event.keystroke, window, cx);
        });
        if handled_global_shortcut {
            cx.stop_propagation();
            return;
        }

        if key != "tab" {
            return;
        }
        if modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        // Use the live active pane focus handle so that it stays correct after
        // tab/workspace switches (which replace the focus handle).
        if !view
            .read(cx)
            .workspace
            .workspace
            .active_pane
            .terminal_focus
            .is_focused(window)
        {
            return;
        }
        let key_event = KeyDownEvent {
            keystroke: event.keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        };
        view.update(cx, |this, cx| {
            this.handle_terminal_key_down(&key_event, window, cx);
            window.focus(&this.workspace.workspace.active_pane.terminal_focus, cx);
        });
        cx.stop_propagation();
    })
}

impl AppView {
    pub fn new(runtime: TokioHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::ui::initialize_application_state(runtime, cx);
        initialize_notification_center(cx);
        Self::bootstrap(AppWindowRole::Primary, window, cx)
    }

    pub fn new_detached(runtime: TokioHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::ui::initialize_application_state(runtime, cx);
        initialize_notification_center(cx);
        Self::bootstrap(AppWindowRole::Detached, window, cx)
    }

    fn bootstrap(window_role: AppWindowRole, window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::ui::components::notifications::set_notification_action_handler(
            cx,
            |action, window, cx| match action {
                AppNotificationAction::OpenSyncSettings => {
                    let Some(main_view) = crate::ui::system_tray::main_window_view(cx) else {
                        log::warn!(
                            "cannot open sync settings because the main AppView is unavailable"
                        );
                        return;
                    };
                    if !crate::ui::system_tray::is_main_window(window, cx) {
                        crate::ui::restore_main_window(cx);
                    }
                    let Some(main_window) = crate::ui::system_tray::main_window_handle(cx) else {
                        return;
                    };
                    if let Err(error) = main_window.update(cx, |_, window, cx| {
                        main_view.update(cx, |view, cx| {
                            view.navigate_to_settings_destination(SettingsDestination::Sync, cx);
                            view.sync_window_overlays(window, cx);
                        });
                    }) {
                        log::warn!("cannot open sync settings in the main window: {error:?}");
                    }
                }
            },
        );
        let terminal_focus = cx.focus_handle();
        let application = crate::ui::application::application_state(cx);
        let ApplicationBootstrapSnapshot {
            services,
            settings_store,
            profiles,
            proxies,
            selected_profile,
            known_hosts_entries,
            snippets: _,
            managed_keys,
            chat_service,
            chat_sessions,
            chat_status_message: _,
            mut status_message,
            vault,
            generations,
            port_forwards: _,
            bridge_status,
            bridge_sync_result,
            bridge_security,
            sync_executor,
            auto_sync,
        } = application.read(cx).snapshot();
        let initialization_warning =
            miaominal_paths::initialization_outcome()
                .ok()
                .and_then(|context| {
                    context.warning()?;
                    let key = match context.warning_kind() {
                        Some(
                            miaominal_paths::RuntimeWarningKind::DataMigrationBeforeSwitchFailed,
                        ) => "settings.about.data_directory.migration_before_switch_warning",
                        Some(miaominal_paths::RuntimeWarningKind::DataMigrationCleanupPending) => {
                            "settings.about.data_directory.migration_cleanup_warning"
                        }
                        None => "settings.about.data_directory.migration_warning",
                    };
                    Some(i18n::string(key))
                });
        if let Some(warning) = initialization_warning.as_ref() {
            status_message = warning.clone();
        }
        let rename_input = new_input_state(
            i18n::string("placeholders.workspace.tab_name"),
            "",
            false,
            window,
            cx,
        );
        let rename_subscription = cx.subscribe(
            &rename_input,
            |this: &mut AppView, _, event: &InputEvent, cx| match event {
                InputEvent::PressEnter { .. } => this.commit_rename_tab(cx),
                InputEvent::Blur => this.commit_rename_tab(cx),
                _ => {}
            },
        );
        let keystroke_interceptor = build_keystroke_interceptor(cx);

        let workspace_forms = WorkspaceForms { rename_input };
        let workspace = WorkspaceModel::new(
            TabState::new_hosts(TabId::new(0)),
            TabId::new(1),
            Self::initial_workspace(terminal_focus.clone()),
        );
        let local_vault_status = match vault.status {
            ApplicationVaultStatus::Disabled => LocalVaultStatus::Disabled,
            ApplicationVaultStatus::Locked => LocalVaultStatus::Locked,
            ApplicationVaultStatus::Unlocked => LocalVaultStatus::Unlocked,
        };
        let auto_collect_session_monitoring =
            settings_store.settings().auto_collect_session_monitoring;
        let controllers = ControllerSet::new(
            super::controllers::ControllerSetArgs {
                hosts: super::HostsControllerArgs {
                    catalog_query: application.read(cx).catalog_query(),
                    profile_commit: crate::ui::application::profile_commit_port(cx),
                    session_store: services.session_store.clone(),
                    secrets: vault.secrets.clone(),
                    profiles: profiles.clone(),
                    proxies: proxies.clone(),
                    selected_profile,
                    managed_keys: managed_keys.clone(),
                    local_vault_status,
                },
                snippet_store: services.snippet_store.clone(),
                known_hosts_entries,
                session: SessionControllerArgs {
                    runtime: services.runtime.clone(),
                    secrets: vault.secrets.clone(),
                    known_hosts: services.known_hosts.clone(),
                    profiles,
                    proxies: proxies.clone(),
                    selected_profile,
                    terminal_focus,
                    local_vault_status,
                    auto_collect_session_monitoring,
                    ssh_bridge_service: services.ssh_bridge_service.clone(),
                    port_forward_manager: services.port_forward_manager.clone(),
                    rdp_service: services.rdp_service.clone(),
                },
                agent: AgentControllerArgs {
                    task_runtime: services.runtime.clone(),
                    agent_service: services.agent_service.clone(),
                    secrets: vault.secrets.clone(),
                    known_hosts: services.known_hosts.clone(),
                    chat_service,
                    chat_sessions,
                    local_vault_status,
                },
                sftp: SftpControllerArgs {
                    service: miaominal_services::SftpService::new(
                        services.runtime.clone(),
                        vault.secrets.clone(),
                        services.known_hosts.clone(),
                    ),
                    local_hidden_columns: settings_store
                        .settings()
                        .local_sftp_hidden_columns
                        .clone(),
                    remote_hidden_columns: settings_store
                        .settings()
                        .remote_sftp_hidden_columns
                        .clone(),
                },
                keychain: KeychainControllerArgs {
                    managed_key_query: crate::ui::application::ApplicationState::managed_key_ports(
                        &application,
                    )
                    .0,
                    managed_key_commit:
                        crate::ui::application::ApplicationState::managed_key_ports(&application).1,
                    managed_keys,
                    runtime: services.runtime.clone(),
                    keychain_store: services.keychain_store.clone(),
                    secrets: vault.secrets.clone(),
                    known_hosts: services.known_hosts.clone(),
                    local_vault_status,
                },
                settings: SettingsControllerArgs {
                    persistence:
                        crate::ui::application::ApplicationState::settings_persistence_port(
                            &application,
                        ),
                    auto_sync_control:
                        crate::ui::application::ApplicationState::auto_sync_control_port(
                            &application,
                        ),
                    proxy_commit: crate::ui::application::ApplicationState::proxy_commit_port(
                        &application,
                    ),
                    runtime: services.runtime.clone(),
                    proxy_store: services.proxy_store.clone(),
                    proxies: proxies.clone(),
                    settings_store,
                    secrets: vault.secrets.clone(),
                    sync_engine: vault.sync_engine.clone(),
                    local_vault_status,
                    ssh_bridge_service: services.ssh_bridge_service.clone(),
                    open_ssh_integration_service: services.open_ssh_integration_service.clone(),
                    sync_executor: sync_executor.clone(),
                    auto_sync: auto_sync.clone(),
                },
            },
            window,
            cx,
        );
        controllers.settings.update(cx, |controller, cx| {
            controller.set_local_vault_session_passphrase(vault.session_passphrase.clone());
            controller.apply_application_bridge_snapshot(
                bridge_status,
                bridge_sync_result,
                bridge_security,
                window.is_window_active()
                    && crate::ui::bridge_security_platform::is_app_foreground(),
                window,
                cx,
            );
        });
        let mut controller_subscriptions = controllers.root_subscriptions(window, cx);
        if window_role == AppWindowRole::Primary {
            controller_subscriptions.push(cx.observe_window_activation(
                window,
                |_view, window, cx| {
                    if window.is_window_active() {
                        show_pending_app_notifications(window, cx);
                    }
                },
            ));
        }
        controller_subscriptions.push(cx.observe_in(
            &application,
            window,
            |this, application, window, cx| {
                let snapshot = application.read(cx).snapshot();
                this.apply_application_snapshot(snapshot, window, cx);
            },
        ));

        let search = cx.new(|cx| crate::ui::features::search::SearchView::new(window, cx));
        controller_subscriptions.push(cx.observe(&search, |_, _, cx| cx.notify()));
        controller_subscriptions.push(cx.subscribe_in(
            &search,
            window,
            |this, _, event, window, cx| this.handle_management_search_event(event, window, cx),
        ));
        let mut view = Self {
            controllers,
            workspace,
            shell: ShellUiState {
                topbar_tab_focus: HashMap::new(),
                search,
                search_tab: None,
                search_generation: None,
                unattached_search_query: String::new(),
                pane_animation_frame_pending: false,
                workspace_forms,
                shell_state: ShellState::default(),
                overlay_state: super::overlays::WindowOverlayState::default(),
                status_message,
                deferred_app_command: None,
                dock_action_motion: Default::default(),
            },
            application_generations: generations,
            applying_application_snapshot: false,
            window_role,
            _subscriptions: RootSubscriptions::new(
                build_subscriptions(AppViewSubscriptionsArgs {
                    rename_subscription,
                    keystroke_interceptor,
                }),
                controller_subscriptions,
            ),
        };

        if let Some(warning) = initialization_warning {
            crate::ui::shell::push_app_notification(
                window,
                error_notification(
                    i18n::string("settings.about.data_directory.migration_warning_title"),
                    warning,
                ),
                cx,
            );
        }

        view.refresh_localized_placeholders(window, cx);
        view.sync_terminal_focus_reporting(window, cx);

        view
    }

    pub(in crate::ui::shell) fn refresh_localized_placeholders(
        &self,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.shell
            .search
            .update(cx, |search, cx| search.refresh_locale(window, cx));
        self.controllers.forwarding.update(cx, |controller, cx| {
            controller.refresh_localized_placeholders(window, cx);
        });
        self.controllers.snippets.update(cx, |controller, cx| {
            controller.refresh_localized_placeholders(window, cx);
        });
        self.controllers.hosts.update(cx, |controller, cx| {
            controller.refresh_localized_placeholders(window, cx);
        });
        for domain in locale_refresh_domains() {
            match domain {
                LocaleRefreshDomain::Session => {
                    self.controllers.session.update(cx, |controller, cx| {
                        controller.refresh_localized_placeholders(window, cx);
                    });
                }
                LocaleRefreshDomain::Agent => {
                    self.controllers.agent.update(cx, |controller, cx| {
                        controller.refresh_localized_placeholders(window, cx);
                    });
                }
                LocaleRefreshDomain::Sftp => {
                    self.controllers.sftp.update(cx, |controller, cx| {
                        controller.refresh_localized_placeholders(window, cx);
                    });
                }
                LocaleRefreshDomain::Settings => {
                    self.controllers.settings.update(cx, |controller, cx| {
                        controller.refresh_localized_placeholders(window, cx);
                    });
                }
                LocaleRefreshDomain::Keychain => {
                    self.controllers.keychain.update(cx, |controller, cx| {
                        controller.refresh_localized_placeholders(window, cx);
                    });
                }
            }
        }
        set_input_placeholder(
            &self.shell.workspace_forms.rename_input,
            i18n::string("placeholders.workspace.tab_name"),
            window,
            cx,
        );
    }
    fn initial_workspace(terminal_focus: FocusHandle) -> TabWorkspaceState {
        TabWorkspaceState::new(None, terminal_focus)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_refresh_broadcast_covers_all_controller_domains() {
        assert_eq!(
            locale_refresh_domains(),
            [
                LocaleRefreshDomain::Session,
                LocaleRefreshDomain::Agent,
                LocaleRefreshDomain::Sftp,
                LocaleRefreshDomain::Settings,
                LocaleRefreshDomain::Keychain,
            ]
        );
    }
}
