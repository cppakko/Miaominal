//! Miaominal Android bridge.
//!
//! The crate exposes the mobile backend to Kotlin through UniFFI proc-macros.
//! Interface metadata is embedded in the native library; Kotlin bindings are
//! generated from the built `.so` by `miaominal-uniffi-bindgen`.

mod agent;
mod backend;
mod build_info;
mod credentials;
mod data;
mod dto;
mod error;
mod events;
mod forward;
mod local_file;
mod platform;
mod runtime;
mod session;
mod sftp;
mod sync;
mod sync_dto;
#[cfg(test)]
mod agent_integration_tests;
#[cfg(test)]
mod forward_integration_tests;
#[cfg(test)]
mod session_integration_tests;
#[cfg(test)]
mod sftp_integration_tests;
#[cfg(test)]
mod sync_integration_tests;
#[cfg(test)]
mod test_support;

pub use agent::{
    AgentAuthenticationPromptDto, AgentChannelEvent, AgentEnvVar, AgentEventBatch,
    AgentHostKeyDecision, AgentHostKeyPromptDto, AgentPromptDto, AgentSshSession,
};
pub use backend::{Backend, BackendConfig, BackendSnapshot};
pub use build_info::BuildInfo;
pub use dto::{
    AuthMethodDto, CredentialAction, CredentialHealth, CredentialKindDto, ForwardKindDto,
    ForwardRuleDto, HostDto, HostUpdate, KnownHostDto, ManagedKeyAlgorithmDto, ManagedKeyDto,
    ManagedKeySourceDto, ProfileKindDto, ProxyDto, ProxyUpdate, SettingsDto, SettingsUpdate,
    SnippetDto, SnippetUpdate, VaultModeDto, VaultSnapshot,
};
pub use error::BackendError;
pub use events::{BackendEvent, ConfigArea, EventBatch, EventSubscription, SequencedEvent};
pub use forward::{
    ForwardFailure, ForwardFailureKind, ForwardPhase, ForwardRequest, ForwardRequestKind,
    ForwardSession, ForwardSnapshot,
};
pub use local_file::LocalFileHandle;
pub use platform::{LogLevel, PlatformHost, SafEntry, SafEntryKind, SafWriteStart};
pub use session::{
    MonitorFailureKind, MonitorMetricsDto, MonitorStatus, SshHostKeyDecision, SshMonitorSnapshot,
    SshPromptDto, SshRequestKind, SshSession, SshSessionPhase, SshSessionRequest,
    SshSessionSnapshot, TerminalFrameCell, TerminalFrameModes, TerminalFrameRow, TerminalRenderFrame,
    TerminalSelectionDto, TerminalSelectionKindDto, TerminalUpdate, cell_flags,
};
pub use sftp::{
    SftpDirectorySnapshot, SftpEntryDto, SftpEntryKindDto, SftpOperation, SftpOperationError,
    SftpRequest, SftpRequestKind, SftpSession, SftpSessionPhase, SftpSessionSnapshot, SftpTransfer,
    SftpTransferChildDto, SftpTransferChildStateDto, SftpTransferDirection,
    SftpTransferFailureKind, SftpTransferSnapshot, SftpTransferState,
};
pub use sync_dto::{
    ImportCommitResult, ImportEntryPreviewDto, ImportPreviewDto, ImportSourceDto, ImportWarningDto,
    SyncCapabilityDto, SyncCapabilityReasonDto, SyncCapabilityStateDto, SyncConfigDto,
    SyncConfigUpdate, SyncConflictDto, SyncCredentialKind, SyncEtagKindDto, SyncFailureDto,
    SyncFailureKindDto, SyncInterventionReasonDto, SyncMigrationResult, SyncPhaseDto,
    SyncProviderDto, SyncRemoteStateDto, SyncResolution, SyncRunResult, SyncSnapshotDto,
    SyncStatusDto,
};

/// Version of the bridge contract; kept in sync with the Kotlin client.
pub const ABI_VERSION: &str = "miaominal-android/10";

uniffi::setup_scaffolding!();
