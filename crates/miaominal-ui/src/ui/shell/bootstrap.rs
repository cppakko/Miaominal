use super::*;
use crate::ui::i18n;
use crate::ui::shell::bootstrap_loaders::{LoadedAppData, load_app_data};
use crate::ui::shell::bootstrap_subscriptions::{AppViewSubscriptionsArgs, build_subscriptions};
use gpui_component::WindowExt as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubmitOverlayTarget {
    LocalVault(LocalVaultPassphrasePopupMode),
    AiProvider,
    WebSearch,
    SyncProvider,
}

fn submit_overlay_target(
    local_vault: Option<LocalVaultPassphrasePopupMode>,
    ai_provider: bool,
    web_search: bool,
    sync_provider: bool,
) -> Option<SubmitOverlayTarget> {
    local_vault
        .map(SubmitOverlayTarget::LocalVault)
        .or(ai_provider.then_some(SubmitOverlayTarget::AiProvider))
        .or(web_search.then_some(SubmitOverlayTarget::WebSearch))
        .or(sync_provider.then_some(SubmitOverlayTarget::SyncProvider))
}

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
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
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
            if !window
                .focused_input(cx)
                .is_some_and(|input| input.entity_id() == prompt_input.entity_id())
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

        if key == "enter"
            && !modifiers.control
            && !modifiers.alt
            && !modifiers.platform
            && !modifiers.shift
        {
            let mut submitted_popup = false;
            view.update(cx, |this, cx| {
                let target = submit_overlay_target(
                    this.pending_local_vault_passphrase_popup(cx),
                    this.pending_ai_provider_popup(cx).is_some(),
                    this.pending_web_search_config_popup(cx).is_some(),
                    this.pending_sync_provider_config_popup(cx).is_some(),
                );
                match target {
                    Some(SubmitOverlayTarget::LocalVault(mode)) => {
                        this.controllers.settings.update(cx, |controller, cx| {
                            controller.submit_local_vault_passphrase_popup_action(mode, window, cx);
                        });
                    }
                    Some(SubmitOverlayTarget::AiProvider) => {
                        this.controllers.settings.update(cx, |controller, cx| {
                            controller.submit_ai_provider_save(window, cx);
                        });
                    }
                    Some(SubmitOverlayTarget::WebSearch) => {
                        this.controllers.settings.update(cx, |controller, cx| {
                            controller.submit_web_search_settings_save(window, cx);
                        });
                    }
                    Some(SubmitOverlayTarget::SyncProvider) => {
                        this.controllers.settings.update(cx, |controller, cx| {
                            controller.submit_sync_provider_config_popup_action(cx);
                        });
                    }
                    None => return,
                }
                submitted_popup = true;
            });
            if submitted_popup {
                cx.stop_propagation();
                return;
            }
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
        Self::bootstrap(runtime, window, cx)
    }

    fn bootstrap(runtime: TokioHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let terminal_focus = cx.focus_handle();
        let settings_store = match SettingsStore::load() {
            Ok(store) => store,
            Err(error) => {
                log::warn!("settings unavailable, using defaults: {error:?}");
                SettingsStore::fallback()
            }
        };
        crate::ui::i18n::set_language(settings_store.settings().language);
        miaominal_settings::sync_component_theme(cx);
        let local_vault_enabled = settings_store.settings().local_vault_enabled;
        let LoadedAppData {
            services,
            profiles,
            selected_profile,
            known_hosts_entries,
            snippets,
            selected_snippet,
            managed_keys,
            chat_service,
            chat_sessions,
            status_message,
        } = load_app_data(runtime, local_vault_enabled);
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
        let local_vault_status = if local_vault_enabled {
            LocalVaultStatus::Locked
        } else {
            LocalVaultStatus::Disabled
        };
        let auto_collect_session_monitoring =
            settings_store.settings().auto_collect_session_monitoring;
        let controllers = ControllerSet::new(
            SessionControllerArgs {
                runtime: services.runtime.clone(),
                session_store: services.session_store.clone(),
                snippet_store: services.snippet_store.clone(),
                secrets: services.secrets.clone(),
                known_hosts: services.known_hosts.clone(),
                profiles,
                selected_profile,
                managed_keys: managed_keys.clone(),
                snippets,
                selected_snippet,
                known_hosts_entries,
                terminal_focus,
                local_vault_status,
                auto_collect_session_monitoring,
            },
            AgentControllerArgs {
                task_runtime: services.runtime.clone(),
                agent_service: services.agent_service.clone(),
                secrets: services.secrets.clone(),
                known_hosts: services.known_hosts.clone(),
                chat_service,
                chat_sessions,
                local_vault_status,
            },
            SftpControllerArgs {
                service: miaominal_services::SftpService::new(
                    services.runtime.clone(),
                    services.secrets.clone(),
                    services.known_hosts.clone(),
                ),
                local_hidden_columns: settings_store.settings().local_sftp_hidden_columns.clone(),
                remote_hidden_columns: settings_store.settings().remote_sftp_hidden_columns.clone(),
            },
            KeychainControllerArgs {
                managed_keys,
                runtime: services.runtime.clone(),
                keychain_store: services.keychain_store.clone(),
                secrets: services.secrets.clone(),
                known_hosts: services.known_hosts.clone(),
                local_vault_status,
            },
            SettingsControllerArgs {
                runtime: services.runtime.clone(),
                session_store: services.session_store.clone(),
                snippet_store: services.snippet_store.clone(),
                keychain_store: services.keychain_store.clone(),
                settings_store,
                secrets: services.secrets.clone(),
            },
            window,
            cx,
        );
        let controller_subscriptions = controllers.root_subscriptions(window, cx);

        let mut view = Self {
            controllers,
            workspace,
            shell: ShellUiState {
                workspace_forms,
                shell_state: ShellState::default(),
                exiting_dialogs: Vec::new(),
                status_message,
                deferred_app_command: None,
            },
            _subscriptions: RootSubscriptions::new(
                build_subscriptions(AppViewSubscriptionsArgs {
                    rename_subscription,
                    keystroke_interceptor,
                }),
                controller_subscriptions,
            ),
        };

        view.refresh_localized_placeholders(window, cx);
        view.sync_terminal_focus_reporting(window, cx);

        view
    }

    pub(in crate::ui::shell) fn refresh_localized_placeholders(
        &self,
        window: &mut Window,
        cx: &mut App,
    ) {
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
    fn overlay_submit_priority_is_vault_then_ai_then_web_then_sync() {
        let vault = LocalVaultPassphrasePopupMode::PrimaryAction;
        assert_eq!(
            submit_overlay_target(Some(vault), true, true, true),
            Some(SubmitOverlayTarget::LocalVault(vault))
        );
        assert_eq!(
            submit_overlay_target(None, true, true, true),
            Some(SubmitOverlayTarget::AiProvider)
        );
        assert_eq!(
            submit_overlay_target(None, false, true, true),
            Some(SubmitOverlayTarget::WebSearch)
        );
        assert_eq!(
            submit_overlay_target(None, false, false, true),
            Some(SubmitOverlayTarget::SyncProvider)
        );
        assert_eq!(submit_overlay_target(None, false, false, false), None);
    }

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
