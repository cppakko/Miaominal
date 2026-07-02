use super::super::*;
use crate::ui::i18n;

impl AppView {
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
}
