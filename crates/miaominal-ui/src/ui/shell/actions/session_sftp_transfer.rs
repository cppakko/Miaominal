use super::super::*;
use crate::ui::i18n;
use miaominal_services::{PlannedSftpDownload, SftpService};
use rfd::FileDialog;
use std::path::Path;

struct PreparedSftpDownloads {
    downloads: Vec<PlannedSftpDownload>,
    overwrite_confirmed: bool,
}

fn should_prompt_terminal_sftp_download(
    active_sftp_tab_id: Option<usize>,
    session_sftp_tab_id: Option<usize>,
    target_tab_id: usize,
) -> bool {
    active_sftp_tab_id != Some(target_tab_id) && session_sftp_tab_id == Some(target_tab_id)
}

fn prepare_single_sftp_file_download(
    entry: &SftpEntry,
    local_path: PathBuf,
) -> PreparedSftpDownloads {
    PreparedSftpDownloads {
        downloads: vec![PlannedSftpDownload {
            remote_path: entry.path.clone(),
            local_path,
        }],
        overwrite_confirmed: true,
    }
}

fn should_use_single_file_save_dialog(selected_entries: &[SftpEntry]) -> bool {
    selected_entries.len() == 1 && selected_entries[0].kind == miaominal_sftp::SftpEntryKind::File
}

fn choose_terminal_sftp_download_destination(
    selected_entries: Vec<SftpEntry>,
    initial_directory: &Path,
    window: &Window,
) -> Option<PreparedSftpDownloads> {
    if should_use_single_file_save_dialog(&selected_entries) {
        let entry = &selected_entries[0];
        let mut dialog = FileDialog::new()
            .set_parent(window)
            .set_title(i18n::string("sftp.dialogs.download_file_title"))
            .set_file_name(entry.filename.clone());
        if initial_directory.is_dir() {
            dialog = dialog.set_directory(initial_directory);
        }
        let local_path = dialog.save_file()?;
        return Some(prepare_single_sftp_file_download(entry, local_path));
    }

    let mut dialog = FileDialog::new()
        .set_parent(window)
        .set_title(i18n::string("sftp.dialogs.download_folder_title"));
    if initial_directory.is_dir() {
        dialog = dialog.set_directory(initial_directory);
    }
    let local_base = dialog.pick_folder()?;

    Some(PreparedSftpDownloads {
        downloads: SftpService::plan_downloads(selected_entries, &local_base),
        overwrite_confirmed: false,
    })
}

impl AppView {
    pub(in crate::ui::shell) fn queue_sftp_upload_selected(
        &mut self,
        tab_id: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(selected_paths) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .map(|sftp| sftp.selected_local_paths.clone())
        else {
            return;
        };

        if selected_paths.is_empty() {
            return;
        }

        self.queue_sftp_upload_paths(tab_id, selected_paths, cx);
    }

    pub(in crate::ui::shell) fn queue_sftp_upload_path(
        &mut self,
        tab_id: usize,
        local_path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.queue_sftp_upload_paths(tab_id, vec![local_path], cx);
    }

    pub(in crate::ui::shell) fn queue_sftp_upload_paths(
        &mut self,
        tab_id: usize,
        local_paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let Some((commands, remote_base, remote_entries)) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .and_then(|sftp| {
                Some((
                    sftp.commands.clone()?,
                    sftp.remote_path.clone(),
                    sftp.remote_entries.clone(),
                ))
            })
        else {
            return;
        };

        let known_directory_paths = if local_paths.len() > 1 {
            self.workspace_forms
                .sftp_browser
                .local_table
                .read(cx)
                .delegate()
                .directory_paths_in_selection(&local_paths)
        } else {
            Vec::new()
        };
        let uploads = SftpService::plan_uploads_with_known_directories(
            local_paths,
            &known_directory_paths,
            &remote_base,
        );
        let conflict_count = SftpService::count_remote_conflicts(&uploads, &remote_entries);

        if conflict_count > 0 {
            if let Some(sftp) = self
                .workspace_state
                .tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp_mut)
            {
                sftp.prompt = Some(SftpPromptState {
                    kind: SftpPromptKind::ConfirmOverwrite {
                        conflict_count,
                        pending_uploads: uploads
                            .iter()
                            .map(|upload| (upload.local_path.clone(), upload.remote_path.clone()))
                            .collect(),
                        pending_downloads: Vec::new(),
                    },
                });
            }
            cx.notify();
        } else {
            if let Err(error) = SftpService::queue_uploads(&commands, &uploads) {
                let error = error.to_string();
                self.status_message =
                    i18n::string_args("sftp.messages.upload_queue_failed", &[("error", &error)]);
                cx.notify();
            }
        }
    }

    pub(in crate::ui::shell) fn queue_sftp_download_selected(
        &mut self,
        tab_id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(remote_paths) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .map(|sftp| sftp.selected_remote_paths.clone())
        else {
            return;
        };

        if remote_paths.is_empty() {
            return;
        }

        self.queue_sftp_download_paths(tab_id, remote_paths, window, cx);
    }

    pub(in crate::ui::shell) fn queue_sftp_download_path(
        &mut self,
        tab_id: usize,
        remote_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.queue_sftp_download_paths(tab_id, vec![remote_path], window, cx);
    }

    fn queue_sftp_download_paths(
        &mut self,
        tab_id: usize,
        remote_paths: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((commands, local_base)) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .and_then(|sftp| Some((sftp.commands.clone()?, sftp.local_path.clone())))
        else {
            return;
        };

        let selected_entries: Vec<_> = remote_paths
            .into_iter()
            .filter_map(|remote| self.resolve_remote_sftp_entry(tab_id, &remote, cx))
            .collect();
        if selected_entries.is_empty() {
            return;
        }

        let active_sftp_tab_id = self
            .workspace_state
            .active_topbar_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(|tab| tab.as_sftp().map(|_| tab.id));
        let prompt_for_destination = should_prompt_terminal_sftp_download(
            active_sftp_tab_id,
            self.session_side_panel_sftp_tab_id(),
            tab_id,
        );
        let prepared = if prompt_for_destination {
            let Some(prepared) =
                choose_terminal_sftp_download_destination(selected_entries, &local_base, window)
            else {
                return;
            };
            prepared
        } else {
            PreparedSftpDownloads {
                downloads: SftpService::plan_downloads(selected_entries, &local_base),
                overwrite_confirmed: false,
            }
        };
        let pairs = prepared.downloads;

        let conflict_count = if prepared.overwrite_confirmed {
            0
        } else {
            pairs
                .iter()
                .filter(|download| download.local_path.exists())
                .count()
        };

        if conflict_count > 0 {
            if let Some(sftp) = self
                .workspace_state
                .tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp_mut)
            {
                sftp.prompt = Some(SftpPromptState {
                    kind: SftpPromptKind::ConfirmOverwrite {
                        conflict_count,
                        pending_uploads: Vec::new(),
                        pending_downloads: pairs
                            .iter()
                            .map(|download| {
                                (download.remote_path.clone(), download.local_path.clone())
                            })
                            .collect(),
                    },
                });
            }
            cx.notify();
        } else {
            if let Err(error) = SftpService::queue_downloads(&commands, &pairs) {
                let error = error.to_string();
                self.status_message =
                    i18n::string_args("sftp.messages.download_queue_failed", &[("error", &error)]);
                cx.notify();
            }
        }
    }

    pub(in crate::ui::shell) fn pause_sftp_transfer(
        &mut self,
        tab_id: usize,
        transfer_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        let Some(commands) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .and_then(|sftp| sftp.commands.clone())
        else {
            return;
        };

        if let Err(error) = SftpService::pause_transfer(&commands, transfer_id) {
            let error = error.to_string();
            self.status_message =
                i18n::string_args("sftp.messages.pause_transfer_failed", &[("error", &error)]);
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn resume_sftp_transfer(
        &mut self,
        tab_id: usize,
        transfer_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        let Some(commands) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .and_then(|sftp| sftp.commands.clone())
        else {
            return;
        };

        if let Err(error) = SftpService::resume_transfer(&commands, transfer_id) {
            let error = error.to_string();
            self.status_message =
                i18n::string_args("sftp.messages.resume_transfer_failed", &[("error", &error)]);
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn cancel_sftp_transfer(
        &mut self,
        tab_id: usize,
        transfer_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        let Some(commands) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .and_then(|sftp| sftp.commands.clone())
        else {
            return;
        };

        if let Err(error) = SftpService::cancel_transfer(&commands, transfer_id) {
            let error = error.to_string();
            self.status_message =
                i18n::string_args("sftp.messages.cancel_transfer_failed", &[("error", &error)]);
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn remove_sftp_transfer_record(
        &mut self,
        tab_id: usize,
        transfer_id: TransferId,
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

        let before = sftp.transfers.len();
        sftp.transfers
            .retain(|transfer| transfer.transfer_id != transfer_id);
        if sftp.transfers.len() != before {
            let transfer_id = transfer_id.0.to_string();
            sftp.last_status = i18n::string_args(
                "sftp.messages.removed_transfer_record",
                &[("id", &transfer_id)],
            );
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn toggle_sftp_transfer_expanded(
        &mut self,
        tab_id: usize,
        transfer_id: TransferId,
        cx: &mut Context<Self>,
    ) {
        let Some(transfer) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
            .and_then(|sftp| {
                sftp.transfers
                    .iter_mut()
                    .find(|transfer| transfer.transfer_id == transfer_id)
            })
        else {
            return;
        };

        if transfer.children.is_empty() {
            return;
        }
        transfer.expanded = !transfer.expanded;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote_entry(path: &str, kind: miaominal_sftp::SftpEntryKind) -> SftpEntry {
        SftpEntry {
            filename: SftpService::remote_file_name(path),
            path: path.to_string(),
            kind,
            size: None,
            modified: None,
            attributes: None,
            owner: None,
        }
    }

    fn remote_file(path: &str) -> SftpEntry {
        remote_entry(path, miaominal_sftp::SftpEntryKind::File)
    }

    #[test]
    fn terminal_download_prompts_only_when_target_is_the_session_sftp_tab() {
        assert!(should_prompt_terminal_sftp_download(None, Some(7), 7));
        assert!(!should_prompt_terminal_sftp_download(Some(7), Some(7), 7));
        assert!(!should_prompt_terminal_sftp_download(None, Some(8), 7));
    }

    #[test]
    fn single_file_save_uses_the_exact_native_dialog_path() {
        let entry = remote_file("/srv/archive.zip");
        let local_path = PathBuf::from(r"C:\Downloads\renamed.zip");

        let prepared = prepare_single_sftp_file_download(&entry, local_path.clone());

        assert!(prepared.overwrite_confirmed);
        assert_eq!(prepared.downloads.len(), 1);
        assert_eq!(prepared.downloads[0].remote_path, entry.path);
        assert_eq!(prepared.downloads[0].local_path, local_path);
    }

    #[test]
    fn single_file_dialog_is_reserved_for_confirmed_regular_files() {
        assert!(should_use_single_file_save_dialog(&[remote_file(
            "/srv/archive.zip"
        )]));
        assert!(!should_use_single_file_save_dialog(&[remote_entry(
            "/srv/folder",
            miaominal_sftp::SftpEntryKind::Directory,
        )]));
        assert!(!should_use_single_file_save_dialog(&[remote_entry(
            "/srv/link",
            miaominal_sftp::SftpEntryKind::Symlink,
        )]));
        assert!(!should_use_single_file_save_dialog(&[remote_entry(
            "/srv/device",
            miaominal_sftp::SftpEntryKind::Other,
        )]));
        assert!(!should_use_single_file_save_dialog(&[
            remote_file("/srv/one.txt"),
            remote_file("/srv/two.txt"),
        ]));
    }
}
