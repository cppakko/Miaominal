use crate::ui::contracts::snippets::TransferredSnippetsUi;
use gpui_kit::{App, Context, Window};

use super::model::{TabKindTag, TabPlacement, TabState, TabWorkspaceState, WorkspaceModel};
use super::panes::PaneTabDropZone;
use crate::ui::contracts::session::{
    SessionPurpose, TransferredSessionTab, TransferredSessionWorkspaceUi,
};
use crate::ui::contracts::sftp::{TransferredSftpTab, TransferredSftpWindowUi};
use crate::ui::contracts::{PaneId, TabId};
use crate::ui::i18n;

enum DetachedTabPayload {
    Hosts,
    Session(Box<TransferredSessionTab>),
    Sftp(Box<TransferredSftpTab>),
}

struct DetachedTabEntry {
    original_index: usize,
    tab: TabState,
    payload: DetachedTabPayload,
}

#[derive(Clone)]
pub(crate) struct DetachedSourceState {
    root_id: TabId,
    root_index: usize,
    active_topbar_before: Option<TabId>,
    affected_monitor_profiles: Vec<String>,
}

pub(crate) struct DetachedTabBundle {
    source: DetachedSourceState,
    entries: Vec<DetachedTabEntry>,
    workspace: Option<TabWorkspaceState>,
    session_ui: Option<TransferredSessionWorkspaceUi>,
    snippets_ui: Option<TransferredSnippetsUi>,
    sftp_ui: Option<TransferredSftpWindowUi>,
}

impl DetachedTabBundle {
    pub(crate) fn source(&self) -> DetachedSourceState {
        self.source.clone()
    }

    pub(crate) fn root_id(&self) -> TabId {
        self.source.root_id
    }

    pub(crate) fn max_tab_id(&self) -> TabId {
        self.entries
            .iter()
            .map(|entry| entry.tab.id)
            .max()
            .unwrap_or(self.source.root_id)
    }
}

fn tab_kind_can_open_in_new_window(
    kind: TabKindTag,
    placement: TabPlacement,
    session_purpose: Option<SessionPurpose>,
    top_level_tab_count: usize,
) -> bool {
    if placement != TabPlacement::TopLevel {
        return false;
    }
    match kind {
        TabKindTag::Sftp => true,
        TabKindTag::Session => session_purpose == Some(SessionPurpose::Terminal),
        TabKindTag::LocalTerminal | TabKindTag::Telnet => {
            session_purpose == Some(SessionPurpose::Terminal)
        }
        TabKindTag::Hosts => top_level_tab_count > 1,
    }
}

/// Window assembly exposes identity, opaque feature handoffs and physical binding
/// operations. Workspace owns extraction, installation, rollback and source commit.
pub(crate) trait WorkspaceTransferHost: Sized {
    fn transfer_state(&self) -> &WorkspaceModel;
    fn transfer_state_mut(&mut self) -> &mut WorkspaceModel;
    fn transfer_session_purpose(&self, id: TabId, cx: &App) -> Option<SessionPurpose>;
    fn transfer_has_session(&self, id: TabId, cx: &App) -> bool;
    fn transfer_has_sftp(&self, id: TabId, cx: &App) -> bool;
    fn transfer_terminal_monitor_profile(&self, id: TabId, cx: &App) -> Option<String>;
    fn transfer_visible_sftp_sidecar(&self, root: TabId, cx: &App) -> Option<TabId>;
    fn transfer_take_session(&mut self, id: TabId, cx: &App) -> Option<TransferredSessionTab>;
    fn transfer_take_sftp(&mut self, id: TabId, cx: &App) -> Option<TransferredSftpTab>;
    fn transfer_install_session(
        &mut self,
        id: TabId,
        payload: TransferredSessionTab,
        cx: &mut Context<Self>,
    );
    fn transfer_install_sftp(
        &mut self,
        id: TabId,
        payload: TransferredSftpTab,
        cx: &mut Context<Self>,
    );
    fn transfer_take_session_ui(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TransferredSessionWorkspaceUi;
    fn transfer_take_snippets_ui(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TransferredSnippetsUi;
    fn transfer_restore_snippets_ui(
        &mut self,
        ui: TransferredSnippetsUi,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn transfer_take_sftp_ui(
        &mut self,
        id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TransferredSftpWindowUi>;
    fn transfer_restore_session_ui(
        &mut self,
        ui: TransferredSessionWorkspaceUi,
        active: Option<TabId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn transfer_restore_sftp_ui(
        &mut self,
        ui: TransferredSftpWindowUi,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn transfer_retire_resources(&mut self, id: TabId, kind: TabKindTag, cx: &mut Context<Self>);
    fn transfer_prepare_workspace_for_window(
        &mut self,
        state: &mut TabWorkspaceState,
        cx: &mut Context<Self>,
    );
    fn transfer_window_is_detached(&self) -> bool;
    fn transfer_remove_window(&mut self, window: &mut Window);
    fn transfer_set_status(&mut self, status: String);
    fn transfer_refresh_monitoring(&mut self, profile: &str, cx: &mut Context<Self>);
    fn transfer_owned_tab_indices_for_topbar(&self, root: usize) -> Vec<usize>;
    fn transfer_park_loaded_workspace(&mut self, cx: &mut Context<Self>) -> TabWorkspaceState;
    fn transfer_restore_loaded_workspace(&mut self, state: TabWorkspaceState);
    fn transfer_reset_loaded_workspace(&mut self, cx: &mut Context<Self>);
    fn transfer_load_topbar_workspace(&mut self, index: usize, cx: &mut Context<Self>);
    fn transfer_prune_closed_tab_references(&mut self);
    fn transfer_nearest_visible_tab(&self, index: usize) -> Option<usize>;
    fn transfer_open_hosts_tab(&mut self, cx: &mut Context<Self>);
    fn transfer_restore_active_topbar_sidebar_section(&mut self, cx: &mut Context<Self>);
    fn transfer_sync_transferred_sftp_browser(
        &mut self,
        root: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn transfer_rebind_terminal_focus_reporting(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn transfer_sync_terminal_focus_reporting(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn transfer_sync_session_port_snapshot(&self, cx: &App);
    fn transfer_remove_tab_payload_and_metadata(&mut self, id: TabId, cx: &mut Context<Self>);
    /// Ends window-owned resources that have no tab, such as launched remote desktops.
    fn transfer_retire_window_resources(&mut self, cx: &mut Context<Self>);
    fn transfer_unload_active_topbar_workspace(&mut self, cx: &mut Context<Self>);
    fn transfer_handle_pane_tab_drop(
        &mut self,
        source_tab_id: TabId,
        target_pane_id: PaneId,
        zone: PaneTabDropZone,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
}

pub(crate) trait WorkspaceTransferExt: WorkspaceTransferHost {
    fn can_open_tab_in_new_window(&self, id: TabId, cx: &App) -> bool;
    fn can_transfer_tab_to_existing_window(&self, id: TabId, cx: &App) -> bool;
    fn can_install_detached_tab_bundle(&self, bundle: &DetachedTabBundle, cx: &App) -> bool;
    fn can_merge_transferred_tab_bundle(&self, bundle: &DetachedTabBundle, cx: &App) -> bool;
    fn extract_detached_tab_bundle(
        &mut self,
        id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<DetachedTabBundle>;
    fn install_detached_tab_bundle(
        &mut self,
        bundle: DetachedTabBundle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), Box<DetachedTabBundle>>;
    fn merge_transferred_tab_into_topbar(
        &mut self,
        bundle: DetachedTabBundle,
        insertion_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), Box<DetachedTabBundle>>;
    fn merge_transferred_tab_into_pane(
        &mut self,
        bundle: DetachedTabBundle,
        target_pane_id: PaneId,
        zone: PaneTabDropZone,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), Box<DetachedTabBundle>>;
    fn restore_detached_tab_bundle(
        &mut self,
        bundle: DetachedTabBundle,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn finish_detached_tab_source(
        &mut self,
        source: DetachedSourceState,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn prepare_detached_window_close(&mut self, window: &mut Window, cx: &mut Context<Self>);
}

impl<V: WorkspaceTransferHost + 'static> WorkspaceTransferExt for V {
    fn can_open_tab_in_new_window(&self, tab_id: TabId, cx: &App) -> bool {
        let Some(tab) = self.transfer_state().tabs.get(tab_id) else {
            return false;
        };
        let top_level_tab_count = self
            .transfer_state()
            .tabs
            .iter()
            .filter(|tab| tab.is_top_level())
            .count();
        let session_purpose = tab
            .is_session()
            .then(|| self.transfer_session_purpose(tab_id, cx))
            .flatten();
        if !tab_kind_can_open_in_new_window(
            tab.kind,
            tab.placement,
            session_purpose,
            top_level_tab_count,
        ) {
            return false;
        }
        !tab.is_sftp() || self.transfer_has_sftp(tab_id, cx)
    }

    fn can_transfer_tab_to_existing_window(&self, tab_id: TabId, cx: &App) -> bool {
        let Some(tab) = self.transfer_state().tabs.get(tab_id) else {
            return false;
        };
        if tab.is_hosts() && tab.placement == TabPlacement::TopLevel {
            let top_level_tab_count = self
                .transfer_state()
                .tabs
                .iter()
                .filter(|tab| tab.is_top_level())
                .count();
            return hosts_tab_can_transfer_to_existing_window(
                top_level_tab_count,
                self.transfer_window_is_detached(),
            );
        }
        self.can_open_tab_in_new_window(tab_id, cx)
    }

    fn extract_detached_tab_bundle(
        &mut self,
        root_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<DetachedTabBundle> {
        let root_index = self.transfer_state().tabs.index_of(root_id)?;
        if !self.can_transfer_tab_to_existing_window(root_id, cx) {
            return None;
        }
        let active_topbar_before = self.transfer_state().active_topbar_tab;
        let owned_indices = self.transfer_owned_tab_indices_for_topbar(root_index);

        for index in &owned_indices {
            let tab = self.transfer_state().tabs.at(*index)?;
            if tab.is_session() && !self.transfer_has_session(tab.id, cx) {
                return None;
            }
            if tab.is_sftp() && !self.transfer_has_sftp(tab.id, cx) {
                return None;
            }
        }

        let root_is_session = self
            .transfer_state()
            .tabs
            .get(root_id)
            .is_some_and(|tab| tab.is_session());
        let root_is_sftp = self
            .transfer_state()
            .tabs
            .get(root_id)
            .is_some_and(|tab| tab.is_sftp());
        let mut affected_monitor_profiles = owned_indices
            .iter()
            .filter_map(|index| {
                let tab_id = self.transfer_state().tabs.id_at(*index)?;
                self.transfer_terminal_monitor_profile(tab_id, cx)
            })
            .collect::<Vec<_>>();
        affected_monitor_profiles.sort();
        affected_monitor_profiles.dedup();
        let active_root = active_topbar_before == Some(root_id);
        let visible_sftp_tab_id = active_root
            .then(|| {
                if root_is_sftp {
                    Some(root_id)
                } else if root_is_session {
                    self.transfer_visible_sftp_sidecar(root_id, cx)
                } else {
                    None
                }
            })
            .flatten();
        let sftp_ui =
            visible_sftp_tab_id.and_then(|tab_id| self.transfer_take_sftp_ui(tab_id, window, cx));
        let session_ui =
            (active_root && root_is_session).then(|| self.transfer_take_session_ui(window, cx));
        let snippets_ui =
            (active_root && root_is_session).then(|| self.transfer_take_snippets_ui(window, cx));

        let workspace = if root_is_session {
            if active_topbar_before == Some(root_id) {
                Some(self.transfer_park_loaded_workspace(cx))
            } else {
                self.transfer_state_mut()
                    .take_parked_workspace(root_id)
                    .or_else(|| Some(TabWorkspaceState::new(Some(root_id), cx.focus_handle())))
            }
        } else {
            None
        };

        let mut entries = owned_indices
            .iter()
            .filter_map(|index| {
                self.transfer_state()
                    .tabs
                    .state(self.transfer_state().tabs.id_at(*index)?)
                    .map(|tab| (*index, tab))
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|(index, _)| *index);

        let mut transferred = Vec::with_capacity(entries.len());
        for (original_index, tab) in entries {
            let payload = if tab.is_session() {
                DetachedTabPayload::Session(Box::new(
                    self.transfer_take_session(tab.id, cx)
                        .expect("validated session transfer payload remains available"),
                ))
            } else if tab.is_sftp() {
                DetachedTabPayload::Sftp(Box::new(
                    self.transfer_take_sftp(tab.id, cx)
                        .expect("validated SFTP transfer payload remains available"),
                ))
            } else {
                debug_assert!(tab.is_hosts());
                DetachedTabPayload::Hosts
            };
            self.transfer_state_mut()
                .tabs
                .remove_id(tab.id)
                .expect("validated transfer metadata remains available");
            transferred.push(DetachedTabEntry {
                original_index,
                tab,
                payload,
            });
        }

        Some(DetachedTabBundle {
            source: DetachedSourceState {
                root_id,
                root_index,
                active_topbar_before,
                affected_monitor_profiles,
            },
            entries: transferred,
            workspace,
            session_ui,
            snippets_ui,
            sftp_ui,
        })
    }

    fn can_install_detached_tab_bundle(&self, bundle: &DetachedTabBundle, cx: &App) -> bool {
        self.transfer_state().tabs.ids().all(|id| {
            self.transfer_state()
                .tabs
                .get(id)
                .is_some_and(|tab| tab.is_hosts())
                && !self.transfer_has_session(id, cx)
                && !self.transfer_has_sftp(id, cx)
        }) && self.can_merge_transferred_tab_bundle(bundle, cx)
    }

    fn can_merge_transferred_tab_bundle(&self, bundle: &DetachedTabBundle, cx: &App) -> bool {
        bundle.entries.iter().all(|entry| {
            self.transfer_state().tabs.get(entry.tab.id).is_none()
                && !self.transfer_has_session(entry.tab.id, cx)
                && !self.transfer_has_sftp(entry.tab.id, cx)
        })
    }

    fn install_detached_tab_bundle(
        &mut self,
        bundle: DetachedTabBundle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), Box<DetachedTabBundle>> {
        if !self.can_install_detached_tab_bundle(&bundle, cx) {
            return Err(Box::new(bundle));
        }
        // A detached view is freshly bootstrapped with only a resource-free Hosts tab. There are
        // no session or SFTP controller payloads to retire here, so dropping that metadata
        // directly is intentional. Keep the assertions close to this shortcut so a future
        // bootstrap change cannot silently introduce leaked controller resources.
        let existing_ids = self.transfer_state().tabs.ids().collect::<Vec<_>>();
        for tab_id in existing_ids {
            debug_assert!(
                self.transfer_state()
                    .tabs
                    .get(tab_id)
                    .is_some_and(|tab| tab.is_hosts())
            );
            debug_assert!(!self.transfer_has_session(tab_id, cx));
            debug_assert!(!self.transfer_has_sftp(tab_id, cx));
            self.transfer_state_mut()
                .tabs
                .remove_id(tab_id)
                .expect("fresh detached bootstrap tab remains registered");
        }
        self.transfer_state_mut().active_topbar_tab = None;
        self.transfer_reset_loaded_workspace(cx);

        install_transferred_topbar_bundle(self, bundle, None, window, cx);
        Ok(())
    }

    fn merge_transferred_tab_into_topbar(
        &mut self,
        bundle: DetachedTabBundle,
        insertion_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), Box<DetachedTabBundle>> {
        if !self.can_merge_transferred_tab_bundle(&bundle, cx) {
            return Err(Box::new(bundle));
        }
        self.transfer_unload_active_topbar_workspace(cx);
        let raw_insert_index = self
            .transfer_state()
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| tab.is_top_level())
            .nth(insertion_index)
            .map(|(raw_index, _)| raw_index)
            .unwrap_or_else(|| self.transfer_state().tabs.len());
        install_transferred_topbar_bundle(self, bundle, Some(raw_insert_index), window, cx);
        Ok(())
    }

    fn merge_transferred_tab_into_pane(
        &mut self,
        mut bundle: DetachedTabBundle,
        target_pane_id: PaneId,
        zone: PaneTabDropZone,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), Box<DetachedTabBundle>> {
        let valid_single_session = self.can_merge_transferred_tab_bundle(&bundle, cx)
            && bundle.entries.len() == 1
            && bundle.entries[0].tab.is_session()
            && bundle.entries[0].tab.is_top_level()
            && self
                .transfer_state()
                .active_topbar_tab
                .and_then(|id| self.transfer_state().tabs.get(id))
                .is_some_and(|tab| tab.is_top_level() && tab.is_session())
            && self
                .transfer_state()
                .workspace
                .pane_layout
                .contains(target_pane_id);
        let target_tab_id = if target_pane_id == self.transfer_state().workspace.active_pane_id {
            self.transfer_state().workspace.active_tab
        } else {
            self.transfer_state()
                .workspace
                .parked_panes
                .get(&target_pane_id)
                .and_then(|parked| parked.active_tab)
        };
        let valid_center = zone != PaneTabDropZone::Center
            || target_tab_id
                .and_then(|tab_id| self.transfer_state().tabs.get(tab_id))
                .is_some_and(|tab| tab.is_session());
        if !valid_single_session || !valid_center {
            return Err(Box::new(bundle));
        }

        let root_id = bundle.root_id();
        let max_tab_id = bundle.max_tab_id();
        let affected_monitor_profiles = bundle.source.affected_monitor_profiles.clone();
        let entry = bundle
            .entries
            .pop()
            .expect("validated single-entry bundle has one tab");
        match entry.payload {
            DetachedTabPayload::Session(transferred) => {
                self.transfer_install_session(entry.tab.id, *transferred, cx);
            }
            DetachedTabPayload::Hosts | DetachedTabPayload::Sftp(_) => {
                unreachable!("validated single session entry")
            }
        }
        self.transfer_state_mut().tabs.push(entry.tab);
        self.transfer_state_mut()
            .advance_next_tab_id_past(max_tab_id);

        self.transfer_handle_pane_tab_drop(root_id, target_pane_id, zone, window, cx);
        self.transfer_rebind_terminal_focus_reporting(window, cx);
        self.transfer_sync_terminal_focus_reporting(window, cx);
        self.transfer_sync_session_port_snapshot(cx);
        for profile_id in affected_monitor_profiles {
            self.transfer_refresh_monitoring(&profile_id, cx);
        }
        cx.notify();
        Ok(())
    }

    fn restore_detached_tab_bundle(
        &mut self,
        mut bundle: DetachedTabBundle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        bundle.entries.sort_by_key(|entry| entry.original_index);
        for entry in bundle.entries {
            let tab_id = entry.tab.id;
            install_transferred_tab_payload(self, tab_id, entry.payload, cx);
            self.transfer_state_mut()
                .tabs
                .insert(entry.original_index, entry.tab);
        }

        if let Some(workspace) = bundle.workspace {
            if bundle.source.active_topbar_before == Some(bundle.source.root_id) {
                self.transfer_restore_loaded_workspace(workspace);
            } else {
                self.transfer_state_mut()
                    .park_workspace(bundle.source.root_id, workspace);
            }
        }
        self.transfer_state_mut().active_topbar_tab = bundle.source.active_topbar_before;
        self.transfer_restore_active_topbar_sidebar_section(cx);
        if let Some(ui) = bundle.snippets_ui.take() {
            self.transfer_restore_snippets_ui(ui, window, cx);
        }
        if let Some(session_ui) = bundle.session_ui.take() {
            let active_tab_id = self.transfer_state().workspace.active_tab;
            self.transfer_restore_session_ui(session_ui, active_tab_id, window, cx);
        }
        if let Some(root_id) = self.transfer_state().active_topbar_tab {
            self.transfer_sync_transferred_sftp_browser(root_id, window, cx);
        }
        if let Some(sftp_ui) = bundle.sftp_ui.take() {
            self.transfer_restore_sftp_ui(sftp_ui, window, cx);
        }
        self.transfer_rebind_terminal_focus_reporting(window, cx);
        self.transfer_sync_terminal_focus_reporting(window, cx);
        self.transfer_sync_session_port_snapshot(cx);
        for profile_id in &bundle.source.affected_monitor_profiles {
            self.transfer_refresh_monitoring(profile_id, cx);
        }
        cx.notify();
    }

    fn finish_detached_tab_source(
        &mut self,
        source: DetachedSourceState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.transfer_prune_closed_tab_references();
        if source.active_topbar_before == Some(source.root_id) {
            self.transfer_state_mut().active_topbar_tab = None;
            if let Some(next_index) = self.transfer_nearest_visible_tab(
                source
                    .root_index
                    .min(self.transfer_state().tabs.len().saturating_sub(1)),
            ) {
                self.transfer_state_mut().active_topbar_tab =
                    self.transfer_state().tabs.id_at(next_index);
                if self
                    .transfer_state()
                    .tabs
                    .at(next_index)
                    .is_some_and(|tab| tab.is_session())
                {
                    self.transfer_load_topbar_workspace(next_index, cx);
                } else {
                    self.transfer_reset_loaded_workspace(cx);
                }
            } else {
                self.transfer_reset_loaded_workspace(cx);
            }
        }

        let has_top_level_tab = self
            .transfer_state()
            .tabs
            .iter()
            .any(|tab| tab.is_top_level());
        if !has_top_level_tab {
            if self.transfer_window_is_detached() {
                self.transfer_remove_window(window);
                return;
            }
            self.transfer_open_hosts_tab(cx);
        }

        self.transfer_restore_active_topbar_sidebar_section(cx);

        if let Some(root_id) = self.transfer_state().active_topbar_tab {
            self.transfer_sync_transferred_sftp_browser(root_id, window, cx);
        }

        self.transfer_rebind_terminal_focus_reporting(window, cx);
        self.transfer_sync_terminal_focus_reporting(window, cx);
        self.transfer_sync_session_port_snapshot(cx);
        for profile_id in source.affected_monitor_profiles {
            self.transfer_refresh_monitoring(&profile_id, cx);
        }
        self.transfer_set_status(i18n::string("chrome.messages.opened_in_new_window"));
        cx.notify();
    }

    fn prepare_detached_window_close(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.transfer_window_is_detached() {
            return;
        }
        let tab_ids = self.transfer_state().tabs.ids().collect::<Vec<_>>();
        for tab_id in &tab_ids {
            if let Some(kind) = self.transfer_state().tabs.get(*tab_id).map(|tab| tab.kind) {
                self.transfer_retire_resources(*tab_id, kind, cx);
            }
        }
        for tab_id in tab_ids {
            self.transfer_remove_tab_payload_and_metadata(tab_id, cx);
        }
        self.transfer_retire_window_resources(cx);
        self.transfer_state_mut().active_topbar_tab = None;
        self.transfer_reset_loaded_workspace(cx);
        self.transfer_sync_session_port_snapshot(cx);
    }
}

fn install_transferred_tab_payload<V: WorkspaceTransferHost + 'static>(
    host: &mut V,
    tab_id: TabId,
    payload: DetachedTabPayload,
    cx: &mut Context<V>,
) {
    match payload {
        DetachedTabPayload::Hosts => {}
        DetachedTabPayload::Session(transferred) => {
            host.transfer_install_session(tab_id, *transferred, cx);
        }
        DetachedTabPayload::Sftp(transferred) => {
            host.transfer_install_sftp(tab_id, *transferred, cx);
        }
    }
}

fn install_transferred_topbar_bundle<V: WorkspaceTransferHost + 'static>(
    host: &mut V,
    mut bundle: DetachedTabBundle,
    raw_insert_index: Option<usize>,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    let root_id = bundle.root_id();
    let max_tab_id = bundle.max_tab_id();
    let affected_monitor_profiles = bundle.source.affected_monitor_profiles.clone();
    bundle.entries.sort_by_key(|entry| entry.original_index);
    for entry in bundle.entries {
        let tab_id = entry.tab.id;
        install_transferred_tab_payload(host, tab_id, entry.payload, cx);
        if tab_id == root_id
            && let Some(insert_at) = raw_insert_index
        {
            host.transfer_state_mut().tabs.insert(insert_at, entry.tab);
        } else {
            host.transfer_state_mut().tabs.push(entry.tab);
        }
    }

    host.transfer_state_mut().active_topbar_tab = Some(root_id);
    host.transfer_restore_active_topbar_sidebar_section(cx);
    host.transfer_state_mut()
        .advance_next_tab_id_past(max_tab_id);
    if let Some(mut workspace) = bundle.workspace {
        host.transfer_prepare_workspace_for_window(&mut workspace, cx);
        host.transfer_restore_loaded_workspace(workspace);
    } else {
        host.transfer_reset_loaded_workspace(cx);
    }

    if let Some(ui) = bundle.snippets_ui.take() {
        host.transfer_restore_snippets_ui(ui, window, cx);
    }
    if let Some(session_ui) = bundle.session_ui.take() {
        let active_tab_id = host.transfer_state().workspace.active_tab;
        host.transfer_restore_session_ui(session_ui, active_tab_id, window, cx);
    }

    host.transfer_state_mut()
        .topbar_previous_visible_tabs
        .clear();
    host.transfer_state_mut().topbar_entering_tabs.clear();
    host.transfer_state_mut().topbar_exiting_tabs.clear();
    host.transfer_state_mut().topbar_active_transition = None;
    host.transfer_state_mut().topbar_visible_active_tab_id = Some(root_id);
    host.transfer_sync_transferred_sftp_browser(root_id, window, cx);
    if let Some(sftp_ui) = bundle.sftp_ui.take() {
        host.transfer_restore_sftp_ui(sftp_ui, window, cx);
    }
    host.transfer_rebind_terminal_focus_reporting(window, cx);
    host.transfer_sync_terminal_focus_reporting(window, cx);
    host.transfer_sync_session_port_snapshot(cx);
    for profile_id in affected_monitor_profiles {
        host.transfer_refresh_monitoring(&profile_id, cx);
    }
    if !host
        .transfer_state()
        .tabs
        .get(root_id)
        .is_some_and(|tab| tab.is_hosts())
    {
        host.transfer_state()
            .workspace
            .active_pane
            .terminal_focus
            .focus(window, cx);
    }
    cx.notify();
}

fn hosts_tab_can_transfer_to_existing_window(
    top_level_tab_count: usize,
    window_is_detached: bool,
) -> bool {
    top_level_tab_count > 1 || window_is_detached
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::contracts::PaneId;

    #[test]
    fn existing_window_transfer_allows_sole_detached_hosts_tab_only() {
        assert!(!hosts_tab_can_transfer_to_existing_window(1, false));
        assert!(hosts_tab_can_transfer_to_existing_window(2, false));
        assert!(hosts_tab_can_transfer_to_existing_window(1, true));
        assert!(hosts_tab_can_transfer_to_existing_window(2, true));
    }

    #[test]
    fn detached_window_menu_only_accepts_terminal_workspaces_and_standalone_sftp() {
        assert!(tab_kind_can_open_in_new_window(
            TabKindTag::Session,
            TabPlacement::TopLevel,
            Some(SessionPurpose::Terminal),
            1,
        ));
        assert!(tab_kind_can_open_in_new_window(
            TabKindTag::LocalTerminal,
            TabPlacement::TopLevel,
            Some(SessionPurpose::Terminal),
            1,
        ));
        assert!(tab_kind_can_open_in_new_window(
            TabKindTag::Telnet,
            TabPlacement::TopLevel,
            Some(SessionPurpose::Terminal),
            1,
        ));
        assert!(tab_kind_can_open_in_new_window(
            TabKindTag::Sftp,
            TabPlacement::TopLevel,
            None,
            1,
        ));
        assert!(!tab_kind_can_open_in_new_window(
            TabKindTag::Hosts,
            TabPlacement::TopLevel,
            None,
            1,
        ));
        assert!(tab_kind_can_open_in_new_window(
            TabKindTag::Hosts,
            TabPlacement::TopLevel,
            None,
            2,
        ));
        assert!(!tab_kind_can_open_in_new_window(
            TabKindTag::Session,
            TabPlacement::TopLevel,
            Some(SessionPurpose::PortForwarding),
            2,
        ));
        assert!(!tab_kind_can_open_in_new_window(
            TabKindTag::Session,
            TabPlacement::TopLevel,
            Some(SessionPurpose::ConnectionTest),
            2,
        ));
        assert!(!tab_kind_can_open_in_new_window(
            TabKindTag::Session,
            TabPlacement::WorkspacePane {
                owner: TabId::new(1),
                pane: PaneId(2),
            },
            Some(SessionPurpose::Terminal),
            2,
        ));
        assert!(!tab_kind_can_open_in_new_window(
            TabKindTag::Sftp,
            TabPlacement::SessionSidecar {
                owner: TabId::new(1),
            },
            None,
            2,
        ));
    }
}
