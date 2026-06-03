use super::bootstrap_form_factory::{HostEditorFormsArgs, PanelFormsArgs, WorkspaceFormsArgs};
use super::*;
use crate::ui::i18n;
use crate::ui::shell::bootstrap_loaders::{InitialProfileSelection, LoadedAppData};
use crate::ui::shell::bootstrap_subscriptions::AppViewSubscriptionsArgs;
use gpui_component::IndexPath;
use miaominal_core::profile::{DEFAULT_SESSION_CHARSET, ImportSourceKind};
use miaominal_settings::AppLanguage;
use miaominal_sync::SyncProvider;

impl AppView {
    fn font_family_options(current_font_family: &str) -> Vec<String> {
        let mut families = miaominal_settings::available_font_families();
        let default_font_family = miaominal_settings::default_font_family();
        if !families
            .iter()
            .any(|family| family.eq_ignore_ascii_case(&default_font_family))
        {
            families.push(default_font_family);
        }

        let trimmed_current = current_font_family.trim();
        if !trimmed_current.is_empty()
            && !families
                .iter()
                .any(|family| family.eq_ignore_ascii_case(trimmed_current))
        {
            families.push(trimmed_current.to_string());
        }

        families.sort_by_cached_key(|family| family.to_ascii_lowercase());
        families.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        families
    }

    pub fn new(runtime: TokioHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
            data,
            status_message,
        } = Self::load_app_data(runtime, local_vault_enabled);
        let InitialProfileSelection {
            selected_profile_data,
            editing_auth_method,
            available_groups,
            selected_group,
            selected_existing_group,
        } = Self::initial_profile_selection(&data);
        let name_input = new_input_state(
            i18n::string("placeholders.host_editor.profile_name"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.name.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let group_input = new_input_state(
            i18n::string("placeholders.host_editor.new_group_name"),
            if selected_existing_group.is_none() {
                selected_group.clone()
            } else {
                String::new()
            },
            false,
            window,
            cx,
        );
        let tags_input = new_input_state(
            i18n::string("placeholders.host_editor.tags"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.tags.join(", "))
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let host_input = new_input_state(
            i18n::string("placeholders.host_editor.host"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.host.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let port_input = new_input_state(
            "22",
            selected_profile_data
                .as_ref()
                .map(|profile| profile.port.to_string())
                .unwrap_or_else(|| "22".to_string()),
            false,
            window,
            cx,
        );
        let username_input = new_input_state(
            i18n::string("placeholders.host_editor.username"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.username.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let password_input = new_input_state(
            if selected_profile_data
                .as_ref()
                .is_some_and(|profile| profile.has_stored_password)
            {
                Self::localized_secret_placeholder(true, "placeholders.host_editor.password")
            } else {
                i18n::string("placeholders.host_editor.password")
            },
            String::new(),
            true,
            window,
            cx,
        );
        let private_key_input = new_input_state(
            i18n::string("placeholders.host_editor.private_key_path"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.private_key_path.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let agent_identity_input = new_input_state(
            i18n::string("placeholders.host_editor.agent_identity"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.agent_identity.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let certificate_input = new_input_state(
            i18n::string("placeholders.host_editor.certificate_path"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.certificate_path.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let passphrase_input = new_input_state(
            if selected_profile_data
                .as_ref()
                .is_some_and(|profile| profile.has_stored_passphrase)
            {
                Self::localized_secret_placeholder(true, "placeholders.host_editor.key_passphrase")
            } else {
                i18n::string("placeholders.host_editor.key_passphrase")
            },
            String::new(),
            true,
            window,
            cx,
        );
        let startup_command_value = selected_profile_data
            .as_ref()
            .map(|profile| profile.startup_command.clone())
            .unwrap_or_default();
        let startup_command_input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("bash")
                .indent_guides(false)
                .folding(false)
                .searchable(false)
                .rows(4)
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .placeholder(i18n::string("placeholders.host_editor.startup_command"))
                .default_value(startup_command_value)
        });
        let proxy_jump_profile_ids = selected_profile_data
            .as_ref()
            .map(|profile| profile.proxy_jump_profile_ids.clone())
            .unwrap_or_default();
        let environment_variable_rows = selected_profile_data
            .as_ref()
            .map(|profile| {
                Self::host_editor_environment_variable_rows(
                    &profile.environment_variables,
                    window,
                    cx,
                )
            })
            .unwrap_or_else(|| Self::host_editor_environment_variable_rows(&[], window, cx));
        let managed_key_name_input = new_input_state(
            i18n::string("placeholders.keychain.managed_key_name"),
            "",
            false,
            window,
            cx,
        );
        let managed_key_import_path_input = new_input_state(
            i18n::string("placeholders.keychain.import_private_key_path"),
            "",
            false,
            window,
            cx,
        );
        let managed_key_import_private_key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("bash")
                .indent_guides(false)
                .folding(false)
                .searchable(false)
                .rows(6)
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .placeholder(i18n::string(
                    "placeholders.keychain.import_private_key_body",
                ))
        });
        let managed_key_import_public_key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("bash")
                .indent_guides(false)
                .folding(false)
                .searchable(false)
                .rows(4)
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .placeholder(i18n::string("placeholders.keychain.import_public_key_body"))
        });
        let managed_key_import_passphrase_input = new_input_state(
            i18n::string("placeholders.keychain.import_passphrase_optional"),
            "",
            true,
            window,
            cx,
        );
        let keychain_filter_input = new_input_state(
            i18n::string("placeholders.keychain.filter"),
            "",
            false,
            window,
            cx,
        );
        let keychain_deploy_location_input = new_input_state(
            i18n::string("placeholders.keychain.deploy_location"),
            KEYCHAIN_DEPLOY_DEFAULT_LOCATION,
            false,
            window,
            cx,
        );
        let keychain_deploy_filename_input = new_input_state(
            i18n::string("placeholders.keychain.deploy_filename"),
            KEYCHAIN_DEPLOY_DEFAULT_FILENAME,
            false,
            window,
            cx,
        );
        let keychain_deploy_command_input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("bash")
                .indent_guides(false)
                .folding(false)
                .searchable(false)
                .rows(8)
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .placeholder(i18n::string("placeholders.keychain.deploy_command"))
                .default_value(KEYCHAIN_DEPLOY_DEFAULT_COMMAND)
        });
        let port_forward_label_input = new_input_state(
            i18n::string("placeholders.forward.rule_label"),
            "",
            false,
            window,
            cx,
        );
        let port_forward_listen_host_input = new_input_state(
            i18n::string("placeholders.forward.listen_host"),
            "",
            false,
            window,
            cx,
        );
        let port_forward_listen_port_input = new_input_state(
            i18n::string("placeholders.forward.listen_port"),
            "",
            false,
            window,
            cx,
        );
        let port_forward_target_host_input = new_input_state(
            i18n::string("placeholders.forward.target_host"),
            "",
            false,
            window,
            cx,
        );
        let port_forward_target_port_input = new_input_state(
            i18n::string("placeholders.forward.target_port"),
            "",
            false,
            window,
            cx,
        );
        let snippet_description_input = new_input_state(
            i18n::string("placeholders.snippets.description_example"),
            "",
            false,
            window,
            cx,
        );
        let snippet_package_input = new_input_state(
            i18n::string("placeholders.snippets.new_package_name"),
            "",
            false,
            window,
            cx,
        );
        let snippet_script_input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("bash")
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .placeholder(i18n::string("placeholders.snippets.script_body"))
        });
        let snippet_filter_input = new_input_state(
            i18n::string("placeholders.snippets.filter"),
            "",
            false,
            window,
            cx,
        );
        let session_snippets_filter_input = new_input_state(
            i18n::string("placeholders.workspace.snippet_filter"),
            "",
            false,
            window,
            cx,
        );
        let filter_input = new_input_state(
            i18n::string("placeholders.hosts.filter"),
            "",
            false,
            window,
            cx,
        );
        let forward_filter_input = new_input_state(
            i18n::string("placeholders.forward.filter"),
            "",
            false,
            window,
            cx,
        );
        let local_sftp_path_input = new_input_state(
            i18n::string("placeholders.sftp.local_path"),
            "",
            false,
            window,
            cx,
        );
        let remote_sftp_path_input = new_input_state(
            i18n::string("placeholders.sftp.remote_path"),
            ".",
            false,
            window,
            cx,
        );
        let app_view_weak = cx.weak_entity();
        let local_sftp_table = cx.new(|cx| {
            let mut table = TableState::new(
                SftpBrowserTableDelegate::new(SftpBrowserSide::Local, app_view_weak.clone()),
                window,
                cx,
            )
            .sortable(true)
            .col_movable(false)
            .col_resizable(true)
            .col_selectable(false)
            .row_selectable(false);
            table.col_fixed = false;
            table
        });
        let remote_sftp_table = cx.new(|cx| {
            let mut table = TableState::new(
                SftpBrowserTableDelegate::new(SftpBrowserSide::Remote, app_view_weak.clone()),
                window,
                cx,
            )
            .sortable(true)
            .col_movable(false)
            .col_resizable(true)
            .col_selectable(false)
            .row_selectable(false);
            table.col_fixed = false;
            table
        });
        let rename_input = new_input_state(
            i18n::string("placeholders.workspace.tab_name"),
            "",
            false,
            window,
            cx,
        );
        let sftp_prompt_input = new_input_state(
            i18n::string("placeholders.sftp.remote_path"),
            "",
            false,
            window,
            cx,
        );
        let sftp_inline_rename_input = new_input_state(
            i18n::string("placeholders.sftp.new_name"),
            "",
            false,
            window,
            cx,
        );
        let group_input_subscription = cx.subscribe(
            &group_input,
            |_: &mut AppView, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        let filter_subscription = cx.subscribe(
            &filter_input,
            |_: &mut AppView, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        let forward_filter_subscription = cx.subscribe(
            &forward_filter_input,
            |_: &mut AppView, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        let snippet_filter_subscription = cx.subscribe(
            &snippet_filter_input,
            |_: &mut AppView, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        let session_snippets_filter_subscription = cx.subscribe(
            &session_snippets_filter_input,
            |_: &mut AppView, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        let keychain_filter_subscription = cx.subscribe(
            &keychain_filter_input,
            |_: &mut AppView, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        );
        let rename_subscription = cx.subscribe(
            &rename_input,
            |this: &mut AppView, _, event: &InputEvent, cx| match event {
                InputEvent::PressEnter { .. } => this.commit_rename_tab(cx),
                InputEvent::Blur => this.cancel_rename_tab(cx),
                _ => {}
            },
        );
        let local_sftp_path_subscription = cx.subscribe(
            &local_sftp_path_input,
            |this: &mut AppView, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.commit_sftp_local_path_input(cx);
                }
            },
        );
        let remote_sftp_path_subscription = cx.subscribe(
            &remote_sftp_path_input,
            |this: &mut AppView, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.commit_sftp_remote_path_input(cx);
                }
            },
        );
        let local_sftp_table_subscription =
            cx.subscribe_in(&local_sftp_table, window, Self::on_local_sftp_table_event);
        let remote_sftp_table_subscription =
            cx.subscribe_in(&remote_sftp_table, window, Self::on_remote_sftp_table_event);
        let sftp_prompt_subscription = cx.subscribe(
            &sftp_prompt_input,
            |this: &mut AppView, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.commit_sftp_prompt(cx);
                }
            },
        );
        let sftp_inline_rename_subscription = cx.subscribe(
            &sftp_inline_rename_input,
            |this: &mut AppView, _, event: &InputEvent, cx| match event {
                InputEvent::PressEnter { .. } => this.commit_sftp_inline_rename(cx),
                InputEvent::Blur => this.cancel_sftp_inline_rename(cx),
                _ => {}
            },
        );

        let weak = cx.weak_entity();
        let keystroke_interceptor = cx.intercept_keystrokes(move |event, window, cx| {
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
                    if this.panel_forms.settings.recording_binding.is_some() {
                        this.cancel_recording_key_binding(cx);
                        consumed = true;
                    } else if this.workspace_forms.search.open {
                        this.close_terminal_search(window, cx);
                        consumed = true;
                    } else if this.workspace_state.renaming_tab.is_some() {
                        this.cancel_rename_tab(cx);
                        consumed = true;
                    } else if this
                        .workspace_state
                        .active_topbar_tab
                        .and_then(|index| this.workspace_state.tabs.get(index))
                        .and_then(|tab| tab.as_sftp())
                        .is_some_and(|sftp| sftp.inline_rename.is_some())
                    {
                        this.cancel_sftp_inline_rename(cx);
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
                .panel_forms
                .settings
                .recording_binding
                .is_some()
            {
                return;
            }

            if key == "enter"
                && !modifiers.control
                && !modifiers.alt
                && !modifiers.platform
                && !modifiers.shift
            {
                let mut submit_local_vault_passphrase = false;
                view.update(cx, |this, cx| {
                    if let Some(mode) = this.local_vault_passphrase_popup {
                        this.submit_local_vault_passphrase_popup_action(mode, window, cx);
                        submit_local_vault_passphrase = true;
                    }
                });
                if submit_local_vault_passphrase {
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
                .workspace_state
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
                window.focus(
                    &this.workspace_state.workspace.active_pane.terminal_focus,
                    cx,
                );
            });
            cx.stop_propagation();
        });

        let local_sftp_hidden_columns = settings_store.settings().local_sftp_hidden_columns.clone();
        let remote_sftp_hidden_columns =
            settings_store.settings().remote_sftp_hidden_columns.clone();
        local_sftp_table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .set_hidden_columns(local_sftp_hidden_columns);
            table.refresh(cx);
        });
        remote_sftp_table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .set_hidden_columns(remote_sftp_hidden_columns);
            table.refresh(cx);
        });
        let language_options = AppLanguage::supported_languages()
            .into_iter()
            .map(|language| SelectOption::new(language, language.native_name()))
            .collect::<Vec<_>>();
        let selected_language_index = language_options
            .iter()
            .position(|language| *language.value() == settings_store.settings().language)
            .map(|index| IndexPath::default().row(index));
        let language_select = cx.new(|cx| {
            SelectState::new(
                language_options.clone(),
                selected_language_index,
                window,
                cx,
            )
        });
        let language_select_subscription = cx.subscribe(
            &language_select,
            |this: &mut AppView, _, event: &SelectEvent<Vec<SelectOption<AppLanguage>>>, cx| {
                let SelectEvent::Confirm(selected_language) = event;
                if let Some(language) = selected_language {
                    this.set_language(*language, cx);
                }
            },
        );
        let last_tab_close_behavior_options = miaominal_settings::LastTabCloseBehavior::all()
            .iter()
            .copied()
            .map(|behavior| SelectOption::new(behavior, last_tab_close_behavior_label(behavior)))
            .collect::<Vec<_>>();
        let selected_last_tab_close_behavior_index = last_tab_close_behavior_options
            .iter()
            .position(|behavior| {
                *behavior.value() == settings_store.settings().last_tab_close_behavior
            })
            .map(|index| IndexPath::default().row(index));
        let last_tab_close_behavior_select = cx.new(|cx| {
            SelectState::new(
                last_tab_close_behavior_options,
                selected_last_tab_close_behavior_index,
                window,
                cx,
            )
        });
        let last_tab_close_behavior_select_subscription = cx.subscribe(
            &last_tab_close_behavior_select,
            |this: &mut AppView,
             _,
             event: &SelectEvent<Vec<SelectOption<miaominal_settings::LastTabCloseBehavior>>>,
             cx| {
                let SelectEvent::Confirm(selected_behavior) = event;
                if let Some(behavior) = selected_behavior {
                    this.set_last_tab_close_behavior(*behavior, cx);
                }
            },
        );
        let local_vault_auto_lock_duration_options =
            miaominal_settings::LocalVaultAutoLockDuration::all()
                .iter()
                .copied()
                .map(|duration| {
                    SelectOption::new(duration, local_vault_auto_lock_duration_label(duration))
                })
                .collect::<Vec<_>>();
        let selected_local_vault_auto_lock_duration_index = local_vault_auto_lock_duration_options
            .iter()
            .position(|duration| {
                *duration.value() == settings_store.settings().local_vault_auto_lock_duration
            })
            .map(|index| IndexPath::default().row(index));
        let local_vault_auto_lock_duration_select = cx.new(|cx| {
            SelectState::new(
                local_vault_auto_lock_duration_options,
                selected_local_vault_auto_lock_duration_index,
                window,
                cx,
            )
        });
        let local_vault_auto_lock_duration_select_subscription = cx.subscribe(
            &local_vault_auto_lock_duration_select,
            |this: &mut AppView,
             _,
             event: &SelectEvent<
                Vec<SelectOption<miaominal_settings::LocalVaultAutoLockDuration>>,
            >,
             cx| {
                let SelectEvent::Confirm(selected) = event;
                if let Some(duration) = selected {
                    this.set_local_vault_auto_lock_duration(*duration, cx);
                }
            },
        );
        let monitor_history_options = miaominal_settings::MonitorHistoryDuration::all()
            .iter()
            .copied()
            .map(|duration| SelectOption::new(duration, monitor_history_duration_label(duration)))
            .collect::<Vec<_>>();
        let selected_monitor_history_index = monitor_history_options
            .iter()
            .position(|d| *d.value() == settings_store.settings().monitor_history_duration)
            .map(|i| IndexPath::default().row(i));
        let monitor_history_select = cx.new(|cx| {
            SelectState::new(
                monitor_history_options,
                selected_monitor_history_index,
                window,
                cx,
            )
        });
        let monitor_history_select_subscription = cx.subscribe(
            &monitor_history_select,
            |this: &mut AppView,
             _,
             event: &SelectEvent<Vec<SelectOption<miaominal_settings::MonitorHistoryDuration>>>,
             cx| {
                let SelectEvent::Confirm(selected) = event;
                if let Some(duration) = selected {
                    this.set_monitor_history_duration(*duration, cx);
                }
            },
        );
        let terminal_right_click_behavior_options = vec![
            SelectOption::new(
                miaominal_settings::TerminalRightClickBehavior::ContextMenu,
                i18n::string("settings.key_bindings.context_menu_option"),
            ),
            SelectOption::new(
                miaominal_settings::TerminalRightClickBehavior::CopySelectionOrPaste,
                i18n::string("settings.key_bindings.copy_paste_option"),
            ),
        ];
        let selected_terminal_right_click_behavior_index = terminal_right_click_behavior_options
            .iter()
            .position(|behavior| {
                *behavior.value() == settings_store.settings().terminal_right_click_behavior
            })
            .map(|index| IndexPath::default().row(index));
        let terminal_right_click_behavior_select = cx.new(|cx| {
            SelectState::new(
                terminal_right_click_behavior_options,
                selected_terminal_right_click_behavior_index,
                window,
                cx,
            )
        });
        let terminal_right_click_behavior_select_subscription = cx.subscribe(
            &terminal_right_click_behavior_select,
            |this: &mut AppView,
             _,
             event: &SelectEvent<
                Vec<SelectOption<miaominal_settings::TerminalRightClickBehavior>>,
            >,
             cx| {
                let SelectEvent::Confirm(selected_behavior) = event;
                if let Some(behavior) = selected_behavior {
                    this.set_terminal_right_click_behavior(*behavior, cx);
                }
            },
        );
        let profile_import_source_options = vec![
            ImportSourceKind::OpenSshConfig,
            ImportSourceKind::PuttyRegistry,
            ImportSourceKind::SecureCrtXml,
            ImportSourceKind::FinalShellJson,
        ]
        .into_iter()
        .map(|source| SelectOption::new(source, localized_profile_import_source_label(source)))
        .collect::<Vec<_>>();
        let profile_import_source_select = cx.new(|cx| {
            SelectState::new(
                profile_import_source_options,
                Some(IndexPath::default().row(0)),
                window,
                cx,
            )
        });
        let sync_provider_options = vec![
            SelectOption::new(
                SyncProvider::None,
                i18n::string("settings.sync.providers.none"),
            ),
            SelectOption::new(
                SyncProvider::GithubGist,
                i18n::string("settings.sync.providers.gist"),
            ),
            SelectOption::new(
                SyncProvider::WebDav,
                i18n::string("settings.sync.providers.webdav"),
            ),
        ];
        let sync_engine = if local_vault_enabled {
            SyncEngine::new_locked_vault()
        } else {
            SyncEngine::new()
        };
        let selected_sync_provider_index = sync_provider_options
            .iter()
            .position(|provider| *provider.value() == sync_engine.config_store.config.provider)
            .map(|index| IndexPath::default().row(index));
        let sync_provider_select = cx.new(|cx| {
            SelectState::new(
                sync_provider_options,
                selected_sync_provider_index,
                window,
                cx,
            )
        });
        let sync_secrets = sync_engine
            .config_store
            .get_secrets()
            .unwrap_or_else(|error| {
                log::warn!("failed to load sync secrets from credential store: {error:?}");
                Default::default()
            });
        let sync_github_token = sync_secrets.github_token.unwrap_or_default();
        let sync_github_gist_id = sync_engine
            .config_store
            .config
            .gist_id
            .clone()
            .unwrap_or_default();
        let sync_webdav_url = sync_engine.config_store.config.webdav_url.clone();
        let sync_webdav_username = sync_engine.config_store.config.webdav_username.clone();
        let sync_webdav_password = sync_secrets.webdav_password.unwrap_or_default();
        let sync_passphrase = sync_secrets.passphrase.unwrap_or_default();
        let has_sync_github_token = sync_engine.config_store.config.has_github_token;
        let has_sync_webdav_password = sync_engine.config_store.config.has_webdav_password;
        let sync_passphrase_configured = sync_engine.config_store.config.has_passphrase;
        let sync_provider_select_subscription = cx.subscribe(
            &sync_provider_select,
            |this: &mut AppView, _, event: &SelectEvent<Vec<SelectOption<SyncProvider>>>, cx| {
                let SelectEvent::Confirm(selected_provider) = event;
                if let Some(provider) = selected_provider {
                    this.set_sync_provider(*provider, cx);
                }
            },
        );
        let current_font_family = settings_store.settings().font_family.clone();
        let font_family_options = Self::font_family_options(&current_font_family);
        let default_font_family = miaominal_settings::default_font_family();
        let font_family_select = cx.new(|cx| {
            let mut state = SelectState::new(
                SearchableVec::new(font_family_options.clone()),
                None,
                window,
                cx,
            )
            .searchable(true);

            let selected_font_family = if current_font_family.trim().is_empty() {
                default_font_family.clone()
            } else {
                current_font_family.clone()
            };
            state.set_selected_value(&selected_font_family, window, cx);
            state
        });
        let font_family_subscription = cx.subscribe(
            &font_family_select,
            |this: &mut AppView, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                let SelectEvent::Confirm(selected_font_family) = event;
                if let Some(selected_font_family) = selected_font_family.as_deref() {
                    this.update_font_family(selected_font_family.to_string(), cx);
                }
            },
        );
        let font_fallbacks_initial = settings_store.settings().font_fallbacks.join(", ");
        let font_fallbacks_input = new_input_state("", font_fallbacks_initial, false, window, cx);
        let font_fallbacks_subscription = cx.subscribe(
            &font_fallbacks_input,
            |this: &mut AppView, input, event: &InputEvent, cx| {
                if matches!(
                    event,
                    InputEvent::Change | InputEvent::PressEnter { .. } | InputEvent::Blur
                ) {
                    let value = input.read(cx).value().to_string();
                    this.update_font_fallbacks(value, cx);
                }
            },
        );
        let seed_color = miaominal_settings::Theme::from_settings(settings_store.settings())
            .material
            .source;
        let seed_color_picker =
            cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb(seed_color)));
        let seed_color_subscription = cx.subscribe(
            &seed_color_picker,
            |this: &mut AppView, _, event: &ColorPickerEvent, cx| {
                let ColorPickerEvent::Change(Some(color)) = event else {
                    return;
                };
                this.update_seed_color(color.to_hex(), cx);
            },
        );

        let search_input = new_input_state(
            i18n::string("placeholders.workspace.search_scrollback"),
            "",
            false,
            window,
            cx,
        );
        let search_subscription = cx.subscribe(
            &search_input,
            |this: &mut AppView, input, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    let value = input.read(cx).value().to_string();
                    this.update_terminal_search(value, cx);
                }
                InputEvent::PressEnter { secondary } => {
                    if *secondary {
                        this.terminal_search_prev(cx);
                    } else {
                        this.terminal_search_next(cx);
                    }
                }
                _ => {}
            },
        );
        let terminal_focus_in_subscription =
            cx.on_focus_in(&terminal_focus, window, |this, window, cx| {
                this.sync_terminal_focus_reporting(window, cx);
            });
        let terminal_focus_out_subscription =
            cx.on_focus_out(&terminal_focus, window, |this, _, window, cx| {
                this.sync_terminal_focus_reporting(window, cx);
            });
        let window_activation_subscription =
            cx.observe_window_activation(window, |this, window, cx| {
                this.sync_terminal_focus_reporting(window, cx);
            });

        let group_select = cx.new(|cx| {
            let mut state = SelectState::new(
                SearchableVec::new(available_groups.clone()),
                None,
                window,
                cx,
            );
            if let Some(existing_group) = selected_existing_group.as_ref() {
                state.set_selected_value(existing_group, window, cx);
            }
            state
        });
        let managed_key_select = cx.new(|cx| {
            let mut state = SelectState::new(
                Self::managed_key_options(&data.managed_keys),
                None,
                window,
                cx,
            )
            .searchable(true);
            if let Some(selected_managed_key_id) = selected_profile_data
                .as_ref()
                .map(|profile| profile.managed_key_id.trim().to_string())
                .filter(|managed_key_id| !managed_key_id.is_empty())
            {
                state.set_selected_value(&selected_managed_key_id, window, cx);
            }
            state
        });
        let selected_charset = selected_profile_data
            .as_ref()
            .map(|profile| profile.charset.trim().to_string())
            .filter(|charset| !charset.is_empty())
            .unwrap_or_else(|| DEFAULT_SESSION_CHARSET.to_string());
        let charset_select = cx.new(|cx| {
            let mut state = SelectState::new(
                SearchableVec::new(Self::available_session_charsets()),
                None,
                window,
                cx,
            )
            .searchable(true);
            state.set_selected_value(&selected_charset, window, cx);
            state
        });
        let proxy_jump_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Vec::<ProxyJumpCandidateSelectItem>::new()),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        let snippet_package_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Self::collect_available_snippet_packages(&data.snippets)),
                None,
                window,
                cx,
            )
        });
        let forward_profile_select = cx.new(|cx| {
            SelectState::new(
                Self::forward_profile_options(&data.sessions),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        let keychain_deploy_profile_select = cx.new(|cx| {
            SelectState::new(
                Self::keychain_deploy_profile_options(&data.sessions),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        let group_select_subscription = cx.subscribe(
            &group_select,
            |this: &mut AppView, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                let SelectEvent::Confirm(selected_group) = event;
                if selected_group.is_some() {
                    this.host_editor_forms.creating_new_group = false;
                }
                cx.notify();
            },
        );
        let managed_key_select_subscription = cx.subscribe(
            &managed_key_select,
            |this: &mut AppView,
             _,
             event: &SelectEvent<SearchableVec<ManagedKeySelectItem>>,
             cx| {
                let SelectEvent::Confirm(selected_managed_key_id) = event;
                if selected_managed_key_id.is_some()
                    && this.host_editor_forms.editing_auth_method != AuthMethod::ManagedKey
                {
                    this.host_editor_forms.editing_auth_method = AuthMethod::ManagedKey;
                    cx.notify();
                }
            },
        );
        let proxy_jump_select_subscription = cx.subscribe(
            &proxy_jump_select,
            |this: &mut AppView,
             _,
             event: &SelectEvent<SearchableVec<ProxyJumpCandidateSelectItem>>,
             cx| {
                let SelectEvent::Confirm(selected_profile_id) = event;
                if let Some(selected_profile_id) = selected_profile_id.as_deref() {
                    this.add_proxy_jump_profile(selected_profile_id, cx);
                }
            },
        );
        let snippet_package_select_subscription = cx.subscribe(
            &snippet_package_select,
            |this: &mut AppView, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                let SelectEvent::Confirm(selected_package) = event;
                if selected_package.is_some() {
                    this.panel_forms.snippets.creating_new_package = false;
                }
                cx.notify();
            },
        );
        let forward_profile_select_subscription = cx.subscribe(
            &forward_profile_select,
            |this: &mut AppView,
             _,
             event: &SelectEvent<SearchableVec<ForwardProfileSelectItem>>,
             cx| {
                let SelectEvent::Confirm(selected_profile_id) = event;
                if this.editors.port_forward_editor_rule_id.is_none() {
                    this.select_port_forward_editor_profile(selected_profile_id.clone(), cx);
                }
            },
        );

        let host_editor_forms = Self::build_host_editor_forms(HostEditorFormsArgs {
            name_input,
            group_input,
            group_select,
            managed_key_select,
            proxy_jump_select,
            charset_select,
            creating_new_group: !selected_group.is_empty() && selected_existing_group.is_none(),
            tags_input,
            host_input,
            port_input,
            username_input,
            password_input,
            private_key_input,
            agent_identity_input,
            certificate_input,
            passphrase_input,
            startup_command_input,
            proxy_jump_profile_ids,
            environment_variable_rows,
            editing_auth_method,
            agent_forwarding_enabled: selected_profile_data
                .as_ref()
                .is_some_and(|profile| profile.agent_forwarding),
            shell_type: selected_profile_data
                .as_ref()
                .map(|profile| profile.shell_type)
                .unwrap_or_default(),
        });

        let workspace_forms = Self::build_workspace_forms(WorkspaceFormsArgs {
            rename_input,
            search_input,
            session_snippets_filter_input,
            local_path_input: local_sftp_path_input,
            remote_path_input: remote_sftp_path_input,
            local_table: local_sftp_table,
            remote_table: remote_sftp_table,
            prompt_input: sftp_prompt_input,
            inline_rename_input: sftp_inline_rename_input,
        });
        let sync_github_token_input = new_input_state(
            Self::localized_secret_placeholder(
                has_sync_github_token,
                "settings.sync.placeholders.github_token",
            ),
            sync_github_token,
            true,
            window,
            cx,
        );
        let sync_github_gist_id_input = new_input_state(
            i18n::string("settings.sync.placeholders.gist_id"),
            sync_github_gist_id,
            false,
            window,
            cx,
        );
        let sync_webdav_url_input = new_input_state(
            i18n::string("settings.sync.placeholders.webdav_url"),
            sync_webdav_url,
            false,
            window,
            cx,
        );
        let sync_webdav_username_input = new_input_state(
            i18n::string("settings.sync.placeholders.webdav_username"),
            sync_webdav_username,
            false,
            window,
            cx,
        );
        let sync_webdav_password_input = new_input_state(
            Self::localized_secret_placeholder(
                has_sync_webdav_password,
                "settings.sync.placeholders.webdav_password",
            ),
            sync_webdav_password,
            true,
            window,
            cx,
        );
        let sync_passphrase_input = new_input_state(
            i18n::string("settings.sync.placeholders.passphrase"),
            sync_passphrase,
            true,
            window,
            cx,
        );
        let sync_passphrase_confirmation_input = new_input_state(
            i18n::string("settings.sync.placeholders.passphrase"),
            "",
            true,
            window,
            cx,
        );
        let local_data_reset_confirmation_input = new_input_state(
            i18n::string("settings.about.reset_local.popup.placeholder"),
            "",
            false,
            window,
            cx,
        );
        let local_vault_passphrase_input = new_input_state(
            i18n::string("settings.sync.placeholders.vault_passphrase"),
            "",
            true,
            window,
            cx,
        );
        let local_vault_passphrase_confirmation_input = new_input_state(
            i18n::string("settings.sync.placeholders.vault_passphrase_confirmation"),
            "",
            true,
            window,
            cx,
        );

        let panel_forms = Self::build_panel_forms(PanelFormsArgs {
            filter_input,
            keychain_filter_input,
            managed_key_name_input,
            managed_key_import_path_input,
            managed_key_import_private_key_input,
            managed_key_import_public_key_input,
            managed_key_import_passphrase_input,
            keychain_deploy_profile_select,
            keychain_deploy_location_input,
            keychain_deploy_filename_input,
            keychain_deploy_command_input,
            forward_filter_input,
            forward_profile_select,
            port_forward_label_input,
            port_forward_listen_host_input,
            port_forward_listen_port_input,
            port_forward_target_host_input,
            port_forward_target_port_input,
            snippet_filter_input,
            snippet_description_input,
            snippet_package_input,
            snippet_package_select,
            creating_new_package: data.snippets.is_empty(),
            snippet_script_input,
            language_select,
            last_tab_close_behavior_select,
            local_vault_auto_lock_duration_select,
            monitor_history_select,
            terminal_right_click_behavior_select,
            profile_import_source_select,
            sync_provider_select,
            font_family_select,
            font_fallbacks_input,
            seed_color_picker,
            key_capture_focus: cx.focus_handle(),
            sync_github_token_input,
            sync_github_gist_id_input,
            sync_webdav_url_input,
            sync_webdav_username_input,
            sync_webdav_password_input,
            sync_passphrase_input,
            sync_passphrase_confirmation_input,
            local_data_reset_confirmation_input,
            local_vault_passphrase_input,
            local_vault_passphrase_confirmation_input,
        });

        let mut view = Self {
            services,
            data,
            host_editor_forms,
            workspace_forms,
            panel_forms,
            keychain_page_view: KeychainPageView::ManagedKeys,
            keychain_editor_mode: KeychainEditorMode::Import,
            keychain_deploy_in_progress: false,
            keychain_editor_draft_source: None,
            keychain_deploy_key_id: None,
            workspace_state: WorkspaceState {
                tabs: vec![TabState::new_hosts(0)],
                shared_profile_monitoring: Default::default(),
                monitor_source_tabs: Default::default(),
                active_topbar_tab: Some(0),
                topbar_tab_scroll_handle: ScrollHandle::new(),
                session_monitor_scroll_handle: ScrollHandle::new(),
                topbar_previous_visible_tabs: Vec::new(),
                topbar_entering_tabs: Vec::new(),
                topbar_exiting_tabs: Vec::new(),
                topbar_active_transition: None,
                topbar_visible_active_tab_id: None,
                next_tab_id: 1,
                workspace: Self::initial_workspace(terminal_focus),
                recently_closed_tabs: Vec::new(),
                renaming_tab: None,
                reported_terminal_focus_tab_id: None,
                hosts_to_terminal_transition: None,
                terminal_view_transition: None,
                visible_terminal_view_tab_id: None,
            },
            panel_view: PanelViewState::new(),
            editors: EditorOverlayState::new(),
            shell_state: ShellState::default(),
            panels: PanelState::default(),
            kbi_inputs: Vec::new(),
            dialogs: DialogState::default(),
            onboarding: OnboardingState {
                show_onboarding: settings_store.settings().should_show_onboarding(),
                onboarding_step: OnboardingStep::Welcome,
                visible_onboarding_step: OnboardingStep::Welcome,
                onboarding_step_transition: None,
            },
            status_message,
            settings_store,
            local_vault_status: if local_vault_enabled {
                LocalVaultStatus::Locked
            } else {
                LocalVaultStatus::Disabled
            },
            sync_passphrase_popup: None,
            local_vault_passphrase_popup: None,
            pending_local_vault_unlock_action: None,
            local_vault_unlock_in_progress: false,
            local_vault_disable_in_progress: false,
            local_data_reset_in_progress: false,
            local_vault_session_passphrase: None,
            local_vault_auto_lock_task: None,
            sync: SyncUiState {
                sync_engine,
                sync_status: SyncStatus::Idle,
                active_sync_task: None,
                sync_secret_save_operation: None,
                sync_passphrase_operation: None,
                sync_passphrase_configured,
            },
            secret_visibility: SecretVisibilityState::default(),
            controllers: ControllerSet::new(cx.entity().downgrade()),
            _subscriptions: Self::build_subscriptions(AppViewSubscriptionsArgs {
                font_family_subscription,
                font_fallbacks_subscription,
                seed_color_subscription,
                group_input_subscription,
                group_select_subscription,
                managed_key_select_subscription,
                proxy_jump_select_subscription,
                snippet_package_select_subscription,
                language_select_subscription,
                last_tab_close_behavior_select_subscription,
                local_vault_auto_lock_duration_select_subscription,
                monitor_history_select_subscription,
                terminal_right_click_behavior_select_subscription,
                sync_provider_select_subscription,
                keychain_filter_subscription,
                filter_subscription,
                forward_filter_subscription,
                snippet_filter_subscription,
                forward_profile_select_subscription,
                local_sftp_path_subscription,
                remote_sftp_path_subscription,
                local_sftp_table_subscription,
                remote_sftp_table_subscription,
                rename_subscription,
                sftp_prompt_subscription,
                sftp_inline_rename_subscription,
                search_subscription,
                session_snippets_filter_subscription,
                terminal_focus_in_subscription,
                terminal_focus_out_subscription,
                window_activation_subscription,
                keystroke_interceptor,
            }),
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
        let selected_profile = if self.editors.host_editor_is_new {
            None
        } else {
            self.data
                .selected_profile
                .and_then(|index| self.data.sessions.get(index))
        };
        let has_saved_password =
            selected_profile.is_some_and(|profile| profile.has_stored_password);
        let has_saved_passphrase =
            selected_profile.is_some_and(|profile| profile.has_stored_passphrase);

        set_input_placeholder(
            &self.host_editor_forms.name_input,
            i18n::string("placeholders.host_editor.profile_name"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.host_editor_forms.group_input,
            i18n::string("placeholders.host_editor.new_group_name"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.host_editor_forms.tags_input,
            i18n::string("placeholders.host_editor.tags"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.host_editor_forms.host_input,
            i18n::string("placeholders.host_editor.host"),
            window,
            cx,
        );
        set_input_placeholder(&self.host_editor_forms.port_input, "22", window, cx);
        set_input_placeholder(
            &self.host_editor_forms.username_input,
            i18n::string("placeholders.host_editor.username"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.host_editor_forms.password_input,
            Self::localized_secret_placeholder(
                has_saved_password,
                "placeholders.host_editor.password",
            ),
            window,
            cx,
        );
        set_input_placeholder(
            &self.host_editor_forms.private_key_input,
            i18n::string("placeholders.host_editor.private_key_path"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.host_editor_forms.agent_identity_input,
            i18n::string("placeholders.host_editor.agent_identity"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.host_editor_forms.certificate_input,
            i18n::string("placeholders.host_editor.certificate_path"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.host_editor_forms.passphrase_input,
            Self::localized_secret_placeholder(
                has_saved_passphrase,
                "placeholders.host_editor.key_passphrase",
            ),
            window,
            cx,
        );
        set_input_placeholder(
            &self.host_editor_forms.startup_command_input,
            i18n::string("placeholders.host_editor.startup_command"),
            window,
            cx,
        );
        for row in &self.host_editor_forms.environment_variable_rows {
            set_input_placeholder(
                &row.name_input,
                i18n::string("placeholders.host_editor.environment_variable_name"),
                window,
                cx,
            );
            set_input_placeholder(
                &row.value_input,
                i18n::string("placeholders.host_editor.environment_variable_value"),
                window,
                cx,
            );
        }

        set_input_placeholder(
            &self.panel_forms.keychain.name_input,
            i18n::string("placeholders.keychain.managed_key_name"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.keychain.import_path_input,
            i18n::string("placeholders.keychain.import_private_key_path"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.keychain.import_private_key_input,
            i18n::string("placeholders.keychain.import_private_key_body"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.keychain.import_public_key_input,
            i18n::string("placeholders.keychain.import_public_key_body"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.keychain.import_passphrase_input,
            i18n::string("placeholders.keychain.import_passphrase_optional"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.keychain.filter_input,
            i18n::string("placeholders.keychain.filter"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.keychain.deploy_location_input,
            i18n::string("placeholders.keychain.deploy_location"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.keychain.deploy_filename_input,
            i18n::string("placeholders.keychain.deploy_filename"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.keychain.deploy_command_input,
            i18n::string("placeholders.keychain.deploy_command"),
            window,
            cx,
        );

        set_input_placeholder(
            &self.panel_forms.forwarding.filter_input,
            i18n::string("placeholders.forward.filter"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.forwarding.label_input,
            i18n::string("placeholders.forward.rule_label"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.forwarding.listen_host_input,
            i18n::string("placeholders.forward.listen_host"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.forwarding.listen_port_input,
            i18n::string("placeholders.forward.listen_port"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.forwarding.target_host_input,
            i18n::string("placeholders.forward.target_host"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.forwarding.target_port_input,
            i18n::string("placeholders.forward.target_port"),
            window,
            cx,
        );

        set_input_placeholder(
            &self.panel_forms.snippets.filter_input,
            i18n::string("placeholders.snippets.filter"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.snippets.description_input,
            i18n::string("placeholders.snippets.description_example"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.snippets.package_input,
            i18n::string("placeholders.snippets.new_package_name"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.snippets.script_input,
            i18n::string("placeholders.snippets.script_body"),
            window,
            cx,
        );

        set_input_placeholder(
            &self.panel_forms.hosts.filter_input,
            i18n::string("placeholders.hosts.filter"),
            window,
            cx,
        );

        set_input_placeholder(
            &self.workspace_forms.rename_input,
            i18n::string("placeholders.workspace.tab_name"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.workspace_forms.search.input,
            i18n::string("placeholders.workspace.search_scrollback"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.workspace_forms.snippets_panel.filter_input,
            i18n::string("placeholders.workspace.snippet_filter"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.workspace_forms.sftp_browser.local_path_input,
            i18n::string("placeholders.sftp.local_path"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.workspace_forms.sftp_browser.remote_path_input,
            i18n::string("placeholders.sftp.remote_path"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.workspace_forms.sftp_browser.prompt_input,
            i18n::string("placeholders.sftp.remote_path"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.workspace_forms.sftp_browser.inline_rename_input,
            i18n::string("placeholders.sftp.new_name"),
            window,
            cx,
        );
        let local_sftp_table = self.workspace_forms.sftp_browser.local_table.clone();
        let remote_sftp_table = self.workspace_forms.sftp_browser.remote_table.clone();
        local_sftp_table.update(cx, |table, cx| {
            table.delegate_mut().refresh_localized_text();
            table.refresh(cx);
        });
        remote_sftp_table.update(cx, |table, cx| {
            table.delegate_mut().refresh_localized_text();
            table.refresh(cx);
        });

        set_input_placeholder(
            &self.panel_forms.settings.sync_github_token_input,
            Self::localized_secret_placeholder(
                self.sync.sync_engine.config_store.config.has_github_token,
                "settings.sync.placeholders.github_token",
            ),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.settings.sync_github_gist_id_input,
            i18n::string("settings.sync.placeholders.gist_id"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.settings.sync_webdav_url_input,
            i18n::string("settings.sync.placeholders.webdav_url"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.settings.sync_webdav_username_input,
            i18n::string("settings.sync.placeholders.webdav_username"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.settings.sync_webdav_password_input,
            Self::localized_secret_placeholder(
                self.sync
                    .sync_engine
                    .config_store
                    .config
                    .has_webdav_password,
                "settings.sync.placeholders.webdav_password",
            ),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.settings.sync_passphrase_input,
            i18n::string("settings.sync.placeholders.passphrase"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.panel_forms.settings.local_vault_passphrase_input,
            i18n::string("settings.sync.placeholders.vault_passphrase"),
            window,
            cx,
        );
        set_input_placeholder(
            &self
                .panel_forms
                .settings
                .local_vault_passphrase_confirmation_input,
            i18n::string("settings.sync.placeholders.vault_passphrase_confirmation"),
            window,
            cx,
        );
    }

    fn initial_workspace(terminal_focus: FocusHandle) -> TabWorkspaceState {
        TabWorkspaceState::new(None, terminal_focus)
    }

    fn available_session_charsets() -> Vec<String> {
        [
            "UTF-8",
            "GB18030",
            "GBK",
            "GB2312",
            "Big5",
            "Shift_JIS",
            "EUC-JP",
            "EUC-KR",
            "ISO-8859-1",
            "ISO-8859-15",
            "Windows-1252",
            "KOI8-R",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    pub(in crate::ui::shell) fn localized_secret_placeholder(
        has_saved: bool,
        fallback_key: &'static str,
    ) -> String {
        if has_saved {
            i18n::string("placeholders.saved.keep_existing")
        } else {
            i18n::string(fallback_key)
        }
    }
}
