use super::super::*;
use miaominal_services::SftpService;

impl AppView {
    pub(in crate::ui::shell) fn display_sftp_local_path(path: &std::path::Path) -> SharedString {
        SftpService::display_local_path(path).into()
    }

    pub(in crate::ui::shell) fn sftp_browser_table_tab_id(&self, cx: &App) -> Option<usize> {
        self.workspace_forms
            .sftp_browser
            .remote_table
            .read(cx)
            .delegate()
            .tab_id()
    }

    pub(in crate::ui::shell) fn should_sync_sftp_browser_for_tab(&self, tab_id: usize) -> bool {
        let active_tab_matches = self
            .workspace_state
            .active_topbar_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .is_some_and(|tab| tab.id == tab_id && tab.as_sftp().is_some());
        if active_tab_matches {
            return true;
        }

        self.panels.session_side_panel_open
            && self.panels.session_side_panel_view == SessionSidePanelView::Sftp
            && self.session_side_panel_sftp_tab_id() == Some(tab_id)
    }

    fn sync_sftp_path_inputs(
        &mut self,
        local_path: SharedString,
        remote_path: SharedString,
        cx: &mut Context<Self>,
    ) {
        let local_input = self.workspace_forms.sftp_browser.local_path_input.clone();
        let remote_input = self.workspace_forms.sftp_browser.remote_path_input.clone();

        self.with_active_window(cx, move |window, cx| {
            local_input.update(cx, |input, cx| {
                input.set_value(local_path.clone(), window, cx);
            });
            remote_input.update(cx, |input, cx| {
                input.set_value(remote_path.clone(), window, cx);
            });
        });
    }

    pub(in crate::ui::shell) fn sync_sftp_path_inputs_for_tab(
        &mut self,
        tab_id: usize,
        cx: &mut Context<Self>,
    ) {
        if !self.should_sync_sftp_browser_for_tab(tab_id) {
            return;
        }

        let Some(tab) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
        else {
            return;
        };

        let Some(sftp) = tab.as_sftp() else {
            return;
        };

        self.sync_sftp_path_inputs(
            Self::display_sftp_local_path(&sftp.local_path),
            sftp.remote_path.clone().into(),
            cx,
        );
    }

    pub(in crate::ui::shell) fn sync_active_sftp_path_inputs(&mut self, cx: &mut Context<Self>) {
        let Some(active_index) = self.workspace_state.active_topbar_tab else {
            return;
        };
        let Some(tab_id) = self
            .workspace_state
            .tabs
            .get(active_index)
            .map(|tab| tab.id)
        else {
            return;
        };
        self.sync_sftp_path_inputs_for_tab(tab_id, cx);
    }

    pub(in crate::ui::shell) fn sync_sftp_tables_for_tab(
        &mut self,
        tab_id: usize,
        cx: &mut Context<Self>,
    ) {
        if !self.should_sync_sftp_browser_for_tab(tab_id) {
            return;
        }

        let Some(tab) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
        else {
            return;
        };

        let Some(sftp) = tab.as_sftp() else {
            return;
        };

        let local_rows: Vec<_> = sftp
            .local_entries
            .iter()
            .map(SftpBrowserTableRow::from_local)
            .collect();
        let remote_rows: Vec<_> = sftp
            .remote_entries
            .iter()
            .map(SftpBrowserTableRow::from_remote)
            .collect();
        let selected_local_paths: Vec<_> = sftp
            .selected_local_paths
            .iter()
            .map(|selected| selected.display().to_string())
            .collect();
        let selected_local_path = sftp
            .selected_local_path
            .as_ref()
            .map(|selected| selected.display().to_string());
        let selected_remote_paths = sftp.selected_remote_paths.clone();
        let selected_remote_path = sftp.selected_remote_path.clone();
        let remote_loading = sftp.loading_remote;

        self.workspace_forms
            .sftp_browser
            .local_table
            .update(cx, |table, cx| {
                table.delegate_mut().set_rows(local_rows, false, tab_id);
                table
                    .delegate_mut()
                    .set_selected_paths(selected_local_paths.clone(), selected_local_path.clone());
                table.set_right_clicked_row(None, cx);
                table.refresh(cx);
            });

        self.workspace_forms
            .sftp_browser
            .remote_table
            .update(cx, |table, cx| {
                table
                    .delegate_mut()
                    .set_rows(remote_rows, remote_loading, tab_id);
                table.delegate_mut().set_selected_paths(
                    selected_remote_paths.clone(),
                    selected_remote_path.clone(),
                );
                table.set_right_clicked_row(None, cx);
                table.refresh(cx);
            });
    }

    pub(in crate::ui::shell) fn sync_sftp_selection_for_tab(
        &mut self,
        tab_id: usize,
        cx: &mut Context<Self>,
    ) {
        if !self.should_sync_sftp_browser_for_tab(tab_id) {
            return;
        }

        let Some(tab) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
        else {
            return;
        };

        let Some(sftp) = tab.as_sftp() else {
            return;
        };

        let selected_local_paths: Vec<_> = sftp
            .selected_local_paths
            .iter()
            .map(|selected| selected.display().to_string())
            .collect();
        let selected_local_path = sftp
            .selected_local_path
            .as_ref()
            .map(|selected| selected.display().to_string());
        let selected_remote_paths = sftp.selected_remote_paths.clone();
        let selected_remote_path = sftp.selected_remote_path.clone();

        self.workspace_forms
            .sftp_browser
            .local_table
            .update(cx, |table, cx| {
                table
                    .delegate_mut()
                    .set_selected_paths(selected_local_paths.clone(), selected_local_path.clone());
                table.set_right_clicked_row(None, cx);
                cx.notify();
            });

        self.workspace_forms
            .sftp_browser
            .remote_table
            .update(cx, |table, cx| {
                table.delegate_mut().set_selected_paths(
                    selected_remote_paths.clone(),
                    selected_remote_path.clone(),
                );
                table.set_right_clicked_row(None, cx);
                cx.notify();
            });
    }

    pub(in crate::ui::shell) fn sync_sftp_selection_for_side(
        &mut self,
        tab_id: usize,
        side: SftpBrowserSide,
        cx: &mut Context<Self>,
    ) {
        if !self.should_sync_sftp_browser_for_tab(tab_id) {
            return;
        }

        let Some(tab) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
        else {
            return;
        };

        let Some(sftp) = tab.as_sftp() else {
            return;
        };

        match side {
            SftpBrowserSide::Local => {
                let selected_paths: Vec<_> = sftp
                    .selected_local_paths
                    .iter()
                    .map(|selected| selected.display().to_string())
                    .collect();
                let selected_path = sftp
                    .selected_local_path
                    .as_ref()
                    .map(|selected| selected.display().to_string());

                self.workspace_forms
                    .sftp_browser
                    .local_table
                    .update(cx, |table, cx| {
                        table
                            .delegate_mut()
                            .set_selected_paths(selected_paths, selected_path);
                        table.set_right_clicked_row(None, cx);
                        cx.notify();
                    });
            }
            SftpBrowserSide::Remote => {
                let selected_paths = sftp.selected_remote_paths.clone();
                let selected_path = sftp.selected_remote_path.clone();

                self.workspace_forms
                    .sftp_browser
                    .remote_table
                    .update(cx, |table, cx| {
                        table
                            .delegate_mut()
                            .set_selected_paths(selected_paths, selected_path);
                        table.set_right_clicked_row(None, cx);
                        cx.notify();
                    });
            }
        }
    }

    pub(in crate::ui::shell) fn sync_active_sftp_tables(&mut self, cx: &mut Context<Self>) {
        let Some(active_index) = self.workspace_state.active_topbar_tab else {
            return;
        };
        let Some(tab_id) = self
            .workspace_state
            .tabs
            .get(active_index)
            .map(|tab| tab.id)
        else {
            return;
        };
        self.sync_sftp_tables_for_tab(tab_id, cx);
    }

    pub(in crate::ui::shell) fn active_sftp_tab_id(&self) -> Option<usize> {
        self.workspace_state
            .active_topbar_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(|tab| tab.as_sftp().map(|_| tab.id))
    }
}
