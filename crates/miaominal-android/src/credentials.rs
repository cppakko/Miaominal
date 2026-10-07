//! Credential backend hosted by the Android process.
//!
//! The shared secret stores still decide *what* to store (account naming,
//! managed-key chunking, vault encryption); this adapter only moves the
//! ciphertext across the FFI to the Kotlin Android Keystore implementation.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use miaominal_secrets::credential_backend::CredentialBackend;

use crate::error::BackendError;
use crate::platform::PlatformHost;

pub(crate) struct PlatformCredentialBackend {
    host: Arc<dyn PlatformHost>,
}

impl PlatformCredentialBackend {
    pub(crate) fn new(host: Arc<dyn PlatformHost>) -> Self {
        Self { host }
    }
}

fn platform_error(error: BackendError) -> anyhow::Error {
    anyhow!("platform credential backend failed: {error}")
}

impl CredentialBackend for PlatformCredentialBackend {
    fn name(&self) -> &'static str {
        "android-platform"
    }

    fn get(&self, _service: &str, account: &str) -> Result<Option<String>> {
        self.host
            .credential_get(account.to_string())
            .map_err(platform_error)
    }

    fn set(&self, _service: &str, account: &str, value: &str) -> Result<()> {
        self.host
            .credential_set(account.to_string(), value.to_string())
            .map_err(platform_error)
    }

    fn delete(&self, _service: &str, account: &str) -> Result<()> {
        self.host
            .credential_delete(account.to_string())
            .map_err(platform_error)
    }
}

/// Best-effort classification of a failed credential read for the UI.
pub(crate) fn classify_credential_error(error: &anyhow::Error) -> crate::dto::CredentialHealth {
    use crate::dto::CredentialHealth;

    let message = error.chain().map(ToString::to_string).collect::<Vec<_>>().join(": ");
    if message.contains("invalidated") || message.contains("KeyPermanentlyInvalidated") {
        return CredentialHealth::Invalidated;
    }
    if message.contains("corrupt")
        || message.contains("decrypt")
        || message.contains("decryption")
        || message.contains("parse")
    {
        return CredentialHealth::Corrupt;
    }
    CredentialHealth::Invalidated
}
