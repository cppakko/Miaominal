//! Physical detached-window operations and feature-port wiring. Workspace owns
//! resource extraction, installation, rollback and source completion.
use super::*;
use crate::ui::workspace::transaction::{TransferPhase, TransferTransaction};
use crate::ui::workspace::transfer::{WorkspaceTransferExt, WorkspaceTransferHost};
use std::{cell::RefCell, rc::Rc};

impl AppView {
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
                self.report_transfer_failure(error.to_string(), cx);
                return;
            }
        };
        let Some(bundle) = self.extract_detached_tab_bundle(tab_id, window, cx) else {
            let (target_window, _) = target.into_parts();
            let _ = target_window.update(cx, |_, window, _| window.remove_window());
            return;
        };
        let source = bundle.source();
        let transaction = Rc::new(RefCell::new(TransferTransaction::prepare(bundle)));
        let target_transaction = transaction.clone();
        let (target_window, target_view) = target.into_parts();
        // WindowHandle::update rejects a closed/type-changed window before
        // invoking this closure. Destination preflight also returns the complete
        // bundle, so every failed install still has one source-owned payload.
        let result = target_window.update(cx, move |_, target_window, cx| {
            target_view.update(cx, |target, cx| {
                target_transaction.borrow_mut().install(|bundle| {
                    target
                        .install_detached_tab_bundle(bundle, target_window, cx)
                        .map_err(|bundle| {
                            (
                                "destination window contains live resources".to_string(),
                                bundle,
                            )
                        })
                })
            })?;
            target_window.activate_window();
            Ok::<(), String>(())
        });
        let error = match result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(error) => Some(error.to_string()),
        };
        if let Some(error) = error {
            if let Some(bundle) = transaction.borrow_mut().rollback() {
                self.restore_detached_tab_bundle(bundle, window, cx);
            }
            self.report_transfer_failure(error, cx);
            let _ = target_window.update(cx, |_, target_window, _| target_window.remove_window());
            return;
        }
        transaction.borrow_mut().commit();
        debug_assert_eq!(transaction.borrow().phase(), TransferPhase::Committed);
        self.finish_detached_tab_source(source, window, cx);
    }

    fn report_transfer_failure(&mut self, error: String, cx: &mut Context<Self>) {
        log::error!("failed to install detached tab window: {error}");
        self.shell.status_message = i18n::string_args(
            "chrome.messages.open_in_new_window_failed",
            &[("error", &error)],
        );
        cx.notify();
    }
}

impl WorkspaceTransferHost for AppView {
    fn transfer_state(&self) -> &WorkspaceModel {
        &self.workspace
    }
    fn transfer_state_mut(&mut self) -> &mut WorkspaceModel {
        &mut self.workspace
    }
    fn transfer_session_purpose(&self, id: TabId, cx: &App) -> Option<SessionPurpose> {
        self.controllers.session.read(cx).tab_purpose(id)
    }
    fn transfer_has_session(&self, id: TabId, cx: &App) -> bool {
        self.controllers.session.read(cx).tab(id).is_some()
    }
    fn transfer_has_sftp(&self, id: TabId, cx: &App) -> bool {
        self.controllers.sftp.read(cx).tab(id).is_some()
    }
    fn transfer_terminal_monitor_profile(&self, id: TabId, cx: &App) -> Option<String> {
        let controller = self.controllers.session.read(cx);
        let tab = controller.tab(id)?;
        (tab.purpose == SessionPurpose::Terminal).then(|| tab.profile_id.clone())
    }
    fn transfer_visible_sftp_sidecar(&self, root: TabId, cx: &App) -> Option<TabId> {
        let session = self.controllers.session.read(cx);
        (session.side_panel_open() && session.side_panel_view() == SessionSidePanelView::Sftp)
            .then(|| self.session_side_panel_sftp_tab_id(cx))
            .flatten()
            .filter(|id| {
                self.workspace
                    .tabs
                    .get(*id)
                    .is_some_and(|tab| tab.owner() == Some(root))
            })
    }
    fn transfer_take_session(&mut self, id: TabId, cx: &App) -> Option<TransferredSessionTab> {
        self.controllers.session.read(cx).take_tab_for_transfer(id)
    }
    fn transfer_take_sftp(&mut self, id: TabId, cx: &App) -> Option<TransferredSftpTab> {
        self.controllers.sftp.read(cx).take_tab_for_transfer(id)
    }
    fn transfer_install_session(
        &mut self,
        id: TabId,
        payload: TransferredSessionTab,
        cx: &mut Context<Self>,
    ) {
        let installed = self.controllers.session.update(cx, |controller, cx| {
            controller.insert_transferred_tab(payload, cx)
        });
        debug_assert_eq!(installed, id);
    }
    fn transfer_install_sftp(
        &mut self,
        id: TabId,
        payload: TransferredSftpTab,
        cx: &mut Context<Self>,
    ) {
        let installed = self.controllers.sftp.update(cx, |controller, cx| {
            controller.insert_transferred_tab(payload, cx)
        });
        debug_assert_eq!(installed, id);
    }
    fn transfer_take_session_ui(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TransferredSessionWorkspaceUi {
        self.controllers.session.update(cx, |controller, cx| {
            controller.take_workspace_ui_for_transfer(window, cx)
        })
    }
    fn transfer_take_sftp_ui(
        &mut self,
        id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TransferredSftpWindowUi> {
        self.controllers.sftp.update(cx, |controller, cx| {
            controller.take_window_ui_for_transfer(id, window, cx)
        })
    }
    fn transfer_restore_session_ui(
        &mut self,
        ui: TransferredSessionWorkspaceUi,
        active: Option<TabId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.controllers.session.update(cx, |controller, cx| {
            controller.restore_workspace_ui_after_transfer(ui, active, window, cx)
        });
    }
    fn transfer_restore_sftp_ui(
        &mut self,
        ui: TransferredSftpWindowUi,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.controllers.sftp.update(cx, |controller, cx| {
            controller.restore_window_ui_after_transfer(ui, window, cx)
        });
    }
    fn transfer_retire_resources(&mut self, id: TabId, kind: TabKindTag, cx: &mut Context<Self>) {
        match kind {
            TabKindTag::Session | TabKindTag::LocalTerminal => {
                self.controllers.session.read(cx).retire_tab_resources(id);
            }
            TabKindTag::Sftp => self.controllers.sftp.read(cx).retire_tab_resources(id),
            TabKindTag::Hosts => {}
        }
    }
    fn transfer_prepare_workspace_for_window(
        &mut self,
        state: &mut TabWorkspaceState,
        cx: &mut Context<Self>,
    ) {
        Self::prepare_transferred_workspace_for_window(state, cx);
    }
    fn transfer_window_is_detached(&self) -> bool {
        self.window_role == AppWindowRole::Detached
    }
    fn transfer_remove_window(&mut self, window: &mut Window) {
        window.remove_window();
    }
    fn transfer_set_status(&mut self, status: String) {
        self.shell.status_message = status;
    }
    fn transfer_refresh_monitoring(&mut self, profile: &str, cx: &mut Context<Self>) {
        self.refresh_profile_monitoring(profile, None, cx);
    }
    fn transfer_owned_tab_indices_for_topbar(&self, root: usize) -> Vec<usize> {
        self.owned_tab_indices_for_topbar(root)
    }
    fn transfer_park_loaded_workspace(&mut self, cx: &mut Context<Self>) -> TabWorkspaceState {
        self.park_loaded_workspace(cx)
    }
    fn transfer_restore_loaded_workspace(&mut self, state: TabWorkspaceState) {
        self.restore_loaded_workspace(state);
    }
    fn transfer_reset_loaded_workspace(&mut self, cx: &mut Context<Self>) {
        self.reset_loaded_workspace(cx);
    }
    fn transfer_load_topbar_workspace(&mut self, index: usize, cx: &mut Context<Self>) {
        self.load_topbar_workspace(index, cx);
    }
    fn transfer_prune_closed_tab_references(&mut self) {
        self.prune_closed_tab_references();
    }
    fn transfer_nearest_visible_tab(&self, index: usize) -> Option<usize> {
        self.nearest_visible_tab(index)
    }
    fn transfer_open_hosts_tab(&mut self, cx: &mut Context<Self>) {
        self.open_hosts_tab(cx);
    }
    fn transfer_restore_active_topbar_sidebar_section(&mut self, cx: &mut Context<Self>) {
        self.restore_active_topbar_sidebar_section(cx);
    }
    fn transfer_sync_transferred_sftp_browser(
        &mut self,
        root: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let browser = self
            .workspace
            .tabs
            .get(root)
            .filter(|tab| tab.is_sftp())
            .map(|tab| tab.id)
            .or_else(|| self.transfer_visible_sftp_sidecar(root, cx));
        let Some(id) = browser else {
            return;
        };
        let prompt = self.should_prompt_sftp_download_destination(id, cx);
        self.controllers.sftp.update(cx, |controller, cx| {
            controller.set_download_destination_prompt_tab(id, prompt);
            controller.sync_path_inputs_for_tab(id, window, cx);
            controller.sync_tables_for_tab(id, cx);
        });
    }
    fn transfer_rebind_terminal_focus_reporting(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rebind_terminal_focus_reporting(window, cx);
    }
    fn transfer_sync_terminal_focus_reporting(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_terminal_focus_reporting(window, cx);
    }
    fn transfer_sync_session_port_snapshot(&self, cx: &App) {
        self.sync_session_port_snapshot(cx);
    }
    fn transfer_remove_tab_payload_and_metadata(&mut self, id: TabId, cx: &mut Context<Self>) {
        self.remove_tab_payload_and_metadata(id, cx);
    }
}
use crate::ui::i18n;
