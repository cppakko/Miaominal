//! Typed DTOs for the sync and configuration-import interfaces.
//!
//! The bridge never exposes raw provider payloads, credentials, or `anyhow`
//! errors: every state that the UI reacts to has a stable enumerable form, and
//! failure details are sanitized before they cross the FFI.

use crate::events::ConfigArea;
use miaominal_core::profile::{
    AuthMethod, ImportField, ImportIssueKind, ImportIssueReason, ImportSourceKind,
    ImportedSessionDraft,
};
use miaominal_sync::capability::{CapabilityReason, CapabilityReport, CapabilityState, EtagKind};
use miaominal_sync::{SyncConfig, SyncInterventionReason, SyncProvider, SyncStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncProviderDto {
    None,
    GithubGist,
    WebDav,
}

impl From<SyncProvider> for SyncProviderDto {
    fn from(provider: SyncProvider) -> Self {
        match provider {
            SyncProvider::None => Self::None,
            SyncProvider::GithubGist => Self::GithubGist,
            SyncProvider::WebDav => Self::WebDav,
        }
    }
}

impl From<SyncProviderDto> for SyncProvider {
    fn from(provider: SyncProviderDto) -> Self {
        match provider {
            SyncProviderDto::None => Self::None,
            SyncProviderDto::GithubGist => Self::GithubGist,
            SyncProviderDto::WebDav => Self::WebDav,
        }
    }
}

/// Provider configuration and credential *presence*; never the secret value.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncConfigDto {
    pub provider: SyncProviderDto,
    pub gist_id: Option<String>,
    pub webdav_url: String,
    pub webdav_username: String,
    pub has_github_token: bool,
    pub has_webdav_password: bool,
    pub has_passphrase: bool,
    pub auto_sync_enabled: bool,
    pub last_sync_at_ms: u64,
    pub device_id: String,
    /// Pre-probe compatibility exemption carried over from a legacy config.
    pub legacy_auto_sync_compat: bool,
    /// User consent for last-write-wins WebDAV uploads without preconditions.
    pub webdav_unsafe_write_consent: bool,
    /// The device has completed a capability probe for the current provider.
    pub capability_probe_migrated: bool,
}

impl From<&SyncConfig> for SyncConfigDto {
    fn from(config: &SyncConfig) -> Self {
        Self {
            provider: config.provider.into(),
            gist_id: config.gist_id.clone(),
            webdav_url: config.webdav_url.clone(),
            webdav_username: config.webdav_username.clone(),
            has_github_token: config.has_github_token,
            has_webdav_password: config.has_webdav_password,
            has_passphrase: config.has_passphrase,
            auto_sync_enabled: config.auto_sync_enabled,
            last_sync_at_ms: config.last_sync_at.saturating_mul(1_000),
            device_id: config.device_id.clone(),
            legacy_auto_sync_compat: config.legacy_auto_sync_compat,
            webdav_unsafe_write_consent: config.webdav_unsafe_write_consent,
            capability_probe_migrated: config.capability_probe_migrated,
        }
    }
}

/// Field-update semantics: `None` keeps the stored value; `Some("")` clears an
/// optional text field. Knowledge of the previous secret is never required.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct SyncConfigUpdate {
    pub provider: Option<SyncProviderDto>,
    pub gist_id: Option<String>,
    pub webdav_url: Option<String>,
    pub webdav_username: Option<String>,
    pub auto_sync_enabled: Option<bool>,
    pub webdav_unsafe_write_consent: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncCredentialKind {
    GithubToken,
    WebdavPassword,
    Passphrase,
}

/// Outcome of copying the legacy Kotlin device entries into the Rust
/// credential store; the legacy entries are removed only after verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct SyncMigrationResult {
    pub migrated_github_token: bool,
    pub migrated_webdav_password: bool,
    pub migrated_passphrase: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncCapabilityStateDto {
    Unchecked,
    Checking,
    Supported,
    Unsupported,
    Incomplete,
}

impl From<CapabilityState> for SyncCapabilityStateDto {
    fn from(state: CapabilityState) -> Self {
        match state {
            CapabilityState::Unchecked => Self::Unchecked,
            CapabilityState::Checking => Self::Checking,
            CapabilityState::Supported => Self::Supported,
            CapabilityState::Unsupported => Self::Unsupported,
            CapabilityState::Incomplete => Self::Incomplete,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncCapabilityReasonDto {
    VersionUnavailable,
    ConditionalRead,
    ConditionalWrite,
    Network,
    Authentication,
    Permission,
    Redirect,
    Http,
    Cancelled,
    Cleanup,
    Configuration,
}

impl From<CapabilityReason> for SyncCapabilityReasonDto {
    fn from(reason: CapabilityReason) -> Self {
        match reason {
            CapabilityReason::VersionUnavailable => Self::VersionUnavailable,
            CapabilityReason::ConditionalRead => Self::ConditionalRead,
            CapabilityReason::ConditionalWrite => Self::ConditionalWrite,
            CapabilityReason::Network => Self::Network,
            CapabilityReason::Authentication => Self::Authentication,
            CapabilityReason::Permission => Self::Permission,
            CapabilityReason::Redirect => Self::Redirect,
            CapabilityReason::Http => Self::Http,
            CapabilityReason::Cancelled => Self::Cancelled,
            CapabilityReason::Cleanup => Self::Cleanup,
            CapabilityReason::Configuration => Self::Configuration,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncEtagKindDto {
    Missing,
    Weak,
    Strong,
    Invalid,
}

impl From<EtagKind> for SyncEtagKindDto {
    fn from(kind: EtagKind) -> Self {
        match kind {
            EtagKind::Missing => Self::Missing,
            EtagKind::Weak => Self::Weak,
            EtagKind::Strong => Self::Strong,
            EtagKind::Invalid => Self::Invalid,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncCapabilityDto {
    pub state: SyncCapabilityStateDto,
    pub reason: Option<SyncCapabilityReasonDto>,
    /// Non-sensitive step label, for example `upload-precondition`.
    pub step: String,
    pub http_status: Option<u16>,
    pub etag_kind: SyncEtagKindDto,
    pub checked_at_ms: u64,
    /// A probe may leave a disposable remote file behind when cleanup failed;
    /// the UI only needs to know that it will be retried, not the file path.
    pub cleanup_pending: bool,
}

impl From<&CapabilityReport> for SyncCapabilityDto {
    fn from(report: &CapabilityReport) -> Self {
        Self {
            state: report.state.into(),
            reason: report.reason.map(Into::into),
            step: report.step.clone(),
            http_status: report.http_status,
            etag_kind: report.etag_kind.into(),
            checked_at_ms: report.checked_at.saturating_mul(1_000),
            cleanup_pending: report.cleanup_file.is_some(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncInterventionReasonDto {
    BothSidesChanged,
    RemoteChangedBeforePush,
    UnsafeProviderWrite,
    LocalChangedDuringPull,
    MissingSyncBaseline,
    SyncConfigurationChanged,
}

impl From<SyncInterventionReason> for SyncInterventionReasonDto {
    fn from(reason: SyncInterventionReason) -> Self {
        match reason {
            SyncInterventionReason::BothSidesChanged => Self::BothSidesChanged,
            SyncInterventionReason::RemoteChangedBeforePush => Self::RemoteChangedBeforePush,
            SyncInterventionReason::UnsafeProviderWrite => Self::UnsafeProviderWrite,
            SyncInterventionReason::LocalChangedDuringPull => Self::LocalChangedDuringPull,
            SyncInterventionReason::MissingSyncBaseline => Self::MissingSyncBaseline,
            SyncInterventionReason::SyncConfigurationChanged => Self::SyncConfigurationChanged,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncFailureKindDto {
    Authentication,
    Network,
    Capability,
    Conflict,
    Locked,
    NotConfigured,
    Provider,
    Storage,
    Cancelled,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncFailureDto {
    pub kind: SyncFailureKindDto,
    /// Sanitized, bounded message; never contains credentials or payload bytes.
    pub detail: String,
}

/// Result of the most recent sync operation. `Idle` means nothing ran yet.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SyncStatusDto {
    Idle,
    RemoteBindingRequired {
        provider: SyncProviderDto,
    },
    Pulled {
        at_ms: u64,
    },
    Pushed {
        at_ms: u64,
    },
    UpToDate {
        at_ms: u64,
    },
    PullRequired {
        remote_at_ms: Option<u64>,
        reason: SyncInterventionReasonDto,
    },
    Cancelled,
    Failed {
        failure: SyncFailureDto,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncPhaseDto {
    Disabled,
    Idle,
    CheckingCapability,
    CheckingRemote,
    Pulling,
    Pushing,
    /// Vault is locked; automatic sync is paused and nothing is uploaded.
    PausedVaultLocked,
    /// Provider credentials or the encryption passphrase are missing.
    PausedCredentials,
    /// The remote cannot prove safe writes; a manual decision is required.
    PausedCapability,
    /// Local and remote both changed; the user picks a side.
    Conflict,
    Cancelling,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncConflictDto {
    pub remote_at_ms: Option<u64>,
    pub reason: SyncInterventionReasonDto,
}

/// Which remote state was observed by a lightweight check.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SyncRemoteStateDto {
    Disabled,
    BindingRequired {
        provider: SyncProviderDto,
    },
    Missing,
    NotModified,
    UpToDate,
    Updated {
        synced_at_ms: u64,
        content_revision: Option<String>,
    },
}

/// Pull-based snapshot of the sync runtime; progress is never queued.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncSnapshotDto {
    pub phase: SyncPhaseDto,
    pub status: SyncStatusDto,
    pub config: SyncConfigDto,
    pub capability: SyncCapabilityDto,
    pub vault_locked: bool,
    pub pending_conflict: Option<SyncConflictDto>,
    pub last_failure: Option<SyncFailureDto>,
    /// Unix ms of the last automatic check; `0` when none has run.
    pub last_auto_check_at_ms: u64,
}

/// Outcome of one pull/push/capability/cancelable sync call.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncRunResult {
    pub status: SyncStatusDto,
    pub config: SyncConfigDto,
    /// Snapshot areas the operation wrote (the host refreshes those areas).
    pub changed_areas: Vec<ConfigArea>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SyncResolution {
    KeepLocal,
    KeepRemote,
}

// ---------------------------------------------------------------------------
// Configuration import
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ImportSourceDto {
    OpenSsh,
    Putty,
    SecureCrt,
    FinalShell,
}

impl From<ImportSourceDto> for ImportSourceKind {
    fn from(source: ImportSourceDto) -> Self {
        match source {
            ImportSourceDto::OpenSsh => Self::OpenSshConfig,
            ImportSourceDto::Putty => Self::PuttyRegistry,
            ImportSourceDto::SecureCrt => Self::SecureCrtXml,
            ImportSourceDto::FinalShell => Self::FinalShellJson,
        }
    }
}

/// One host as shown in the confirmation list. Only displayed fields appear
/// here; the complete parsed entry stays in the backend until commit.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ImportEntryPreviewDto {
    pub name: String,
    pub group: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: String,
    pub private_key_path: String,
    pub certificate_path: String,
    pub has_password: bool,
    pub has_passphrase: bool,
    pub agent_forwarding: bool,
    pub startup_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ImportWarningDto {
    pub kind: String,
    pub entry_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ImportPreviewDto {
    /// Handle for [`crate::Backend::commit_import`]/[`crate::Backend::discard_import`].
    pub preview_id: u64,
    pub source_label: String,
    pub entries: Vec<ImportEntryPreviewDto>,
    pub warnings: Vec<ImportWarningDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ImportCommitResult {
    pub imported: u32,
    pub warnings: Vec<ImportWarningDto>,
}

pub(crate) fn entry_preview(draft: &ImportedSessionDraft) -> ImportEntryPreviewDto {
    let auth_method = match draft.auth_method {
        AuthMethod::Password => "password",
        AuthMethod::KeyFile => "key-file",
        AuthMethod::ManagedKey => "managed-key",
        AuthMethod::Agent => "agent",
        AuthMethod::KeyboardInteractive => "keyboard-interactive",
    };
    ImportEntryPreviewDto {
        name: draft.name.clone(),
        group: draft.group.clone(),
        host: draft.host.clone(),
        port: draft.port,
        username: draft.username.clone(),
        auth_method: auth_method.to_string(),
        private_key_path: draft.private_key_path.clone(),
        certificate_path: draft.certificate_path.clone(),
        has_password: draft
            .password
            .as_deref()
            .is_some_and(|value| !value.is_empty()),
        has_passphrase: draft
            .passphrase
            .as_deref()
            .is_some_and(|value| !value.is_empty()),
        agent_forwarding: draft.agent_forwarding,
        startup_command: draft.startup_command.clone(),
    }
}

pub(crate) fn warning_dto(
    kind: ImportIssueKind,
    entry_name: &str,
    reason: &ImportIssueReason,
) -> ImportWarningDto {
    let kind = match kind {
        ImportIssueKind::UnsupportedProtocol => "unsupported_protocol",
        ImportIssueKind::MissingRequiredField => "missing_required_field",
        ImportIssueKind::UnsupportedCredential => "unsupported_credential",
        ImportIssueKind::UnsupportedFeature => "unsupported_feature",
        ImportIssueKind::InvalidEntry => "invalid_entry",
    };
    let reason = match reason {
        ImportIssueReason::UnsupportedHostPattern => "unsupported_host_pattern".to_string(),
        ImportIssueReason::ProxyJumpNotImported => "proxy_jump_not_imported".to_string(),
        ImportIssueReason::IncludeNotExpanded => "include_not_expanded".to_string(),
        ImportIssueReason::MatchNotEvaluated { expression } => {
            format!("match_not_evaluated: {expression}")
        }
        ImportIssueReason::MultipleIdentityFiles => "multiple_identity_files".to_string(),
        ImportIssueReason::NoLiteralHostAlias => "no_literal_host_alias".to_string(),
        ImportIssueReason::MissingField { field } => match field {
            ImportField::Host => "missing_field: host".to_string(),
            ImportField::Username => "missing_field: username".to_string(),
            ImportField::Protocol => "missing_field: protocol".to_string(),
        },
        ImportIssueReason::UnsupportedProtocol { protocol } => {
            format!("unsupported_protocol: {protocol}")
        }
        ImportIssueReason::InvalidPort { value } => format!("invalid_port: {value}"),
        ImportIssueReason::CredentialProfileNotFound { profile } => {
            format!("credential_profile_not_found: {profile}")
        }
        ImportIssueReason::EncryptedPasswordNotImported => {
            "encrypted_password_not_imported".to_string()
        }
        ImportIssueReason::PasswordCouldNotBeDecoded => "password_could_not_be_decoded".to_string(),
        ImportIssueReason::KeyReferenceNotImported => "key_reference_not_imported".to_string(),
        ImportIssueReason::GlobalPublicKeyPathMissing => {
            "global_public_key_path_missing".to_string()
        }
        ImportIssueReason::AgentForwardingUnresolved { value } => {
            format!("agent_forwarding_unresolved: {value}")
        }
    };
    ImportWarningDto {
        kind: kind.to_string(),
        entry_name: entry_name.to_string(),
        reason,
    }
}

pub(crate) fn status_dto(status: &SyncStatus) -> SyncStatusDto {
    match status {
        SyncStatus::Idle => SyncStatusDto::Idle,
        SyncStatus::Syncing => SyncStatusDto::Idle,
        SyncStatus::RemoteBindingRequired { provider } => SyncStatusDto::RemoteBindingRequired {
            provider: (*provider).into(),
        },
        SyncStatus::Pulled { at } => SyncStatusDto::Pulled {
            at_ms: at.saturating_mul(1_000),
        },
        SyncStatus::Pushed { at } => SyncStatusDto::Pushed {
            at_ms: at.saturating_mul(1_000),
        },
        SyncStatus::UpToDate { at } => SyncStatusDto::UpToDate {
            at_ms: at.saturating_mul(1_000),
        },
        SyncStatus::PullRequired { remote_at, reason } => SyncStatusDto::PullRequired {
            remote_at_ms: remote_at.map(|at| at.saturating_mul(1_000)),
            reason: reason.clone().into(),
        },
        SyncStatus::Error(message) => SyncStatusDto::Failed {
            failure: SyncFailureDto {
                kind: SyncFailureKindDto::Other,
                detail: sanitize_detail(message),
            },
        },
    }
}

/// Keep failure text bounded and strip anything that could carry a token.
pub(crate) fn sanitize_detail(detail: &str) -> String {
    let mut sanitized = detail.lines().next().unwrap_or_default().trim().to_string();
    let lower = sanitized.to_ascii_lowercase();
    for marker in ["bearer ", "token ", "password ", "authorization:"] {
        if let Some(index) = lower.find(marker) {
            sanitized.truncate(index + marker.len());
            sanitized.push_str("<redacted>");
        }
    }
    if sanitized.len() > 200 {
        sanitized.truncate(200);
    }
    sanitized
}
