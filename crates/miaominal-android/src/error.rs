//! Error categories shared by the Android bridge.
//!
//! The categories are stable identifiers for the Kotlin side; user-facing text
//! is produced by the UI layer, not by these messages.
//!
//! Struct fields intentionally use distinct names across variants: the generated
//! Kotlin sealed class cannot resolve shared field names, and a field named
//! `message` would collide with `Throwable.message`.

use thiserror::Error;

#[derive(Debug, Error, uniffi::Error)]
pub enum BackendError {
    #[error("the backend handle is closed")]
    Closed,
    #[error("invalid argument: {detail}")]
    InvalidArgument { detail: String },
    #[error("native runtime is unavailable: {runtime_failure}")]
    RuntimeUnavailable { runtime_failure: String },
    #[error("platform bridge failure: {platform_failure}")]
    Platform { platform_failure: String },
    #[error("platform bridge raised an unexpected error: {reason}")]
    UnexpectedPlatform { reason: String },
    #[error("authentication failed: {authentication_failure}")]
    AuthenticationFailed { authentication_failure: String },
    #[error("host key confirmation is required: {trust_detail}")]
    TrustPending { trust_detail: String },
    #[error("the local vault is locked")]
    VaultLocked,
    #[error("permission denied: {permission_detail}")]
    PermissionDenied { permission_detail: String },
    #[error("network error: {network_failure}")]
    Network { network_failure: String },
    #[error("conflict: {conflict_detail}")]
    Conflict { conflict_detail: String },
    #[error("the operation was cancelled")]
    Cancelled,
    #[error("the session is not ready: {session_detail}")]
    SessionNotReady { session_detail: String },
    #[error("the request {request_id} is no longer pending")]
    RequestExpired { request_id: u64 },
    #[error("unsupported capability: {capability}")]
    Unsupported { capability: String },
    #[error("storage is unavailable: {storage_failure}")]
    Storage { storage_failure: String },
    #[error("not enough space: {space_detail}")]
    NoSpace { space_detail: String },
    #[error("the local file handle is no longer usable: {handle_detail}")]
    LocalHandleInvalid { handle_detail: String },
    #[error("sync conflict: {conflict_detail}")]
    SyncConflict { conflict_detail: String },
    #[error("sync capability check failed: {capability_detail}")]
    SyncCapability { capability_detail: String },
    #[error("sync authentication failed: {sync_detail}")]
    SyncAuthentication { sync_detail: String },
    #[error("sync is not configured: {sync_detail}")]
    SyncNotConfigured { sync_detail: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for BackendError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::UnexpectedPlatform {
            reason: error.reason,
        }
    }
}

impl BackendError {
    /// Classify a storage/service failure into a stable category.
    ///
    /// The shared layers use `anyhow`; the bridge is the boundary that turns
    /// those messages into the interface contract.
    pub(crate) fn from_storage(error: anyhow::Error) -> Self {
        if miaominal_secrets::SecretStore::is_locked_error(&error) {
            return Self::VaultLocked;
        }
        let message = error
            .chain()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(": ");
        if message.contains("permission denied") || message.contains("os error 13") {
            return Self::PermissionDenied {
                permission_detail: message,
            };
        }
        if message.contains("os error 28")
            || message.contains("no space left")
            || message.contains("ENOSPC")
        {
            return Self::NoSpace {
                space_detail: message,
            };
        }
        if message.contains("vault decryption failed")
            || message.contains("authentication failed")
            || message.contains("wrong passphrase")
        {
            return Self::AuthenticationFailed {
                authentication_failure: message,
            };
        }
        Self::Storage {
            storage_failure: message,
        }
    }
}
