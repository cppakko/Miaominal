use crate::ui::i18n;
use miaominal_core::profile::{ImportField, ImportIssueKind, ImportIssueReason, ImportSourceKind};
use miaominal_settings::{
    LastTabCloseBehavior, LocalVaultAutoLockDuration, MonitorHistoryDuration, ThemeId,
};

pub(in crate::ui::shell) fn last_tab_close_behavior_label(
    behavior: LastTabCloseBehavior,
) -> String {
    i18n::string(match behavior {
        LastTabCloseBehavior::ExitApplication => "enum.last_tab_close_behavior.exit_application",
        LastTabCloseBehavior::OpenNewHomeTab => "enum.last_tab_close_behavior.open_new_home_tab",
    })
}

pub(in crate::ui::shell) fn monitor_history_duration_label(
    duration: MonitorHistoryDuration,
) -> String {
    i18n::string(match duration {
        MonitorHistoryDuration::OneMinute => "enum.monitor_history.one_minute",
        MonitorHistoryDuration::FiveMinutes => "enum.monitor_history.five_minutes",
        MonitorHistoryDuration::TenMinutes => "enum.monitor_history.ten_minutes",
        MonitorHistoryDuration::ThirtyMinutes => "enum.monitor_history.thirty_minutes",
    })
}

pub(in crate::ui::shell) fn local_vault_auto_lock_duration_label(
    duration: LocalVaultAutoLockDuration,
) -> String {
    i18n::string(match duration {
        LocalVaultAutoLockDuration::Off => "enum.local_vault_auto_lock.off",
        LocalVaultAutoLockDuration::FiveMinutes => "enum.local_vault_auto_lock.five_minutes",
        LocalVaultAutoLockDuration::FifteenMinutes => "enum.local_vault_auto_lock.fifteen_minutes",
        LocalVaultAutoLockDuration::OneHour => "enum.local_vault_auto_lock.one_hour",
        LocalVaultAutoLockDuration::OneDay => "enum.local_vault_auto_lock.one_day",
    })
}

pub(in crate::ui::shell) fn theme_id_label(theme_id: ThemeId) -> String {
    i18n::string(match theme_id {
        ThemeId::Light => "enum.theme.light",
        ThemeId::Dark => "enum.theme.dark",
    })
}

pub(in crate::ui::shell) fn localized_profile_import_source_label(
    source: ImportSourceKind,
) -> String {
    i18n::string(match source {
        ImportSourceKind::OpenSshConfig => "settings.connections.import_sources.openssh",
        ImportSourceKind::PuttyRegistry => "settings.connections.import_sources.putty",
        ImportSourceKind::SecureCrtXml => "settings.connections.import_sources.securecrt",
        ImportSourceKind::FinalShellJson => "settings.connections.import_sources.finalshell",
    })
}

pub(in crate::ui::shell) fn localized_profile_import_issue_kind(kind: ImportIssueKind) -> String {
    i18n::string(match kind {
        ImportIssueKind::UnsupportedProtocol => {
            "settings.connections.import_result.kinds.unsupported_protocol"
        }
        ImportIssueKind::MissingRequiredField => {
            "settings.connections.import_result.kinds.missing_required_field"
        }
        ImportIssueKind::UnsupportedCredential => {
            "settings.connections.import_result.kinds.unsupported_credential"
        }
        ImportIssueKind::UnsupportedFeature => {
            "settings.connections.import_result.kinds.unsupported_feature"
        }
        ImportIssueKind::InvalidEntry => "settings.connections.import_result.kinds.invalid_entry",
    })
}

pub(in crate::ui::shell) fn localized_profile_import_issue_reason(
    reason: &ImportIssueReason,
) -> String {
    match reason {
        ImportIssueReason::UnsupportedHostPattern => {
            i18n::string("settings.connections.import_result.reasons.unsupported_host_pattern")
        }
        ImportIssueReason::ProxyJumpNotImported => {
            i18n::string("settings.connections.import_result.reasons.proxy_jump_not_imported")
        }
        ImportIssueReason::IncludeNotExpanded => {
            i18n::string("settings.connections.import_result.reasons.include_not_expanded")
        }
        ImportIssueReason::MatchNotEvaluated { expression } => i18n::string_args(
            "settings.connections.import_result.reasons.match_not_evaluated",
            &[("expression", expression)],
        ),
        ImportIssueReason::MultipleIdentityFiles => {
            i18n::string("settings.connections.import_result.reasons.multiple_identity_files")
        }
        ImportIssueReason::NoLiteralHostAlias => {
            i18n::string("settings.connections.import_result.reasons.no_literal_host_alias")
        }
        ImportIssueReason::MissingField { field } => {
            let field = localized_profile_import_field(field);
            i18n::string_args(
                "settings.connections.import_result.reasons.missing_field",
                &[("field", &field)],
            )
        }
        ImportIssueReason::UnsupportedProtocol { protocol } => i18n::string_args(
            "settings.connections.import_result.reasons.unsupported_protocol",
            &[("protocol", protocol)],
        ),
        ImportIssueReason::InvalidPort { value } => i18n::string_args(
            "settings.connections.import_result.reasons.invalid_port",
            &[("value", value)],
        ),
        ImportIssueReason::CredentialProfileNotFound { profile } => i18n::string_args(
            "settings.connections.import_result.reasons.credential_profile_not_found",
            &[("profile", profile)],
        ),
        ImportIssueReason::EncryptedPasswordNotImported => i18n::string(
            "settings.connections.import_result.reasons.encrypted_password_not_imported",
        ),
        ImportIssueReason::PasswordCouldNotBeDecoded => {
            i18n::string("settings.connections.import_result.reasons.password_could_not_be_decoded")
        }
        ImportIssueReason::KeyReferenceNotImported => {
            i18n::string("settings.connections.import_result.reasons.key_reference_not_imported")
        }
        ImportIssueReason::GlobalPublicKeyPathMissing => i18n::string(
            "settings.connections.import_result.reasons.global_public_key_path_missing",
        ),
        ImportIssueReason::AgentForwardingUnresolved { value } => i18n::string_args(
            "settings.connections.import_result.reasons.agent_forwarding_unresolved",
            &[("value", value)],
        ),
    }
}

fn localized_profile_import_field(field: &ImportField) -> String {
    i18n::string(match field {
        ImportField::Host => "settings.connections.import_result.fields.host",
        ImportField::Username => "settings.connections.import_result.fields.username",
        ImportField::Protocol => "settings.connections.import_result.fields.protocol",
    })
}
