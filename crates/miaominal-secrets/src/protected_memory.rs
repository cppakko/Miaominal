use anyhow::{Result, anyhow, bail};
use memsafe::Secret;
use rand::{TryRng, rngs::SysRng};
use std::fmt;
use std::sync::{Arc, Mutex};
use zeroize::Zeroize;

pub const MAX_VAULT_PASSPHRASE_BYTES: usize = 1024;

#[derive(Clone)]
pub struct ProtectedPassphrase {
    inner: Arc<ProtectedPassphraseInner>,
}

struct ProtectedPassphraseInner {
    len: usize,
    secret: Mutex<Option<Secret<MAX_VAULT_PASSPHRASE_BYTES>>>,
    cache_key: Mutex<Option<Secret<32>>>,
}

impl ProtectedPassphrase {
    pub fn try_from_string(mut value: String) -> Result<Self> {
        let len = value.len();
        if len > MAX_VAULT_PASSPHRASE_BYTES {
            value.zeroize();
            bail!("local vault passphrase must not exceed {MAX_VAULT_PASSPHRASE_BYTES} bytes");
        }

        let secret = Secret::try_from(value).map_err(|(mut source, error)| {
            source.zeroize();
            anyhow!("failed to allocate protected memory for local vault passphrase: {error}")
        })?;
        let mut cache_key_result = Ok(());
        let mut os_rng = SysRng;
        let cache_key = Secret::new_with(|bytes| {
            cache_key_result = os_rng.try_fill_bytes(bytes).map_err(|error| {
                anyhow!("failed to generate protected memory cache key from OS RNG: {error}")
            });
        })
        .map_err(|error| {
            anyhow!("failed to allocate protected memory for local vault cache key: {error}")
        })?;
        cache_key_result?;

        Ok(Self {
            inner: Arc::new(ProtectedPassphraseInner {
                len,
                secret: Mutex::new(Some(secret)),
                cache_key: Mutex::new(Some(cache_key)),
            }),
        })
    }

    pub fn len(&self) -> usize {
        self.inner.len
    }

    pub fn is_empty(&self) -> bool {
        self.inner.len == 0
    }

    pub fn shares_allocation_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn revoke(&self) {
        let mut secret = match self.inner.secret.lock() {
            Ok(secret) => secret,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut cache_key = match self.inner.cache_key.lock() {
            Ok(cache_key) => cache_key,
            Err(poisoned) => poisoned.into_inner(),
        };

        secret.take();
        cache_key.take();
    }

    pub(crate) fn with_bytes<T>(&self, operation: impl FnOnce(&[u8]) -> Result<T>) -> Result<T> {
        let mut secret = self
            .inner
            .secret
            .lock()
            .map_err(|_| anyhow!("local vault protected-memory lock poisoned"))?;
        let secret = secret
            .as_mut()
            .ok_or_else(|| anyhow!("local vault session has been revoked"))?;
        let view = secret
            .read()
            .map_err(|error| anyhow!("failed to unseal local vault passphrase: {error}"))?;

        operation(&view[..self.inner.len])
    }

    pub(crate) fn with_session_material<T>(
        &self,
        operation: impl FnOnce(&[u8], &[u8; 32]) -> Result<T>,
    ) -> Result<T> {
        let mut secret = self
            .inner
            .secret
            .lock()
            .map_err(|_| anyhow!("local vault protected-memory lock poisoned"))?;
        let mut cache_key = self
            .inner
            .cache_key
            .lock()
            .map_err(|_| anyhow!("local vault protected-memory cache-key lock poisoned"))?;
        let secret = secret
            .as_mut()
            .ok_or_else(|| anyhow!("local vault session has been revoked"))?;
        let cache_key = cache_key
            .as_mut()
            .ok_or_else(|| anyhow!("local vault session has been revoked"))?;
        let passphrase_view = secret
            .read()
            .map_err(|error| anyhow!("failed to unseal local vault passphrase: {error}"))?;
        let cache_key_view = cache_key
            .read()
            .map_err(|error| anyhow!("failed to unseal local vault cache key: {error}"))?;

        operation(&passphrase_view[..self.inner.len], &cache_key_view)
    }
}

impl fmt::Debug for ProtectedPassphrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProtectedPassphrase([REDACTED])")
    }
}

pub(crate) struct ProtectedDerivedKey {
    secret: Secret<32>,
}

impl ProtectedDerivedKey {
    pub(crate) fn try_new(operation: impl FnOnce(&mut [u8; 32]) -> Result<()>) -> Result<Self> {
        let mut operation_result = Ok(());
        let secret = Secret::new_with(|bytes| {
            operation_result = operation(bytes);
        })
        .map_err(|error| anyhow!("failed to allocate protected memory for derived key: {error}"))?;
        operation_result?;

        Ok(Self { secret })
    }

    pub(crate) fn with_bytes<T>(
        &mut self,
        operation: impl FnOnce(&[u8; 32]) -> Result<T>,
    ) -> Result<T> {
        let view = self
            .secret
            .read()
            .map_err(|error| anyhow!("failed to unseal derived key: {error}"))?;
        operation(&view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_passphrase_accepts_the_maximum_byte_length() {
        let passphrase =
            ProtectedPassphrase::try_from_string("a".repeat(MAX_VAULT_PASSPHRASE_BYTES))
                .expect("maximum-length passphrase should be accepted");

        assert_eq!(passphrase.len(), MAX_VAULT_PASSPHRASE_BYTES);
    }

    #[test]
    fn protected_passphrase_rejects_values_over_the_byte_limit() {
        let error =
            ProtectedPassphrase::try_from_string("a".repeat(MAX_VAULT_PASSPHRASE_BYTES + 1))
                .expect_err("oversized passphrase should be rejected");

        assert!(error.to_string().contains("1024 bytes"));
    }

    #[test]
    fn protected_passphrase_counts_utf8_bytes() {
        let accepted = "猫".repeat(MAX_VAULT_PASSPHRASE_BYTES / "猫".len());
        let rejected = "猫".repeat(MAX_VAULT_PASSPHRASE_BYTES / "猫".len() + 1);

        assert!(ProtectedPassphrase::try_from_string(accepted).is_ok());
        assert!(ProtectedPassphrase::try_from_string(rejected).is_err());
    }

    #[test]
    fn revoking_one_clone_revokes_every_clone() {
        let passphrase = ProtectedPassphrase::try_from_string("correct horse".to_string())
            .expect("passphrase should be protected");
        let clone = passphrase.clone();

        clone.revoke();

        assert!(passphrase.with_bytes(|_| Ok(())).is_err());
        assert!(clone.with_bytes(|_| Ok(())).is_err());
    }

    #[test]
    fn protected_passphrase_provides_a_random_cache_key_with_its_bytes() {
        let passphrase = ProtectedPassphrase::try_from_string("correct horse".to_string())
            .expect("passphrase should be protected");
        let other = ProtectedPassphrase::try_from_string("correct horse".to_string())
            .expect("second passphrase should be protected");

        let cache_key = passphrase
            .with_session_material(|bytes, cache_key| {
                assert_eq!(bytes, b"correct horse");
                Ok(*cache_key)
            })
            .expect("passphrase and cache key should be readable through protected memory");
        let other_cache_key = other
            .with_session_material(|_, cache_key| Ok(*cache_key))
            .expect("second cache key should be readable through protected memory");

        assert_ne!(cache_key, other_cache_key);
    }

    #[test]
    fn clones_share_the_same_cache_key() {
        let passphrase = ProtectedPassphrase::try_from_string("correct horse".to_string())
            .expect("passphrase should be protected");
        let clone = passphrase.clone();

        let cache_key = passphrase
            .with_session_material(|_, cache_key| Ok(*cache_key))
            .expect("cache key should be readable through protected memory");
        let clone_cache_key = clone
            .with_session_material(|_, cache_key| Ok(*cache_key))
            .expect("clone cache key should be readable through protected memory");

        assert_eq!(cache_key, clone_cache_key);
    }

    #[test]
    fn revoking_one_clone_revokes_passphrase_and_cache_key_for_every_clone() {
        let passphrase = ProtectedPassphrase::try_from_string("correct horse".to_string())
            .expect("passphrase should be protected");
        let clone = passphrase.clone();

        clone.revoke();

        assert!(passphrase.with_bytes(|_| Ok(())).is_err());
        assert!(clone.with_bytes(|_| Ok(())).is_err());
        assert!(passphrase.with_session_material(|_, _| Ok(())).is_err());
        assert!(clone.with_session_material(|_, _| Ok(())).is_err());
    }

    #[test]
    fn clones_report_that_they_share_the_same_allocation() {
        let passphrase = ProtectedPassphrase::try_from_string("correct horse".to_string())
            .expect("passphrase should be protected");
        let clone = passphrase.clone();
        let other = ProtectedPassphrase::try_from_string("correct horse".to_string())
            .expect("second passphrase should be protected");

        assert!(passphrase.shares_allocation_with(&clone));
        assert!(!passphrase.shares_allocation_with(&other));
    }

    #[test]
    fn protected_passphrase_debug_is_redacted() {
        let passphrase = ProtectedPassphrase::try_from_string("correct horse".to_string())
            .expect("passphrase should be protected");

        assert_eq!(format!("{passphrase:?}"), "ProtectedPassphrase([REDACTED])");
    }

    #[test]
    fn protected_derived_key_is_initialized_and_read_through_guards() {
        let mut key = ProtectedDerivedKey::try_new(|bytes| {
            bytes.fill(7);
            Ok(())
        })
        .expect("derived key should be protected");

        key.with_bytes(|bytes| {
            assert_eq!(bytes, &[7; 32]);
            Ok(())
        })
        .expect("derived key should be readable through its guard");
    }
}
