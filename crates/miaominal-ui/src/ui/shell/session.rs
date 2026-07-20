use super::*;
use crate::ui::i18n;
use gpui_component::WindowExt as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LastTabCloseAction {
    Quit,
    OpenHome,
}

fn last_tab_close_action(behavior: miaominal_settings::LastTabCloseBehavior) -> LastTabCloseAction {
    match behavior {
        miaominal_settings::LastTabCloseBehavior::ExitApplication => LastTabCloseAction::Quit,
        miaominal_settings::LastTabCloseBehavior::OpenNewHomeTab => LastTabCloseAction::OpenHome,
    }
}

#[cfg(test)]
fn should_notify_for_session_output(inactive_tab: bool, had_activity: bool) -> bool {
    !inactive_tab || !had_activity
}

fn reopened_sftp_owner(
    closed_tab: &ClosedSftpTabState,
    reopened: &HashMap<TabId, TabId>,
) -> Option<TabId> {
    reopened.get(&closed_tab.owner).copied()
}

impl AppView {
    pub(in crate::ui::shell) fn profile_requires_local_vault_unlock(
        &self,
        profile: &SessionProfile,
        cx: &App,
    ) -> bool {
        self.controllers
            .session
            .read(cx)
            .profile_requires_local_vault_unlock(profile)
    }

    pub(in crate::ui::shell) fn prompt_local_vault_unlock(&mut self, cx: &mut Context<Self>) {
        self.shell.status_message =
            i18n::string("settings.sync.vault.access_required_error.message");
        self.shell.deferred_app_command = None;
        self.open_local_vault_passphrase_popup_in_active_window(
            LocalVaultPassphrasePopupMode::PrimaryAction,
            cx,
        );
    }

    pub(in crate::ui::shell) fn prompt_local_vault_unlock_in_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.shell.status_message =
            i18n::string("settings.sync.vault.access_required_error.message");
        self.shell.deferred_app_command = None;
        self.open_local_vault_passphrase_popup(
            LocalVaultPassphrasePopupMode::PrimaryAction,
            window,
            cx,
        );
    }

    pub(in crate::ui::shell) fn prompt_local_vault_unlock_for_action(
        &mut self,
        command: DeferredAppCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.shell.status_message =
            i18n::string("settings.sync.vault.access_required_error.message");
        self.shell.deferred_app_command = Some(command);
        self.open_local_vault_passphrase_popup(
            LocalVaultPassphrasePopupMode::PrimaryAction,
            window,
            cx,
        );
    }

    fn prompt_local_vault_unlock_for_profile(
        &mut self,
        profile: SessionProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_local_vault_unlock_for_action(
            DeferredAppCommand::Session(SessionDeferredCommand::OpenProfile(Box::new(profile))),
            window,
            cx,
        );
    }

    pub(in crate::ui::shell) fn start_profile_connection_test(
        &mut self,
        profile: SessionProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.profile_requires_local_vault_unlock(&profile, cx) {
            self.prompt_local_vault_unlock_in_window(window, cx);
            return;
        }

        let remove_tab_ids: Vec<TabId> = self
            .workspace
            .tabs
            .iter()
            .filter_map(|tab| {
                (self.controllers.session.read(cx).tab_purpose(tab.id)
                    == Some(SessionPurpose::ConnectionTest))
                .then_some(tab.id)
            })
            .collect();

        for tab_id in remove_tab_ids.iter().rev().copied() {
            self.controllers
                .session
                .read(cx)
                .retire_tab_resources(tab_id);
            let _ = self.remove_tab_payload_and_metadata(tab_id, cx);
        }
        if !remove_tab_ids.is_empty() {
            self.prune_closed_tab_references();
        }

        let (columns, lines) = self.estimated_terminal_size(window);
        let connection = self.controllers.session.read(cx).start_terminal_session(
            profile.clone(),
            columns,
            lines,
            false,
        );

        let tab_id = self.workspace.allocate_tab_id();

        let (tab, session) =
            SessionController::build_connection_test_tab(tab_id, &profile, connection.commands);
        self.insert_session_tab(tab, session, cx);
        self.controllers.session.update(cx, |controller, cx| {
            controller.spawn_session_event_loop(tab_id, connection.events, cx);
        });
        let profile_summary = profile.summary();
        self.shell.status_message = i18n::string_args(
            "session.messages.testing_connection_to",
            &[("profile", &profile_summary)],
        );
        cx.notify();
    }

    fn ordered_session_tab_ids(&self) -> Vec<TabId> {
        self.workspace
            .tabs
            .iter()
            .filter(|tab| tab.is_session())
            .map(|tab| tab.id)
            .collect()
    }

    pub(in crate::ui::shell) fn refresh_profile_monitoring(
        &self,
        profile_id: &str,
        excluded_tab_id: Option<TabId>,
        cx: &App,
    ) {
        let ordered_tab_ids = self.ordered_session_tab_ids();
        self.controllers
            .session
            .read(cx)
            .refresh_profile_monitoring(
                profile_id,
                excluded_tab_id,
                &ordered_tab_ids,
                self.controllers
                    .settings
                    .read(cx)
                    .settings()
                    .auto_collect_session_monitoring,
            );
    }

    pub(in crate::ui::shell) fn build_session_tab(
        &mut self,
        profile: SessionProfile,
        cx: &App,
    ) -> (TabState, SessionTabState) {
        let auto_collect_monitoring = self
            .controllers
            .session
            .read(cx)
            .shared_monitoring_enabled(&profile.id)
            .unwrap_or(
                self.controllers
                    .settings
                    .read(cx)
                    .settings()
                    .auto_collect_session_monitoring,
            );

        let terminal = TerminalState::default();

        let tab_id = self.workspace.allocate_tab_id();

        SessionController::build_pending_tab(tab_id, profile, terminal, auto_collect_monitoring)
    }

    fn estimated_terminal_size(&self, window: &Window) -> (usize, usize) {
        let cell_width = terminal_cell_width(window);
        let line_height = terminal_line_height(window);

        let (available_width, available_height) =
            if let Some(bounds) = self.workspace.workspace.active_pane.terminal_bounds {
                (bounds.size.width, bounds.size.height)
            } else {
                let win = window.bounds().size;
                let width = (win.width - px(TERMINAL_PANEL_BORDER)).max(px(cell_width * 2.0));
                let height = (win.height
                    - px(top_bar_height() + FOOTER_HEIGHT + TERMINAL_PANEL_BORDER))
                .max(px(line_height));
                (width, height)
            };

        let columns = (f32::from(available_width) / cell_width)
            .floor()
            .max(miaominal_terminal::MIN_TERMINAL_COLUMNS as f32) as usize;
        let lines = (f32::from(available_height) / line_height).floor().max(1.0) as usize;
        (columns, lines)
    }

    pub(in crate::ui::shell) fn open_session_tab(
        &mut self,
        profile: SessionProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.profile_requires_local_vault_unlock(&profile, cx) {
            self.prompt_local_vault_unlock_for_profile(profile, window, cx);
            return;
        }

        let should_animate_hosts_to_terminal = self
            .workspace
            .active_topbar_tab
            .and_then(|tab_id| self.workspace.tabs.get(tab_id))
            .is_some_and(|tab| tab.is_hosts())
            && self.shell.shell_state.sidebar_section == SidebarSection::Hosts;
        let preserve_host_editor_sidebar = should_animate_hosts_to_terminal
            && self
                .controllers
                .session
                .read(cx)
                .editor_state()
                .host_editor_open;
        let replace_index = self
            .workspace
            .active_topbar_tab
            .and_then(|tab_id| self.workspace.tabs.index_of(tab_id))
            .filter(|&index| {
                self.workspace
                    .tabs
                    .at(index)
                    .is_some_and(|tab| tab.is_top_level() && tab.is_hosts())
            });
        let (mut tab, session) = self.build_session_tab(profile.clone(), cx);
        tab.title = self.unique_topbar_tab_title(&tab.title, None);

        self.unload_active_topbar_workspace(cx);

        let index = if let Some(index) = replace_index {
            self.replace_with_session_tab(index, tab, session, cx);
            index
        } else {
            self.insert_session_tab(tab, session, cx);
            self.workspace.tabs.len() - 1
        };

        self.workspace.active_topbar_tab = self.workspace.tabs.id_at(index);
        self.load_topbar_workspace(index, cx);
        self.rebind_terminal_focus_reporting(window, cx);

        self.shell.shell_state.sidebar_section = SidebarSection::Hosts;
        if !preserve_host_editor_sidebar {
            self.controllers
                .session
                .read(cx)
                .set_host_editor_state(false, false);
        }
        self.shell.status_message = i18n::string_args(
            "session.messages.opening_tab_for",
            &[("profile", &profile.name)],
        );
        window.focus(&self.workspace.workspace.active_pane.terminal_focus, cx);
        self.sync_terminal_focus_reporting(window, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn unique_topbar_tab_title(
        &self,
        desired_title: &str,
        excluding_index: Option<usize>,
    ) -> String {
        let base_title = desired_title.trim();
        let base_title = if base_title.is_empty() {
            i18n::string("placeholders.workspace.tab_name")
        } else {
            base_title.to_string()
        };
        let mut candidate = base_title.clone();
        let mut suffix = 2usize;
        while self
            .workspace
            .tabs
            .iter()
            .enumerate()
            .any(|(index, tab)| Some(index) != excluding_index && tab.title == candidate)
        {
            candidate = format!("{base_title} ({suffix})");
            suffix = suffix.saturating_add(1);
        }
        candidate
    }

    pub(in crate::ui::shell) fn open_sftp_tab_for_session(
        &mut self,
        tab_id: Option<TabId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile = tab_id.and_then(|tab_id| {
            self.controllers
                .session
                .read(cx)
                .resolved_profile_for_tab(tab_id)
        });

        let Some(profile) = profile else {
            self.shell.status_message =
                i18n::string("session.messages.open_sftp_requires_active_ssh");
            cx.notify();
            return;
        };

        self.open_sftp_tab(profile, window, cx);
    }

    fn push_recently_closed_tab(&mut self, bundle: ClosedTabBundle) {
        const MAX_RECENTLY_CLOSED_TABS: usize = 20;

        self.workspace.recently_closed_tabs.push(bundle);
        if self.workspace.recently_closed_tabs.len() > MAX_RECENTLY_CLOSED_TABS {
            let overflow = self.workspace.recently_closed_tabs.len() - MAX_RECENTLY_CLOSED_TABS;
            self.workspace.recently_closed_tabs.drain(0..overflow);
        }
    }

    fn restore_closed_workspace_slot(slot: &mut Option<TabId>, reopened: &HashMap<TabId, TabId>) {
        *slot = reopened_tab_id(*slot, reopened);
    }

    fn restore_closed_workspace(
        &self,
        mut workspace: TabWorkspaceState,
        reopened: &HashMap<TabId, TabId>,
    ) -> TabWorkspaceState {
        Self::restore_closed_workspace_slot(&mut workspace.active_tab, reopened);
        for parked in workspace.parked_panes.values_mut() {
            Self::restore_closed_workspace_slot(&mut parked.active_tab, reopened);
        }
        workspace
    }

    fn build_closed_tab_bundle(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Option<ClosedTabBundle> {
        let tab = self.workspace.tabs.at(index)?;
        if !tab.is_top_level() {
            return None;
        }
        let tab_id = tab.id;
        let kind = tab.kind;

        match kind {
            TabKindTag::Hosts => Some(ClosedTabBundle::Hosts),
            TabKindTag::Sftp => {
                let sftp = self.sftp_tab(tab_id, cx)?;
                let profile = self
                    .controllers
                    .session
                    .read(cx)
                    .profiles()
                    .iter()
                    .find(|profile| profile.id == sftp.profile_id)
                    .cloned()?;
                Some(ClosedTabBundle::Sftp { profile })
            }
            TabKindTag::Session => {
                let mut removed = self.owned_tab_indices_for_topbar(index);
                removed.sort_unstable();
                removed.dedup();
                let session_removed = removed
                    .iter()
                    .copied()
                    .filter(|&remove_index| {
                        self.workspace.tabs.at(remove_index).is_some_and(|tab| {
                            self.controllers
                                .session
                                .read(cx)
                                .tab_purpose(tab.id)
                                .is_some()
                        })
                    })
                    .collect::<Vec<_>>();

                let tabs = session_removed
                    .iter()
                    .filter_map(|&remove_index| {
                        let tab = self.workspace.tabs.at(remove_index)?;
                        let profile = self
                            .controllers
                            .session
                            .read(cx)
                            .reopen_profile_for_tab(tab.id)?;
                        Some(ClosedSessionTabState {
                            tab_id: tab.id,
                            profile,
                            hidden_from_topbar: !tab.is_top_level(),
                        })
                    })
                    .collect::<Vec<_>>();
                if tabs.is_empty() {
                    return None;
                }

                let sftp_tabs = removed
                    .iter()
                    .filter_map(|&remove_index| {
                        let tab = self.workspace.tabs.at(remove_index)?;
                        if !tab.is_sftp() {
                            return None;
                        }
                        let owner = tab.owner()?;
                        let profile_id = self.sftp_tab(tab.id, cx)?.profile_id.clone();
                        let profile = self
                            .controllers
                            .session
                            .read(cx)
                            .profiles()
                            .iter()
                            .find(|profile| profile.id == profile_id)
                            .cloned()
                            .or_else(|| {
                                self.session_tab(owner, cx)
                                    .and_then(|session| session.pending_profile.clone())
                                    .filter(|profile| profile.id == profile_id)
                            })?;
                        Some(ClosedSftpTabState {
                            tab_id: tab.id,
                            owner,
                            profile,
                        })
                    })
                    .collect::<Vec<_>>();

                let workspace =
                    if self.workspace.active_topbar_tab == self.workspace.tabs.id_at(index) {
                        self.unload_active_topbar_workspace(cx);
                        self.workspace
                            .tabs
                            .id_at(index)
                            .and_then(|tab_id| self.workspace.take_parked_workspace(tab_id))
                    } else {
                        self.workspace
                            .tabs
                            .id_at(index)
                            .and_then(|tab_id| self.workspace.take_parked_workspace(tab_id))
                    };

                Some(ClosedTabBundle::SessionWorkspace {
                    tabs,
                    sftp_tabs,
                    workspace,
                })
            }
        }
    }

    pub(in crate::ui::shell) fn activate_next_topbar_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let visible_tabs = self
            .workspace
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| tab.is_top_level().then_some(index))
            .collect::<Vec<_>>();
        if visible_tabs.len() <= 1 {
            return;
        }

        let next_index = self
            .workspace
            .active_topbar_tab
            .and_then(|active| {
                visible_tabs
                    .iter()
                    .position(|&index| self.workspace.tabs.id_at(index) == Some(active))
            })
            .map(|current| visible_tabs[(current + 1) % visible_tabs.len()])
            .unwrap_or(visible_tabs[0]);
        self.activate_tab(next_index, window, cx);
    }

    pub(in crate::ui::shell) fn close_active_topbar_tab_shortcut(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .workspace
            .active_topbar_tab
            .and_then(|tab_id| self.workspace.tabs.index_of(tab_id))
        else {
            self.shell.status_message = i18n::string("session.messages.no_tab_to_close");
            cx.notify();
            return;
        };

        self.close_tab(index, window, cx);
    }

    pub(in crate::ui::shell) fn reopen_last_closed_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bundle) = self.workspace.recently_closed_tabs.pop() else {
            self.shell.status_message = i18n::string("session.messages.no_recently_closed_tab");
            cx.notify();
            return;
        };

        match bundle {
            ClosedTabBundle::Hosts => self.open_hosts_tab(cx),
            ClosedTabBundle::Sftp { profile } => {
                if self.profile_requires_local_vault_unlock(&profile, cx) {
                    self.workspace
                        .recently_closed_tabs
                        .push(ClosedTabBundle::Sftp { profile });
                    self.prompt_local_vault_unlock_in_window(window, cx);
                    return;
                }
                self.open_sftp_tab(profile, window, cx)
            }
            ClosedTabBundle::SessionWorkspace {
                tabs,
                sftp_tabs,
                workspace,
            } => {
                if tabs.is_empty() {
                    self.shell.status_message =
                        i18n::string("session.messages.no_recently_closed_tab");
                    cx.notify();
                    return;
                }

                if tabs
                    .iter()
                    .map(|closed_tab| &closed_tab.profile)
                    .chain(sftp_tabs.iter().map(|closed_tab| &closed_tab.profile))
                    .any(|profile| self.profile_requires_local_vault_unlock(profile, cx))
                {
                    self.workspace
                        .recently_closed_tabs
                        .push(ClosedTabBundle::SessionWorkspace {
                            tabs,
                            sftp_tabs,
                            workspace,
                        });
                    self.prompt_local_vault_unlock_in_window(window, cx);
                    return;
                }

                self.unload_active_topbar_workspace(cx);

                let mut reopened_indices = Vec::with_capacity(tabs.len());
                let mut reopened_ids = HashMap::with_capacity(tabs.len());
                for closed_tab in tabs {
                    let (mut tab, session) = self.build_session_tab(closed_tab.profile, cx);
                    tab.placement = if closed_tab.hidden_from_topbar {
                        TabPlacement::Background
                    } else {
                        TabPlacement::TopLevel
                    };
                    let reopened_id = tab.id;
                    self.insert_session_tab(tab, session, cx);
                    reopened_indices.push(self.workspace.tabs.len() - 1);
                    reopened_ids.insert(closed_tab.tab_id, reopened_id);
                }

                for closed_tab in sftp_tabs {
                    let Some(owner) = reopened_sftp_owner(&closed_tab, &reopened_ids) else {
                        continue;
                    };
                    let reopened_id = self.workspace.allocate_tab_id();
                    let mut tab = TabState::new_sftp(reopened_id, &closed_tab.profile);
                    tab.placement = TabPlacement::SessionSidecar { owner };
                    let start_result = self.controllers.sftp.update(cx, |controller, cx| {
                        controller.start_tab(
                            reopened_id,
                            closed_tab.profile.clone(),
                            Some(owner),
                            cx,
                        )
                    });
                    if let Err(error) = start_result {
                        log::warn!("failed to reopen session SFTP sidecar: {error}");
                        continue;
                    }
                    self.insert_sftp_tab(tab, cx);
                    reopened_ids.insert(closed_tab.tab_id, reopened_id);
                }

                let Some(visible_index) = reopened_indices
                    .iter()
                    .copied()
                    .find(|&tab_index| {
                        self.workspace
                            .tabs
                            .at(tab_index)
                            .is_some_and(|tab| tab.is_top_level())
                    })
                    .or_else(|| reopened_indices.first().copied())
                else {
                    self.shell.status_message =
                        i18n::string("session.messages.no_recently_closed_tab");
                    cx.notify();
                    return;
                };

                if let Some(workspace) = workspace {
                    let workspace = self.restore_closed_workspace(workspace, &reopened_ids);
                    if let Some(owner) = self.workspace.tabs.id_at(visible_index) {
                        self.workspace.sync_workspace_placements(owner, &workspace);
                        self.workspace.park_workspace(owner, workspace);
                    }
                }

                self.workspace.active_topbar_tab = self.workspace.tabs.id_at(visible_index);
                self.load_topbar_workspace(visible_index, cx);
                self.rebind_terminal_focus_reporting(window, cx);
                self.shell.shell_state.sidebar_section = SidebarSection::Hosts;
                self.controllers
                    .session
                    .read(cx)
                    .set_host_editor_state(false, false);
                let title = self
                    .workspace
                    .tabs
                    .at(visible_index)
                    .expect("reopened tab remains registered")
                    .title
                    .clone();
                self.shell.status_message =
                    i18n::string_args("session.messages.reopened_tab", &[("title", &title)]);
                window.focus(&self.workspace.workspace.active_pane.terminal_focus, cx);
                self.sync_terminal_focus_reporting(window, cx);
                cx.notify();
            }
        }
    }

    pub(in crate::ui::shell) fn handle_global_shortcut(
        &mut self,
        keystroke: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let bindings = self
            .controllers
            .settings
            .read(cx)
            .settings()
            .key_bindings
            .clone();

        if bindings.next_tab.matches_keystroke(keystroke) {
            self.activate_next_topbar_tab(window, cx);
            return true;
        }
        if bindings.close_tab.matches_keystroke(keystroke) {
            self.close_active_topbar_tab_shortcut(window, cx);
            return true;
        }
        if bindings.reopen_tab.matches_keystroke(keystroke) {
            self.reopen_last_closed_tab(window, cx);
            return true;
        }
        if bindings.open_settings.matches_keystroke(keystroke) {
            self.set_sidebar_section(SidebarSection::Settings, cx);
            return true;
        }

        false
    }

    pub(in crate::ui::shell) fn close_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.workspace.tabs.len() {
            return;
        }

        let closed_bundle = self.build_closed_tab_bundle(index, cx);
        let tab = self
            .workspace
            .tabs
            .at(index)
            .expect("validated close index remains registered");
        let root_tab_id = tab.id;
        let title = tab.title.clone();
        let Some(close_plan) = self.close_plan_for_index(index) else {
            return;
        };
        debug_assert_eq!(close_plan.root(), root_tab_id);
        let mut affected_monitor_profiles = HashSet::new();
        let sftp_controller = self.controllers.sftp.clone();

        for step in close_plan.resource_first_steps() {
            match step {
                ClosePlanStep::Retire(tab_id) => {
                    if let Some((purpose, profile_id)) = self
                        .controllers
                        .session
                        .read(cx)
                        .retire_tab_resources(tab_id)
                    {
                        if purpose == SessionPurpose::Terminal {
                            affected_monitor_profiles.insert(profile_id);
                        }
                    } else if let Some(sftp) = sftp_controller.read(cx).remove_tab_state(tab_id)
                        && let Some(commands) = sftp.commands.as_ref()
                        && let Err(error) = commands.close()
                    {
                        log::debug!("failed to close SFTP tab {} cleanly: {error:?}", tab_id);
                    }
                }
                ClosePlanStep::Commit(tab_id) => {
                    let _ = self.remove_tab_payload_and_metadata(tab_id, cx);
                }
            }
        }

        self.prune_closed_tab_references();
        for profile_id in affected_monitor_profiles {
            self.refresh_profile_monitoring(&profile_id, None, cx);
        }

        if let Some(bundle) = closed_bundle {
            self.push_recently_closed_tab(bundle);
        }

        if self.workspace.active_topbar_tab.is_none() {
            if let Some(next_index) =
                self.nearest_visible_tab(index.min(self.workspace.tabs.len().saturating_sub(1)))
            {
                self.workspace.active_topbar_tab = self.workspace.tabs.id_at(next_index);
                if self
                    .workspace
                    .tabs
                    .at(next_index)
                    .is_some_and(|tab| tab.is_session())
                {
                    self.load_topbar_workspace(next_index, cx);
                } else {
                    self.reset_loaded_workspace(cx);
                }
            } else {
                self.reset_loaded_workspace(cx);
            }
        }

        if let Some(active_session_tab_id) = self.workspace.workspace.active_tab {
            self.controllers
                .session
                .read(cx)
                .clear_tab_activity(active_session_tab_id);
        }

        if self.workspace.tabs.is_empty() {
            self.workspace.active_topbar_tab = None;
            self.workspace.workspace.active_tab = None;
            match last_tab_close_action(
                self.controllers
                    .settings
                    .read(cx)
                    .settings()
                    .last_tab_close_behavior,
            ) {
                LastTabCloseAction::Quit => {
                    cx.quit();
                }
                LastTabCloseAction::OpenHome => {
                    self.open_hosts_tab(cx);
                    self.rebind_terminal_focus_reporting(window, cx);
                    self.sync_terminal_focus_reporting(window, cx);
                }
            }
            return;
        }

        self.rebind_terminal_focus_reporting(window, cx);
        self.sync_terminal_focus_reporting(window, cx);
        self.sync_session_port_snapshot(cx);
        self.shell.status_message =
            i18n::string_args("session.messages.closed_tab", &[("title", &title)]);
        cx.notify();
    }

    pub(in crate::ui::shell) fn activate_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target_tab) = self.workspace.tabs.at(index) else {
            return;
        };
        if !target_tab.is_top_level() {
            return;
        }
        let target_tab_id = target_tab.id;
        let target_is_session = target_tab.is_session();
        let target_is_sftp = target_tab.is_sftp();

        let previous_active_tab_id = self.workspace.active_topbar_tab;
        let previous_active_tab =
            previous_active_tab_id.and_then(|tab_id| self.workspace.tabs.get(tab_id));
        let previous_active_is_hosts = previous_active_tab.is_some_and(|tab| tab.is_hosts())
            && self.shell.shell_state.sidebar_section == SidebarSection::Hosts;
        let target_is_terminal = self.workspace.tabs.at(index).is_some_and(|tab| {
            self.controllers.session.read(cx).tab_purpose(tab.id) == Some(SessionPurpose::Terminal)
        });
        let preserve_host_editor_sidebar = previous_active_is_hosts
            && target_is_terminal
            && self
                .controllers
                .session
                .read(cx)
                .editor_state()
                .host_editor_open;

        if self.workspace.active_topbar_tab != self.workspace.tabs.id_at(index) {
            self.unload_active_topbar_workspace(cx);
            self.workspace.active_topbar_tab = self.workspace.tabs.id_at(index);
            if target_is_session {
                self.load_topbar_workspace(index, cx);
            } else {
                self.reset_loaded_workspace(cx);
            }
            self.rebind_terminal_focus_reporting(window, cx);
        }

        self.shell.shell_state.sidebar_section = SidebarSection::Hosts;

        if !preserve_host_editor_sidebar {
            self.controllers
                .session
                .read(cx)
                .set_host_editor_state(false, false);
        }

        if let Some(active_session_tab_id) = self.workspace.workspace.active_tab {
            self.controllers
                .session
                .read(cx)
                .clear_tab_activity(active_session_tab_id);
        }

        if target_is_session {
            window.focus(&self.workspace.workspace.active_pane.terminal_focus, cx);
        }

        if target_is_sftp {
            self.controllers.sftp.read(cx).reset_path_editing();
            self.sync_active_sftp_path_inputs(cx);
            self.sync_active_sftp_tables(cx);
        }
        if self.controllers.session.read(cx).side_panel_open()
            && self.controllers.session.read(cx).side_panel_view() == SessionSidePanelView::Sftp
            && target_is_session
        {
            self.ensure_session_side_panel_sftp_tab(target_tab_id, cx);
        }

        self.sync_terminal_focus_reporting(window, cx);
        self.sync_session_port_snapshot(cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn sync_session_terminal_size_from_metrics(
        &mut self,
        index: usize,
        bounds: Bounds<Pixels>,
        cell_width: f32,
        line_height: f32,
        allow_pending_start: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let columns = (f32::from(bounds.size.width) / cell_width.max(1.0))
            .floor()
            .max(miaominal_terminal::MIN_TERMINAL_COLUMNS as f32) as usize;
        let lines = (f32::from(bounds.size.height) / line_height.max(1.0))
            .floor()
            .max(1.0) as usize;

        self.sync_session_terminal_size(index, columns, lines, true, allow_pending_start, cx)
    }

    fn sync_session_terminal_size(
        &mut self,
        index: usize,
        columns: usize,
        lines: usize,
        bounds_known: bool,
        allow_pending_start: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if index >= self.workspace.tabs.len() {
            return false;
        }
        let Some(tab_id) = self.workspace.tabs.id_at(index) else {
            return false;
        };
        let (size_changed, events) = self
            .controllers
            .session
            .read(cx)
            .resize_terminal_for_viewport(
                tab_id,
                columns,
                lines,
                bounds_known,
                allow_pending_start,
            );
        if let Some(events) = events {
            self.controllers.session.update(cx, |controller, cx| {
                controller.spawn_session_event_loop(tab_id, events, cx);
            });
        }
        size_changed
    }

    pub(in crate::ui::shell) fn handle_session_event_outcome(
        &mut self,
        tab_id: TabId,
        outcome: SessionEventOutcome,
        cx: &mut Context<Self>,
    ) {
        let SessionEventOutcome {
            tab_status,
            clipboard_writes,
            notification,
            removal,
            schedule_reconnect_error,
            refresh_monitoring_profile,
            mut should_notify,
        } = outcome;

        if let Some(status) = tab_status
            && let Some(mut tab) = self.workspace.tabs.get_mut(tab_id)
        {
            tab.status = status;
        }

        let _ = refresh_monitoring_profile;

        for content in clipboard_writes {
            cx.write_to_clipboard(ClipboardItem::new_string(content));
            self.shell.status_message = i18n::string("session.messages.clipboard_osc52");
            should_notify = true;
        }

        if let Some(notification) = notification {
            let notification = match notification.tone {
                SessionNotificationTone::Success => {
                    success_notification(notification.title, notification.message)
                }
                SessionNotificationTone::Error => {
                    error_notification(notification.title, notification.message)
                }
            }
            .id1::<AppView>(SharedString::from(notification.id));
            self.with_active_window(cx, move |window, cx| {
                window.push_notification(notification, cx);
            });
        }

        let _ = schedule_reconnect_error;

        match removal {
            Some(SessionEventTabRemoval::PortForward { status_message, .. }) => {
                let _ = self.remove_tab_metadata_after_controller_close(tab_id, cx);
                self.prune_closed_tab_references();
                self.shell.status_message = status_message;
                cx.notify();
                return;
            }
            Some(SessionEventTabRemoval::ConnectionTest { status_message }) => {
                let _ = self.remove_tab_metadata_after_controller_close(tab_id, cx);
                self.prune_closed_tab_references();
                self.shell.status_message = status_message;
                cx.notify();
                return;
            }
            None => {}
        }

        if should_notify {
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn close_other_tabs(
        &mut self,
        keep_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if keep_index >= self.workspace.tabs.len() {
            return;
        }

        let kept_id = self
            .workspace
            .tabs
            .id_at(keep_index)
            .expect("validated kept tab remains registered");
        let mut keep_indices = self.owned_tab_indices_for_topbar(keep_index);
        keep_indices.sort_unstable();
        keep_indices.dedup();

        let remove_indices: Vec<usize> = (0..self.workspace.tabs.len())
            .filter(|index| keep_indices.binary_search(index).is_err())
            .collect();
        let remove_ids = remove_indices
            .iter()
            .filter_map(|index| self.workspace.tabs.id_at(*index))
            .collect::<Vec<_>>();
        let mut affected_monitor_profiles = HashSet::new();
        let sftp_controller = self.controllers.sftp.clone();

        for tab_id in remove_ids.iter().copied() {
            if let Some((purpose, profile_id)) = self
                .controllers
                .session
                .read(cx)
                .retire_tab_resources(tab_id)
            {
                if purpose == SessionPurpose::Terminal {
                    affected_monitor_profiles.insert(profile_id);
                }
            } else if let Some(sftp) = sftp_controller.read(cx).remove_tab_state(tab_id)
                && let Some(commands) = sftp.commands.as_ref()
                && let Err(error) = commands.close()
            {
                log::debug!("failed to close SFTP tab {} cleanly: {error:?}", tab_id);
            }
        }
        for tab_id in remove_ids.into_iter().rev() {
            let _ = self.remove_tab_payload_and_metadata(tab_id, cx);
        }

        self.prune_closed_tab_references();
        for profile_id in affected_monitor_profiles {
            self.refresh_profile_monitoring(&profile_id, None, cx);
        }

        if let Some(new_index) = self.workspace.tabs.index_of(kept_id) {
            let should_load =
                self.workspace.active_topbar_tab != self.workspace.tabs.id_at(new_index);
            self.workspace.active_topbar_tab = self.workspace.tabs.id_at(new_index);
            if self
                .workspace
                .tabs
                .at(new_index)
                .is_some_and(|tab| tab.is_session())
            {
                if should_load {
                    self.load_topbar_workspace(new_index, cx);
                }
            } else {
                self.reset_loaded_workspace(cx);
            }
        } else {
            self.workspace.active_topbar_tab = None;
            self.reset_loaded_workspace(cx);
        }

        self.workspace.renaming_tab = None;
        self.rebind_terminal_focus_reporting(window, cx);
        self.sync_terminal_focus_reporting(window, cx);
        self.shell.status_message = i18n::string("session.messages.closed_other_tabs");
        cx.notify();
    }

    pub(in crate::ui::shell) fn duplicate_profile_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile = self.workspace.tabs.at(index).and_then(|tab| {
            self.controllers
                .session
                .read(cx)
                .resolved_profile_for_tab(tab.id)
        });

        let Some(profile) = profile else {
            self.shell.status_message =
                i18n::string("session.messages.source_profile_not_found_for_tab");
            cx.notify();
            return;
        };

        self.open_session_tab(profile, window, cx);
        self.shell.status_message = i18n::string("session.messages.opened_same_profile_tab");
    }

    pub(in crate::ui::shell) fn begin_rename_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((tab_id, title)) = self
            .workspace
            .tabs
            .at(index)
            .map(|tab| (tab.id, tab.title.clone()))
        else {
            return;
        };
        self.workspace.renaming_tab = Some(tab_id);
        set_input_value(&self.shell.workspace_forms.rename_input, title, window, cx);
        let rename_input = self.shell.workspace_forms.rename_input.clone();
        rename_input.update(cx, |input, cx| {
            input.focus(window, cx);
        });
        cx.notify();
        cx.on_next_frame(window, move |this, window, cx| {
            if this.workspace.renaming_tab != Some(tab_id) {
                return;
            }

            let rename_input = this.shell.workspace_forms.rename_input.clone();
            rename_input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
            window.dispatch_action(Box::new(gpui_component::input::SelectAll), cx);
        });
    }

    pub(in crate::ui::shell) fn commit_rename_tab(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.workspace.renaming_tab else {
            return;
        };
        let new_title = self
            .shell
            .workspace_forms
            .rename_input
            .read(cx)
            .value()
            .to_string();
        let trimmed = new_title.trim();
        if !trimmed.is_empty() {
            let excluding_index = self.workspace.tabs.index_of(tab_id);
            let title = self.unique_topbar_tab_title(trimmed, excluding_index);
            if let Some(mut tab) = self.workspace.tabs.get_mut(tab_id) {
                tab.title = title;
            }
        }
        self.workspace.renaming_tab = None;
        self.sync_session_port_snapshot(cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn cancel_rename_tab(&mut self, cx: &mut Context<Self>) {
        if self.workspace.renaming_tab.is_some() {
            self.workspace.renaming_tab = None;
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn reorder_tab(
        &mut self,
        from: usize,
        to: usize,
        cx: &mut Context<Self>,
    ) {
        if from >= self.workspace.tabs.len() {
            return;
        }
        let to = to.min(self.workspace.tabs.len().saturating_sub(1));
        if from == to {
            return;
        }

        let dest = to.min(self.workspace.tabs.len());
        self.workspace.tabs.move_to(from, dest);

        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_terminal_output_only_notifies_for_new_activity() {
        assert!(should_notify_for_session_output(true, false));
        assert!(!should_notify_for_session_output(true, true));
        assert!(should_notify_for_session_output(false, true));
    }

    #[test]
    fn last_tab_close_behavior_maps_to_quit_or_home() {
        assert_eq!(
            last_tab_close_action(miaominal_settings::LastTabCloseBehavior::ExitApplication),
            LastTabCloseAction::Quit
        );
        assert_eq!(
            last_tab_close_action(miaominal_settings::LastTabCloseBehavior::OpenNewHomeTab),
            LastTabCloseAction::OpenHome
        );
    }

    #[test]
    fn session_sftp_sidecar_owner_is_remapped_to_reopened_session() {
        let old_owner = TabId::new(10);
        let new_owner = TabId::new(20);
        let closed = ClosedSftpTabState {
            tab_id: TabId::new(11),
            owner: old_owner,
            profile: SessionProfile::blank("profile", 1),
        };
        let reopened = HashMap::from([(old_owner, new_owner)]);

        assert_eq!(reopened_sftp_owner(&closed, &reopened), Some(new_owner));
        assert_eq!(closed.tab_id, TabId::new(11));
        assert_eq!(closed.profile.id, "profile");
    }
}
