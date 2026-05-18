use super::super::*;
use crate::domain::profile::{ImportSourceKind, ImportedBatch};
use crate::infra::config_store::import::import_profiles_from_path;
use crate::services::ImportedProfilesResult;
use crate::ui::i18n;
use gpui_component::WindowExt as _;
use rfd::FileDialog;
use std::path::PathBuf;

impl AppView {
    pub(in crate::ui::shell) fn selected_profile_import_source(
        &self,
        cx: &App,
    ) -> ImportSourceKind {
        self.panel_forms
            .settings
            .profile_import_source_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(ImportSourceKind::OpenSshConfig)
    }

    pub(in crate::ui::shell) fn import_profiles_from_selected_source(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = self.selected_profile_import_source(cx);
        let Some(path) = build_profile_import_dialog(source).pick_file() else {
            return;
        };

        if !source.accepts_path(&path) {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::InvalidInput,
                i18n::string_args(
                    "settings.connections.import_messages.file_type_mismatch",
                    &[("source", &localized_profile_import_source_label(source))],
                ),
                cx,
            );
            return;
        }

        match import_profiles_from_path(source, &path) {
            Ok(batch) => self.apply_imported_profiles(source, batch, window, cx),
            Err(error) => {
                let error = error.to_string();
                self.notify_validation_failure_in_window(
                    window,
                    ValidationNotificationKind::InvalidInput,
                    i18n::string_args(
                        "settings.connections.import_messages.failed",
                        &[
                            ("source", &localized_profile_import_source_label(source)),
                            ("error", &error),
                        ],
                    ),
                    cx,
                );
            }
        }
    }

    fn apply_imported_profiles(
        &mut self,
        source: ImportSourceKind,
        batch: ImportedBatch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if batch.sessions.is_empty() {
            self.notify_validation_failure_in_window(
                window,
                ValidationNotificationKind::InvalidInput,
                i18n::string_args(
                    "settings.connections.import_messages.no_profiles",
                    &[("source", &localized_profile_import_source_label(source))],
                ),
                cx,
            );
            return;
        }

        let ImportedProfilesResult {
            imported_count,
            warning_count,
        } = match self
            .profile_service()
            .import_profiles(&mut self.data.sessions, batch)
        {
            Ok(result) => result,
            Err(error) => {
                self.notify_validation_failure_in_window(
                    window,
                    ValidationNotificationKind::InvalidInput,
                    i18n::string_args(
                        "settings.connections.import_messages.failed",
                        &[
                            ("source", &localized_profile_import_source_label(source)),
                            ("error", &error.to_string()),
                        ],
                    ),
                    cx,
                );
                return;
            }
        };

        let imported_count = imported_count.to_string();
        let warning_count_text = warning_count.to_string();
        let message = if warning_count == 0 {
            i18n::string_args(
                "settings.connections.import_messages.imported",
                &[
                    ("source", &localized_profile_import_source_label(source)),
                    ("imported", &imported_count),
                ],
            )
        } else {
            i18n::string_args(
                "settings.connections.import_messages.imported_with_warnings",
                &[
                    ("source", &localized_profile_import_source_label(source)),
                    ("imported", &imported_count),
                    ("warnings", &warning_count_text),
                ],
            )
        };
        self.status_message = message.clone();
        window.push_notification(
            Self::success_notification(
                i18n::string(if warning_count == 0 {
                    "settings.connections.import_messages.success_title"
                } else {
                    "settings.connections.import_messages.partial_success_title"
                }),
                message,
            ),
            cx,
        );
        cx.notify();
    }
}

fn localized_profile_import_source_label(source: ImportSourceKind) -> String {
    i18n::string(match source {
        ImportSourceKind::OpenSshConfig => "settings.connections.import_sources.openssh",
        ImportSourceKind::PuttyRegistry => "settings.connections.import_sources.putty",
        ImportSourceKind::SecureCrtXml => "settings.connections.import_sources.securecrt",
        ImportSourceKind::FinalShellJson => "settings.connections.import_sources.finalshell",
    })
}

fn build_profile_import_dialog(source: ImportSourceKind) -> FileDialog {
    let dialog = FileDialog::new().set_title(i18n::string_args(
        "settings.connections.import_messages.dialog_title",
        &[("source", &localized_profile_import_source_label(source))],
    ));

    match source {
        ImportSourceKind::OpenSshConfig => {
            if let Some(directory) = default_ssh_config_directory() {
                dialog.set_directory(directory)
            } else {
                dialog
            }
        }
        ImportSourceKind::PuttyRegistry => dialog.add_filter("PuTTY registry export", &["reg"]),
        ImportSourceKind::SecureCrtXml => dialog.add_filter("SecureCRT XML export", &["xml"]),
        ImportSourceKind::FinalShellJson => dialog.add_filter("FinalShell export", &["json"]),
    }
}

fn default_ssh_config_directory() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|home| home.join(".ssh"))
        .filter(|path| path.exists())
}
