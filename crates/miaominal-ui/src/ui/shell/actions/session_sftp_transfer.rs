use super::super::*;
use crate::ui::i18n;
use miaominal_services::SftpService;

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

        self.queue_sftp_download_paths(tab_id, remote_paths, cx);
    }

    pub(in crate::ui::shell) fn queue_sftp_download_path(
        &mut self,
        tab_id: usize,
        remote_path: String,
        cx: &mut Context<Self>,
    ) {
        self.queue_sftp_download_paths(tab_id, vec![remote_path], cx);
    }

    fn queue_sftp_download_paths(
        &mut self,
        tab_id: usize,
        remote_paths: Vec<String>,
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

        let selected_entries = remote_paths
            .into_iter()
            .filter_map(|remote| self.resolve_remote_sftp_entry(tab_id, &remote, cx))
            .collect();
        let pairs = SftpService::plan_downloads(selected_entries, &local_base);

        let conflict_count = pairs
            .iter()
            .filter(|download| download.local_path.exists())
            .count();

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
