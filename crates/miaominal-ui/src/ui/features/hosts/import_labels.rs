use crate::ui::i18n;
use miaominal_core::profile::{ImportField, ImportIssueKind, ImportIssueReason, ImportSourceKind};

pub(super) fn localized_profile_import_source_label(source: ImportSourceKind) -> String {
    i18n::string(match source {
        ImportSourceKind::OpenSshConfig => "settings.connections.import_sources.openssh",
        ImportSourceKind::PuttyRegistry => "settings.connections.import_sources.putty",
        ImportSourceKind::SecureCrtXml => "settings.connections.import_sources.securecrt",
        ImportSourceKind::FinalShellJson => "settings.connections.import_sources.finalshell",
    })
}

pub(super) fn localized_profile_import_issue_kind(kind: ImportIssueKind) -> String {
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

pub(super) fn localized_profile_import_issue_reason(reason: &ImportIssueReason) -> String {
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
