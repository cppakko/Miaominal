use super::*;
use crate::ui::shell::{
    ValidationNotificationKind, localized_profile_import_source_label, success_notification,
};
use miaominal_core::profile::{ImportSourceKind, ImportedBatch};
use miaominal_services::ImportedProfilesResult;
use miaominal_storage::config_store::import::import_profiles_from_path;
use rfd::FileDialog;
use std::path::PathBuf;

impl SessionController {
    pub(in crate::ui::shell) fn import_profiles_from_source(
        &self,
        source: ImportSourceKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = build_profile_import_dialog(source).pick_file() else {
            return;
        };

        if !source.accepts_path(&path) {
            self.report_profile_import_validation(
                i18n::string_args(
                    "settings.connections.import_messages.file_type_mismatch",
                    &[("source", &localized_profile_import_source_label(source))],
                ),
                window,
                cx,
            );
            return;
        }

        match import_profiles_from_path(source, &path) {
            Ok(batch) => self.apply_imported_profiles(source, batch, window, cx),
            Err(error) => {
                self.report_profile_import_validation(
                    i18n::string_args(
                        "settings.connections.import_messages.failed",
                        &[
                            ("source", &localized_profile_import_source_label(source)),
                            ("error", &error.to_string()),
                        ],
                    ),
                    window,
                    cx,
                );
            }
        }
    }

    fn apply_imported_profiles(
        &self,
        source: ImportSourceKind,
        batch: ImportedBatch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if batch.sessions.is_empty() {
            self.report_profile_import_validation(
                i18n::string_args(
                    "settings.connections.import_messages.no_profiles",
                    &[("source", &localized_profile_import_source_label(source))],
                ),
                window,
                cx,
            );
            return;
        }

        let ImportedProfilesResult {
            imported_count,
            warning_count,
        } = match self.import_profiles(batch) {
            Ok(result) => result,
            Err(error) => {
                self.report_profile_import_validation(
                    i18n::string_args(
                        "settings.connections.import_messages.failed",
                        &[
                            ("source", &localized_profile_import_source_label(source)),
                            ("error", &error.to_string()),
                        ],
                    ),
                    window,
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
        cx.emit(AppCommand::Feedback(message.clone()));
        window.push_notification(
            success_notification(
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

    fn report_profile_import_validation(
        &self,
        message: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.emit(AppCommand::Feedback(message.clone()));
        window.push_notification(
            validation_notification(ValidationNotificationKind::InvalidInput, message),
            cx,
        );
        cx.notify();
    }
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
        ImportSourceKind::PuttyRegistry => dialog.add_filter(
            i18n::string("settings.connections.import_messages.putty_filter"),
            &["reg"],
        ),
        ImportSourceKind::SecureCrtXml => dialog.add_filter(
            i18n::string("settings.connections.import_messages.securecrt_filter"),
            &["xml"],
        ),
        ImportSourceKind::FinalShellJson => dialog.add_filter(
            i18n::string("settings.connections.import_messages.finalshell_filter"),
            &["json"],
        ),
    }
}

fn default_ssh_config_directory() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|home| home.join(".ssh"))
        .filter(|path| path.exists())
}
