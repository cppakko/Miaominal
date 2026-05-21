use super::super::*;
use crate::ui::i18n;
use notify::{EventKind, RecursiveMode, Watcher};
use std::time::Duration;

impl AppView {
    pub(in crate::ui::shell) fn open_remote_file_for_editing(
        &mut self,
        tab_id: usize,
        remote_path: String,
        cx: &mut Context<Self>,
    ) {
        let Some(sftp) = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
        else {
            return;
        };

        if let Some(session) = sftp.edit_sessions.get(&remote_path) {
            if let Err(error) = open::that(&session.temp_path) {
                let error = error.to_string();
                self.status_message =
                    i18n::string_args("sftp.messages.open_editor_failed", &[("error", &error)]);
                cx.notify();
            }
            return;
        }

        let Some(commands) = sftp.commands.clone() else {
            return;
        };

        let filename = std::path::Path::new(&remote_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());

        let temp_path = std::env::temp_dir()
            .join("miaominal_edit")
            .join(tab_id.to_string())
            .join(&filename);

        if let Some(parent) = temp_path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            let error = error.to_string();
            self.status_message = i18n::string_args(
                "sftp.messages.create_temp_directory_failed",
                &[("error", &error)],
            );
            cx.notify();
            return;
        }

        let transfer_id = match commands.queue_download(remote_path.clone(), temp_path) {
            Ok(id) => id,
            Err(error) => {
                let error = error.to_string();
                self.status_message = i18n::string_args(
                    "sftp.messages.queue_edit_download_failed",
                    &[("error", &error)],
                );
                cx.notify();
                return;
            }
        };

        if let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        {
            sftp.edit_pending_downloads.insert(transfer_id, remote_path);
        }

        cx.notify();
    }

    pub(in crate::ui::shell) fn on_edit_download_complete(
        &mut self,
        tab_id: usize,
        temp_path: PathBuf,
        remote_path: String,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = open::that(&temp_path) {
            let error = error.to_string();
            self.status_message =
                i18n::string_args("sftp.messages.open_editor_failed", &[("error", &error)]);
            cx.notify();
            return;
        }

        let (sender, mut receiver) = futures::channel::mpsc::unbounded::<()>();

        let watch_path = temp_path.clone();
        let watch_sender = sender;
        let watcher_result =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let Ok(event) = res else { return };
                // Trigger only when our specific file is modified or recreated (atomic saves).
                let is_relevant = matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                ) && event.paths.iter().any(|p| p == &watch_path);
                if is_relevant {
                    let _ = watch_sender.unbounded_send(());
                }
            });

        let mut watcher = match watcher_result {
            Ok(w) => w,
            Err(error) => {
                let error = error.to_string();
                self.status_message = i18n::string_args(
                    "sftp.messages.create_file_watcher_failed",
                    &[("error", &error)],
                );
                cx.notify();
                return;
            }
        };

        // Watch the parent directory non-recursively so we catch atomic (rename-based) saves.
        let watch_dir = temp_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| temp_path.clone());

        if let Err(error) = watcher.watch(&watch_dir, RecursiveMode::NonRecursive) {
            let error = error.to_string();
            self.status_message = i18n::string_args(
                "sftp.messages.watch_temp_directory_failed",
                &[("error", &error)],
            );
            cx.notify();
            return;
        }

        let remote_path_for_task = remote_path.clone();
        let temp_path_for_task = temp_path.clone();

        let watch_task = cx.spawn(async move |this, cx| {
            while receiver.next().await.is_some() {
                if this
                    .update(cx, |this, cx| {
                        this.on_edit_file_changed(
                            tab_id,
                            remote_path_for_task.clone(),
                            temp_path_for_task.clone(),
                            cx,
                        );
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        let session = SftpEditSession {
            temp_path,
            _watcher: watcher,
            debounce_task: None,
            _watch_task: watch_task,
        };

        if let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        {
            sftp.edit_sessions.insert(remote_path, session);
        }

        cx.notify();
    }

    fn on_edit_file_changed(
        &mut self,
        tab_id: usize,
        remote_path: String,
        temp_path: PathBuf,
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

        let Some(session) = sftp.edit_sessions.get_mut(&remote_path) else {
            return;
        };

        let Some(commands) = sftp.commands.clone() else {
            return;
        };

        // Drop the previous task to cancel any pending upload before scheduling a new one.
        session.debounce_task = None;

        let remote_path_for_task = remote_path.clone();
        let temp_path_for_task = temp_path.clone();

        let debounce_task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;

            this.update(cx, |this, cx| {
                if let Err(error) = commands.queue_upload(temp_path_for_task, remote_path_for_task)
                {
                    let error = error.to_string();
                    this.status_message =
                        i18n::string_args("sftp.messages.edit_upload_failed", &[("error", &error)]);
                    cx.notify();
                }
            })
            .ok();
        });

        // Store back — need to re-borrow since the spawn closure moved things.
        if let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
            && let Some(session) = sftp.edit_sessions.get_mut(&remote_path)
        {
            session.debounce_task = Some(debounce_task);
        }
    }
}
