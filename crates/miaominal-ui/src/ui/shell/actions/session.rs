use super::super::*;
use crate::ui::i18n;
use crate::ui::shell::state::SessionFailureStatus;
use gpui_component::WindowExt as _;
use miaominal_services::TerminalService;

const TERMINAL_OUTPUT_BATCH_MAX_CHUNKS: usize = 64;
const TERMINAL_OUTPUT_BATCH_MAX_BYTES: usize = 256 * 1024;

fn coalesce_session_output(
    mut chunk: Vec<u8>,
    events: &mut SessionEventReceiver,
) -> (Vec<u8>, Option<SessionEvent>) {
    let mut chunks = 1usize;

    while chunks < TERMINAL_OUTPUT_BATCH_MAX_CHUNKS && chunk.len() < TERMINAL_OUTPUT_BATCH_MAX_BYTES
    {
        match events.try_recv() {
            Ok(SessionEvent::Output(next)) => {
                chunk.extend_from_slice(&next);
                chunks += 1;
            }
            Ok(event) => return (chunk, Some(event)),
            Err(_) => break,
        }
    }

    (chunk, None)
}

impl AppView {
    fn terminal_service(&self) -> TerminalService {
        TerminalService::new(
            self.services.runtime.clone(),
            self.services.secrets.clone(),
            self.services.known_hosts.clone(),
        )
    }

    pub(in crate::ui::shell) fn profile_requires_local_vault_unlock(
        &self,
        profile: &SessionProfile,
    ) -> bool {
        if self.local_vault_status != LocalVaultStatus::Locked {
            return false;
        }

        match profile.effective_auth_method() {
            AuthMethod::Password => profile.password.is_empty() && profile.has_stored_password,
            AuthMethod::KeyFile => profile.passphrase.is_empty() && profile.has_stored_passphrase,
            AuthMethod::ManagedKey => !profile.managed_key_id.trim().is_empty(),
            AuthMethod::Agent | AuthMethod::KeyboardInteractive => false,
        }
    }

    pub(in crate::ui::shell) fn prompt_local_vault_unlock(&mut self, cx: &mut Context<Self>) {
        self.status_message = i18n::string("settings.sync.vault.access_required_error.message");
        self.pending_local_vault_unlock_action = None;
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
        self.status_message = i18n::string("settings.sync.vault.access_required_error.message");
        self.pending_local_vault_unlock_action = None;
        self.open_local_vault_passphrase_popup(
            LocalVaultPassphrasePopupMode::PrimaryAction,
            window,
            cx,
        );
    }

    pub(in crate::ui::shell) fn prompt_local_vault_unlock_for_action(
        &mut self,
        action: PendingLocalVaultUnlockAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.status_message = i18n::string("settings.sync.vault.access_required_error.message");
        self.pending_local_vault_unlock_action = Some(action);
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
            PendingLocalVaultUnlockAction::OpenSession(Box::new(profile)),
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
        if self.profile_requires_local_vault_unlock(&profile) {
            self.prompt_local_vault_unlock_in_window(window, cx);
            return;
        }

        let remove_indices: Vec<usize> = self
            .workspace_state
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| {
                tab.as_session()
                    .filter(|session| session.purpose == SessionPurpose::ConnectionTest)
                    .map(|_| index)
            })
            .collect();

        for remove_index in remove_indices.iter().rev().copied() {
            let Some(session) = self
                .workspace_state
                .tabs
                .get(remove_index)
                .and_then(TabState::as_session)
            else {
                continue;
            };
            if let Some(commands) = session.commands.as_ref() {
                let _ = commands.close();
            }
            self.workspace_state.tabs.remove(remove_index);
        }
        if !remove_indices.is_empty() {
            self.remap_all_tab_indices_after_removal(&remove_indices);
        }

        let (columns, lines) = self.estimated_terminal_size(window);
        let connection = self.terminal_service().start_session(
            profile.clone(),
            self.data.sessions.clone(),
            columns,
            lines,
            false,
        );

        let tab_id = {
            let next_id = self.workspace_state.next_tab_id;
            self.workspace_state.next_tab_id += 1;
            next_id
        };

        self.workspace_state
            .tabs
            .push(TabState::new_connection_test(
                tab_id,
                &profile,
                connection.commands,
            ));
        self.spawn_session_event_loop(tab_id, connection.events, cx);
        let profile_summary = profile.summary();
        self.status_message = i18n::string_args(
            "session.messages.testing_connection_to",
            &[("profile", &profile_summary)],
        );
        cx.notify();
    }

    pub(in crate::ui::shell) fn shared_monitoring_state_for_profile<'a>(
        &'a self,
        profile_id: &str,
        fallback: &'a SessionMonitoringState,
    ) -> &'a SessionMonitoringState {
        self.workspace_state
            .shared_profile_monitoring
            .get(profile_id)
            .unwrap_or(fallback)
    }

    fn ensure_shared_monitoring_state(
        &mut self,
        profile_id: &str,
        enabled: bool,
    ) -> &mut SessionMonitoringState {
        self.workspace_state
            .shared_profile_monitoring
            .entry(profile_id.to_string())
            .or_insert_with(|| SessionMonitoringState::new(enabled))
    }

    fn monitoring_enabled_for_profile(&self, profile_id: &str) -> bool {
        self.workspace_state
            .shared_profile_monitoring
            .get(profile_id)
            .map(|state| state.auto_collect_enabled)
            .or_else(|| {
                self.workspace_state.tabs.iter().find_map(|tab| {
                    let session = tab.as_session()?;
                    (session.purpose == SessionPurpose::Terminal
                        && session.profile_id == profile_id)
                        .then_some(session.monitoring.auto_collect_enabled)
                })
            })
            .unwrap_or(
                self.settings_store
                    .settings()
                    .auto_collect_session_monitoring,
            )
    }

    fn session_commands_by_tab_id(&self, tab_id: usize) -> Option<SessionCommandSender> {
        self.workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_session)
            .and_then(|session| session.commands.clone())
    }

    fn can_session_source_monitoring(session: &SessionTabState) -> bool {
        session.purpose == SessionPurpose::Terminal
            && session.commands.is_some()
            && matches!(
                session.connection_state,
                SessionConnectionState::Connecting | SessionConnectionState::Ready
            )
    }

    fn current_monitor_source_tab_id(
        &self,
        profile_id: &str,
        excluded_tab_id: Option<usize>,
    ) -> Option<usize> {
        let source_tab_id = self
            .workspace_state
            .monitor_source_tabs
            .get(profile_id)
            .copied()?;
        if Some(source_tab_id) == excluded_tab_id {
            return None;
        }

        self.workspace_state.tabs.iter().find_map(|tab| {
            let session = tab.as_session()?;
            (tab.id == source_tab_id
                && session.profile_id == profile_id
                && Self::can_session_source_monitoring(session))
            .then_some(source_tab_id)
        })
    }

    fn next_monitor_source_tab_id(
        &self,
        profile_id: &str,
        excluded_tab_id: Option<usize>,
    ) -> Option<usize> {
        self.workspace_state.tabs.iter().find_map(|tab| {
            if Some(tab.id) == excluded_tab_id {
                return None;
            }

            let session = tab.as_session()?;
            (session.profile_id == profile_id && Self::can_session_source_monitoring(session))
                .then_some(tab.id)
        })
    }

    fn claim_profile_monitor_source(
        &mut self,
        profile_id: &str,
        tab_id: usize,
        enabled: bool,
    ) -> bool {
        self.ensure_shared_monitoring_state(profile_id, enabled)
            .set_enabled(enabled);

        if !self.monitoring_enabled_for_profile(profile_id) {
            self.workspace_state.monitor_source_tabs.remove(profile_id);
            return false;
        }

        if let Some(source_tab_id) = self.current_monitor_source_tab_id(profile_id, None) {
            return source_tab_id == tab_id;
        }

        self.workspace_state
            .monitor_source_tabs
            .insert(profile_id.to_string(), tab_id);
        true
    }

    pub(in crate::ui::shell) fn set_profile_monitoring_enabled(
        &mut self,
        profile_id: &str,
        enabled: bool,
    ) -> Result<bool, String> {
        let current_source = self.current_monitor_source_tab_id(profile_id, None);

        for tab in &mut self.workspace_state.tabs {
            let Some(session) = tab.as_session_mut() else {
                continue;
            };
            if session.purpose != SessionPurpose::Terminal || session.profile_id != profile_id {
                continue;
            }

            session.monitoring.set_enabled(enabled);
        }

        self.ensure_shared_monitoring_state(profile_id, enabled)
            .set_enabled(enabled);

        if !enabled {
            self.workspace_state.monitor_source_tabs.remove(profile_id);
            if let Some(source_tab_id) = current_source
                && let Some(commands) = self.session_commands_by_tab_id(source_tab_id)
            {
                commands
                    .set_monitoring_enabled(false)
                    .map_err(|error| error.to_string())?;
                return Ok(true);
            }
            return Ok(false);
        }

        let Some(source_tab_id) =
            current_source.or_else(|| self.next_monitor_source_tab_id(profile_id, None))
        else {
            self.workspace_state.monitor_source_tabs.remove(profile_id);
            return Ok(false);
        };

        self.workspace_state
            .monitor_source_tabs
            .insert(profile_id.to_string(), source_tab_id);
        if let Some(commands) = self.session_commands_by_tab_id(source_tab_id) {
            commands
                .set_monitoring_enabled(true)
                .map_err(|error| error.to_string())?;
            return Ok(true);
        }

        Ok(false)
    }

    fn apply_profile_monitor_snapshot(
        &mut self,
        profile_id: &str,
        tab_id: usize,
        enabled: bool,
        snapshot: SessionMonitorSnapshot,
    ) {
        if let Some(source_tab_id) = self.current_monitor_source_tab_id(profile_id, None) {
            if source_tab_id != tab_id {
                return;
            }
        } else if enabled {
            self.workspace_state
                .monitor_source_tabs
                .insert(profile_id.to_string(), tab_id);
        }

        self.ensure_shared_monitoring_state(profile_id, enabled)
            .apply_snapshot(snapshot);
    }

    fn apply_profile_monitor_error(
        &mut self,
        profile_id: &str,
        tab_id: usize,
        enabled: bool,
        error: String,
    ) {
        if let Some(source_tab_id) = self.current_monitor_source_tab_id(profile_id, None) {
            if source_tab_id != tab_id {
                return;
            }
        } else if enabled {
            self.workspace_state
                .monitor_source_tabs
                .insert(profile_id.to_string(), tab_id);
        }

        self.ensure_shared_monitoring_state(profile_id, enabled)
            .report_error(error);
    }

    pub(in crate::ui::shell) fn refresh_profile_monitoring(
        &mut self,
        profile_id: &str,
        excluded_tab_id: Option<usize>,
    ) {
        let has_terminal_tabs = self.workspace_state.tabs.iter().any(|tab| {
            tab.as_session().is_some_and(|session| {
                session.purpose == SessionPurpose::Terminal && session.profile_id == profile_id
            })
        });
        if !has_terminal_tabs {
            self.workspace_state
                .shared_profile_monitoring
                .remove(profile_id);
            self.workspace_state.monitor_source_tabs.remove(profile_id);
            return;
        }

        if !self.monitoring_enabled_for_profile(profile_id) {
            self.workspace_state.monitor_source_tabs.remove(profile_id);
            return;
        }

        if let Some(source_tab_id) = self.current_monitor_source_tab_id(profile_id, excluded_tab_id)
        {
            self.workspace_state
                .monitor_source_tabs
                .insert(profile_id.to_string(), source_tab_id);
            return;
        }

        let Some(source_tab_id) = self.next_monitor_source_tab_id(profile_id, excluded_tab_id)
        else {
            self.workspace_state.monitor_source_tabs.remove(profile_id);
            return;
        };

        self.workspace_state
            .monitor_source_tabs
            .insert(profile_id.to_string(), source_tab_id);
        if let Some(commands) = self.session_commands_by_tab_id(source_tab_id)
            && let Err(error) = commands.set_monitoring_enabled(true)
        {
            let message = error.to_string();
            self.ensure_shared_monitoring_state(profile_id, true)
                .report_error(message.clone());
            self.workspace_state.monitor_source_tabs.remove(profile_id);
            log::debug!("failed to promote session monitoring source: {message}");
        }
    }

    pub(in crate::ui::shell) fn build_session_tab(&mut self, profile: SessionProfile) -> TabState {
        let auto_collect_monitoring = self
            .workspace_state
            .shared_profile_monitoring
            .get(&profile.id)
            .map(|state| state.auto_collect_enabled)
            .unwrap_or(
                self.settings_store
                    .settings()
                    .auto_collect_session_monitoring,
            );

        let terminal = TerminalState::default();

        let tab_id = {
            let next_id = self.workspace_state.next_tab_id;
            self.workspace_state.next_tab_id += 1;
            next_id
        };

        TabState::new_session_pending(tab_id, profile, terminal, auto_collect_monitoring)
    }

    pub(in crate::ui::shell) fn enable_active_session_monitoring(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let Some(profile_id) = self
            .workspace_state
            .workspace
            .active_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(TabState::as_session)
            .filter(|session| session.purpose == SessionPurpose::Terminal)
            .map(|session| session.profile_id.clone())
        else {
            return;
        };

        self.status_message = match self.set_profile_monitoring_enabled(&profile_id, true) {
            Ok(true) => i18n::string("status.session_monitoring_started"),
            Ok(false) => i18n::string("status.session_monitoring_pending"),
            Err(message) => {
                self.ensure_shared_monitoring_state(&profile_id, true)
                    .report_error(message.clone());
                i18n::string_args(
                    "status.session_monitoring_start_failed",
                    &[("error", &message)],
                )
            }
        };
        cx.notify();
    }

    fn estimated_terminal_size(&self, window: &Window) -> (usize, usize) {
        let cell_width = terminal_cell_width(window);
        let line_height = terminal_line_height(window);

        let (available_width, available_height) = if let Some(bounds) =
            self.workspace_state.workspace.active_pane.terminal_bounds
        {
            (bounds.size.width, bounds.size.height)
        } else {
            let win = window.bounds().size;
            let width = (win.width - px(TERMINAL_PANEL_BORDER)).max(px(cell_width * 2.0));
            let height = (win.height - px(TOP_BAR_HEIGHT + FOOTER_HEIGHT + TERMINAL_PANEL_BORDER))
                .max(px(line_height));
            (width, height)
        };

        let columns = (f32::from(available_width) / cell_width)
            .floor()
            .max(miaominal_terminal::MIN_TERMINAL_COLUMNS as f32) as usize;
        let lines = (f32::from(available_height) / line_height).floor().max(1.0) as usize;
        (columns, lines)
    }

    pub(in crate::ui::shell) fn connect_profile(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = self.data.sessions.get(index).cloned() else {
            return;
        };

        self.data.selected_profile = Some(index);
        self.open_session_tab(profile, window, cx);
    }

    pub(in crate::ui::shell) fn open_session_tab(
        &mut self,
        profile: SessionProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.profile_requires_local_vault_unlock(&profile) {
            self.prompt_local_vault_unlock_for_profile(profile, window, cx);
            return;
        }

        let should_animate_hosts_to_terminal = self
            .workspace_state
            .active_topbar_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .is_some_and(TabState::is_hosts)
            && self.panel_view.sidebar_section == SidebarSection::Hosts;
        let preserve_host_editor_sidebar =
            should_animate_hosts_to_terminal && self.editors.host_editor_open;
        let replace_index = self.workspace_state.active_topbar_tab.filter(|&index| {
            self.workspace_state
                .tabs
                .get(index)
                .is_some_and(|tab| !tab.hidden_from_topbar && tab.is_hosts())
        });
        let mut tab = self.build_session_tab(profile.clone());
        tab.title = self.unique_topbar_tab_title(&tab.title, None);

        self.unload_active_topbar_workspace(cx);

        let index = if let Some(index) = replace_index {
            self.workspace_state.tabs[index] = tab;
            index
        } else {
            self.workspace_state.tabs.push(tab);
            self.workspace_state.tabs.len() - 1
        };

        self.workspace_state.active_topbar_tab = Some(index);
        self.load_topbar_workspace(index, cx);
        self.rebind_terminal_focus_reporting(window, cx);

        self.panel_view.sidebar_section = SidebarSection::Hosts;
        if !preserve_host_editor_sidebar {
            self.editors.host_editor_open = false;
            self.editors.host_editor_is_new = false;
        }
        self.status_message = i18n::string_args(
            "session.messages.opening_tab_for",
            &[("profile", &profile.name)],
        );
        window.focus(
            &self.workspace_state.workspace.active_pane.terminal_focus,
            cx,
        );
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
            .workspace_state
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
        tab_index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile = tab_index
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(TabState::as_session)
            .and_then(|session| {
                self.data
                    .sessions
                    .iter()
                    .find(|profile| profile.id == session.profile_id)
                    .cloned()
                    .or_else(|| session.pending_profile.clone())
            });

        let Some(profile) = profile else {
            self.status_message = i18n::string("session.messages.open_sftp_requires_active_ssh");
            cx.notify();
            return;
        };

        self.open_sftp_tab(profile, window, cx);
    }

    fn profile_for_session_reopen(
        &self,
        profile_id: &str,
        pending_profile: &Option<SessionProfile>,
    ) -> Option<SessionProfile> {
        pending_profile.clone().or_else(|| {
            self.data
                .sessions
                .iter()
                .find(|profile| profile.id == profile_id)
                .cloned()
        })
    }

    fn push_recently_closed_tab(&mut self, bundle: ClosedTabBundle) {
        const MAX_RECENTLY_CLOSED_TABS: usize = 20;

        self.workspace_state.recently_closed_tabs.push(bundle);
        if self.workspace_state.recently_closed_tabs.len() > MAX_RECENTLY_CLOSED_TABS {
            let overflow =
                self.workspace_state.recently_closed_tabs.len() - MAX_RECENTLY_CLOSED_TABS;
            self.workspace_state.recently_closed_tabs.drain(0..overflow);
        }
    }

    fn normalize_closed_workspace_slot(slot: &mut Option<usize>, removed: &[usize]) {
        let Some(global_index) = *slot else {
            return;
        };
        *slot = removed.binary_search(&global_index).ok();
    }

    fn normalize_closed_workspace(
        &self,
        mut workspace: TabWorkspaceState,
        removed: &[usize],
    ) -> TabWorkspaceState {
        Self::normalize_closed_workspace_slot(&mut workspace.active_tab, removed);
        for parked in workspace.parked_panes.values_mut() {
            Self::normalize_closed_workspace_slot(&mut parked.active_tab, removed);
        }
        workspace
    }

    fn restore_closed_workspace_slot(slot: &mut Option<usize>, reopened: &[usize]) {
        let Some(local_index) = *slot else {
            return;
        };
        *slot = reopened.get(local_index).copied();
    }

    fn restore_closed_workspace(
        &self,
        mut workspace: TabWorkspaceState,
        reopened: &[usize],
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
        let tab = self.workspace_state.tabs.get(index)?;
        if tab.hidden_from_topbar {
            return None;
        }

        match &tab.kind {
            TabKind::Hosts => Some(ClosedTabBundle::Hosts),
            TabKind::Sftp(sftp) => {
                let profile = self
                    .data
                    .sessions
                    .iter()
                    .find(|profile| profile.id == sftp.profile_id)
                    .cloned()?;
                Some(ClosedTabBundle::Sftp { profile })
            }
            TabKind::Session(_) => {
                let mut removed = self.owned_tab_indices_for_topbar(index);
                removed.sort_unstable();
                removed.dedup();
                let session_removed = removed
                    .iter()
                    .copied()
                    .filter(|&remove_index| {
                        self.workspace_state
                            .tabs
                            .get(remove_index)
                            .and_then(TabState::as_session)
                            .is_some()
                    })
                    .collect::<Vec<_>>();

                let tabs = session_removed
                    .iter()
                    .filter_map(|&remove_index| {
                        let tab = self.workspace_state.tabs.get(remove_index)?;
                        let session = tab.as_session()?;
                        let profile = self.profile_for_session_reopen(
                            &session.profile_id,
                            &session.pending_profile,
                        )?;
                        Some(ClosedSessionTabState {
                            profile,
                            hidden_from_topbar: tab.hidden_from_topbar,
                        })
                    })
                    .collect::<Vec<_>>();
                if tabs.is_empty() {
                    return None;
                }

                let workspace = if self.workspace_state.active_topbar_tab == Some(index) {
                    self.unload_active_topbar_workspace(cx);
                    self.workspace_state
                        .tabs
                        .get_mut(index)
                        .and_then(|tab| tab.workspace.take())
                } else {
                    self.workspace_state
                        .tabs
                        .get_mut(index)
                        .and_then(|tab| tab.workspace.take())
                }
                .map(|workspace| self.normalize_closed_workspace(workspace, &session_removed));

                Some(ClosedTabBundle::SessionWorkspace { tabs, workspace })
            }
        }
    }

    pub(in crate::ui::shell) fn activate_next_topbar_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let visible_tabs = self
            .workspace_state
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| (!tab.hidden_from_topbar).then_some(index))
            .collect::<Vec<_>>();
        if visible_tabs.len() <= 1 {
            return;
        }

        let next_index = self
            .workspace_state
            .active_topbar_tab
            .and_then(|active| visible_tabs.iter().position(|&index| index == active))
            .map(|current| visible_tabs[(current + 1) % visible_tabs.len()])
            .unwrap_or(visible_tabs[0]);
        self.activate_tab(next_index, window, cx);
    }

    pub(in crate::ui::shell) fn close_active_topbar_tab_shortcut(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.workspace_state.active_topbar_tab else {
            self.status_message = i18n::string("session.messages.no_tab_to_close");
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
        let Some(bundle) = self.workspace_state.recently_closed_tabs.pop() else {
            self.status_message = i18n::string("session.messages.no_recently_closed_tab");
            cx.notify();
            return;
        };

        match bundle {
            ClosedTabBundle::Hosts => self.open_hosts_tab(cx),
            ClosedTabBundle::Sftp { profile } => {
                if self.profile_requires_local_vault_unlock(&profile) {
                    self.workspace_state
                        .recently_closed_tabs
                        .push(ClosedTabBundle::Sftp { profile });
                    self.prompt_local_vault_unlock_in_window(window, cx);
                    return;
                }
                self.open_sftp_tab(profile, window, cx)
            }
            ClosedTabBundle::SessionWorkspace { tabs, workspace } => {
                if tabs.is_empty() {
                    self.status_message = i18n::string("session.messages.no_recently_closed_tab");
                    cx.notify();
                    return;
                }

                if tabs
                    .iter()
                    .any(|closed_tab| self.profile_requires_local_vault_unlock(&closed_tab.profile))
                {
                    self.workspace_state
                        .recently_closed_tabs
                        .push(ClosedTabBundle::SessionWorkspace { tabs, workspace });
                    self.prompt_local_vault_unlock_in_window(window, cx);
                    return;
                }

                self.unload_active_topbar_workspace(cx);

                let mut reopened_indices = Vec::with_capacity(tabs.len());
                for closed_tab in tabs {
                    let mut tab = self.build_session_tab(closed_tab.profile);
                    tab.hidden_from_topbar = closed_tab.hidden_from_topbar;
                    self.workspace_state.tabs.push(tab);
                    reopened_indices.push(self.workspace_state.tabs.len() - 1);
                }

                let Some(visible_index) = reopened_indices
                    .iter()
                    .copied()
                    .find(|&tab_index| !self.workspace_state.tabs[tab_index].hidden_from_topbar)
                    .or_else(|| reopened_indices.first().copied())
                else {
                    self.status_message = i18n::string("session.messages.no_recently_closed_tab");
                    cx.notify();
                    return;
                };

                if let Some(workspace) = workspace {
                    let workspace = self.restore_closed_workspace(workspace, &reopened_indices);
                    if let Some(tab) = self.workspace_state.tabs.get_mut(visible_index) {
                        tab.workspace = Some(workspace);
                    }
                }

                self.workspace_state.active_topbar_tab = Some(visible_index);
                self.load_topbar_workspace(visible_index, cx);
                self.rebind_terminal_focus_reporting(window, cx);
                self.panel_view.sidebar_section = SidebarSection::Hosts;
                self.editors.host_editor_open = false;
                self.editors.host_editor_is_new = false;
                let title = self.workspace_state.tabs[visible_index].title.clone();
                self.status_message =
                    i18n::string_args("session.messages.reopened_tab", &[("title", &title)]);
                window.focus(
                    &self.workspace_state.workspace.active_pane.terminal_focus,
                    cx,
                );
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
        let bindings = self.settings_store.settings().key_bindings.clone();

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
        if index >= self.workspace_state.tabs.len() {
            return;
        }

        let closed_bundle = self.build_closed_tab_bundle(index, cx);
        let title = self.workspace_state.tabs[index].title.clone();
        let mut removed = if self.workspace_state.tabs[index].hidden_from_topbar {
            vec![index]
        } else {
            self.owned_tab_indices_for_topbar(index)
        };
        removed.sort_unstable();
        removed.dedup();
        let mut affected_monitor_profiles = HashSet::new();

        for remove_index in removed.iter().rev().copied() {
            let tab = self.workspace_state.tabs.remove(remove_index);
            let tab_id = tab.id;
            match tab.kind {
                TabKind::Session(session) => {
                    if session.purpose == SessionPurpose::Terminal {
                        affected_monitor_profiles.insert(session.profile_id.clone());
                    }
                    if let Some(commands) = session.commands
                        && let Err(error) = commands.close()
                    {
                        log::debug!("failed to close tab {} cleanly: {error:?}", tab_id);
                    }
                }
                TabKind::Sftp(sftp) => {
                    if let Some(commands) = sftp.commands
                        && let Err(error) = commands.close()
                    {
                        log::debug!("failed to close SFTP tab {} cleanly: {error:?}", tab_id);
                    }
                }
                TabKind::Hosts => {}
            }
        }

        self.remap_all_tab_indices_after_removal(&removed);
        for profile_id in affected_monitor_profiles {
            self.refresh_profile_monitoring(&profile_id, None);
        }

        if let Some(bundle) = closed_bundle {
            self.push_recently_closed_tab(bundle);
        }

        if self.workspace_state.active_topbar_tab.is_none() {
            if let Some(next_index) = self
                .nearest_visible_tab(index.min(self.workspace_state.tabs.len().saturating_sub(1)))
            {
                self.workspace_state.active_topbar_tab = Some(next_index);
                if self.workspace_state.tabs[next_index].as_session().is_some() {
                    self.load_topbar_workspace(next_index, cx);
                } else {
                    self.reset_loaded_workspace(cx);
                }
            } else {
                self.reset_loaded_workspace(cx);
            }
        }

        if let Some(active_session_index) = self.workspace_state.workspace.active_tab
            && let Some(session) = self
                .workspace_state
                .tabs
                .get_mut(active_session_index)
                .and_then(TabState::as_session_mut)
        {
            session.has_activity = false;
        }

        if self
            .workspace_state
            .renaming_tab
            .is_some_and(|renaming| removed.binary_search(&renaming).is_ok())
        {
            self.workspace_state.renaming_tab = None;
        }

        if self.workspace_state.tabs.is_empty() {
            self.workspace_state.active_topbar_tab = None;
            self.workspace_state.workspace.active_tab = None;
            match self.settings_store.settings().last_tab_close_behavior {
                miaominal_settings::LastTabCloseBehavior::ExitApplication => {
                    cx.quit();
                }
                miaominal_settings::LastTabCloseBehavior::OpenNewHomeTab => {
                    self.open_hosts_tab(cx);
                    self.rebind_terminal_focus_reporting(window, cx);
                    self.sync_terminal_focus_reporting(window, cx);
                }
            }
            return;
        }

        self.rebind_terminal_focus_reporting(window, cx);
        self.sync_terminal_focus_reporting(window, cx);
        self.status_message =
            i18n::string_args("session.messages.closed_tab", &[("title", &title)]);
        cx.notify();
    }

    pub(in crate::ui::shell) fn activate_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.workspace_state.tabs.len()
            || self.workspace_state.tabs[index].hidden_from_topbar
        {
            return;
        }

        let previous_active_index = self.workspace_state.active_topbar_tab;
        let previous_active_tab =
            previous_active_index.and_then(|active| self.workspace_state.tabs.get(active));
        let previous_active_is_hosts = previous_active_tab.is_some_and(TabState::is_hosts)
            && self.panel_view.sidebar_section == SidebarSection::Hosts;
        let target_is_terminal = self.workspace_state.tabs[index]
            .as_session()
            .is_some_and(|session| session.purpose == SessionPurpose::Terminal);
        let preserve_host_editor_sidebar =
            previous_active_is_hosts && target_is_terminal && self.editors.host_editor_open;

        if self.workspace_state.active_topbar_tab != Some(index) {
            self.unload_active_topbar_workspace(cx);
            self.workspace_state.active_topbar_tab = Some(index);
            if self.workspace_state.tabs[index].as_session().is_some() {
                self.load_topbar_workspace(index, cx);
            } else {
                self.reset_loaded_workspace(cx);
            }
            self.rebind_terminal_focus_reporting(window, cx);
        }

        self.panel_view.sidebar_section = SidebarSection::Hosts;

        if !preserve_host_editor_sidebar {
            self.editors.host_editor_open = false;
            self.editors.host_editor_is_new = false;
        }

        if let Some(active_session_index) = self.workspace_state.workspace.active_tab
            && let Some(session) = self
                .workspace_state
                .tabs
                .get_mut(active_session_index)
                .and_then(TabState::as_session_mut)
        {
            session.has_activity = false;
        }

        if self.workspace_state.tabs[index].as_session().is_some() {
            window.focus(
                &self.workspace_state.workspace.active_pane.terminal_focus,
                cx,
            );
        }

        if self.workspace_state.tabs[index].as_sftp().is_some() {
            self.reset_sftp_path_editing();
            self.sync_active_sftp_path_inputs(cx);
            self.sync_active_sftp_tables(cx);
        }
        if self.panels.session_side_panel_open
            && self.panels.session_side_panel_view == SessionSidePanelView::Sftp
            && self.workspace_state.tabs[index].as_session().is_some()
        {
            let session_tab_id = self.workspace_state.tabs[index].id;
            self.ensure_session_side_panel_sftp_tab(session_tab_id, cx);
        }

        self.sync_terminal_focus_reporting(window, cx);
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
        if index >= self.workspace_state.tabs.len() {
            return false;
        }

        let mut pending_profile = None;
        let mut live_resize = false;

        let (size_changed, monitoring_enabled, profile_id, tab_id) = {
            let Some(tab) = self.workspace_state.tabs.get_mut(index) else {
                return false;
            };
            let tab_id = tab.id;
            let Some(session) = tab.as_session_mut() else {
                return false;
            };

            let size_changed = session.terminal.resize(columns, lines);
            let monitoring_enabled = session.monitoring.auto_collect_enabled;
            let profile_id = session.profile_id.clone();
            if bounds_known && session.commands.is_none() {
                if allow_pending_start {
                    // Wait for one stable frame before spawning the remote PTY so
                    // the initial winsize comes from the settled terminal viewport.
                    pending_profile = session.pending_profile.take();
                }
            } else {
                live_resize = size_changed;
            }

            (size_changed, monitoring_enabled, profile_id, tab_id)
        };
        let monitoring_enabled =
            self.claim_profile_monitor_source(&profile_id, tab_id, monitoring_enabled);

        if let Some(profile) = pending_profile {
            let connection = self.terminal_service().start_session(
                profile,
                self.data.sessions.clone(),
                columns,
                lines,
                monitoring_enabled,
            );

            let tab_id = {
                let Some(tab) = self.workspace_state.tabs.get_mut(index) else {
                    return size_changed;
                };
                let Some(session) = tab.as_session_mut() else {
                    return size_changed;
                };

                session.commands = Some(connection.commands);
                tab.id
            };
            self.spawn_session_event_loop(tab_id, connection.events, cx);
            return true;
        }

        if live_resize {
            let Some(tab) = self.workspace_state.tabs.get_mut(index) else {
                return size_changed;
            };
            let Some(session) = tab.as_session_mut() else {
                return size_changed;
            };

            if let Some(commands) = session.commands.as_ref()
                && let Err(error) = commands.resize(columns, lines)
            {
                log::debug!("failed to resize remote PTY: {error:?}");
            }
        }

        size_changed
    }

    pub(in crate::ui::shell) fn resolve_host_key_prompt(
        &mut self,
        decision: HostKeyDecision,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.pending_host_key_session_index() else {
            return;
        };
        let Some((prompt, commands)) = self.workspace_state.tabs.get_mut(index).and_then(|tab| {
            let session = tab.as_session_mut()?;
            let prompt = session.pending_host_key.take()?;
            Some((prompt, session.commands.clone()))
        }) else {
            return;
        };

        self.start_dialog_exit(DialogOverlaySnapshot::HostKey(prompt.clone()), cx);

        let Some(commands) = commands else {
            return;
        };
        if let Err(error) = commands.respond_host_key(decision) {
            log::warn!("failed to deliver host key decision: {error:?}");
        }
        match decision {
            HostKeyDecision::AcceptOnce => {
                self.status_message = i18n::string_args(
                    "session.messages.accepted_host_key_session_only",
                    &[("host", &prompt.host)],
                );
            }
            HostKeyDecision::AcceptAndSave => {
                self.status_message = i18n::string_args(
                    "session.messages.trusting_host_key",
                    &[("host", &prompt.host)],
                );
                self.refresh_known_hosts();
            }
            HostKeyDecision::Reject => {
                self.status_message = i18n::string_args(
                    "session.messages.rejected_host_key",
                    &[("host", &prompt.host)],
                );
            }
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn refresh_known_hosts(&mut self) {
        match self.services.known_hosts.list() {
            Ok(entries) => self.data.known_hosts_entries = entries,
            Err(error) => log::warn!("failed to refresh known_hosts list: {error:?}"),
        }
    }

    pub(in crate::ui::shell) fn remove_known_host(
        &mut self,
        host: String,
        port: u16,
        cx: &mut Context<Self>,
    ) {
        match self.services.known_hosts.remove(&host, port) {
            Ok(true) => {
                self.refresh_known_hosts();
                let port_text = port.to_string();
                self.status_message = i18n::string_args(
                    "session.messages.removed_host_key",
                    &[("host", &host), ("port", &port_text)],
                );
            }
            Ok(false) => {
                let port_text = port.to_string();
                self.status_message = i18n::string_args(
                    "session.messages.no_host_key_entry",
                    &[("host", &host), ("port", &port_text)],
                );
            }
            Err(error) => {
                let error = error.to_string();
                self.status_message = i18n::string_args(
                    "session.messages.remove_host_key_failed",
                    &[("error", &error)],
                );
            }
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn spawn_session_event_loop(
        &self,
        tab_id: usize,
        mut events: SessionEventReceiver,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let mut pending_event = None;

            loop {
                let event = if let Some(event) = pending_event.take() {
                    event
                } else {
                    let Some(event) = events.next().await else {
                        break;
                    };
                    event
                };
                let event = match event {
                    SessionEvent::Output(chunk) => {
                        let (chunk, pending) = coalesce_session_output(chunk, &mut events);
                        pending_event = pending;
                        SessionEvent::Output(chunk)
                    }
                    event => event,
                };

                if this
                    .update(cx, |this, cx| this.handle_session_event(tab_id, event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn handle_session_event(&mut self, tab_id: usize, event: SessionEvent, cx: &mut Context<Self>) {
        let Some(tab_index) = self
            .workspace_state
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
        else {
            return;
        };
        let active_tab = self.workspace_state.workspace.active_tab;
        let inactive_tab = active_tab != Some(tab_index);
        let mut clipboard_writes: Vec<String> = Vec::new();
        let mut failure_notification = None;
        let mut success_notification = None;
        let mut remove_port_forward_tab = false;
        let mut remove_connection_test_tab = false;
        let mut close_connection_test_commands = None;
        let mut remove_port_forward_profile_id = None;
        let mut port_forward_status_message = None;
        let mut port_forward_rule_enabled_update: Option<(String, String, bool)> = None;
        let mut connection_test_status_message = None;
        let mut schedule_reconnect_error: Option<String> = None;
        let mut record_connected_profile_id: Option<String> = None;
        let mut monitor_snapshot: Option<(String, bool, SessionMonitorSnapshot)> = None;
        let mut monitor_error: Option<(String, bool, String)> = None;
        let mut refresh_monitoring_profile: Option<String> = None;

        {
            let tab = &mut self.workspace_state.tabs[tab_index];
            let tab_title = tab.title.clone();
            let status = &mut tab.status;
            let TabKind::Session(session) = &mut tab.kind else {
                return;
            };
            let is_port_forward_session = session.purpose == SessionPurpose::PortForwarding;
            let is_connection_test = session.purpose == SessionPurpose::ConnectionTest;

            match event {
                SessionEvent::Connected(connection_label) => {
                    session.set_connection_state(SessionConnectionState::Ready);
                    session.reconnect_attempt = 0;
                    if is_connection_test {
                        *status = i18n::string_args(
                            "session.status.test_succeeded",
                            &[("connection", &connection_label)],
                        );
                        success_notification = Some(
                            Self::success_notification(
                                i18n::string("session.notifications.connection_succeeded_title"),
                                i18n::string_args(
                                    "session.messages.connection_succeeded_body",
                                    &[("connection", &connection_label)],
                                ),
                            )
                            .id1::<AppView>(SharedString::from(
                                format!("connection-test-success-{tab_id}"),
                            )),
                        );
                        close_connection_test_commands = session.commands.clone();
                        remove_connection_test_tab = true;
                        connection_test_status_message = Some(i18n::string_args(
                            "session.messages.test_connection_succeeded_for",
                            &[("connection", &connection_label)],
                        ));
                    } else if is_port_forward_session {
                        *status = i18n::string_args(
                            "session.status.forwarding_connected",
                            &[("connection", &connection_label)],
                        );
                        if let Some(rule_id) = session.port_forward_rule_id.clone() {
                            port_forward_rule_enabled_update =
                                Some((session.profile_id.clone(), rule_id, true));
                        }
                    } else {
                        *status = i18n::string_args(
                            "session.status.connected",
                            &[("connection", &connection_label)],
                        );
                        record_connected_profile_id = Some(session.profile_id.clone());
                    }
                    if inactive_tab {
                        session.has_activity = true;
                    }
                }
                SessionEvent::Output(chunk) => {
                    session.bytes_in = session.bytes_in.saturating_add(chunk.len() as u64);
                    session.terminal.push_bytes(&chunk);
                    if let Some(tap) = session.pty_output_tap.as_ref() {
                        // A failed tap closes its channel, but the slot remains reserved until
                        // the owning Agent request performs same-channel cleanup. Releasing it
                        // here would let another request start on the same terminal before the
                        // first request has interrupted its still-running command.
                        let _ = tap.try_send(chunk.clone());
                    }
                    if inactive_tab {
                        session.has_activity = true;
                    }
                }
                SessionEvent::Status(message) => {
                    *status = message.clone();
                    if inactive_tab {
                        session.has_activity = true;
                    }
                }
                SessionEvent::MonitorUpdated(snapshot) => {
                    monitor_snapshot = Some((
                        session.profile_id.clone(),
                        session.monitoring.auto_collect_enabled,
                        snapshot,
                    ));
                }
                SessionEvent::MonitorFailed(error) => {
                    monitor_error = Some((
                        session.profile_id.clone(),
                        session.monitoring.auto_collect_enabled,
                        error,
                    ));
                }
                SessionEvent::Error(error) => {
                    let was_ready =
                        matches!(session.connection_state, SessionConnectionState::Ready);
                    let is_in_reconnect_cycle = session.reconnect_attempt > 0;
                    *status = i18n::string("session.status.error");
                    if !is_port_forward_session
                        && !is_connection_test
                        && (was_ready || is_in_reconnect_cycle)
                    {
                        schedule_reconnect_error = Some(error.clone());
                    } else if !is_port_forward_session && !is_connection_test && !was_ready {
                        session.set_connection_state(SessionConnectionState::Failed {
                            error: error.clone(),
                            status: None,
                        });
                    }
                    if !is_port_forward_session && !is_connection_test {
                        refresh_monitoring_profile = Some(session.profile_id.clone());
                    }
                    session.terminal.push_text(&i18n::string_args(
                        "session.terminal.error_line",
                        &[("error", &error)],
                    ));
                    let notification_title = if is_connection_test {
                        i18n::string("session.notifications.test_connection_failed_title")
                    } else if is_port_forward_session {
                        i18n::string("session.notifications.port_forwarding_failed_title")
                    } else if was_ready {
                        i18n::string("session.notifications.session_error_title")
                    } else {
                        i18n::string("session.notifications.connection_failed_title")
                    };
                    let notification_message = if tab_title.trim().is_empty() {
                        error.clone()
                    } else {
                        format!("{}: {}", tab_title, error)
                    };
                    failure_notification = Some(
                        Self::error_notification(notification_title, notification_message)
                            .id1::<AppView>(SharedString::from(format!(
                                "session-failure-{tab_id}"
                            ))),
                    );
                    if is_connection_test {
                        close_connection_test_commands = session.commands.clone();
                        remove_connection_test_tab = true;
                        connection_test_status_message = Some(i18n::string_args(
                            "session.messages.test_connection_failed_for",
                            &[("profile", &session.profile_id)],
                        ));
                    } else if is_port_forward_session {
                        remove_port_forward_tab = true;
                        remove_port_forward_profile_id = Some(session.profile_id.clone());
                        if let Some(rule_id) = session.port_forward_rule_id.clone() {
                            port_forward_rule_enabled_update =
                                Some((session.profile_id.clone(), rule_id, false));
                        }
                        port_forward_status_message = Some(i18n::string_args(
                            "session.messages.port_forwarding_failed_for",
                            &[("title", &tab_title), ("error", &error)],
                        ));
                    }
                    if inactive_tab {
                        session.has_activity = true;
                    }
                }
                SessionEvent::PortForwardNotice(message) => {
                    session.terminal.push_text(&format!(
                        "{} {message}\r\n",
                        i18n::string("session.terminal.forward_prefix")
                    ));
                    if inactive_tab {
                        session.has_activity = true;
                    }
                }
                SessionEvent::HostKeyPrompt(prompt) => {
                    *status = if prompt.previous_fingerprint.is_some() {
                        i18n::string("session.status.host_key_mismatch")
                    } else {
                        i18n::string("session.status.verify_host_key")
                    };
                    if prompt.previous_fingerprint.is_some() {
                        session.terminal.push_text(&format!(
                            "{} {} {} {}\r\n",
                            i18n::string("session.terminal.host_key_prefix"),
                            prompt.host,
                            prompt.algorithm,
                            prompt.fingerprint
                        ));
                    }
                    session.pending_host_key = Some(prompt);
                    if inactive_tab {
                        session.has_activity = true;
                    }
                }
                SessionEvent::KeyboardInteractivePrompt(challenge) => {
                    *status = if challenge.name.is_empty() {
                        i18n::string("prompts.authentication_challenge")
                    } else {
                        challenge.name.clone()
                    };
                    session.pending_keyboard_interactive = Some(challenge);
                    if inactive_tab {
                        session.has_activity = true;
                    }
                }
                SessionEvent::Closed => {
                    if let Some(tap) = session.pty_output_tap.take() {
                        tap.close();
                    }
                    let already_disconnected = matches!(
                        session.connection_state,
                        SessionConnectionState::Disconnected
                    );
                    if !already_disconnected {
                        let was_ready =
                            matches!(session.connection_state, SessionConnectionState::Ready);
                        *status = i18n::string("session.status.closed");
                        if !is_port_forward_session && !is_connection_test && was_ready {
                            session.set_connection_state(SessionConnectionState::Disconnected);
                        } else if !is_port_forward_session
                            && !is_connection_test
                            && matches!(
                                session.connection_state,
                                SessionConnectionState::Connecting
                            )
                        {
                            let failure_message =
                                i18n::string("session.status.connection_closed_before_ready");
                            session.set_connection_state(SessionConnectionState::Failed {
                                error: failure_message.clone(),
                                status: Some(SessionFailureStatus::Closed),
                            });
                            let notification_message = if tab_title.trim().is_empty() {
                                failure_message.clone()
                            } else {
                                format!("{}: {}", tab_title, failure_message)
                            };
                            failure_notification = Some(
                                Self::error_notification(
                                    i18n::string("session.notifications.connection_closed_title"),
                                    notification_message,
                                )
                                .id1::<AppView>(
                                    SharedString::from(format!("session-failure-{tab_id}")),
                                ),
                            );
                        }
                        session.terminal.push_text(&format!(
                            "{}\r\n",
                            i18n::string("session.terminal.closed_marker")
                        ));
                        if inactive_tab {
                            session.has_activity = true;
                        }
                    }
                    if is_connection_test {
                        if matches!(session.connection_state, SessionConnectionState::Connecting) {
                            failure_notification = Some(
                                Self::error_notification(
                                    i18n::string(
                                        "session.notifications.test_connection_failed_title",
                                    ),
                                    i18n::string_args(
                                        "session.messages.connection_test_closed_before_complete",
                                        &[("title", &tab_title)],
                                    ),
                                )
                                .id1::<AppView>(
                                    SharedString::from(format!("connection-test-failure-{tab_id}")),
                                ),
                            );
                        }
                        remove_connection_test_tab = true;
                        connection_test_status_message = Some(i18n::string_args(
                            "session.messages.finished_test_connection_for",
                            &[("title", &tab_title)],
                        ));
                    } else if is_port_forward_session {
                        remove_port_forward_tab = true;
                        remove_port_forward_profile_id = Some(session.profile_id.clone());
                        if let Some(rule_id) = session.port_forward_rule_id.clone() {
                            port_forward_rule_enabled_update =
                                Some((session.profile_id.clone(), rule_id, false));
                        }
                        port_forward_status_message = Some(i18n::string_args(
                            "session.messages.port_forwarding_disconnected_for",
                            &[("title", &tab_title)],
                        ));
                    } else {
                        refresh_monitoring_profile = Some(session.profile_id.clone());
                    }
                }
            }

            // Drain OSC / Bell events emitted by the alacritty parser as a
            // direct consequence of the bytes we just pushed.
            while let Some(emu_event) = session.terminal.try_recv_event() {
                match emu_event {
                    miaominal_terminal::TerminalEvent::ClipboardStore(content) => {
                        clipboard_writes.push(content);
                    }
                    miaominal_terminal::TerminalEvent::Bell => {
                        if active_tab != Some(tab_index) {
                            session.has_activity = true;
                        }
                    }
                }
            }
        }

        if let Some((profile_id, enabled, snapshot)) = monitor_snapshot {
            self.apply_profile_monitor_snapshot(&profile_id, tab_id, enabled, snapshot);
        }

        if let Some((profile_id, enabled, error)) = monitor_error {
            self.apply_profile_monitor_error(&profile_id, tab_id, enabled, error);
        }

        if let Some((profile_id, rule_id, enabled)) = port_forward_rule_enabled_update
            && self
                .update_port_forward_rule_enabled_state(&profile_id, &rule_id, enabled)
                .is_some()
            && let Err(error) = self.persist_sessions()
        {
            log::warn!("failed to persist port-forward rule state: {error:?}");
        }

        if let Some(profile_id) = refresh_monitoring_profile {
            self.refresh_profile_monitoring(&profile_id, Some(tab_id));
        }

        for content in clipboard_writes {
            cx.write_to_clipboard(ClipboardItem::new_string(content));
            self.status_message = i18n::string("session.messages.clipboard_osc52");
        }

        if let Some(profile_id) = record_connected_profile_id {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if let Some(profile) = self.data.sessions.iter_mut().find(|p| p.id == profile_id) {
                profile.last_connected_at = Some(now);
            }
            let _ = self.persist_sessions();
        }

        if let Some(notification) = failure_notification {
            self.with_active_window(cx, move |window, cx| {
                window.push_notification(notification, cx);
            });
        }

        if let Some(error) = schedule_reconnect_error {
            self.schedule_reconnect(tab_id, error, cx);
        }

        if let Some(notification) = success_notification {
            self.with_active_window(cx, move |window, cx| {
                window.push_notification(notification, cx);
            });
        }

        if remove_port_forward_tab {
            self.workspace_state.tabs.remove(tab_index);
            self.remap_all_tab_indices_after_removal(&[tab_index]);
            let synced_sessions = remove_port_forward_profile_id
                .as_deref()
                .map(|profile_id| self.sync_port_forward_rules_for_profile(profile_id))
                .unwrap_or(0);
            if let Some(message) = port_forward_status_message {
                self.status_message = format!(
                    "{}{}",
                    message,
                    self.synced_sessions_suffix(synced_sessions)
                );
            }
            cx.notify();
            return;
        }

        if remove_connection_test_tab {
            if let Some(commands) = close_connection_test_commands {
                let _ = commands.close();
            }
            self.workspace_state.tabs.remove(tab_index);
            self.remap_all_tab_indices_after_removal(&[tab_index]);
            if let Some(message) = connection_test_status_message {
                self.status_message = message;
            }
            cx.notify();
            return;
        }

        cx.notify();
    }

    pub(in crate::ui::shell) fn close_other_tabs(
        &mut self,
        keep_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if keep_index >= self.workspace_state.tabs.len() {
            return;
        }

        let kept_id = self.workspace_state.tabs[keep_index].id;
        let mut keep_indices = self.owned_tab_indices_for_topbar(keep_index);
        keep_indices.sort_unstable();
        keep_indices.dedup();

        let remove_indices: Vec<usize> = (0..self.workspace_state.tabs.len())
            .filter(|index| keep_indices.binary_search(index).is_err())
            .collect();
        let mut affected_monitor_profiles = HashSet::new();

        for remove_index in remove_indices.iter().rev().copied() {
            let tab = self.workspace_state.tabs.remove(remove_index);
            let tab_id = tab.id;
            match tab.kind {
                TabKind::Session(session) => {
                    if session.purpose == SessionPurpose::Terminal {
                        affected_monitor_profiles.insert(session.profile_id.clone());
                    }
                    if let Some(commands) = session.commands
                        && let Err(error) = commands.close()
                    {
                        log::debug!("failed to close tab {} cleanly: {error:?}", tab_id);
                    }
                }
                TabKind::Sftp(sftp) => {
                    if let Some(commands) = sftp.commands
                        && let Err(error) = commands.close()
                    {
                        log::debug!("failed to close SFTP tab {} cleanly: {error:?}", tab_id);
                    }
                }
                TabKind::Hosts => {}
            }
        }

        self.remap_all_tab_indices_after_removal(&remove_indices);
        for profile_id in affected_monitor_profiles {
            self.refresh_profile_monitoring(&profile_id, None);
        }

        if let Some(new_index) = self
            .workspace_state
            .tabs
            .iter()
            .position(|tab| tab.id == kept_id)
        {
            let should_load = self.workspace_state.active_topbar_tab != Some(new_index);
            self.workspace_state.active_topbar_tab = Some(new_index);
            if self.workspace_state.tabs[new_index].as_session().is_some() {
                if should_load {
                    self.load_topbar_workspace(new_index, cx);
                }
            } else {
                self.reset_loaded_workspace(cx);
            }
        } else {
            self.workspace_state.active_topbar_tab = None;
            self.reset_loaded_workspace(cx);
        }

        self.workspace_state.renaming_tab = None;
        self.rebind_terminal_focus_reporting(window, cx);
        self.sync_terminal_focus_reporting(window, cx);
        self.status_message = i18n::string("session.messages.closed_other_tabs");
        cx.notify();
    }

    pub(in crate::ui::shell) fn duplicate_profile_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile = self
            .workspace_state
            .tabs
            .get(index)
            .and_then(TabState::as_session)
            .and_then(|session| {
                self.data
                    .sessions
                    .iter()
                    .find(|profile| profile.id == session.profile_id)
                    .cloned()
                    .or_else(|| session.pending_profile.clone())
            });

        let Some(profile) = profile else {
            self.status_message = i18n::string("session.messages.source_profile_not_found_for_tab");
            cx.notify();
            return;
        };

        self.open_session_tab(profile, window, cx);
        self.status_message = i18n::string("session.messages.opened_same_profile_tab");
    }

    pub(in crate::ui::shell) fn begin_rename_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(title) = self
            .workspace_state
            .tabs
            .get(index)
            .map(|tab| tab.title.clone())
        else {
            return;
        };
        self.workspace_state.renaming_tab = Some(index);
        set_input_value(&self.workspace_forms.rename_input, title, window, cx);
        let rename_input = self.workspace_forms.rename_input.clone();
        rename_input.update(cx, |input, cx| {
            input.focus(window, cx);
        });
        cx.notify();
        cx.on_next_frame(window, move |this, window, cx| {
            if this.workspace_state.renaming_tab != Some(index) {
                return;
            }

            let rename_input = this.workspace_forms.rename_input.clone();
            rename_input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
            window.dispatch_action(Box::new(gpui_component::input::SelectAll), cx);
        });
    }

    pub(in crate::ui::shell) fn commit_rename_tab(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.workspace_state.renaming_tab else {
            return;
        };
        let new_title = self
            .workspace_forms
            .rename_input
            .read(cx)
            .value()
            .to_string();
        let trimmed = new_title.trim();
        if !trimmed.is_empty() {
            let title = self.unique_topbar_tab_title(trimmed, Some(index));
            if let Some(tab) = self.workspace_state.tabs.get_mut(index) {
                tab.title = title;
            }
        }
        self.workspace_state.renaming_tab = None;
        cx.notify();
    }

    pub(in crate::ui::shell) fn cancel_rename_tab(&mut self, cx: &mut Context<Self>) {
        if self.workspace_state.renaming_tab.is_some() {
            self.workspace_state.renaming_tab = None;
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn reorder_tab(
        &mut self,
        from: usize,
        to: usize,
        cx: &mut Context<Self>,
    ) {
        if from >= self.workspace_state.tabs.len() {
            return;
        }
        let to = to.min(self.workspace_state.tabs.len().saturating_sub(1));
        if from == to {
            return;
        }

        let tab = self.workspace_state.tabs.remove(from);
        let dest = to.min(self.workspace_state.tabs.len());
        self.workspace_state.tabs.insert(dest, tab);

        self.remap_all_tab_indices_after_move(from, dest);

        cx.notify();
    }

    pub(in crate::ui::shell) fn schedule_reconnect(
        &mut self,
        tab_id: usize,
        error: String,
        cx: &mut Context<Self>,
    ) {
        const MAX_RECONNECT_ATTEMPTS: u32 = 10;
        const RECONNECT_DELAYS_SECS: &[u64] = &[1, 2, 4, 8, 16, 30];

        let Some(tab_index) = self
            .workspace_state
            .tabs
            .iter()
            .position(|t| t.id == tab_id)
        else {
            return;
        };

        let next_attempt = self
            .workspace_state
            .tabs
            .get(tab_index)
            .and_then(TabState::as_session)
            .map(|s| s.reconnect_attempt.saturating_add(1))
            .unwrap_or(1);

        if next_attempt > MAX_RECONNECT_ATTEMPTS {
            if let Some(session) = self.workspace_state.tabs[tab_index].as_session_mut() {
                session.set_connection_state(SessionConnectionState::Failed {
                    error,
                    status: None,
                });
                session.reconnect_attempt = 0;
            }
            return;
        }

        if let Some(session) = self.workspace_state.tabs[tab_index].as_session_mut() {
            session.reconnect_attempt = next_attempt;
            session.set_connection_state(SessionConnectionState::Reconnecting {
                error: error.clone(),
                attempt: next_attempt,
            });
        }

        let delay_secs = RECONNECT_DELAYS_SECS
            .get(next_attempt.saturating_sub(1) as usize)
            .copied()
            .unwrap_or(30);
        let delay = std::time::Duration::from_secs(delay_secs);

        let reconnect_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            if this
                .update(cx, |this, cx| {
                    let Some(tab_index) = this
                        .workspace_state
                        .tabs
                        .iter()
                        .position(|t| t.id == tab_id)
                    else {
                        return;
                    };
                    let profile_id = this
                        .workspace_state
                        .tabs
                        .get(tab_index)
                        .and_then(TabState::as_session)
                        .map(|s| s.profile_id.clone());
                    let profile = profile_id
                        .as_deref()
                        .and_then(|id| this.data.sessions.iter().find(|p| p.id == id).cloned());
                    if let Some(session) = this.workspace_state.tabs[tab_index].as_session_mut() {
                        if let Some(profile) = profile {
                            session.commands = None;
                            session.pending_profile = Some(profile);
                            session.set_connection_state(SessionConnectionState::Connecting);
                            session.terminal.push_text(&i18n::string_args(
                                "session.terminal.reconnecting_attempt_marker",
                                &[("attempt", &next_attempt.to_string())],
                            ));
                        } else {
                            session.set_connection_state(SessionConnectionState::Failed {
                                error: error.clone(),
                                status: None,
                            });
                            session.reconnect_attempt = 0;
                        }
                        session.reconnect_task = None;
                    }
                    cx.notify();
                })
                .is_err()
            {
                log::debug!("reconnect task: AppView entity was dropped");
            }
        });

        if let Some(session) = self.workspace_state.tabs[tab_index].as_session_mut() {
            session.reconnect_task = Some(reconnect_task);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[test]
    fn coalesce_session_output_merges_consecutive_output() {
        let (sender, receiver) = mpsc::channel(4);
        let mut receiver = SessionEventReceiver::from(receiver);
        sender
            .try_send(SessionEvent::Output(b"b".to_vec()))
            .expect("output event should send");
        sender
            .try_send(SessionEvent::Output(b"c".to_vec()))
            .expect("output event should send");

        let (chunk, pending) = coalesce_session_output(b"a".to_vec(), &mut receiver);

        assert_eq!(chunk, b"abc");
        assert!(pending.is_none());
    }

    #[test]
    fn coalesce_session_output_preserves_next_non_output_event() {
        let (sender, receiver) = mpsc::channel(4);
        let mut receiver = SessionEventReceiver::from(receiver);
        sender
            .try_send(SessionEvent::Output(b"b".to_vec()))
            .expect("output event should send");
        sender
            .try_send(SessionEvent::Status("ready".into()))
            .expect("status event should send");

        let (chunk, pending) = coalesce_session_output(b"a".to_vec(), &mut receiver);

        assert_eq!(chunk, b"ab");
        assert!(matches!(pending, Some(SessionEvent::Status(status)) if status == "ready"));
    }
}
