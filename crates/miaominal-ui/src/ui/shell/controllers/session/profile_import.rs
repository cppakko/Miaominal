use super::*;
use crate::ui::shell::{
    PendingProfileImportResultState, ValidationNotificationKind,
    localized_profile_import_source_label, success_notification, warning_notification,
};
use miaominal_core::profile::{ImportSourceKind, ImportedBatch};
use miaominal_services::ImportedProfilesResult;
use miaominal_storage::config_store::import::import_profiles_from_path;
use rfd::AsyncFileDialog;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfileImportPresentation {
    NoProfiles,
    FailedWithIssues,
    Success,
    PartialSuccess,
}

fn profile_import_presentation(
    imported_count: usize,
    issue_count: usize,
) -> ProfileImportPresentation {
    match (imported_count, issue_count) {
        (0, 0) => ProfileImportPresentation::NoProfiles,
        (0, _) => ProfileImportPresentation::FailedWithIssues,
        (_, 0) => ProfileImportPresentation::Success,
        _ => ProfileImportPresentation::PartialSuccess,
    }
}

impl SessionController {
    pub(in crate::ui::shell) fn import_profiles_from_source(
        &self,
        source: ImportSourceKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dialog = build_profile_import_dialog(source, window);
        cx.spawn(async move |this, cx| {
            let Some(file) = dialog.pick_file().await else {
                return;
            };
            let path = file.path().to_path_buf();
            cx.update(move |cx| {
                let Some(window_handle) = cx.active_window() else {
                    return;
                };
                let this_for_window = this.clone();
                if let Err(error) = window_handle.update(cx, move |_, window, cx| {
                    let _ = this_for_window.update(cx, |controller, cx| {
                        controller.import_profiles_from_selected_path(source, path, window, cx);
                    });
                }) {
                    log::debug!("failed to apply selected profile import file: {error:?}");
                }
            });
        })
        .detach();
    }

    fn import_profiles_from_selected_path(
        &self,
        source: ImportSourceKind,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            if matches!(
                profile_import_presentation(0, batch.issues.len()),
                ProfileImportPresentation::FailedWithIssues
            ) {
                let message = i18n::string_args(
                    "settings.connections.import_messages.no_profiles",
                    &[("source", &localized_profile_import_source_label(source))],
                );
                cx.emit(AppCommand::Feedback(message));
                self.set_pending_profile_import_result(Some(PendingProfileImportResultState {
                    source,
                    imported_count: 0,
                    issues: batch.issues,
                }));
                cx.notify();
                return;
            }
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
            issues,
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

        let warning_count = issues.len();
        let imported_count_value = imported_count;
        let imported_count = imported_count_value.to_string();
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
        let presentation = profile_import_presentation(imported_count_value, warning_count);
        window.push_notification(
            if matches!(presentation, ProfileImportPresentation::Success) {
                success_notification(
                    i18n::string("settings.connections.import_messages.success_title"),
                    message,
                )
            } else {
                warning_notification(
                    i18n::string("settings.connections.import_messages.partial_success_title"),
                    message,
                )
            },
            cx,
        );
        if matches!(presentation, ProfileImportPresentation::PartialSuccess) {
            self.set_pending_profile_import_result(Some(PendingProfileImportResultState {
                source,
                imported_count: imported_count_value,
                issues,
            }));
        }
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

fn build_profile_import_dialog(source: ImportSourceKind, window: &Window) -> AsyncFileDialog {
    let dialog = AsyncFileDialog::new()
        .set_parent(window)
        .set_title(i18n::string_args(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_result_presentation_distinguishes_all_ui_outcomes() {
        assert_eq!(
            profile_import_presentation(0, 0),
            ProfileImportPresentation::NoProfiles
        );
        assert_eq!(
            profile_import_presentation(0, 2),
            ProfileImportPresentation::FailedWithIssues
        );
        assert_eq!(
            profile_import_presentation(3, 0),
            ProfileImportPresentation::Success
        );
        assert_eq!(
            profile_import_presentation(3, 2),
            ProfileImportPresentation::PartialSuccess
        );
    }
}
