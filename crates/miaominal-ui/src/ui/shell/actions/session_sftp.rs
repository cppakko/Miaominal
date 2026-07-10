use super::super::*;
use crate::ui::i18n;
use gpui_component::WindowExt as _;
use miaominal_services::SftpService;

impl AppView {
    fn sftp_service(&self) -> SftpService {
        SftpService::new(
            self.services.runtime.clone(),
            self.services.secrets.clone(),
            self.services.known_hosts.clone(),
        )
    }

    fn profile_for_session_tab_id(&self, session_tab_id: usize) -> Option<SessionProfile> {
        self.workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == session_tab_id)
            .and_then(TabState::as_session)
            .and_then(|session| {
                self.data
                    .sessions
                    .iter()
                    .find(|profile| profile.id == session.profile_id)
                    .cloned()
                    .or_else(|| session.pending_profile.clone())
            })
    }

    fn reusable_sftp_tab_id_for_session(
        &self,
        session_tab_id: usize,
        profile_id: &str,
    ) -> Option<usize> {
        self.workspace_state.tabs.iter().find_map(|tab| {
            let sftp = tab.as_sftp()?;
            let usable_owner =
                !tab.hidden_from_topbar || sftp.owner_session_tab_id == Some(session_tab_id);
            (sftp.profile_id == profile_id && usable_owner && sftp.commands.is_some())
                .then_some(tab.id)
        })
    }

    pub(in crate::ui::shell) fn session_side_panel_sftp_tab_id(&self) -> Option<usize> {
        let (session_tab_id, profile_id) = self
            .active_terminal_session_index()
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(|tab| {
                tab.as_session()
                    .map(|session| (tab.id, session.profile_id.as_str()))
            })?;

        self.reusable_sftp_tab_id_for_session(session_tab_id, profile_id)
    }

    pub(in crate::ui::shell) fn ensure_session_side_panel_sftp_tab(
        &mut self,
        session_tab_id: usize,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let Some(profile) = self.profile_for_session_tab_id(session_tab_id) else {
            self.status_message = i18n::string("session.messages.open_sftp_requires_active_ssh");
            cx.notify();
            return None;
        };

        if let Some(tab_id) = self.reusable_sftp_tab_id_for_session(session_tab_id, &profile.id) {
            self.sync_sftp_path_inputs_for_tab(tab_id, cx);
            self.sync_sftp_tables_for_tab(tab_id, cx);
            return Some(tab_id);
        }

        if self.profile_requires_local_vault_unlock(&profile) {
            self.prompt_local_vault_unlock(cx);
            return None;
        }

        let tab_id = {
            let next_id = self.workspace_state.next_tab_id;
            self.workspace_state.next_tab_id += 1;
            next_id
        };
        let mut tab = TabState::new_sftp(tab_id, &profile);
        tab.hidden_from_topbar = true;
        if let Some(sftp) = tab.as_sftp_mut() {
            sftp.owner_session_tab_id = Some(session_tab_id);
        }

        let connection = self
            .sftp_service()
            .start_session(profile.clone(), self.data.sessions.clone());
        if let Some(sftp) = tab.as_sftp_mut() {
            sftp.commands = Some(connection.commands);
        }

        self.workspace_state.tabs.push(tab);
        self.refresh_sftp_local_directory(tab_id, cx);
        self.spawn_sftp_event_loop(tab_id, connection.events, cx);
        self.sync_sftp_path_inputs_for_tab(tab_id, cx);
        self.sync_sftp_tables_for_tab(tab_id, cx);
        self.status_message = i18n::string_args(
            "sftp.messages.opened_tab_for",
            &[("profile", &profile.name)],
        );
        cx.notify();
        Some(tab_id)
    }

    fn sftp_browser_event_tab_id(
        &self,
        table_entity: &Entity<TableState<SftpBrowserTableDelegate>>,
        cx: &App,
    ) -> Option<usize> {
        self.active_sftp_tab_id()
            .or_else(|| table_entity.read(cx).delegate().tab_id())
    }

    pub(in crate::ui::shell) fn active_or_browser_sftp_tab_id(&self, cx: &App) -> Option<usize> {
        self.active_sftp_tab_id()
            .or_else(|| self.sftp_browser_table_tab_id(cx))
    }

    pub(in crate::ui::shell) fn resolve_remote_sftp_entry(
        &self,
        tab_id: usize,
        path: &str,
        cx: &App,
    ) -> Option<SftpEntry> {
        if let Some(entry) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .and_then(|sftp| {
                sftp.remote_entries
                    .iter()
                    .find(|entry| entry.path == path)
                    .cloned()
            })
        {
            return Some(entry);
        }

        let row = {
            let table = self.workspace_forms.sftp_browser.remote_table.read(cx);
            let delegate = table.delegate();
            let row_ix = delegate.row_index_by_path(path)?;
            delegate.row(row_ix)?.clone()
        };

        let kind = Self::remote_row_kind(&row);
        let path = row.path.clone();

        Some(SftpEntry {
            filename: if row.name.as_ref().is_empty() {
                SftpService::remote_file_name(&path)
            } else {
                row.name.as_ref().to_string()
            },
            path,
            kind,
            size: row.size,
            modified: row.modified,
            attributes: row.attributes.map(|value| value.to_string()),
            owner: row.owner.map(|value| value.to_string()),
        })
    }

    pub(in crate::ui::shell) fn on_local_sftp_table_event(
        &mut self,
        table_entity: &Entity<TableState<SftpBrowserTableDelegate>>,
        event: &TableEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.sftp_browser_event_tab_id(table_entity, cx) else {
            return;
        };

        match event {
            TableEvent::SelectRow(row_ix) => {
                let modifiers = table_entity.update(cx, |table, _| {
                    table.delegate_mut().take_pending_select_modifiers()
                });
                self.select_sftp_local_row(tab_id, *row_ix, modifiers, cx);
            }
            TableEvent::RightClickedRow(Some(row_ix)) => {
                let Some(row) = self
                    .workspace_forms
                    .sftp_browser
                    .local_table
                    .read(cx)
                    .delegate()
                    .row(*row_ix)
                else {
                    return;
                };

                let clicked_path = PathBuf::from(row.path.as_str());
                let keep_existing_selection = self
                    .workspace_state
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id)
                    .and_then(TabState::as_sftp)
                    .is_some_and(|sftp| {
                        sftp.selected_local_paths
                            .iter()
                            .any(|selected| selected == &clicked_path)
                    });

                if !keep_existing_selection {
                    self.select_sftp_local_path(tab_id, clicked_path, cx);
                }
            }
            TableEvent::DoubleClickedRow(row_ix) => {
                let Some((path, is_directory)) = self
                    .workspace_forms
                    .sftp_browser
                    .local_table
                    .read(cx)
                    .delegate()
                    .row(*row_ix)
                    .map(|row| (PathBuf::from(row.path.as_str()), row.is_directory))
                else {
                    return;
                };
                self.select_sftp_local_path(tab_id, path.clone(), cx);
                if is_directory {
                    self.navigate_sftp_local_into_selected(tab_id, cx);
                } else {
                    self.queue_sftp_upload_path(tab_id, path, cx);
                }
            }
            TableEvent::ClearSelection => {
                table_entity.update(cx, |table, cx| {
                    table.delegate_mut().set_selected_paths(Vec::new(), None);
                    table.set_right_clicked_row(None, cx);
                });
                self.clear_sftp_local_selection(tab_id, cx);
            }
            _ => {}
        }
    }

    pub(in crate::ui::shell) fn on_remote_sftp_table_event(
        &mut self,
        table_entity: &Entity<TableState<SftpBrowserTableDelegate>>,
        event: &TableEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.sftp_browser_event_tab_id(table_entity, cx) else {
            return;
        };

        match event {
            TableEvent::SelectRow(row_ix) => {
                let modifiers = table_entity.update(cx, |table, _| {
                    table.delegate_mut().take_pending_select_modifiers()
                });
                self.select_sftp_remote_row(tab_id, *row_ix, modifiers, cx);
            }
            TableEvent::RightClickedRow(Some(row_ix)) => {
                let Some(row) = self
                    .workspace_forms
                    .sftp_browser
                    .remote_table
                    .read(cx)
                    .delegate()
                    .row(*row_ix)
                else {
                    return;
                };

                let clicked_path = row.path.clone();
                let keep_existing_selection = self
                    .workspace_state
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id)
                    .and_then(TabState::as_sftp)
                    .is_some_and(|sftp| {
                        sftp.selected_remote_paths
                            .iter()
                            .any(|selected| selected == &clicked_path)
                    });

                if !keep_existing_selection {
                    self.select_sftp_remote_path(tab_id, clicked_path, cx);
                }
            }
            TableEvent::DoubleClickedRow(row_ix) => {
                let Some((remote_path, is_directory)) = self
                    .workspace_forms
                    .sftp_browser
                    .remote_table
                    .read(cx)
                    .delegate()
                    .row(*row_ix)
                    .map(|row| (row.path.clone(), row.is_directory))
                else {
                    return;
                };
                self.select_sftp_remote_path(tab_id, remote_path.clone(), cx);
                if is_directory {
                    self.navigate_sftp_remote_into_selected(tab_id, cx);
                } else {
                    self.queue_sftp_download_path(tab_id, remote_path, cx);
                }
            }
            TableEvent::ClearSelection => {
                table_entity.update(cx, |table, cx| {
                    table.delegate_mut().set_selected_paths(Vec::new(), None);
                    table.set_right_clicked_row(None, cx);
                });
                self.clear_sftp_remote_selection(tab_id, cx);
            }
            _ => {}
        }
    }

    fn local_sftp_attributes(metadata: &std::fs::Metadata) -> Option<String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            return Some(format!("{:o}", metadata.permissions().mode() & 0o777));
        }

        #[cfg(not(unix))]
        {
            Some(if metadata.permissions().readonly() {
                i18n::string("sftp.attributes.readonly")
            } else {
                i18n::string("sftp.attributes.read_write")
            })
        }
    }

    fn local_sftp_owner(metadata: &std::fs::Metadata) -> Option<String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            return Some(format!("{}:{}", metadata.uid(), metadata.gid()));
        }

        #[cfg(not(unix))]
        {
            let _ = metadata;
            None
        }
    }

    pub(in crate::ui::shell) fn open_sftp_tab(
        &mut self,
        profile: SessionProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.profile_requires_local_vault_unlock(&profile) {
            self.prompt_local_vault_unlock_in_window(window, cx);
            return;
        }

        let tab_id = {
            let next_id = self.workspace_state.next_tab_id;
            self.workspace_state.next_tab_id += 1;
            next_id
        };
        let tab = TabState::new_sftp(tab_id, &profile);

        self.unload_active_topbar_workspace(cx);
        self.workspace_state.tabs.push(tab);
        let index = self.workspace_state.tabs.len() - 1;
        let connection = self
            .sftp_service()
            .start_session(profile.clone(), self.data.sessions.clone());
        if let Some(sftp) = self
            .workspace_state
            .tabs
            .get_mut(index)
            .and_then(TabState::as_sftp_mut)
        {
            sftp.commands = Some(connection.commands);
        }
        self.workspace_state.active_topbar_tab = Some(index);
        self.reset_sftp_path_editing();
        self.reset_loaded_workspace(cx);
        self.rebind_terminal_focus_reporting(window, cx);
        self.sync_terminal_focus_reporting(window, cx);
        self.panel_view.sidebar_section = SidebarSection::Hosts;
        self.editors.host_editor_open = false;
        self.editors.host_editor_is_new = false;
        self.refresh_sftp_local_directory(tab_id, cx);
        self.spawn_sftp_event_loop(tab_id, connection.events, cx);
        self.sync_sftp_path_inputs_for_tab(tab_id, cx);
        self.sync_sftp_tables_for_tab(tab_id, cx);
        self.status_message = i18n::string_args(
            "sftp.messages.opened_tab_for",
            &[("profile", &profile.name)],
        );
        cx.notify();
    }

    pub(in crate::ui::shell) fn spawn_sftp_event_loop(
        &self,
        tab_id: usize,
        mut events: FuturesUnboundedReceiver<SftpEvent>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this
                    .update(cx, |this, cx| this.handle_sftp_event(tab_id, event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn handle_sftp_event(&mut self, tab_id: usize, event: SftpEvent, cx: &mut Context<Self>) {
        let Some(tab_index) = self
            .workspace_state
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
        else {
            return;
        };
        let is_active_tab = self
            .workspace_state
            .active_topbar_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .map(|tab| tab.id)
            == Some(tab_id);
        let is_visible_browser_tab = is_active_tab || self.should_sync_sftp_browser_for_tab(tab_id);
        let remote_path_submit_pending =
            is_visible_browser_tab && self.workspace_forms.sftp_browser.remote_path_submit_pending;

        let tab = &mut self.workspace_state.tabs[tab_index];
        let TabState { status, kind, .. } = tab;
        let TabKind::Sftp(sftp) = kind else {
            return;
        };
        let mut should_sync_paths = false;
        let mut refresh_local_directory = false;
        let mut refresh_remote_directory = None;
        let mut subdirectory_listing: Option<(String, Vec<SftpEntry>)> = None;
        let mut edit_complete: Option<(PathBuf, String)> = None;
        let mut validation_notification = None;
        let mut download_done_filename: Option<String> = None;
        let mut transfer_failed_notification: Option<String> = None;
        let mut open_global_progress_center = false;
        let mut remote_table_loading_finished = false;
        let mut clear_remote_table_loading = false;
        let mut failed_remote_expand_path = None;

        match event {
            SftpEvent::Status(message) => {
                *status = message.clone();
                sftp.last_status = message;
                sftp.last_error = None;
            }
            SftpEvent::DirectoryListing { path, entries } => {
                *status = i18n::string_args("sftp.ui.remote_path_label", &[("path", &path)]);
                sftp.remote_path = path;
                sftp.remote_entries = entries;
                sftp.selected_remote_path = None;
                sftp.selected_remote_paths.clear();
                sftp.remote_selection_anchor = None;
                sftp.loading_remote = false;
                sftp.last_error = None;
                let item_count = sftp.remote_entries.len().to_string();
                sftp.last_status = i18n::string_args(
                    "sftp.messages.loaded_remote_items",
                    &[("count", &item_count)],
                );
                sftp.remote_drag_selection = None;
                sftp.suppress_remote_clear_click = false;
                should_sync_paths = true;
            }
            SftpEvent::TransferQueued {
                transfer_id,
                direction,
                source,
                destination,
            } => {
                sftp.transfers.insert(
                    0,
                    SftpTransferRow {
                        transfer_id,
                        direction,
                        source,
                        destination,
                        bytes_complete: 0,
                        bytes_total: None,
                        status: SftpTransferStatus::Queued,
                        bytes_per_second: None,
                        last_progress_at: None,
                        last_bytes_complete: 0,
                    },
                );
                open_global_progress_center = true;
                let transfer_id = transfer_id.0.to_string();
                sftp.last_status =
                    i18n::string_args("sftp.messages.transfer_queued", &[("id", &transfer_id)]);
            }
            SftpEvent::TransferProgress {
                transfer_id,
                bytes_complete,
                bytes_total,
            } => {
                if let Some(transfer) = sftp
                    .transfers
                    .iter_mut()
                    .find(|transfer| transfer.transfer_id == transfer_id)
                {
                    let now = std::time::Instant::now();
                    // Only recompute speed when at least 500 ms have elapsed since the last
                    // sample point. Without this guard, events that queue up in the channel
                    // would be processed back-to-back with microsecond-level elapsed times,
                    // inflating the estimated speed by orders of magnitude.
                    if let Some(sample_at) = transfer.last_progress_at {
                        let elapsed = now.duration_since(sample_at).as_secs_f64();
                        if elapsed >= 0.5 {
                            let delta = bytes_complete.saturating_sub(transfer.last_bytes_complete);
                            transfer.bytes_per_second = Some((delta as f64 / elapsed) as u64);
                            transfer.last_progress_at = Some(now);
                            transfer.last_bytes_complete = bytes_complete;
                        }
                    } else {
                        transfer.last_progress_at = Some(now);
                        transfer.last_bytes_complete = bytes_complete;
                    }
                    transfer.bytes_complete = bytes_complete;
                    transfer.bytes_total = bytes_total;
                    if !matches!(transfer.status, SftpTransferStatus::Paused) {
                        transfer.status = SftpTransferStatus::Running;
                    }
                }
            }
            SftpEvent::TransferPaused { transfer_id } => {
                if let Some(transfer) = sftp
                    .transfers
                    .iter_mut()
                    .find(|transfer| transfer.transfer_id == transfer_id)
                {
                    transfer.status = SftpTransferStatus::Paused;
                    transfer.bytes_per_second = None;
                    transfer.last_progress_at = None;
                }
                let transfer_id = transfer_id.0.to_string();
                sftp.last_status =
                    i18n::string_args("sftp.messages.transfer_paused", &[("id", &transfer_id)]);
            }
            SftpEvent::TransferResumed { transfer_id } => {
                if let Some(transfer) = sftp
                    .transfers
                    .iter_mut()
                    .find(|transfer| transfer.transfer_id == transfer_id)
                {
                    transfer.status = SftpTransferStatus::Running;
                }
                let transfer_id = transfer_id.0.to_string();
                sftp.last_status =
                    i18n::string_args("sftp.messages.transfer_resumed", &[("id", &transfer_id)]);
            }
            SftpEvent::TransferDone { transfer_id } => {
                // Check if this download was initiated by the "Edit" action before
                // mutating transfer state, so we can skip the local refresh.
                let edit_remote_path = sftp.edit_pending_downloads.remove(&transfer_id);

                if let Some(transfer) = sftp
                    .transfers
                    .iter_mut()
                    .find(|transfer| transfer.transfer_id == transfer_id)
                {
                    transfer.status = SftpTransferStatus::Done;
                    transfer.bytes_per_second = None;
                    transfer.last_progress_at = None;
                    if let Some(total) = transfer.bytes_total {
                        transfer.bytes_complete = total;
                    }
                    match transfer.direction {
                        TransferDirection::Upload => {
                            refresh_remote_directory = Some(sftp.remote_path.clone());
                        }
                        TransferDirection::Download => {
                            // Only refresh the local directory for ordinary downloads.
                            // Edit-initiated downloads land in the system temp dir,
                            // not the current local path, so no refresh is needed.
                            if edit_remote_path.is_none() {
                                refresh_local_directory = true;
                                download_done_filename = Some(
                                    transfer
                                        .source
                                        .file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| transfer.destination.clone()),
                                );
                            }
                        }
                    }
                }
                let transfer_id = transfer_id.0.to_string();
                sftp.last_status =
                    i18n::string_args("sftp.messages.transfer_finished", &[("id", &transfer_id)]);

                if let Some(remote_path) = edit_remote_path {
                    let temp_path = std::env::temp_dir()
                        .join("miaominal_edit")
                        .join(tab_id.to_string())
                        .join(
                            std::path::Path::new(&remote_path)
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "file".into()),
                        );
                    edit_complete = Some((temp_path, remote_path));
                }
            }
            SftpEvent::TransferCancelled { transfer_id } => {
                if let Some(transfer) = sftp
                    .transfers
                    .iter_mut()
                    .find(|transfer| transfer.transfer_id == transfer_id)
                {
                    transfer.status = SftpTransferStatus::Cancelled;
                    transfer.bytes_per_second = None;
                    transfer.last_progress_at = None;
                }
                let transfer_id = transfer_id.0.to_string();
                sftp.last_status =
                    i18n::string_args("sftp.messages.transfer_cancelled", &[("id", &transfer_id)]);
            }
            SftpEvent::TransferFailed {
                transfer_id,
                message,
            } => {
                if let Some(transfer) = sftp
                    .transfers
                    .iter_mut()
                    .find(|transfer| transfer.transfer_id == transfer_id)
                {
                    transfer.status = SftpTransferStatus::Failed(message.clone());
                }
                *status = i18n::string("session.status.error");
                sftp.last_error = Some(message.clone());
                let transfer_id = transfer_id.0.to_string();
                sftp.last_status =
                    i18n::string_args("sftp.messages.transfer_failed", &[("id", &transfer_id)]);
                transfer_failed_notification = Some(message.clone());
                self.status_message = message;
            }
            SftpEvent::Error {
                context,
                path,
                message,
            } => {
                *status = i18n::string("session.status.error");
                if context == "list_directory" {
                    sftp.loading_remote = false;
                    remote_table_loading_finished = true;
                } else if context == "list_subdirectory" {
                    failed_remote_expand_path = path;
                }
                sftp.last_error = Some(format!("{context}: {message}"));
                sftp.last_status =
                    i18n::string_args("sftp.messages.context_failed", &[("context", &context)]);
                let notification_message = i18n::string_args(
                    "sftp.messages.operation_failed",
                    &[("context", &context), ("message", &message)],
                );
                self.status_message = notification_message.clone();
                if remote_path_submit_pending {
                    validation_notification = Some(notification_message);
                }
            }
            SftpEvent::Closed => {
                *status = i18n::string("session.status.closed");
                sftp.commands = None;
                sftp.loading_remote = false;
                sftp.last_status = i18n::string("sftp.messages.session_closed");
                clear_remote_table_loading = true;
            }
            SftpEvent::SubdirectoryListing {
                parent_path,
                entries,
            } => {
                if sftp
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.starts_with("list_subdirectory:"))
                {
                    *status = i18n::string_args(
                        "sftp.ui.remote_path_label",
                        &[("path", &sftp.remote_path)],
                    );
                    sftp.last_error = None;
                    let item_count = entries.len().to_string();
                    sftp.last_status = i18n::string_args(
                        "sftp.messages.loaded_remote_items",
                        &[("count", &item_count)],
                    );
                }
                subdirectory_listing = Some((parent_path, entries));
            }
        }

        if is_visible_browser_tab
            && (remote_table_loading_finished
                || clear_remote_table_loading
                || failed_remote_expand_path.is_some())
        {
            let remote_table = self.workspace_forms.sftp_browser.remote_table.clone();
            remote_table.update(cx, |table, cx| {
                if table.delegate().tab_id() != Some(tab_id) {
                    return;
                }

                if clear_remote_table_loading {
                    table.delegate_mut().cancel_all_loading();
                } else {
                    if remote_table_loading_finished {
                        table.delegate_mut().set_loading(false);
                    }
                    if let Some(path) = failed_remote_expand_path.as_deref() {
                        table.delegate_mut().cancel_expand(path);
                    }
                }
                table.refresh(cx);
            });
        }

        if should_sync_paths {
            self.sync_sftp_path_inputs_for_tab(tab_id, cx);
            self.sync_sftp_tables_for_tab(tab_id, cx);
            if is_visible_browser_tab {
                self.workspace_forms.sftp_browser.remote_path_editing = false;
                self.workspace_forms.sftp_browser.remote_path_submit_pending = false;
            }
        }

        if open_global_progress_center {
            self.apply_sftp_progress_center_visibility(true);
        }

        if refresh_local_directory {
            self.refresh_sftp_local_directory(tab_id, cx);
        }

        if let Some(path) = refresh_remote_directory {
            self.request_sftp_remote_directory(tab_id, path, cx);
        }

        if let Some((parent_path, entries)) = subdirectory_listing {
            self.receive_sftp_subdirectory_listing(parent_path, entries, cx);
        }

        if let Some((temp_path, remote_path)) = edit_complete {
            self.on_edit_download_complete(tab_id, temp_path, remote_path, cx);
        }

        if let Some(message) = validation_notification {
            self.workspace_forms.sftp_browser.remote_path_submit_pending = false;
            self.notify_validation_failure(ValidationNotificationKind::InvalidInput, message, cx);
            return;
        }

        if let Some(filename) = download_done_filename {
            let title = i18n::string("sftp.notifications.download_complete_title");
            let body = i18n::string_args(
                "sftp.notifications.download_complete_message",
                &[("filename", &filename)],
            );
            let notification = Self::success_notification(title, body);
            self.with_active_window(cx, move |window, cx| {
                window.push_notification(notification, cx);
            });
        }

        if let Some(message) = transfer_failed_notification {
            let title = i18n::string("sftp.notifications.transfer_failed_title");
            let notification = Self::error_notification(title, message);
            self.with_active_window(cx, move |window, cx| {
                window.push_notification(notification, cx);
            });
        }

        cx.notify();
    }

    pub(in crate::ui::shell) fn expand_sftp_directory(
        &mut self,
        tab_id: usize,
        side: SftpBrowserSide,
        path: String,
        cx: &mut Context<Self>,
    ) {
        match side {
            SftpBrowserSide::Local => {
                let path_buf = std::path::PathBuf::from(&path);
                let children = match Self::read_local_sftp_entries(&path_buf) {
                    Ok(entries) => entries
                        .iter()
                        .map(SftpBrowserTableRow::from_local)
                        .collect::<Vec<_>>(),
                    Err(error) => {
                        let error = error.to_string();
                        self.status_message = i18n::string_args(
                            "status.sftp.expand_local_failed",
                            &[("path", &path), ("error", &error)],
                        );
                        cx.notify();
                        self.workspace_forms
                            .sftp_browser
                            .local_table
                            .update(cx, |table, cx| {
                                table.delegate_mut().cancel_expand(&path);
                                cx.notify();
                            });
                        return;
                    }
                };
                self.workspace_forms
                    .sftp_browser
                    .local_table
                    .update(cx, |table, cx| {
                        table.delegate_mut().receive_children(path, children, cx);
                    });
            }
            SftpBrowserSide::Remote => {
                let Some(sftp) = self
                    .workspace_state
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id)
                    .and_then(TabState::as_sftp)
                else {
                    return;
                };
                if let Some(commands) = sftp.commands.as_ref()
                    && let Err(error) = commands.list_subdirectory(&path)
                {
                    let error = error.to_string();
                    self.status_message = i18n::string_args(
                        "status.sftp.expand_remote_failed",
                        &[("path", &path), ("error", &error)],
                    );
                    let remote_table = self.workspace_forms.sftp_browser.remote_table.clone();
                    remote_table.update(cx, |table, cx| {
                        table.delegate_mut().cancel_expand(&path);
                        cx.notify();
                    });
                }
                cx.notify();
            }
        }
    }

    pub(in crate::ui::shell) fn receive_sftp_subdirectory_listing(
        &mut self,
        parent_path: String,
        entries: Vec<SftpEntry>,
        cx: &mut Context<Self>,
    ) {
        let children = entries
            .iter()
            .map(SftpBrowserTableRow::from_remote)
            .collect::<Vec<_>>();
        self.workspace_forms
            .sftp_browser
            .remote_table
            .update(cx, |table, cx| {
                table
                    .delegate_mut()
                    .receive_children(parent_path, children, cx);
            });
    }

    pub(in crate::ui::shell) fn refresh_sftp_local_directory(
        &mut self,
        tab_id: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_index) = self
            .workspace_state
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
        else {
            return;
        };
        let mut should_sync_paths = false;

        {
            let Some(sftp) = self
                .workspace_state
                .tabs
                .get_mut(tab_index)
                .and_then(TabState::as_sftp_mut)
            else {
                return;
            };

            match Self::read_local_sftp_entries(&sftp.local_path) {
                Ok(entries) => {
                    sftp.local_entries = entries;
                    sftp.selected_local_paths.retain(|selected| {
                        sftp.local_entries
                            .iter()
                            .any(|entry| &entry.path == selected)
                    });
                    if let Some(selected) = sftp.selected_local_path.as_ref()
                        && !sftp
                            .local_entries
                            .iter()
                            .any(|entry| &entry.path == selected)
                    {
                        sftp.selected_local_path = None;
                    }
                    if sftp.selected_local_path.is_none() {
                        sftp.selected_local_path = sftp.selected_local_paths.first().cloned();
                    }
                    if let Some(selected) = sftp.selected_local_path.clone()
                        && !sftp
                            .selected_local_paths
                            .iter()
                            .any(|path| path == &selected)
                    {
                        sftp.selected_local_paths.insert(0, selected);
                    }
                    if let Some(anchor) = sftp.local_selection_anchor.as_ref()
                        && !sftp.local_entries.iter().any(|entry| &entry.path == anchor)
                    {
                        sftp.local_selection_anchor = None;
                    }
                    should_sync_paths = true;
                }
                Err(error) => {
                    sftp.last_error = Some(error.to_string());
                    let path = sftp.local_path.display().to_string();
                    let error = error.to_string();
                    self.status_message = i18n::string_args(
                        "sftp.messages.local_read_failed",
                        &[("path", &path), ("error", &error)],
                    );
                }
            }
        }

        if should_sync_paths {
            self.sync_sftp_path_inputs_for_tab(tab_id, cx);
            self.sync_sftp_tables_for_tab(tab_id, cx);
        }

        cx.notify();
    }

    fn read_local_sftp_entries(path: &std::path::Path) -> Result<Vec<LocalSftpEntry>> {
        let mut entries = Vec::new();
        let directory_path = path.display().to_string();
        for entry in std::fs::read_dir(path).with_context(|| {
            i18n::string_args(
                "errors.sftp.local_read.directory",
                &[("path", &directory_path)],
            )
        })? {
            let entry = entry.with_context(|| {
                i18n::string_args("errors.sftp.local_read.entry", &[("path", &directory_path)])
            })?;
            let entry_path = entry.path();
            let entry_path_text = entry_path.display().to_string();
            let metadata = entry.metadata().with_context(|| {
                i18n::string_args(
                    "errors.sftp.local_read.metadata",
                    &[("path", &entry_path_text)],
                )
            })?;
            entries.push(LocalSftpEntry {
                filename: entry.file_name().to_string_lossy().into_owned(),
                is_directory: metadata.is_dir(),
                size: metadata.is_file().then_some(metadata.len()),
                modified: metadata.modified().ok(),
                attributes: Self::local_sftp_attributes(&metadata),
                owner: Self::local_sftp_owner(&metadata),
                path: entry_path,
            });
        }

        entries.sort_by(|left, right| {
            right.is_directory.cmp(&left.is_directory).then_with(|| {
                left.filename
                    .to_lowercase()
                    .cmp(&right.filename.to_lowercase())
            })
        });
        Ok(entries)
    }

    pub(in crate::ui::shell) fn clear_sftp_local_selection(
        &mut self,
        tab_id: usize,
        cx: &mut Context<Self>,
    ) {
        self.set_sftp_local_selection(tab_id, Vec::new(), None, cx);
    }

    pub(in crate::ui::shell) fn clear_sftp_remote_selection(
        &mut self,
        tab_id: usize,
        cx: &mut Context<Self>,
    ) {
        self.set_sftp_remote_selection(tab_id, Vec::new(), None, cx);
    }

    pub(in crate::ui::shell) fn handle_sftp_blank_click(
        &mut self,
        tab_id: usize,
        side: SftpBrowserSide,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        header_height: Pixels,
        cx: &mut Context<Self>,
    ) {
        if position.y <= bounds.origin.y + header_height {
            return;
        }

        let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        else {
            return;
        };

        let suppress_click = match side {
            SftpBrowserSide::Local => &mut sftp.suppress_local_clear_click,
            SftpBrowserSide::Remote => &mut sftp.suppress_remote_clear_click,
        };
        if *suppress_click {
            *suppress_click = false;
            return;
        }

        let has_selection = match side {
            SftpBrowserSide::Local => !sftp.selected_local_paths.is_empty(),
            SftpBrowserSide::Remote => !sftp.selected_remote_paths.is_empty(),
        };

        if !has_selection {
            return;
        }

        match side {
            SftpBrowserSide::Local => self.clear_sftp_local_selection(tab_id, cx),
            SftpBrowserSide::Remote => self.clear_sftp_remote_selection(tab_id, cx),
        }
        self.sync_sftp_selection_for_tab(tab_id, cx);
    }

    fn select_sftp_local_row(
        &mut self,
        tab_id: usize,
        row_ix: usize,
        modifiers: SftpBrowserSelectionModifiers,
        cx: &mut Context<Self>,
    ) {
        let Some(clicked_path) = self
            .workspace_forms
            .sftp_browser
            .local_table
            .read(cx)
            .delegate()
            .row(row_ix)
            .map(|row| PathBuf::from(row.path.as_str()))
        else {
            return;
        };

        if modifiers.shift {
            let (anchor, existing_paths) = self
                .workspace_state
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp)
                .map(|sftp| {
                    (
                        sftp.local_selection_anchor
                            .clone()
                            .or_else(|| sftp.selected_local_path.clone())
                            .unwrap_or_else(|| clicked_path.clone()),
                        sftp.selected_local_paths.clone(),
                    )
                })
                .unwrap_or_else(|| (clicked_path.clone(), Vec::new()));
            let range_paths = self.sftp_local_paths_in_click_range(&anchor, row_ix, cx);
            let mut next_paths = if modifiers.toggle {
                existing_paths
            } else {
                Vec::new()
            };

            for path in range_paths {
                if !next_paths.iter().any(|current| current == &path) {
                    next_paths.push(path);
                }
            }

            self.set_sftp_local_selection(tab_id, next_paths, Some(clicked_path), cx);
            self.set_sftp_local_selection_anchor(tab_id, Some(anchor));
        } else if modifiers.toggle {
            let (mut next_paths, current_primary) = self
                .workspace_state
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp)
                .map(|sftp| {
                    (
                        sftp.selected_local_paths.clone(),
                        sftp.selected_local_path.clone(),
                    )
                })
                .unwrap_or_default();

            let was_selected = next_paths.iter().any(|path| path == &clicked_path);
            if was_selected {
                next_paths.retain(|path| path != &clicked_path);
            } else {
                next_paths.push(clicked_path.clone());
            }

            let next_primary = if next_paths.is_empty() {
                None
            } else if was_selected {
                current_primary
                    .filter(|primary| next_paths.iter().any(|path| path == primary))
                    .or_else(|| next_paths.first().cloned())
            } else {
                Some(clicked_path.clone())
            };
            let next_anchor = (!next_paths.is_empty()).then_some(clicked_path);

            self.set_sftp_local_selection(tab_id, next_paths, next_primary, cx);
            self.set_sftp_local_selection_anchor(tab_id, next_anchor);
        } else {
            self.select_sftp_local_path(tab_id, clicked_path, cx);
        }

        self.sync_sftp_selection_for_side(tab_id, SftpBrowserSide::Local, cx);
    }

    fn select_sftp_remote_row(
        &mut self,
        tab_id: usize,
        row_ix: usize,
        modifiers: SftpBrowserSelectionModifiers,
        cx: &mut Context<Self>,
    ) {
        let Some(clicked_path) = self
            .workspace_forms
            .sftp_browser
            .remote_table
            .read(cx)
            .delegate()
            .row(row_ix)
            .map(|row| row.path.clone())
        else {
            return;
        };

        if modifiers.shift {
            let (anchor, existing_paths) = self
                .workspace_state
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp)
                .map(|sftp| {
                    (
                        sftp.remote_selection_anchor
                            .clone()
                            .or_else(|| sftp.selected_remote_path.clone())
                            .unwrap_or_else(|| clicked_path.clone()),
                        sftp.selected_remote_paths.clone(),
                    )
                })
                .unwrap_or_else(|| (clicked_path.clone(), Vec::new()));
            let range_paths = self.sftp_remote_paths_in_click_range(&anchor, row_ix, cx);
            let mut next_paths = if modifiers.toggle {
                existing_paths
            } else {
                Vec::new()
            };

            for path in range_paths {
                if !next_paths.iter().any(|current| current == &path) {
                    next_paths.push(path);
                }
            }

            self.set_sftp_remote_selection(tab_id, next_paths, Some(clicked_path), cx);
            self.set_sftp_remote_selection_anchor(tab_id, Some(anchor));
        } else if modifiers.toggle {
            let (mut next_paths, current_primary) = self
                .workspace_state
                .tabs
                .iter()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp)
                .map(|sftp| {
                    (
                        sftp.selected_remote_paths.clone(),
                        sftp.selected_remote_path.clone(),
                    )
                })
                .unwrap_or_default();

            let was_selected = next_paths.iter().any(|path| path == &clicked_path);
            if was_selected {
                next_paths.retain(|path| path != &clicked_path);
            } else {
                next_paths.push(clicked_path.clone());
            }

            let next_primary = if next_paths.is_empty() {
                None
            } else if was_selected {
                current_primary
                    .filter(|primary| next_paths.iter().any(|path| path == primary))
                    .or_else(|| next_paths.first().cloned())
            } else {
                Some(clicked_path.clone())
            };
            let next_anchor = (!next_paths.is_empty()).then_some(clicked_path);

            self.set_sftp_remote_selection(tab_id, next_paths, next_primary, cx);
            self.set_sftp_remote_selection_anchor(tab_id, next_anchor);
        } else {
            self.select_sftp_remote_path(tab_id, clicked_path, cx);
        }

        self.sync_sftp_selection_for_side(tab_id, SftpBrowserSide::Remote, cx);
    }

    fn sftp_local_paths_in_click_range(
        &self,
        anchor: &std::path::Path,
        row_ix: usize,
        cx: &App,
    ) -> Vec<PathBuf> {
        let table = self.workspace_forms.sftp_browser.local_table.read(cx);
        let delegate = table.delegate();
        let anchor_key = anchor.display().to_string();
        let anchor_ix = delegate.row_index_by_path(&anchor_key).unwrap_or(row_ix);

        delegate
            .paths_in_row_range(anchor_ix, row_ix)
            .into_iter()
            .map(PathBuf::from)
            .collect()
    }

    fn sftp_remote_paths_in_click_range(
        &self,
        anchor: &str,
        row_ix: usize,
        cx: &App,
    ) -> Vec<String> {
        let table = self.workspace_forms.sftp_browser.remote_table.read(cx);
        let delegate = table.delegate();
        let anchor_ix = delegate.row_index_by_path(anchor).unwrap_or(row_ix);

        delegate.paths_in_row_range(anchor_ix, row_ix)
    }

    fn set_sftp_local_selection_anchor(&mut self, tab_id: usize, anchor: Option<PathBuf>) {
        if let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        {
            sftp.local_selection_anchor = anchor;
        }
    }

    fn set_sftp_remote_selection_anchor(&mut self, tab_id: usize, anchor: Option<String>) {
        if let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        {
            sftp.remote_selection_anchor = anchor;
        }
    }

    pub(in crate::ui::shell) fn set_sftp_local_selection(
        &mut self,
        tab_id: usize,
        paths: Vec<PathBuf>,
        primary: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        else {
            return;
        };

        let mut unique_paths = Vec::new();
        for path in paths {
            if !unique_paths.iter().any(|current| current == &path) {
                unique_paths.push(path);
            }
        }

        let primary = primary.or_else(|| unique_paths.first().cloned());
        if let Some(primary_path) = primary.clone()
            && !unique_paths.iter().any(|path| path == &primary_path)
        {
            unique_paths.insert(0, primary_path.clone());
        }

        if unique_paths.is_empty() {
            sftp.local_selection_anchor = None;
        }
        if sftp.selected_local_path == primary && sftp.selected_local_paths == unique_paths {
            return;
        }

        sftp.selected_local_path = primary;
        sftp.selected_local_paths = unique_paths;
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_sftp_remote_selection(
        &mut self,
        tab_id: usize,
        paths: Vec<String>,
        primary: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        else {
            return;
        };

        let mut unique_paths = Vec::new();
        for path in paths {
            if !unique_paths.iter().any(|current| current == &path) {
                unique_paths.push(path);
            }
        }

        let primary = primary.or_else(|| unique_paths.first().cloned());
        if let Some(primary_path) = primary.clone()
            && !unique_paths.iter().any(|path| path == &primary_path)
        {
            unique_paths.insert(0, primary_path.clone());
        }

        if unique_paths.is_empty() {
            sftp.remote_selection_anchor = None;
        }
        if sftp.selected_remote_path == primary && sftp.selected_remote_paths == unique_paths {
            return;
        }

        sftp.selected_remote_path = primary;
        sftp.selected_remote_paths = unique_paths;
        cx.notify();
    }

    pub(in crate::ui::shell) fn select_sftp_local_path(
        &mut self,
        tab_id: usize,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.set_sftp_local_selection(tab_id, vec![path.clone()], Some(path.clone()), cx);
        self.set_sftp_local_selection_anchor(tab_id, Some(path));
    }

    pub(in crate::ui::shell) fn select_sftp_remote_path(
        &mut self,
        tab_id: usize,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.set_sftp_remote_selection(tab_id, vec![path.clone()], Some(path.clone()), cx);
        self.set_sftp_remote_selection_anchor(tab_id, Some(path));
    }

    pub(in crate::ui::shell) fn set_sftp_local_path_editing(
        &mut self,
        editing: bool,
        cx: &mut Context<Self>,
    ) {
        if self.workspace_forms.sftp_browser.local_path_editing != editing {
            self.workspace_forms.sftp_browser.local_path_editing = editing;
            cx.notify();
            if editing {
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(0))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        let input = this.workspace_forms.sftp_browser.local_path_input.clone();
                        this.with_active_window(cx, move |window, cx| {
                            input.update(cx, |input, cx| {
                                input.focus(window, cx);
                            });
                        });
                    });
                })
                .detach();
            }
        }
    }

    pub(in crate::ui::shell) fn set_sftp_remote_path_editing(
        &mut self,
        editing: bool,
        cx: &mut Context<Self>,
    ) {
        if self.workspace_forms.sftp_browser.remote_path_editing != editing {
            self.workspace_forms.sftp_browser.remote_path_editing = editing;
            cx.notify();
            if editing {
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(0))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        let input = this.workspace_forms.sftp_browser.remote_path_input.clone();
                        this.with_active_window(cx, move |window, cx| {
                            input.update(cx, |input, cx| {
                                input.focus(window, cx);
                            });
                        });
                    });
                })
                .detach();
            }
        }
    }

    pub(in crate::ui::shell) fn reset_sftp_path_editing(&mut self) {
        self.workspace_forms.sftp_browser.local_path_editing = false;
        self.workspace_forms.sftp_browser.remote_path_editing = false;
        self.workspace_forms.sftp_browser.remote_path_submit_pending = false;
    }

    pub(in crate::ui::shell) fn navigate_sftp_local_to_path(
        &mut self,
        tab_id: usize,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let normalized = path.canonicalize().unwrap_or(path);

        if let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        {
            sftp.local_path = normalized;
            sftp.selected_local_path = None;
            sftp.selected_local_paths.clear();
        }

        self.refresh_sftp_local_directory(tab_id, cx);
    }

    pub(in crate::ui::shell) fn navigate_sftp_local_into_selected(
        &mut self,
        tab_id: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_index) = self
            .workspace_state
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
        else {
            return;
        };
        let next_path = self
            .workspace_state
            .tabs
            .get(tab_index)
            .and_then(TabState::as_sftp)
            .and_then(|sftp| sftp.selected_local_path.clone());
        let Some(next_path) = next_path else {
            return;
        };
        if !next_path.is_dir() {
            return;
        }

        self.navigate_sftp_local_to_path(tab_id, next_path, cx);
    }

    pub(in crate::ui::shell) fn navigate_sftp_local_up(
        &mut self,
        tab_id: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_index) = self
            .workspace_state
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
        else {
            return;
        };
        let next_path = self
            .workspace_state
            .tabs
            .get(tab_index)
            .and_then(TabState::as_sftp)
            .and_then(|sftp| sftp.local_path.parent().map(|path| path.to_path_buf()));
        let Some(next_path) = next_path else {
            return;
        };

        self.navigate_sftp_local_to_path(tab_id, next_path, cx);
    }

    pub(in crate::ui::shell) fn request_sftp_remote_directory(
        &mut self,
        tab_id: usize,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.request_sftp_remote_directory_with_source(tab_id, path, false, cx);
    }

    fn request_sftp_remote_directory_with_source(
        &mut self,
        tab_id: usize,
        path: String,
        from_path_input: bool,
        cx: &mut Context<Self>,
    ) {
        self.workspace_forms.sftp_browser.remote_path_submit_pending = from_path_input;
        let mut validation_message = None;

        let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        else {
            return;
        };

        sftp.loading_remote = true;
        sftp.last_error = None;
        if let Some(commands) = sftp.commands.as_ref()
            && let Err(error) = commands.list_directory(path)
        {
            sftp.loading_remote = false;
            sftp.last_error = Some(error.to_string());
            let error = error.to_string();
            let message = i18n::string_args("sftp.messages.refresh_failed", &[("error", &error)]);
            self.status_message = message.clone();
            if from_path_input {
                validation_message = Some(message);
            }
        }
        let remote_loading = sftp.loading_remote;
        if self.should_sync_sftp_browser_for_tab(tab_id) {
            let remote_table = self.workspace_forms.sftp_browser.remote_table.clone();
            remote_table.update(cx, |table, cx| {
                if table.delegate().tab_id() == Some(tab_id) {
                    table.delegate_mut().set_loading(remote_loading);
                    table.refresh(cx);
                }
            });
        }
        if let Some(message) = validation_message {
            self.workspace_forms.sftp_browser.remote_path_submit_pending = false;
            self.notify_validation_failure(ValidationNotificationKind::InvalidInput, message, cx);
        } else {
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn commit_sftp_local_path_input(&mut self, cx: &mut Context<Self>) {
        let value = self
            .workspace_forms
            .sftp_browser
            .local_path_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let Some((tab_id, current_path)) = self
            .active_or_browser_sftp_tab_id(cx)
            .and_then(|tab_id| {
                self.workspace_state
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id)
            })
            .and_then(|tab| tab.as_sftp().map(|sftp| (tab.id, sftp.local_path.clone())))
        else {
            return;
        };

        let next_path = if value.is_empty() {
            current_path
        } else {
            let candidate = PathBuf::from(&value);
            if candidate.is_absolute() {
                candidate
            } else {
                current_path.join(candidate)
            }
        };

        if !next_path.exists() || !next_path.is_dir() {
            let path = next_path.display().to_string();
            let message =
                i18n::string_args("sftp.messages.local_path_not_directory", &[("path", &path)]);
            self.notify_validation_failure(ValidationNotificationKind::InvalidInput, message, cx);
            return;
        }

        let normalized = next_path.canonicalize().unwrap_or(next_path);
        self.workspace_forms.sftp_browser.local_path_editing = false;
        self.navigate_sftp_local_to_path(tab_id, normalized, cx);
    }

    pub(in crate::ui::shell) fn commit_sftp_remote_path_input(&mut self, cx: &mut Context<Self>) {
        let value = self
            .workspace_forms
            .sftp_browser
            .remote_path_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let Some((tab_id, current_path)) = self
            .active_or_browser_sftp_tab_id(cx)
            .and_then(|tab_id| {
                self.workspace_state
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id)
            })
            .and_then(|tab| tab.as_sftp().map(|sftp| (tab.id, sftp.remote_path.clone())))
        else {
            return;
        };

        let next_path = if value.is_empty() {
            current_path
        } else if value.starts_with('/') {
            value
        } else {
            Self::join_remote_path(&current_path, &value)
        };

        self.request_sftp_remote_directory_with_source(tab_id, next_path, true, cx);
    }

    pub(in crate::ui::shell) fn navigate_sftp_remote_into_selected(
        &mut self,
        tab_id: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(selected_path) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .and_then(|sftp| sftp.selected_remote_path.clone())
        else {
            return;
        };

        let Some(entry) = self.resolve_remote_sftp_entry(tab_id, &selected_path, cx) else {
            return;
        };

        if entry.kind == miaominal_sftp::SftpEntryKind::Directory {
            self.request_sftp_remote_directory(tab_id, entry.path, cx);
        }
    }

    pub(in crate::ui::shell) fn navigate_sftp_remote_up(
        &mut self,
        tab_id: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(current_path) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .map(|sftp| sftp.remote_path.clone())
        else {
            return;
        };
        let next_path = Self::remote_parent_path(&current_path);
        self.request_sftp_remote_directory(tab_id, next_path, cx);
    }

    pub(in crate::ui::shell) fn begin_sftp_rename_selected(
        &mut self,
        tab_id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selected_path) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .and_then(|sftp| {
                if sftp.selected_remote_paths.len() != 1 {
                    return None;
                }
                sftp.selected_remote_path.clone()
            })
        else {
            self.status_message = i18n::string("status.sftp.rename_requires_single_remote_entry");
            cx.notify();
            return;
        };

        let Some(entry) = self.resolve_remote_sftp_entry(tab_id, &selected_path, cx) else {
            self.status_message = i18n::string("status.sftp.rename_requires_single_remote_entry");
            cx.notify();
            return;
        };

        let from = entry.path.clone();
        let filename = entry.filename.clone();
        let parent = Self::remote_parent_path(&entry.path);

        let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        else {
            return;
        };

        sftp.inline_rename = Some(InlineRenameState {
            from: from.clone(),
            parent,
        });
        set_input_value(
            &self.workspace_forms.sftp_browser.inline_rename_input,
            filename,
            window,
            cx,
        );
        self.workspace_forms
            .sftp_browser
            .remote_table
            .update(cx, |table, cx| {
                table.delegate_mut().inline_rename_path = Some(from);
                table.refresh(cx);
            });
        let inline_rename_input = self
            .workspace_forms
            .sftp_browser
            .inline_rename_input
            .clone();
        inline_rename_input.update(cx, |input, cx| {
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn commit_sftp_inline_rename(&mut self, cx: &mut Context<Self>) {
        let Some((tab_id, commands, rename_state)) = self
            .active_or_browser_sftp_tab_id(cx)
            .and_then(|tab_id| {
                self.workspace_state
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id)
            })
            .and_then(|tab| {
                let sftp = tab.as_sftp()?;
                let rename_state = sftp.inline_rename.clone()?;
                Some((tab.id, sftp.commands.clone()?, rename_state))
            })
        else {
            return;
        };

        let value = self
            .workspace_forms
            .sftp_browser
            .inline_rename_input
            .read(cx)
            .value()
            .trim()
            .to_string();

        if value.is_empty() {
            self.notify_validation_failure(
                ValidationNotificationKind::RequiredInputMissing,
                i18n::string("errors.sftp.validation.name_required"),
                cx,
            );
            return;
        }

        let to = Self::join_remote_path(&rename_state.parent, &value);
        if rename_state.from == to {
            if let Some(sftp) = self
                .workspace_state
                .tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp_mut)
            {
                sftp.inline_rename = None;
            }
            self.workspace_forms
                .sftp_browser
                .remote_table
                .update(cx, |table, cx| {
                    table.delegate_mut().inline_rename_path = None;
                    table.refresh(cx);
                });
            self.status_message = i18n::string("sftp.messages.name_unchanged");
            cx.notify();
            return;
        }

        let from = rename_state.from.clone();
        let parent = rename_state.parent.clone();
        match commands.rename(from.clone(), to.clone()) {
            Ok(_) => {
                if let Some(sftp) = self
                    .workspace_state
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id == tab_id)
                    .and_then(TabState::as_sftp_mut)
                {
                    sftp.inline_rename = None;
                }
                self.workspace_forms
                    .sftp_browser
                    .remote_table
                    .update(cx, |table, cx| {
                        table.delegate_mut().inline_rename_path = None;
                        table.refresh(cx);
                    });
                self.status_message =
                    i18n::string_args("sftp.messages.renaming", &[("from", &from), ("to", &to)]);
                self.request_sftp_remote_directory(tab_id, parent, cx);
            }
            Err(error) => {
                let error = error.to_string();
                self.status_message =
                    i18n::string_args("sftp.messages.action_failed", &[("error", &error)]);
                cx.notify();
            }
        }
    }

    pub(in crate::ui::shell) fn cancel_sftp_inline_rename(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.active_or_browser_sftp_tab_id(cx) else {
            return;
        };
        let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        else {
            return;
        };

        if sftp.inline_rename.is_some() {
            sftp.inline_rename = None;
            self.workspace_forms
                .sftp_browser
                .remote_table
                .update(cx, |table, cx| {
                    table.delegate_mut().inline_rename_path = None;
                    table.refresh(cx);
                });
            cx.notify();
        }
    }

    fn remote_parent_path(path: &str) -> String {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() || trimmed == "/" {
            return "/".into();
        }

        match trimmed.rsplit_once('/') {
            Some(("", _)) | None => "/".into(),
            Some((parent, _)) => parent.to_string(),
        }
    }

    fn remote_row_kind(row: &SftpBrowserTableRow) -> miaominal_sftp::SftpEntryKind {
        row.kind
    }

    pub(in crate::ui::shell) fn join_remote_path(base: &str, name: &str) -> String {
        SftpService::join_remote_path(base, name)
    }
}
