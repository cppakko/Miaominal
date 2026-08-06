use std::{cell::RefCell, rc::Rc};

use gpui::{Context, Window};

use super::*;
use crate::ui::i18n;

enum DetachedTabPayload {
    Session(Box<TransferredSessionTab>),
    Sftp(Box<TransferredSftpTab>),
}

struct DetachedTabEntry {
    original_index: usize,
    tab: TabState,
    payload: DetachedTabPayload,
}

struct DetachedSourceState {
    root_id: TabId,
    root_index: usize,
    active_topbar_before: Option<TabId>,
    affected_monitor_profiles: Vec<String>,
}

struct DetachedTabBundle {
    source: DetachedSourceState,
    entries: Vec<DetachedTabEntry>,
    workspace: Option<TabWorkspaceState>,
    session_ui: Option<TransferredSessionWorkspaceUi>,
    sftp_ui: Option<TransferredSftpWindowUi>,
}

impl DetachedTabBundle {
    fn root_id(&self) -> TabId {
        self.source.root_id
    }

    fn max_tab_id(&self) -> TabId {
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
) -> bool {
    if placement != TabPlacement::TopLevel {
        return false;
    }
    match kind {
        TabKindTag::Sftp => true,
        TabKindTag::Session => session_purpose == Some(SessionPurpose::Terminal),
        TabKindTag::Hosts => false,
    }
}

impl AppView {
    pub(in crate::ui::shell) fn can_open_tab_in_new_window(&self, tab_id: TabId, cx: &App) -> bool {
        let Some(tab) = self.workspace.tabs.get(tab_id) else {
            return false;
        };
        let session_purpose = tab
            .is_session()
            .then(|| self.controllers.session.read(cx).tab_purpose(tab_id))
            .flatten();
        if !tab_kind_can_open_in_new_window(tab.kind, tab.placement, session_purpose) {
            return false;
        }
        !tab.is_sftp() || self.sftp_tab(tab_id, cx).is_some()
    }

    pub(in crate::ui::shell) fn open_tab_in_new_window(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_open_tab_in_new_window(tab_id, cx) {
            return;
        }

        let target = match crate::ui::windowing::open_detached_window(cx) {
            Ok(target) => target,
            Err(error) => {
                log::error!("failed to open detached tab window: {error:?}");
                self.shell.status_message = i18n::string_args(
                    "chrome.messages.open_in_new_window_failed",
                    &[("error", &error.to_string())],
                );
                cx.notify();
                return;
            }
        };

        let bundle = match self.extract_detached_tab_bundle(tab_id, window, cx) {
            Some(bundle) => bundle,
            None => {
                let (target_window, _) = target.into_parts();
                let _ = target_window.update(cx, |_, window, _| window.remove_window());
                return;
            }
        };
        let source = DetachedSourceState {
            root_id: bundle.source.root_id,
            root_index: bundle.source.root_index,
            active_topbar_before: bundle.source.active_topbar_before,
            affected_monitor_profiles: bundle.source.affected_monitor_profiles.clone(),
        };
        let bundle = Rc::new(RefCell::new(Some(bundle)));
        let target_bundle = bundle.clone();
        let (target_window, target_view) = target.into_parts();
        let install_result = target_window.update(cx, move |_, target_window, cx| {
            target_view.update(cx, |target, cx| {
                let bundle = target_bundle
                    .borrow_mut()
                    .take()
                    .expect("detached tab bundle is installed exactly once");
                target.install_detached_tab_bundle(bundle, target_window, cx);
            });
            target_window.activate_window();
        });

        let install_error = install_result.err().map(|error| error.to_string());
        if let Some(error) = install_error {
            log::error!("failed to install detached tab window: {error}");
            if let Some(bundle) = bundle.borrow_mut().take() {
                self.restore_detached_tab_bundle(bundle, window, cx);
            }
            self.shell.status_message = i18n::string_args(
                "chrome.messages.open_in_new_window_failed",
                &[("error", &error)],
            );
            let _ = target_window.update(cx, |_, target_window, _| {
                target_window.remove_window();
            });
            cx.notify();
            return;
        }

        self.finish_detached_tab_source(source, window, cx);
    }

    fn extract_detached_tab_bundle(
        &mut self,
        root_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<DetachedTabBundle> {
        let root_index = self.workspace.tabs.index_of(root_id)?;
        if !self.can_open_tab_in_new_window(root_id, cx) {
            return None;
        }
        let active_topbar_before = self.workspace.active_topbar_tab;
        let owned_indices = self.owned_tab_indices_for_topbar(root_index);

        for index in &owned_indices {
            let tab = self.workspace.tabs.at(*index)?;
            if tab.is_session() && self.controllers.session.read(cx).tab(tab.id).is_none() {
                return None;
            }
            if tab.is_sftp() && self.controllers.sftp.read(cx).tab(tab.id).is_none() {
                return None;
            }
        }

        let root_is_session = self
            .workspace
            .tabs
            .get(root_id)
            .is_some_and(|tab| tab.is_session());
        let root_is_sftp = self
            .workspace
            .tabs
            .get(root_id)
            .is_some_and(|tab| tab.is_sftp());
        let mut affected_monitor_profiles = owned_indices
            .iter()
            .filter_map(|index| {
                let tab_id = self.workspace.tabs.id_at(*index)?;
                let session = self.controllers.session.read(cx).tab(tab_id)?;
                (session.purpose == SessionPurpose::Terminal).then(|| session.profile_id.clone())
            })
            .collect::<Vec<_>>();
        affected_monitor_profiles.sort();
        affected_monitor_profiles.dedup();
        let active_root = active_topbar_before == Some(root_id);
        let visible_sftp_tab_id = active_root
            .then(|| {
                if root_is_sftp {
                    Some(root_id)
                } else if root_is_session
                    && self.controllers.session.read(cx).side_panel_open()
                    && self.controllers.session.read(cx).side_panel_view()
                        == SessionSidePanelView::Sftp
                {
                    self.session_side_panel_sftp_tab_id(cx).filter(|tab_id| {
                        self.workspace
                            .tabs
                            .get(*tab_id)
                            .is_some_and(|tab| tab.owner() == Some(root_id))
                    })
                } else {
                    None
                }
            })
            .flatten();
        let sftp_ui = visible_sftp_tab_id.and_then(|tab_id| {
            self.controllers.sftp.update(cx, |controller, cx| {
                controller.take_window_ui_for_transfer(tab_id, root_is_session, window, cx)
            })
        });
        let session_ui = (active_root && root_is_session).then(|| {
            self.controllers.session.update(cx, |controller, cx| {
                controller.take_workspace_ui_for_transfer(window, cx)
            })
        });

        let workspace = if root_is_session {
            if active_topbar_before == Some(root_id) {
                Some(self.park_loaded_workspace(cx))
            } else {
                self.workspace
                    .take_parked_workspace(root_id)
                    .or_else(|| Some(TabWorkspaceState::new(Some(root_id), cx.focus_handle())))
            }
        } else {
            None
        };

        let mut entries = owned_indices
            .iter()
            .filter_map(|index| {
                self.workspace
                    .tabs
                    .state(self.workspace.tabs.id_at(*index)?)
                    .map(|tab| (*index, tab))
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|(index, _)| *index);

        let mut transferred = Vec::with_capacity(entries.len());
        for (original_index, tab) in entries {
            let payload = if tab.is_session() {
                DetachedTabPayload::Session(Box::new(
                    self.controllers
                        .session
                        .read(cx)
                        .take_tab_for_transfer(tab.id)
                        .expect("validated session transfer payload remains available"),
                ))
            } else {
                DetachedTabPayload::Sftp(Box::new(
                    self.controllers
                        .sftp
                        .read(cx)
                        .take_tab_for_transfer(tab.id)
                        .expect("validated SFTP transfer payload remains available"),
                ))
            };
            self.workspace
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
            sftp_ui,
        })
    }

    fn install_detached_tab_bundle(
        &mut self,
        mut bundle: DetachedTabBundle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A detached view is freshly bootstrapped with only a resource-free Hosts tab. There are
        // no session or SFTP controller payloads to retire here, so dropping that metadata
        // directly is intentional. Keep the assertions close to this shortcut so a future
        // bootstrap change cannot silently introduce leaked controller resources.
        let existing_ids = self.workspace.tabs.ids().collect::<Vec<_>>();
        for tab_id in existing_ids {
            debug_assert!(
                self.workspace
                    .tabs
                    .get(tab_id)
                    .is_some_and(|tab| tab.is_hosts())
            );
            debug_assert!(self.controllers.session.read(cx).tab(tab_id).is_none());
            debug_assert!(self.controllers.sftp.read(cx).tab(tab_id).is_none());
            self.workspace
                .tabs
                .remove_id(tab_id)
                .expect("fresh detached bootstrap tab remains registered");
        }
        self.workspace.active_topbar_tab = None;
        self.reset_loaded_workspace(cx);

        let root_id = bundle.root_id();
        let max_tab_id = bundle.max_tab_id();
        let affected_monitor_profiles = bundle.source.affected_monitor_profiles.clone();
        bundle.entries.sort_by_key(|entry| entry.original_index);
        for entry in bundle.entries {
            let tab_id = entry.tab.id;
            match entry.payload {
                DetachedTabPayload::Session(transferred) => {
                    self.controllers.session.update(cx, |controller, cx| {
                        controller.insert_transferred_tab(tab_id, *transferred, cx);
                    });
                }
                DetachedTabPayload::Sftp(transferred) => {
                    self.controllers.sftp.update(cx, |controller, cx| {
                        controller.insert_transferred_tab(tab_id, *transferred, cx);
                    });
                }
            }
            self.workspace.tabs.push(entry.tab);
        }

        self.workspace.active_topbar_tab = Some(root_id);
        self.workspace.advance_next_tab_id_past(max_tab_id);
        if let Some(mut workspace) = bundle.workspace {
            Self::prepare_transferred_workspace_for_window(&mut workspace, cx);
            self.restore_loaded_workspace(workspace);
        } else {
            self.reset_loaded_workspace(cx);
        }

        if let Some(session_ui) = bundle.session_ui.take() {
            let active_tab_id = self.workspace.workspace.active_tab;
            self.controllers.session.update(cx, |controller, cx| {
                controller.restore_workspace_ui_after_transfer(
                    session_ui,
                    active_tab_id,
                    window,
                    cx,
                );
            });
        }

        self.workspace.topbar_previous_visible_tabs.clear();
        self.workspace.topbar_entering_tabs.clear();
        self.workspace.topbar_exiting_tabs.clear();
        self.workspace.topbar_active_transition = None;
        self.workspace.topbar_visible_active_tab_id = Some(root_id);
        self.sync_transferred_sftp_browser(root_id, window, cx);
        if let Some(sftp_ui) = bundle.sftp_ui.take() {
            self.controllers.sftp.update(cx, |controller, cx| {
                controller.restore_window_ui_after_transfer(sftp_ui, window, cx);
            });
        }
        self.rebind_terminal_focus_reporting(window, cx);
        self.sync_terminal_focus_reporting(window, cx);
        self.sync_session_port_snapshot(cx);
        for profile_id in affected_monitor_profiles {
            self.refresh_profile_monitoring(&profile_id, None, cx);
        }
        self.workspace
            .workspace
            .active_pane
            .terminal_focus
            .focus(window, cx);
        cx.notify();
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
            match entry.payload {
                DetachedTabPayload::Session(transferred) => {
                    self.controllers.session.update(cx, |controller, cx| {
                        controller.insert_transferred_tab(tab_id, *transferred, cx);
                    });
                }
                DetachedTabPayload::Sftp(transferred) => {
                    self.controllers.sftp.update(cx, |controller, cx| {
                        controller.insert_transferred_tab(tab_id, *transferred, cx);
                    });
                }
            }
            self.workspace.tabs.insert(entry.original_index, entry.tab);
        }

        if let Some(workspace) = bundle.workspace {
            if bundle.source.active_topbar_before == Some(bundle.source.root_id) {
                self.restore_loaded_workspace(workspace);
            } else {
                self.workspace
                    .park_workspace(bundle.source.root_id, workspace);
            }
        }
        self.workspace.active_topbar_tab = bundle.source.active_topbar_before;
        if let Some(session_ui) = bundle.session_ui.take() {
            let active_tab_id = self.workspace.workspace.active_tab;
            self.controllers.session.update(cx, |controller, cx| {
                controller.restore_workspace_ui_after_transfer(
                    session_ui,
                    active_tab_id,
                    window,
                    cx,
                );
            });
        }
        if let Some(root_id) = self.workspace.active_topbar_tab {
            self.sync_transferred_sftp_browser(root_id, window, cx);
        }
        if let Some(sftp_ui) = bundle.sftp_ui.take() {
            self.controllers.sftp.update(cx, |controller, cx| {
                controller.restore_window_ui_after_transfer(sftp_ui, window, cx);
            });
        }
        self.rebind_terminal_focus_reporting(window, cx);
        self.sync_terminal_focus_reporting(window, cx);
        self.sync_session_port_snapshot(cx);
        for profile_id in &bundle.source.affected_monitor_profiles {
            self.refresh_profile_monitoring(profile_id, None, cx);
        }
        cx.notify();
    }

    fn finish_detached_tab_source(
        &mut self,
        source: DetachedSourceState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prune_closed_tab_references();
        if source.active_topbar_before == Some(source.root_id) {
            self.workspace.active_topbar_tab = None;
            if let Some(next_index) = self.nearest_visible_tab(
                source
                    .root_index
                    .min(self.workspace.tabs.len().saturating_sub(1)),
            ) {
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

        let has_top_level_tab = self.workspace.tabs.iter().any(|tab| tab.is_top_level());
        if !has_top_level_tab {
            match self.window_role {
                AppWindowRole::Primary => self.open_hosts_tab(cx),
                AppWindowRole::Detached => {
                    window.remove_window();
                    return;
                }
            }
        }

        if let Some(root_id) = self.workspace.active_topbar_tab {
            self.sync_transferred_sftp_browser(root_id, window, cx);
        }

        self.rebind_terminal_focus_reporting(window, cx);
        self.sync_terminal_focus_reporting(window, cx);
        self.sync_session_port_snapshot(cx);
        for profile_id in source.affected_monitor_profiles {
            self.refresh_profile_monitoring(&profile_id, None, cx);
        }
        self.shell.status_message = i18n::string("chrome.messages.opened_in_new_window");
        cx.notify();
    }

    fn sync_transferred_sftp_browser(
        &mut self,
        root_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let browser_tab_id = self
            .workspace
            .tabs
            .get(root_id)
            .filter(|tab| tab.is_sftp())
            .map(|tab| tab.id)
            .or_else(|| {
                (self.controllers.session.read(cx).side_panel_open()
                    && self.controllers.session.read(cx).side_panel_view()
                        == SessionSidePanelView::Sftp)
                    .then(|| self.session_side_panel_sftp_tab_id(cx))
                    .flatten()
            });
        let Some(tab_id) = browser_tab_id else {
            return;
        };
        let prompt_download_destination = self.should_prompt_sftp_download_destination(tab_id, cx);
        self.controllers.sftp.update(cx, |controller, cx| {
            controller.set_download_destination_prompt_tab(tab_id, prompt_download_destination);
            controller.sync_path_inputs_for_tab(tab_id, window, cx);
            controller.sync_tables_for_tab(tab_id, cx);
        });
    }

    pub fn prepare_detached_window_close(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.window_role != AppWindowRole::Detached {
            return;
        }
        let tab_ids = self.workspace.tabs.ids().collect::<Vec<_>>();
        for tab_id in &tab_ids {
            if self
                .workspace
                .tabs
                .get(*tab_id)
                .is_some_and(|tab| tab.is_session())
            {
                let _ = self
                    .controllers
                    .session
                    .read(cx)
                    .retire_tab_resources(*tab_id);
            } else if self
                .workspace
                .tabs
                .get(*tab_id)
                .is_some_and(|tab| tab.is_sftp())
                && let Some(sftp) = self.controllers.sftp.read(cx).remove_tab_state(*tab_id)
                && let Some(commands) = sftp.commands.as_ref()
                && let Err(error) = commands.close()
            {
                log::debug!("failed to close detached SFTP tab {tab_id} cleanly: {error:?}");
            }
        }
        for tab_id in tab_ids {
            let _ = self.remove_tab_payload_and_metadata(tab_id, cx);
        }
        self.workspace.active_topbar_tab = None;
        self.reset_loaded_workspace(cx);
        self.sync_session_port_snapshot(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detached_window_menu_only_accepts_terminal_workspaces_and_standalone_sftp() {
        assert!(tab_kind_can_open_in_new_window(
            TabKindTag::Session,
            TabPlacement::TopLevel,
            Some(SessionPurpose::Terminal),
        ));
        assert!(tab_kind_can_open_in_new_window(
            TabKindTag::Sftp,
            TabPlacement::TopLevel,
            None,
        ));
        assert!(!tab_kind_can_open_in_new_window(
            TabKindTag::Hosts,
            TabPlacement::TopLevel,
            None,
        ));
        assert!(!tab_kind_can_open_in_new_window(
            TabKindTag::Session,
            TabPlacement::TopLevel,
            Some(SessionPurpose::PortForwarding),
        ));
        assert!(!tab_kind_can_open_in_new_window(
            TabKindTag::Session,
            TabPlacement::TopLevel,
            Some(SessionPurpose::ConnectionTest),
        ));
        assert!(!tab_kind_can_open_in_new_window(
            TabKindTag::Session,
            TabPlacement::WorkspacePane {
                owner: TabId::new(1),
                pane: PaneId(2),
            },
            Some(SessionPurpose::Terminal),
        ));
        assert!(!tab_kind_can_open_in_new_window(
            TabKindTag::Sftp,
            TabPlacement::SessionSidecar {
                owner: TabId::new(1),
            },
            None,
        ));
    }
}
