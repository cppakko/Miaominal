use super::super::*;
use crate::ui::i18n;
use crate::ui::shell::state::{SftpDragSelectionContext, SftpDragSelectionState};
use gpui_component::{WindowExt as _, scroll::ScrollbarHandle as _};
use miaominal_services::SftpService;

const SFTP_DRAG_SELECTION_THRESHOLD: f32 = 4.0;
const SFTP_DRAG_AUTO_SCROLL_EDGE_ZONE: f32 = 72.0;
const SFTP_DRAG_AUTO_SCROLL_MIN_STEP: f32 = 0.75;
const SFTP_DRAG_AUTO_SCROLL_MAX_STEP: f32 = 14.0;
const SFTP_DRAG_AUTO_SCROLL_MAX_RATIO: f32 = 2.25;
const SFTP_DRAG_AUTO_SCROLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);

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

    fn active_or_browser_sftp_tab_id(&self, cx: &App) -> Option<usize> {
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
                sftp.layout.progress_center_visible = true;
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
            SftpEvent::Error { context, message } => {
                *status = i18n::string("session.status.error");
                sftp.loading_remote = false;
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
            }
            SftpEvent::SubdirectoryListing {
                parent_path,
                entries,
            } => {
                subdirectory_listing = Some((parent_path, entries));
            }
        }

        if should_sync_paths {
            self.sync_sftp_path_inputs_for_tab(tab_id, cx);
            self.sync_sftp_tables_for_tab(tab_id, cx);
            if is_visible_browser_tab {
                self.workspace_forms.sftp_browser.remote_path_editing = false;
                self.workspace_forms.sftp_browser.remote_path_submit_pending = false;
            }
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

    pub(in crate::ui::shell) fn begin_sftp_drag_selection(
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

        let relative_position =
            Point::new(position.x - bounds.origin.x, position.y - bounds.origin.y);
        let scroll_offset = self.sftp_drag_selection_scroll_offset(side, cx);
        let anchor_content_y =
            relative_position.y.as_f32() - header_height.as_f32() + scroll_offset;

        let generation = {
            let Some(sftp) = self
                .workspace_state
                .tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp_mut)
            else {
                return;
            };

            sftp.drag_selection_generation = sftp.drag_selection_generation.wrapping_add(1);
            let generation = sftp.drag_selection_generation;
            sftp.drag_selection_context = Some(SftpDragSelectionContext {
                side,
                tab_id,
                last_position: position,
                panel_bounds: bounds,
                row_height: header_height,
                anchor_content_y,
                generation,
            });

            // Only record the candidate start position. The actual drag state is created lazily
            // in update_sftp_drag_selection once the pointer moves past the threshold, so that
            // a simple click never creates a drag state and never interferes with row selection.
            match side {
                SftpBrowserSide::Local => {
                    sftp.local_drag_candidate = Some(relative_position);
                    sftp.local_drag_selection = None;
                    sftp.suppress_local_clear_click = false;
                }
                SftpBrowserSide::Remote => {
                    sftp.remote_drag_candidate = Some(relative_position);
                    sftp.remote_drag_selection = None;
                    sftp.suppress_remote_clear_click = false;
                }
            }

            generation
        };

        self.start_sftp_drag_selection_auto_scroll(tab_id, generation, cx);
    }

    pub(in crate::ui::shell) fn update_active_sftp_drag_selection(
        &mut self,
        tab_id: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(context) = self.sftp_drag_selection_context_for_tab(tab_id) else {
            return false;
        };

        self.update_sftp_drag_selection(
            context.tab_id,
            context.side,
            position,
            context.panel_bounds,
            context.row_height,
            cx,
        )
    }

    pub(in crate::ui::shell) fn finish_active_sftp_drag_selection(
        &mut self,
        tab_id: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(context) = self.sftp_drag_selection_context_for_tab(tab_id) else {
            return false;
        };

        self.finish_sftp_drag_selection(
            context.tab_id,
            context.side,
            position,
            context.panel_bounds,
            context.row_height,
            cx,
        )
    }

    pub(in crate::ui::shell) fn finish_any_active_sftp_drag_selection(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        let contexts = self
            .workspace_state
            .tabs
            .iter()
            .filter_map(|tab| {
                let context = tab.as_sftp()?.drag_selection_context?;
                (context.tab_id == tab.id).then_some(context)
            })
            .collect::<Vec<_>>();

        if contexts.is_empty() {
            return false;
        }

        for context in contexts {
            self.finish_sftp_drag_selection(
                context.tab_id,
                context.side,
                context.last_position,
                context.panel_bounds,
                context.row_height,
                cx,
            );
        }

        cx.notify();
        true
    }

    fn sftp_drag_selection_context_for_tab(
        &self,
        tab_id: usize,
    ) -> Option<SftpDragSelectionContext> {
        self.workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .and_then(|sftp| sftp.drag_selection_context)
            .filter(|context| context.tab_id == tab_id)
    }

    fn clear_sftp_drag_selection_context(&mut self, tab_id: usize) -> bool {
        let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        else {
            return false;
        };

        let had_context = sftp.drag_selection_context.take().is_some();
        if had_context {
            sftp.drag_selection_generation = sftp.drag_selection_generation.wrapping_add(1);
        }
        had_context
    }

    fn sftp_drag_selection_scroll_offset(&self, side: SftpBrowserSide, cx: &App) -> f32 {
        let offset = match side {
            SftpBrowserSide::Local => {
                self.workspace_forms
                    .sftp_browser
                    .local_table
                    .read(cx)
                    .vertical_scroll_handle
                    .offset()
                    .y
            }
            SftpBrowserSide::Remote => {
                self.workspace_forms
                    .sftp_browser
                    .remote_table
                    .read(cx)
                    .vertical_scroll_handle
                    .offset()
                    .y
            }
        };

        -offset.as_f32()
    }

    fn sftp_drag_selection_relative_position(
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        row_height: Pixels,
    ) -> Point<Pixels> {
        let body_top = bounds.origin.y + row_height;
        let body_bottom = (bounds.origin.y + bounds.size.height).max(body_top);
        let clamped_y = position.y.max(body_top).min(body_bottom);

        Point::new(position.x - bounds.origin.x, clamped_y - bounds.origin.y)
    }

    fn start_sftp_drag_selection_auto_scroll(
        &mut self,
        tab_id: usize,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(SFTP_DRAG_AUTO_SCROLL_INTERVAL)
                    .await;

                let keep_scrolling = this
                    .update(cx, |this, cx| {
                        this.tick_sftp_drag_selection_auto_scroll(tab_id, generation, cx)
                    })
                    .unwrap_or(false);

                if !keep_scrolling {
                    break;
                }
            }
        })
        .detach();
    }

    fn tick_sftp_drag_selection_auto_scroll(
        &mut self,
        tab_id: usize,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(context) = self.sftp_drag_selection_context_for_tab(tab_id) else {
            return false;
        };
        if context.generation != generation {
            return false;
        }

        let selection_active = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .is_some_and(|sftp| match context.side {
                SftpBrowserSide::Local => sftp.local_drag_selection.is_some(),
                SftpBrowserSide::Remote => sftp.remote_drag_selection.is_some(),
            });
        if !selection_active {
            return true;
        }

        let Some(step) = Self::sftp_drag_selection_auto_scroll_step(context) else {
            return true;
        };

        if self.scroll_sftp_drag_selection_table(context, step, cx) {
            self.update_sftp_drag_selection(
                context.tab_id,
                context.side,
                context.last_position,
                context.panel_bounds,
                context.row_height,
                cx,
            );
        }

        true
    }

    fn sftp_drag_selection_auto_scroll_step(context: SftpDragSelectionContext) -> Option<f32> {
        let body_top = (context.panel_bounds.origin.y + context.row_height).as_f32();
        let body_bottom =
            (context.panel_bounds.origin.y + context.panel_bounds.size.height).as_f32();
        if body_bottom <= body_top {
            return None;
        }

        let edge_zone = SFTP_DRAG_AUTO_SCROLL_EDGE_ZONE.min((body_bottom - body_top) / 2.0);
        if edge_zone < 1.0 {
            return None;
        }

        let pointer_y = context.last_position.y.as_f32();
        let hot_top = body_top + edge_zone;
        let hot_bottom = body_bottom - edge_zone;

        let signed_distance = if pointer_y < hot_top {
            -(hot_top - pointer_y)
        } else if pointer_y > hot_bottom {
            pointer_y - hot_bottom
        } else {
            return None;
        };

        let ratio = (signed_distance.abs() / edge_zone).clamp(0.0, SFTP_DRAG_AUTO_SCROLL_MAX_RATIO);
        let eased = ratio.powf(1.2);
        Some(
            (eased * SFTP_DRAG_AUTO_SCROLL_MAX_STEP).max(SFTP_DRAG_AUTO_SCROLL_MIN_STEP)
                * signed_distance.signum(),
        )
    }

    fn scroll_sftp_drag_selection_table(
        &mut self,
        context: SftpDragSelectionContext,
        step: f32,
        cx: &mut Context<Self>,
    ) -> bool {
        match context.side {
            SftpBrowserSide::Local => self
                .workspace_forms
                .sftp_browser
                .local_table
                .update(cx, |table, cx| {
                    Self::scroll_sftp_table_by_step(table, step, cx)
                }),
            SftpBrowserSide::Remote => self
                .workspace_forms
                .sftp_browser
                .remote_table
                .update(cx, |table, cx| {
                    Self::scroll_sftp_table_by_step(table, step, cx)
                }),
        }
    }

    fn scroll_sftp_table_by_step(
        table: &mut TableState<SftpBrowserTableDelegate>,
        step: f32,
        cx: &mut Context<TableState<SftpBrowserTableDelegate>>,
    ) -> bool {
        let current_offset = table.vertical_scroll_handle.offset();
        let max_offset = table
            .vertical_scroll_handle
            .0
            .borrow()
            .base_handle
            .max_offset();
        let next_y = (current_offset.y.as_f32() - step).clamp(-max_offset.y.as_f32(), 0.0);

        if (next_y - current_offset.y.as_f32()).abs() < 0.5 {
            return false;
        }

        table
            .vertical_scroll_handle
            .set_offset(Point::new(current_offset.x, px(next_y)));
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn update_sftp_drag_selection(
        &mut self,
        tab_id: usize,
        side: SftpBrowserSide,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        row_height: Pixels,
        cx: &mut Context<Self>,
    ) -> bool {
        let relative_position =
            Self::sftp_drag_selection_relative_position(position, bounds, row_height);
        let scroll_offset = self.sftp_drag_selection_scroll_offset(side, cx);

        let drag = {
            let Some(sftp) = self
                .workspace_state
                .tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp_mut)
            else {
                return false;
            };

            let anchor_view_y = if let Some(context) = sftp.drag_selection_context.as_mut()
                && context.tab_id == tab_id
                && context.side == side
            {
                context.last_position = position;
                context.panel_bounds = bounds;
                context.row_height = row_height;
                Some(px(
                    context.anchor_content_y - scroll_offset + row_height.as_f32()
                ))
            } else {
                None
            };

            let drag = match side {
                SftpBrowserSide::Local => sftp.local_drag_selection.as_mut(),
                SftpBrowserSide::Remote => sftp.remote_drag_selection.as_mut(),
            };

            if let Some(drag) = drag {
                if let Some(anchor_view_y) = anchor_view_y {
                    drag.start.y = anchor_view_y;
                }
                drag.update(relative_position);
                Some((*drag, false))
            } else {
                let candidate = match side {
                    SftpBrowserSide::Local => sftp.local_drag_candidate,
                    SftpBrowserSide::Remote => sftp.remote_drag_candidate,
                };

                candidate.and_then(|candidate_start| {
                    let mut state = SftpDragSelectionState::new(candidate_start);
                    if let Some(anchor_view_y) = anchor_view_y {
                        state.start.y = anchor_view_y;
                    }
                    state.update(relative_position);
                    state
                        .exceeds_threshold(px(SFTP_DRAG_SELECTION_THRESHOLD))
                        .then(|| {
                            match side {
                                SftpBrowserSide::Local => {
                                    sftp.local_drag_candidate = None;
                                    sftp.local_drag_selection = Some(state);
                                }
                                SftpBrowserSide::Remote => {
                                    sftp.remote_drag_candidate = None;
                                    sftp.remote_drag_selection = Some(state);
                                }
                            }
                            (state, true)
                        })
                })
            }
        };

        let Some((drag, force_selection_update)) = drag else {
            return false;
        };

        self.apply_sftp_drag_selection(tab_id, side, drag, row_height, force_selection_update, cx);
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn finish_sftp_drag_selection(
        &mut self,
        tab_id: usize,
        side: SftpBrowserSide,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        row_height: Pixels,
        cx: &mut Context<Self>,
    ) -> bool {
        let relative_position =
            Self::sftp_drag_selection_relative_position(position, bounds, row_height);
        let scroll_offset = self.sftp_drag_selection_scroll_offset(side, cx);

        let drag = {
            let Some(sftp) = self
                .workspace_state
                .tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp_mut)
            else {
                return false;
            };

            match side {
                SftpBrowserSide::Local => sftp.local_drag_candidate = None,
                SftpBrowserSide::Remote => sftp.remote_drag_candidate = None,
            }

            let mut drag = match side {
                SftpBrowserSide::Local => sftp.local_drag_selection.take(),
                SftpBrowserSide::Remote => sftp.remote_drag_selection.take(),
            };

            if let Some(context) = sftp.drag_selection_context
                && context.tab_id == tab_id
                && context.side == side
                && let Some(drag) = drag.as_mut()
            {
                drag.start.y = px(context.anchor_content_y - scroll_offset + row_height.as_f32());
            }

            drag
        };

        self.clear_sftp_drag_selection_context(tab_id);

        let Some(mut drag) = drag else {
            return false;
        };

        drag.update(relative_position);
        if !drag.exceeds_threshold(px(SFTP_DRAG_SELECTION_THRESHOLD)) {
            cx.notify();
            return false;
        }

        self.apply_sftp_drag_selection(tab_id, side, drag, row_height, true, cx);
        if let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        {
            match side {
                SftpBrowserSide::Local => sftp.suppress_local_clear_click = true,
                SftpBrowserSide::Remote => sftp.suppress_remote_clear_click = true,
            }
        }
        cx.notify();
        true
    }

    fn apply_sftp_drag_selection(
        &mut self,
        tab_id: usize,
        side: SftpBrowserSide,
        drag: SftpDragSelectionState,
        row_height: Pixels,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let (row_range, selected_paths) =
            self.sftp_drag_selection_paths(side, drag, row_height, cx);

        let row_range_changed = {
            let Some(sftp) = self
                .workspace_state
                .tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp_mut)
            else {
                return;
            };

            let drag = match side {
                SftpBrowserSide::Local => sftp.local_drag_selection.as_mut(),
                SftpBrowserSide::Remote => sftp.remote_drag_selection.as_mut(),
            };

            if force {
                if let Some(drag) = drag {
                    drag.set_last_row_range(row_range);
                }
                true
            } else {
                drag.is_some_and(|drag| drag.set_last_row_range(row_range))
            }
        };

        if !row_range_changed {
            return;
        }

        match side {
            SftpBrowserSide::Local => {
                let selected_paths: Vec<PathBuf> =
                    selected_paths.into_iter().map(PathBuf::from).collect();
                let primary = selected_paths.first().cloned();
                self.set_sftp_local_selection(tab_id, selected_paths, primary, cx);
            }
            SftpBrowserSide::Remote => {
                let primary = selected_paths.first().cloned();
                self.set_sftp_remote_selection(tab_id, selected_paths, primary, cx);
            }
        }

        self.sync_sftp_selection_for_side(tab_id, side, cx);
    }

    fn sftp_drag_selection_paths(
        &self,
        side: SftpBrowserSide,
        drag: SftpDragSelectionState,
        row_height: Pixels,
        cx: &App,
    ) -> (Option<(usize, usize)>, Vec<String>) {
        match side {
            SftpBrowserSide::Local => {
                let table = self.workspace_forms.sftp_browser.local_table.read(cx);
                let row_range = Self::sftp_drag_selection_row_range(&table, drag, row_height);
                let selected_paths = row_range
                    .map(|(start, end)| table.delegate().paths_in_row_range(start, end))
                    .unwrap_or_default();
                (row_range, selected_paths)
            }
            SftpBrowserSide::Remote => {
                let table = self.workspace_forms.sftp_browser.remote_table.read(cx);
                let row_range = Self::sftp_drag_selection_row_range(&table, drag, row_height);
                let selected_paths = row_range
                    .map(|(start, end)| table.delegate().paths_in_row_range(start, end))
                    .unwrap_or_default();
                (row_range, selected_paths)
            }
        }
    }

    fn sftp_drag_selection_row_range(
        table: &TableState<SftpBrowserTableDelegate>,
        drag: SftpDragSelectionState,
        row_height: Pixels,
    ) -> Option<(usize, usize)> {
        let row_count = table.delegate().row_count();
        if row_count == 0 || row_height <= px(0.0) {
            return None;
        }

        let row_height_px = row_height.as_f32();
        let bounds = drag.bounds();
        let body_top = row_height_px;
        let scroll_offset = -table.vertical_scroll_handle.offset().y.as_f32();
        let content_top = bounds.origin.y.as_f32() - body_top + scroll_offset;
        let content_bottom =
            bounds.origin.y.as_f32() + bounds.size.height.as_f32() - body_top + scroll_offset;
        let total_height = row_count as f32 * row_height_px;

        if content_bottom < 0.0 || content_top >= total_height {
            return None;
        }

        let content_top = content_top.clamp(0.0, total_height);
        let content_bottom = content_bottom.clamp(0.0, total_height);
        let start_row = (content_top / row_height_px)
            .floor()
            .clamp(0.0, row_count.saturating_sub(1) as f32) as usize;
        let end_y = if content_bottom <= content_top {
            content_top
        } else {
            content_bottom - 0.1
        };
        let end_row = (end_y / row_height_px)
            .floor()
            .clamp(0.0, row_count.saturating_sub(1) as f32) as usize;

        Some((start_row, end_row))
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
        anchor: &PathBuf,
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
        self.sync_sftp_tables_for_tab(tab_id, cx);
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

    pub(in crate::ui::shell) fn begin_sftp_create_directory(
        &mut self,
        tab_id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(parent) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .map(|sftp| sftp.remote_path.clone())
        else {
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

        sftp.prompt = Some(SftpPromptState {
            kind: SftpPromptKind::CreateRemoteDirectory { parent },
        });
        set_input_placeholder(
            &self.workspace_forms.sftp_browser.prompt_input,
            i18n::string("sftp.prompts.directory_name_placeholder"),
            window,
            cx,
        );
        set_input_value(
            &self.workspace_forms.sftp_browser.prompt_input,
            "",
            window,
            cx,
        );
        cx.notify();
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

    pub(in crate::ui::shell) fn delete_sftp_remote_selected(
        &mut self,
        tab_id: usize,
        cx: &mut Context<Self>,
    ) {
        let Some((refresh_path, selected_paths)) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .and_then(|sftp| {
                sftp.commands.as_ref()?;
                Some((sftp.remote_path.clone(), sftp.selected_remote_paths.clone()))
            })
        else {
            self.status_message = i18n::string("sftp.messages.select_remote_entry_first");
            cx.notify();
            return;
        };

        let selected_entries = selected_paths
            .into_iter()
            .filter_map(|path| {
                self.resolve_remote_sftp_entry(tab_id, &path, cx)
                    .map(|entry| {
                        (
                            entry.path,
                            entry.kind == miaominal_sftp::SftpEntryKind::Directory,
                        )
                    })
            })
            .collect::<Vec<_>>();

        if selected_entries.is_empty() {
            self.status_message = i18n::string("sftp.messages.select_remote_entry_first");
            cx.notify();
            return;
        }

        if let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        {
            sftp.prompt = Some(SftpPromptState {
                kind: SftpPromptKind::ConfirmDelete {
                    entries: selected_entries,
                    refresh_path,
                },
            });
        }
        cx.notify();
    }

    fn execute_sftp_delete(
        &mut self,
        tab_id: usize,
        commands: &SftpCommandSender,
        entries: Vec<(String, bool)>,
        refresh_path: String,
        cx: &mut Context<Self>,
    ) {
        let mut deleted_count = 0_usize;
        let mut first_error = None;

        for (path, is_directory) in entries {
            let result = if is_directory {
                commands.remove_directory(path.clone())
            } else {
                commands.remove_file(path.clone())
            };

            match result {
                Ok(()) => {
                    deleted_count += 1;
                }
                Err(error) if first_error.is_none() => {
                    let error = error.to_string();
                    first_error = Some(i18n::string_args(
                        "sftp.messages.delete_failed_for",
                        &[("path", &path), ("error", &error)],
                    ));
                }
                Err(_) => {}
            }
        }

        if deleted_count > 0 {
            if deleted_count == 1 {
                self.status_message = i18n::string("sftp.messages.removing_one_remote_entry");
            } else {
                let deleted_count = deleted_count.to_string();
                self.status_message = i18n::string_args(
                    "sftp.messages.removing_remote_entries",
                    &[("count", &deleted_count)],
                );
            }
            self.request_sftp_remote_directory(tab_id, refresh_path, cx);
            return;
        }

        if let Some(error) = first_error {
            self.status_message = error;
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn skip_sftp_overwrite_prompt(&mut self, cx: &mut Context<Self>) {
        let Some((tab_id, commands, prompt)) = self
            .active_or_browser_sftp_tab_id(cx)
            .and_then(|tab_id| {
                self.workspace_state
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id)
            })
            .and_then(|tab| {
                let sftp = tab.as_sftp()?;
                Some((tab.id, sftp.commands.clone()?, sftp.prompt.clone()?))
            })
        else {
            return;
        };

        let exit_snapshot = DialogOverlaySnapshot::SftpPrompt {
            tab_id,
            prompt: prompt.clone(),
        };

        let SftpPromptKind::ConfirmOverwrite {
            pending_uploads,
            pending_downloads,
            ..
        } = prompt.kind
        else {
            return;
        };

        let remote_entries = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .map(|sftp| sftp.remote_entries.clone())
            .unwrap_or_default();

        if let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        {
            sftp.prompt = None;
        }

        self.start_dialog_exit(exit_snapshot, cx);

        for (local, remote) in pending_uploads {
            if remote_entries.iter().any(|e| e.path == remote) {
                continue;
            }
            if let Err(error) = commands.queue_upload(local, remote) {
                let error = error.to_string();
                self.status_message =
                    i18n::string_args("sftp.messages.upload_queue_failed", &[("error", &error)]);
                cx.notify();
                return;
            }
        }

        for (remote, local) in pending_downloads {
            if local.exists() {
                continue;
            }
            if let Err(error) = commands.queue_download(remote, local) {
                let error = error.to_string();
                self.status_message =
                    i18n::string_args("sftp.messages.download_queue_failed", &[("error", &error)]);
                cx.notify();
                return;
            }
        }

        cx.notify();
    }

    pub(in crate::ui::shell) fn commit_sftp_prompt(&mut self, cx: &mut Context<Self>) {
        let Some((tab_id, commands, prompt)) = self
            .active_or_browser_sftp_tab_id(cx)
            .and_then(|tab_id| {
                self.workspace_state
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id)
            })
            .and_then(|tab| {
                let sftp = tab.as_sftp()?;
                Some((tab.id, sftp.commands.clone()?, sftp.prompt.clone()?))
            })
        else {
            return;
        };

        let exit_snapshot = DialogOverlaySnapshot::SftpPrompt {
            tab_id,
            prompt: prompt.clone(),
        };

        if let SftpPromptKind::ConfirmDelete {
            entries,
            refresh_path,
        } = prompt.kind
        {
            if let Some(sftp) = self
                .workspace_state
                .tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp_mut)
            {
                sftp.prompt = None;
            }
            self.start_dialog_exit(exit_snapshot, cx);
            self.execute_sftp_delete(tab_id, &commands, entries, refresh_path, cx);
            return;
        }

        if let SftpPromptKind::ConfirmOverwrite {
            pending_uploads,
            pending_downloads,
            ..
        } = prompt.kind
        {
            if let Some(sftp) = self
                .workspace_state
                .tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp_mut)
            {
                sftp.prompt = None;
            }
            self.start_dialog_exit(exit_snapshot, cx);
            for (local, remote) in pending_uploads {
                if let Err(error) = commands.queue_upload(local, remote) {
                    let error = error.to_string();
                    self.status_message = i18n::string_args(
                        "sftp.messages.upload_queue_failed",
                        &[("error", &error)],
                    );
                    cx.notify();
                    return;
                }
            }
            for (remote, local) in pending_downloads {
                if let Err(error) = commands.queue_download(remote, local) {
                    let error = error.to_string();
                    self.status_message = i18n::string_args(
                        "sftp.messages.download_queue_failed",
                        &[("error", &error)],
                    );
                    cx.notify();
                    return;
                }
            }
            cx.notify();
            return;
        }

        let value = self
            .workspace_forms
            .sftp_browser
            .prompt_input
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

        let result = match prompt.kind {
            SftpPromptKind::CreateRemoteDirectory { parent } => {
                let path = Self::join_remote_path(&parent, &value);
                let status_message = i18n::string_args(
                    "sftp.messages.creating_remote_directory",
                    &[("path", &path)],
                );
                commands
                    .create_directory(path)
                    .map(|_| (parent, status_message))
            }
            SftpPromptKind::ConfirmOverwrite { .. } | SftpPromptKind::ConfirmDelete { .. } => {
                unreachable!()
            }
        };

        match result {
            Ok((refresh_path, status_message)) => {
                if let Some(sftp) = self
                    .workspace_state
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id == tab_id)
                    .and_then(TabState::as_sftp_mut)
                {
                    sftp.prompt = None;
                }
                self.start_dialog_exit(exit_snapshot, cx);
                self.status_message = status_message;
                self.request_sftp_remote_directory(tab_id, refresh_path, cx);
            }
            Err(error) => {
                let error = error.to_string();
                self.status_message =
                    i18n::string_args("sftp.messages.action_failed", &[("error", &error)]);
                cx.notify();
            }
        }
    }

    pub(in crate::ui::shell) fn cancel_sftp_prompt(&mut self, cx: &mut Context<Self>) {
        let Some((tab_id, prompt)) = self
            .active_or_browser_sftp_tab_id(cx)
            .and_then(|tab_id| {
                self.workspace_state
                    .tabs
                    .iter()
                    .find(|tab| tab.id == tab_id)
            })
            .and_then(|tab| {
                tab.as_sftp()
                    .and_then(|sftp| sftp.prompt.clone().map(|prompt| (tab.id, prompt)))
            })
        else {
            return;
        };

        if let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        {
            sftp.prompt = None;
        }

        self.start_dialog_exit(DialogOverlaySnapshot::SftpPrompt { tab_id, prompt }, cx);
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
