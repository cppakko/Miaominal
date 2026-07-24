use super::*;

impl AppView {
    fn profile_for_session_tab_id(
        &self,
        session_tab_id: TabId,
        cx: &App,
    ) -> Option<SessionProfile> {
        self.session_tab(session_tab_id, cx).and_then(|session| {
            self.controllers
                .session
                .read(cx)
                .profiles()
                .iter()
                .find(|profile| profile.id == session.profile_id)
                .cloned()
                .or_else(|| session.pending_profile.clone())
        })
    }

    fn reusable_sftp_tab_id_for_session(
        &self,
        session_tab_id: TabId,
        profile_id: &str,
        cx: &App,
    ) -> Option<TabId> {
        self.workspace.tabs.iter().find_map(|tab| {
            let sftp = self.sftp_tab(tab.id, cx)?;
            let usable_owner = tab.is_top_level() || tab.owner() == Some(session_tab_id);
            (sftp.profile_id == profile_id && usable_owner && sftp.commands.is_some())
                .then_some(tab.id)
        })
    }

    pub(in crate::ui::shell) fn session_side_panel_sftp_tab_id(&self, cx: &App) -> Option<TabId> {
        let (session_tab_id, profile_id) = self
            .active_terminal_session_index(cx)
            .and_then(|index| self.workspace.tabs.id_at(index))
            .and_then(|tab_id| {
                self.session_tab(tab_id, cx)
                    .map(|session| (tab_id, session.profile_id.clone()))
            })?;

        self.reusable_sftp_tab_id_for_session(session_tab_id, &profile_id, cx)
    }

    pub(in crate::ui::shell) fn ensure_session_side_panel_sftp_tab(
        &mut self,
        session_tab_id: TabId,
        cx: &mut Context<Self>,
    ) -> Option<TabId> {
        let Some(profile) = self.profile_for_session_tab_id(session_tab_id, cx) else {
            self.shell.status_message =
                i18n::string("session.messages.open_sftp_requires_active_ssh");
            cx.notify();
            return None;
        };

        if let Some(tab_id) = self.reusable_sftp_tab_id_for_session(session_tab_id, &profile.id, cx)
        {
            self.sync_sftp_path_inputs_for_tab(tab_id, cx);
            self.sync_sftp_tables_for_tab(tab_id, cx);
            return Some(tab_id);
        }

        if self.profile_requires_local_vault_unlock(&profile, cx) {
            self.defer_app_command(
                DeferredAppCommand::Sftp(SftpDeferredCommand::OpenProfile {
                    profile_id: profile.id,
                    owner: Some(session_tab_id),
                }),
                cx,
            );
            return None;
        }

        let tab_id = self.workspace.allocate_tab_id();
        let mut tab = TabState::new_sftp(tab_id, &profile);
        tab.placement = TabPlacement::SessionSidecar {
            owner: session_tab_id,
        };
        let start_result = self.controllers.sftp.update(cx, |controller, cx| {
            controller.start_tab(tab_id, profile.clone(), Some(session_tab_id), cx)
        });
        if let Err(error) = start_result {
            log::warn!("failed to open session SFTP tab: {error}");
            self.shell.status_message =
                i18n::string("session.messages.open_sftp_requires_active_ssh");
            cx.notify();
            return None;
        }
        self.insert_sftp_tab(tab, cx);
        self.sync_sftp_path_inputs_for_tab(tab_id, cx);
        self.sync_sftp_tables_for_tab(tab_id, cx);
        self.shell.status_message = i18n::string_args(
            "sftp.messages.opened_tab_for",
            &[("profile", &profile.name)],
        );
        cx.notify();
        Some(tab_id)
    }

    pub(in crate::ui::shell) fn open_sftp_tab(
        &mut self,
        profile: SessionProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.profile_requires_local_vault_unlock(&profile, cx) {
            self.prompt_local_vault_unlock_for_action(
                DeferredAppCommand::Sftp(SftpDeferredCommand::OpenProfile {
                    profile_id: profile.id,
                    owner: None,
                }),
                window,
                cx,
            );
            return;
        }

        let tab_id = self.workspace.allocate_tab_id();
        let tab = TabState::new_sftp(tab_id, &profile);
        let start_result = self.controllers.sftp.update(cx, |controller, cx| {
            controller.start_tab(tab_id, profile.clone(), None, cx)
        });
        if let Err(error) = start_result {
            log::warn!("failed to open SFTP tab: {error}");
            self.shell.status_message = i18n::string("trusted.messages.profile_not_found");
            cx.notify();
            return;
        }
        self.unload_active_topbar_workspace(cx);
        self.insert_sftp_tab(tab, cx);
        let index = self.workspace.tabs.len() - 1;
        self.workspace.active_topbar_tab = self.workspace.tabs.id_at(index);
        self.controllers.sftp.read(cx).reset_path_editing();
        self.reset_loaded_workspace(cx);
        self.rebind_terminal_focus_reporting(window, cx);
        self.sync_terminal_focus_reporting(window, cx);
        self.shell.shell_state.sidebar_section = SidebarSection::Hosts;
        self.controllers
            .session
            .read(cx)
            .set_host_editor_state(false, false);
        self.sync_sftp_path_inputs_for_tab(tab_id, cx);
        self.sync_sftp_tables_for_tab(tab_id, cx);
        self.shell.status_message = i18n::string_args(
            "sftp.messages.opened_tab_for",
            &[("profile", &profile.name)],
        );
        cx.notify();
    }

    pub(in crate::ui::shell) fn open_hosts_tab(&mut self, cx: &mut Context<Self>) {
        self.unload_active_topbar_workspace(cx);

        let tab_id = self.workspace.allocate_tab_id();
        self.workspace.push_hosts_tab(TabState::new_hosts(tab_id));
        self.workspace.active_topbar_tab = Some(tab_id);
        self.workspace.workspace.active_tab = None;
        self.shell.shell_state.sidebar_section = SidebarSection::Hosts;
        self.controllers
            .session
            .read(cx)
            .set_host_editor_state(false, false);
        self.shell.status_message = i18n::string("navigation.messages.opened_new_hosts_tab");
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_sidebar_section(
        &mut self,
        section: SidebarSection,
        cx: &mut Context<Self>,
    ) {
        let preserve_hosts_tab_selection = self.active_tab_is_hosts();

        if self.shell.shell_state.sidebar_section == section
            && (self.workspace.active_topbar_tab.is_none() || preserve_hosts_tab_selection)
        {
            return;
        }

        self.unload_active_topbar_workspace(cx);
        self.shell.shell_state.sidebar_section = section;
        if !preserve_hosts_tab_selection {
            self.workspace.active_topbar_tab = None;
        }
        self.workspace.workspace.active_tab = None;
        if section != SidebarSection::PortForwarding {
            self.controllers
                .session
                .read(cx)
                .clear_port_forward_editor();
        }
        if section != SidebarSection::Snippets {
            self.controllers
                .session
                .read(cx)
                .set_snippets_editor_open(false);
        }
        if section != SidebarSection::Keychain {
            self.controllers
                .keychain
                .update(cx, |controller, cx| controller.dismiss_editor(cx));
        }
        if section == SidebarSection::Keychain {
            self.controllers
                .keychain
                .update(cx, |controller, cx| controller.refresh_keychain_data(cx));
        } else {
            let title = section.title();
            self.shell.status_message = i18n::string_args(
                "navigation.messages.viewing_section",
                &[("section", &title)],
            );
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn open_terminal_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self
            .workspace
            .workspace
            .active_tab
            .filter(|tab_id| self.session_tab(*tab_id, cx).is_some())
        else {
            return;
        };
        let controller = self.controllers.session.clone();
        controller.update(cx, |controller, cx| {
            controller.open_terminal_search(tab_id, window, cx);
        });
    }

    pub(in crate::ui::shell) fn close_terminal_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let terminal_focus = self.workspace.workspace.active_pane.terminal_focus.clone();
        let controller = self.controllers.session.clone();
        controller.update(cx, |controller, cx| {
            controller.close_terminal_search(&terminal_focus, window, cx);
        });
    }

    pub(in crate::ui::shell) fn with_active_window(
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

    pub(in crate::ui::shell) fn open_sync_provider_config_popup(
        &mut self,
        provider: SyncProvider,
        window: &mut super::Window,
        cx: &mut Context<Self>,
    ) {
        let controller = self.controllers.settings.clone();
        if let Some(popup) = controller.update(cx, |controller, cx| {
            controller.open_sync_provider_config_popup(provider, window, cx)
        }) {
            self.prepare_sync_overlay(DialogOverlaySnapshot::SyncProviderConfigPopup(popup), cx);
        }
    }

    fn prepare_sync_overlay(&mut self, snapshot: DialogOverlaySnapshot, cx: &mut Context<Self>) {
        let stable_key = snapshot.stable_key();
        self.shell
            .exiting_dialogs
            .retain(|dialog| dialog.snapshot.stable_key() != stable_key);
        cx.notify();
    }

    pub(in crate::ui::shell) fn apply_sync_reload(
        &mut self,
        reload: SyncReloadResult,
        cx: &mut Context<Self>,
    ) {
        let any_reload_failed = reload.any_failed();
        let SyncReloadResult {
            settings,
            sessions,
            snippets,
            managed_keys,
        } = reload;
        let mut settings = Some(settings);
        let mut sessions = Some(sessions);
        let mut snippets = Some(snippets);
        let mut managed_keys = Some(managed_keys);

        for domain in sync_reload_domains() {
            match domain {
                SyncReloadDomain::Settings => match settings
                    .take()
                    .expect("settings reload is distributed once")
                {
                    Ok(store) => {
                        self.controllers.settings.update(cx, |controller, cx| {
                            controller.replace_settings_store(store, cx);
                        });
                        miaominal_settings::sync_component_theme(cx);
                    }
                    Err(error) => log::warn!("failed to reload settings after sync: {error}"),
                },
                SyncReloadDomain::Sessions => {
                    match sessions.take().expect("session reload is distributed once") {
                        Ok(sessions) => {
                            self.controllers.session.read(cx).replace_profiles(sessions)
                        }
                        Err(error) => log::warn!("failed to reload sessions after sync: {error}"),
                    }
                }
                SyncReloadDomain::Snippets => match snippets
                    .take()
                    .expect("snippet reload is distributed once")
                {
                    Ok(snippets) => self.controllers.session.read(cx).replace_snippets(snippets),
                    Err(error) => log::warn!("failed to reload snippets after sync: {error}"),
                },
                SyncReloadDomain::ManagedKeys => match managed_keys
                    .take()
                    .expect("managed key reload is distributed once")
                {
                    Ok(keys) => self.controllers.keychain.update(cx, |controller, cx| {
                        controller.replace_managed_keys(keys, cx);
                    }),
                    Err(error) => log::warn!("failed to reload keys after sync: {error}"),
                },
            }
        }

        if any_reload_failed {
            let notification = crate::ui::shell::error_notification(
                i18n::string("settings.sync.status.notifications.reload_failed_title"),
                i18n::string("settings.sync.status.notifications.reload_failed_message"),
            );
            if let Some(window_handle) = cx.active_window() {
                let _ = window_handle.update(cx, move |_, window, cx| {
                    window.push_notification(notification, cx);
                });
            }
        }

        let settings_controller = self.controllers.settings.read(cx);
        let settings = settings_controller.settings().clone();
        let settings_forms = settings_controller.forms();
        let ai_provider_options = ai_provider_select_options(&settings);
        let current_selected = settings_forms
            .ai_provider_select
            .read(cx)
            .selected_value()
            .cloned();
        let selected_provider_id = current_selected
            .as_deref()
            .filter(|id| {
                ai_provider_options
                    .iter()
                    .any(|option| option.value() == *id)
            })
            .or_else(|| {
                ai_provider_options
                    .first()
                    .map(|option| option.value().as_str())
            })
            .map(ToOwned::to_owned);
        if selected_provider_id.as_deref()
            != Some(settings.selected_ai_provider_id.as_deref().unwrap_or(""))
            && let Some(ref id) = selected_provider_id
        {
            let mut settings_store = self.controllers.settings.read(cx).settings_store();
            settings_store.update(|settings| {
                settings.selected_ai_provider_id = Some(id.clone());
            });
            self.controllers.settings.update(cx, |controller, cx| {
                controller.replace_settings_store(settings_store, cx);
            });
        }
        let ai_provider_select = settings_forms.ai_provider_select;
        let provider_id = selected_provider_id.clone();
        self.with_active_window(cx, move |window, cx| {
            ai_provider_select.update(cx, |select, cx| {
                select.set_items(ai_provider_options, window, cx);
                if let Some(id) = provider_id.as_ref() {
                    select.set_selected_value(id, window, cx);
                } else {
                    select.set_selected_index(None, window, cx);
                }
            });
        });

        let managed_key_options =
            ManagedKeySelectItem::sorted_items(self.controllers.keychain.read(cx).managed_keys());
        self.controllers.session.update(cx, |controller, cx| {
            controller.sync_managed_key_select_in_active_window(managed_key_options, None, cx);
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn edit_ai_provider(
        &mut self,
        provider_id: String,
        window: &mut super::Window,
        cx: &mut Context<Self>,
    ) {
        let controller = self.controllers.settings.clone();
        if let Some(popup) = controller.update(cx, |controller, cx| {
            controller.edit_ai_provider(provider_id, window, cx)
        }) {
            self.prepare_ai_provider_popup_overlay(popup, cx);
        }
    }

    fn prepare_ai_provider_popup_overlay(
        &mut self,
        popup: PendingAiProviderPopupState,
        cx: &mut Context<Self>,
    ) {
        let stable_key = DialogOverlaySnapshot::AiProviderPopup(popup).stable_key();
        self.shell
            .exiting_dialogs
            .retain(|dialog| dialog.snapshot.stable_key() != stable_key);
        cx.notify();
    }

    pub(in crate::ui::shell) fn open_web_search_config_popup(
        &mut self,
        window: &mut super::Window,
        cx: &mut Context<Self>,
    ) {
        let controller = self.controllers.settings.clone();
        let popup = controller.update(cx, |controller, cx| {
            controller.open_web_search_config_popup(window, cx)
        });
        let Some(popup) = popup else {
            return;
        };

        let stable_key = DialogOverlaySnapshot::WebSearchConfigPopup(popup).stable_key();
        self.shell
            .exiting_dialogs
            .retain(|dialog| dialog.snapshot.stable_key() != stable_key);
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_auto_collect_session_monitoring(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let controller = self.controllers.settings.clone();
        let changed = controller.update(cx, |controller, cx| {
            controller.set_auto_collect_session_monitoring(enabled, cx)
        });
        if !changed {
            return;
        }

        self.controllers.session.update(cx, |controller, cx| {
            controller.apply_auto_collect_monitoring_preference(enabled, cx);
        });
        cx.notify();
    }

    pub(super) fn invalidate_terminal_metrics(&mut self) {
        self.workspace.workspace.active_pane.terminal_bounds = None;
        self.workspace.workspace.active_pane.terminal_cell_width = terminal_cell_width_default();
        self.workspace.workspace.active_pane.terminal_line_height = terminal_line_height_default();

        for parked in self.workspace.workspace.parked_panes.values_mut() {
            parked.terminal_bounds = None;
            parked.terminal_cell_width = terminal_cell_width_default();
            parked.terminal_line_height = terminal_line_height_default();
        }
    }

    fn start_local_data_reset(&mut self, window: &mut super::Window, cx: &mut Context<Self>) {
        if self
            .controllers
            .settings
            .read(cx)
            .local_data_reset_in_progress()
        {
            return;
        }

        let session_ids = self
            .controllers
            .session
            .read(cx)
            .profiles()
            .iter()
            .map(|session| session.id.clone())
            .collect();
        let managed_key_ids = self.controllers.keychain.read(cx).managed_key_ids();
        let ai_provider_ids = self
            .controllers
            .settings
            .read(cx)
            .settings()
            .clone()
            .ai_providers
            .into_iter()
            .map(|provider| provider.id)
            .collect();
        let agent_controller = self.controllers.agent.clone();
        agent_controller.update(cx, |controller, cx| {
            controller.close_chat_history(cx);
        });
        self.controllers.settings.update(cx, |controller, cx| {
            controller.start_local_data_reset(
                session_ids,
                managed_key_ids,
                ai_provider_ids,
                agent_controller,
                window,
                cx,
            );
        });
    }

    fn schedule_application_rebuild(&mut self, cx: &mut Context<Self>) {
        let runtime = self.controllers.settings.read(cx).runtime();
        let notification_window = cx.active_window();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(0))
                .await;

            let Some(window_handle) = notification_window else {
                let _ = this.update(cx, |this, cx| {
                    this.shell.status_message = i18n::string(
                        "settings.about.reset_local.notifications.window_unavailable.message",
                    );
                    cx.notify();
                });
                return;
            };

            let this_for_window = this.clone();
            let fallback_this = this.clone();
            let update_result = window_handle.update(cx, move |_, window, cx| {
                if let Err(error) = this_for_window.update(cx, move |this, cx| {
                    *this = AppView::new(runtime, window, cx);
                    let message =
                        i18n::string("settings.about.reset_local.notifications.success.message");
                    this.shell.status_message = message.clone();
                    window.push_notification(
                        crate::ui::shell::success_notification(
                            i18n::string("settings.about.reset_local.notifications.success.title"),
                            message,
                        ),
                        cx,
                    );
                    cx.notify();
                }) {
                    log::debug!("failed to rebuild application after local data reset: {error:?}");
                }
            });

            if let Err(error) = update_result {
                log::debug!("failed to access active window after local data reset: {error:?}");
                let _ = fallback_this.update(cx, |this, cx| {
                    this.shell.status_message = i18n::string(
                        "settings.about.reset_local.notifications.window_unavailable.message",
                    );
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn handle_tab_open_request(&mut self, request: &TabOpenRequest, cx: &mut Context<Self>) {
        match request {
            TabOpenRequest::NewProfileEditor => {
                let entity = cx.entity();
                cx.defer(move |cx| {
                    let Some(window_handle) = cx.active_window() else {
                        return;
                    };
                    let entity_for_window = entity.clone();
                    if let Err(error) = window_handle.update(cx, move |_, window, cx| {
                        entity_for_window.update(cx, |this, cx| {
                            let managed_key_options = ManagedKeySelectItem::sorted_items(
                                this.controllers.keychain.read(cx).managed_keys(),
                            );
                            this.shell.shell_state.sidebar_section = super::SidebarSection::Hosts;
                            this.controllers.session.update(cx, |controller, cx| {
                                controller.add_profile(managed_key_options, window, cx);
                            });
                        });
                    }) {
                        log::debug!("failed to open new host profile editor: {error:?}");
                    }
                });
            }
            TabOpenRequest::ProfileEditor {
                profile_id,
                open_hosts_tab,
            } => {
                let profile_id = profile_id.clone();
                let open_hosts_tab = *open_hosts_tab;
                let entity = cx.entity();
                cx.defer(move |cx| {
                    let Some(window_handle) = cx.active_window() else {
                        return;
                    };
                    let entity_for_window = entity.clone();
                    if let Err(error) = window_handle.update(cx, move |_, window, cx| {
                        entity_for_window.update(cx, |this, cx| {
                            let index = this
                                .controllers
                                .session
                                .read(cx)
                                .profiles()
                                .iter()
                                .position(|profile| profile.id == profile_id);
                            let Some(index) = index else {
                                this.shell.status_message =
                                    i18n::string("trusted.messages.profile_not_found");
                                cx.notify();
                                return;
                            };
                            let managed_key_options = ManagedKeySelectItem::sorted_items(
                                this.controllers.keychain.read(cx).managed_keys(),
                            );
                            if open_hosts_tab {
                                this.open_hosts_tab(cx);
                            } else {
                                this.shell.shell_state.sidebar_section =
                                    super::SidebarSection::Hosts;
                            }
                            this.controllers.session.update(cx, |controller, cx| {
                                controller.open_profile_editor(
                                    index,
                                    managed_key_options,
                                    window,
                                    cx,
                                );
                            });
                        });
                    }) {
                        log::debug!("failed to open linked host profile: {error:?}");
                    }
                });
            }
            TabOpenRequest::ProfileConnectionTest { profile } => {
                let profile = (**profile).clone();
                let entity = cx.entity();
                cx.defer(move |cx| {
                    let Some(window_handle) = cx.active_window() else {
                        return;
                    };
                    let entity_for_window = entity.clone();
                    if let Err(error) = window_handle.update(cx, move |_, window, cx| {
                        entity_for_window.update(cx, |this, cx| {
                            this.start_profile_connection_test(profile, window, cx);
                        });
                    }) {
                        log::debug!("failed to start profile connection test: {error:?}");
                    }
                });
            }
            TabOpenRequest::Sftp { profile_id, owner } => {
                if let Some(owner) = owner {
                    self.ensure_session_side_panel_sftp_tab(*owner, cx);
                    return;
                }

                let profile_id = profile_id.clone();
                let entity = cx.entity();
                cx.defer(move |cx| {
                    let Some(window_handle) = cx.active_window() else {
                        return;
                    };
                    let entity_for_window = entity.clone();
                    if let Err(error) = window_handle.update(cx, move |_, window, cx| {
                        entity_for_window.update(cx, |this, cx| {
                            let profile = this
                                .controllers
                                .session
                                .read(cx)
                                .profiles()
                                .iter()
                                .find(|profile| profile.id == profile_id)
                                .cloned();
                            let Some(profile) = profile else {
                                this.shell.status_message =
                                    i18n::string("trusted.messages.profile_not_found");
                                cx.notify();
                                return;
                            };
                            this.open_sftp_tab(profile, window, cx);
                        });
                    }) {
                        log::debug!("failed to open profile SFTP tab: {error:?}");
                    }
                });
            }
            TabOpenRequest::Session { profile_id } => {
                let profile_id = profile_id.clone();
                let entity = cx.entity();
                cx.defer(move |cx| {
                    let Some(window_handle) = cx.active_window() else {
                        return;
                    };
                    let entity_for_window = entity.clone();
                    if let Err(error) = window_handle.update(cx, move |_, window, cx| {
                        entity_for_window.update(cx, |this, cx| {
                            let profile = this
                                .controllers
                                .session
                                .read(cx)
                                .profiles()
                                .iter()
                                .enumerate()
                                .find(|(_, profile)| profile.id == profile_id)
                                .map(|(index, profile)| (index, profile.clone()));
                            let Some((index, profile)) = profile else {
                                this.shell.status_message =
                                    i18n::string("trusted.messages.profile_not_found");
                                cx.notify();
                                return;
                            };
                            this.controllers
                                .session
                                .read(cx)
                                .set_selected_profile(Some(index));
                            this.open_session_tab(profile, window, cx);
                        });
                    }) {
                        log::debug!("failed to open profile session tab: {error:?}");
                    }
                });
            }
            TabOpenRequest::PortForwarding {
                profile_id,
                rule_id,
            } => {
                let profile_id = profile_id.clone();
                let rule_id = rule_id.clone();
                let entity = cx.entity();
                cx.defer(move |cx| {
                    entity.update(cx, |this, cx| {
                        let tab_id = this.workspace.allocate_tab_id();
                        let controller = this.controllers.session.clone();
                        let start = controller.update(cx, |controller, cx| {
                            controller.start_port_forward_session(tab_id, &profile_id, &rule_id, cx)
                        });
                        let Some(PortForwardSessionStart {
                            tab,
                            events,
                            feedback,
                        }) = start
                        else {
                            return;
                        };
                        this.register_session_tab_metadata(tab, cx);
                        controller.update(cx, |controller, cx| {
                            controller.spawn_session_event_loop(tab_id, events, cx);
                        });
                        this.shell.status_message = feedback;
                        cx.notify();
                    });
                });
            }
        }
    }

    fn profile_save_requires_local_vault_unlock(&self, profile: &SessionProfile, cx: &App) -> bool {
        let settings = self.controllers.settings.read(cx);
        settings.sync_requires_local_vault_unlock()
            || (settings.local_vault_status() == LocalVaultStatus::Locked
                && (!profile.password.is_empty() || !profile.passphrase.is_empty()))
    }

    fn handle_profile_save_request(
        &mut self,
        profile: SessionProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.profile_save_requires_local_vault_unlock(&profile, cx) {
            self.defer_app_command_in_window(
                DeferredAppCommand::Session(SessionDeferredCommand::SaveProfile),
                window,
                cx,
            );
            return;
        }

        let managed_key_options =
            ManagedKeySelectItem::sorted_items(self.controllers.keychain.read(cx).managed_keys());
        let controller = self.controllers.session.clone();
        controller.update(cx, |controller, cx| {
            controller.commit_profile_save_request(profile, managed_key_options, window, cx);
        });
    }

    fn handle_snippet_save_request(
        &mut self,
        snippet: miaominal_core::snippet::SnippetRecord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .controllers
            .settings
            .read(cx)
            .sync_requires_local_vault_unlock()
        {
            self.defer_app_command_in_window(
                DeferredAppCommand::Session(SessionDeferredCommand::SaveSnippet),
                window,
                cx,
            );
            return;
        }

        let controller = self.controllers.session.clone();
        controller.update(cx, |controller, cx| {
            controller.commit_snippet_save_request(snippet, window, cx);
        });
    }

    pub(super) fn handle_app_command_in_window(
        &mut self,
        command: &AppCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            AppCommand::TerminalFocusReportingRequested => {
                self.sync_terminal_focus_reporting(window, cx);
                cx.notify();
            }
            AppCommand::WindowActivationChanged { active } => {
                self.sync_terminal_focus_reporting(window, cx);
                if !active {
                    self.finish_any_active_sftp_drag_selection(cx);
                }
                cx.notify();
            }
            AppCommand::TerminalMenuRequested { pane_id, command } => {
                if let Some(pane_id) = pane_id {
                    self.set_active_pane(*pane_id, window, cx);
                }
                match command {
                    TerminalMenuCommand::Copy => {
                        if let Some(tab_id) = self.workspace.workspace.active_tab {
                            let controller = self.controllers.session.clone();
                            controller.update(cx, |controller, cx| {
                                controller.copy_terminal_selection(tab_id, cx);
                            });
                        }
                        window.focus(&self.workspace.workspace.active_pane.terminal_focus, cx);
                        self.sync_terminal_focus_reporting(window, cx);
                    }
                    TerminalMenuCommand::Paste => {
                        if let Some(tab_id) = self.workspace.workspace.active_tab {
                            let controller = self.controllers.session.clone();
                            controller.update(cx, |controller, cx| {
                                controller.paste_terminal_clipboard(tab_id, cx);
                            });
                        }
                        window.focus(&self.workspace.workspace.active_pane.terminal_focus, cx);
                        self.sync_terminal_focus_reporting(window, cx);
                    }
                    TerminalMenuCommand::Split(direction) => {
                        self.split_active_pane(*direction, window, cx)
                    }
                    TerminalMenuCommand::OpenSftp => self.open_sftp_tab_for_session(
                        self.workspace.workspace.active_tab,
                        window,
                        cx,
                    ),
                    TerminalMenuCommand::ClosePane => self.close_active_pane(window, cx),
                }
            }
            AppCommand::VaultActionRequested(request) => {
                self.handle_local_vault_action_request(request.clone(), window, cx)
            }
            AppCommand::LocalDataResetRequested => self.start_local_data_reset(window, cx),
            AppCommand::SaveProfileRequested(profile) => {
                self.handle_profile_save_request((**profile).clone(), window, cx)
            }
            AppCommand::SaveSnippetRequested(snippet) => {
                self.handle_snippet_save_request((**snippet).clone(), window, cx)
            }
            AppCommand::ImportProfilesRequested(source) => {
                let controller = self.controllers.session.clone();
                controller.update(cx, |controller, cx| {
                    controller.import_profiles_from_source(*source, window, cx);
                });
            }
            AppCommand::SessionEventApplied { tab_id, outcome } => {
                self.handle_session_event_outcome(*tab_id, outcome.clone(), window, cx)
            }
            _ => self.handle_app_command(command, cx),
        }
    }

    fn handle_app_command(&mut self, command: &AppCommand, cx: &mut Context<Self>) {
        match command {
            AppCommand::Feedback(message) => self.shell.status_message = message.clone(),
            AppCommand::TabStatusChanged { tab_id, status } => {
                if let Some(mut tab) = self.workspace.tabs.get_mut(*tab_id) {
                    tab.status = status.clone();
                }
            }
            AppCommand::TerminalScrolledToBottom(tab_id) => {
                if let Some(pane_id) = self.pane_of_tab(*tab_id) {
                    self.touch_terminal_scrollbar_visibility(pane_id, cx);
                }
            }
            AppCommand::TerminalFocusReportingRequested
            | AppCommand::WindowActivationChanged { .. }
            | AppCommand::TerminalMenuRequested { .. }
            | AppCommand::VaultActionRequested(_)
            | AppCommand::LocalDataResetRequested
            | AppCommand::SaveProfileRequested(_)
            | AppCommand::SaveSnippetRequested(_)
            | AppCommand::ImportProfilesRequested(_)
            | AppCommand::SessionEventApplied { .. } => {}
            AppCommand::ManagedKeysChanged(change) => self.handle_managed_keys_change(change, cx),
            AppCommand::SidebarSectionRequested(section) => self.set_sidebar_section(*section, cx),
            AppCommand::EnsureSessionSftpRequested(tab_id) => {
                self.ensure_session_side_panel_sftp_tab(*tab_id, cx);
            }
            AppCommand::SessionMonitoringPreferenceChanged(enabled) => {
                self.set_auto_collect_session_monitoring(*enabled, cx)
            }
            AppCommand::AgentModePreferenceChanged(mode) => {
                self.controllers.settings.update(cx, |controller, cx| {
                    controller.persist_agent_mode_preference(*mode, cx);
                });
            }
            AppCommand::PersistSftpBrowserHiddenColumns {
                side,
                hidden_columns,
            } => {
                self.controllers.settings.update(cx, |controller, cx| {
                    controller.persist_sftp_browser_hidden_columns(
                        *side,
                        hidden_columns.clone(),
                        cx,
                    );
                });
            }
            AppCommand::VaultUnlockRequested(command) => match command {
                Some(command) => self.defer_app_command(command.clone(), cx),
                None => self.prompt_local_vault_unlock(cx),
            },
            AppCommand::CredentialsChanged => {
                let controller = self.controllers.settings.clone();
                if let Some(result) = controller.update(cx, |controller, _| {
                    controller.take_local_vault_operation_result()
                }) {
                    self.apply_local_vault_operation_result(result, cx);
                }
            }
            AppCommand::LocaleRefresh => {
                if let Some(window_handle) = cx.active_window()
                    && let Err(error) = window_handle.update(cx, |_, window, cx| {
                        self.refresh_localized_placeholders(window, cx);
                    })
                {
                    log::debug!(
                        "failed to refresh localized placeholders after language change: {error:?}"
                    );
                }
            }
            AppCommand::RebuildApplication => self.schedule_application_rebuild(cx),
            AppCommand::OverlayDismissed(snapshot) => self.start_dialog_exit(snapshot.clone(), cx),
            AppCommand::OpenTab(request) => self.handle_tab_open_request(request, cx),
            AppCommand::SyncReloaded(reload) => self.apply_sync_reload((**reload).clone(), cx),
            AppCommand::CloseTab(tab_id) => {
                let tab_id = *tab_id;
                let entity = cx.entity();
                cx.defer(move |cx| {
                    entity.update(cx, |this, cx| {
                        let _ = this.remove_tab_metadata_after_controller_close(tab_id, cx);
                        this.prune_closed_tab_references();
                        cx.notify();
                    });
                });
            }
        }
        cx.notify();
    }

    fn handle_managed_keys_change(&mut self, change: &ManagedKeysChange, cx: &mut Context<Self>) {
        if let ManagedKeysChange::Removed { key_id } = change {
            let sessions_changed = {
                let mut profiles = self.controllers.session.read(cx).profiles_mut();
                clear_managed_key_profile_references(&mut profiles, key_id)
            };
            let host_editor_forms = self.controllers.session.read(cx).host_editor_forms();

            if host_editor_forms
                .managed_key_select
                .read(cx)
                .selected_value()
                .is_some_and(|selected| selected == key_id)
                && host_editor_forms.editing_auth_method == AuthMethod::ManagedKey
            {
                self.controllers
                    .session
                    .read(cx)
                    .host_editor_forms_mut()
                    .editing_auth_method = AuthMethod::Password;
            }

            if sessions_changed
                && self.controllers.session.read(cx).session_store_available()
                && let Err(error) = self.controllers.session.read(cx).persist_profiles()
            {
                self.shell.status_message = i18n::string_args(
                    "keychain.messages.removed_locally_session_save_failed",
                    &[("error", &error.to_string())],
                );
            }
        }

        let managed_key_options =
            ManagedKeySelectItem::sorted_items(self.controllers.keychain.read(cx).managed_keys());
        self.controllers.session.update(cx, |controller, cx| {
            controller.sync_managed_key_select_in_active_window(managed_key_options, None, cx);
        });
    }

    fn set_deferred_app_command(&mut self, command: DeferredAppCommand) {
        self.shell.status_message =
            i18n::string("settings.sync.vault.access_required_error.message");
        self.shell.deferred_app_command = Some(command);
    }

    fn defer_app_command(&mut self, command: DeferredAppCommand, cx: &mut Context<Self>) {
        self.set_deferred_app_command(command);
        self.open_local_vault_passphrase_popup_in_active_window(
            LocalVaultPassphrasePopupMode::PrimaryAction,
            cx,
        );
    }

    fn defer_app_command_in_window(
        &mut self,
        command: DeferredAppCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_deferred_app_command(command);
        self.open_local_vault_passphrase_popup(
            LocalVaultPassphrasePopupMode::PrimaryAction,
            window,
            cx,
        );
    }

    pub(in crate::ui::shell) fn confirm_profile_delete(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self
            .controllers
            .session
            .read(cx)
            .take_pending_profile_delete()
        else {
            return;
        };

        self.start_dialog_exit(DialogOverlaySnapshot::ProfileDelete(pending.clone()), cx);
        let managed_key_options =
            ManagedKeySelectItem::sorted_items(self.controllers.keychain.read(cx).managed_keys());
        self.controllers.session.update(cx, |controller, cx| {
            controller.delete_profile_by_id(
                &pending.profile_id,
                &pending.profile_name,
                pending.reload_inputs_after_delete,
                managed_key_options,
                window,
                cx,
            );
        });
    }
}
