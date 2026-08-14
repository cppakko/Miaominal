use super::payload::{
    apply_plaintext_payload, build_payload, build_plaintext_payload, decrypt_remote_payload,
    local_data_revision, parse_remote_payload,
};
use super::providers::{PullOutcome, PushCondition, PushOutcome, RemoteBackend};
use super::store::SyncConfigStore;
use crate::{SyncInterventionReason, SyncPayload, SyncPlaintextPayload, SyncProvider, SyncStatus};
use anyhow::{Context, Result};
use miaominal_secrets::{CredentialStore, ProtectedPassphrase, SecretStore};
use miaominal_storage::config_store::store::{SessionStore, SnippetStore};
use miaominal_storage::keychain_store::ManagedKeyStore;
use miaominal_storage::{ProxyStore, SettingsStore};
use std::{error::Error as StdError, fmt};

/// Result of a lightweight remote check. The payload is fetched (or answered
/// with 304 when the persisted ETag matches) but never applied locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteSyncState {
    Disabled,
    BindingRequired(SyncProvider),
    Missing,
    NotModified,
    UpToDate,
    Updated {
        synced_at: u64,
        etag: Option<String>,
        payload_id: Option<String>,
    },
}

enum RemotePayloadState {
    BindingRequired(SyncProvider),
    Missing { etag: Option<String> },
    NotModified,
    Current(SyncPayload, Option<String>),
    Changed(SyncPayload, Option<String>),
}

fn automatic_push_requires_confirmation(
    condition: &PushCondition,
    observed_remote_at: Option<u64>,
    force: bool,
) -> bool {
    matches!(condition, PushCondition::Unconditional) && observed_remote_at.is_some() && !force
}

struct LocalSyncSnapshot {
    plaintext: SyncPlaintextPayload,
    revision: String,
}

#[derive(Debug)]
struct SyncConfigurationChangedDuringPull;

impl fmt::Display for SyncConfigurationChangedDuringPull {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sync configuration changed while the remote payload was being applied")
    }
}

impl StdError for SyncConfigurationChangedDuringPull {}

pub struct SyncEngine {
    pub config_store: SyncConfigStore,
}

impl Default for SyncEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for SyncEngine {
    fn clone(&self) -> Self {
        Self {
            config_store: self.config_store.clone(),
        }
    }
}

impl SyncEngine {
    pub fn new() -> Self {
        let config_store = SyncConfigStore::load().unwrap_or_else(|err| {
            log::warn!("failed to load sync config: {err:?}");
            SyncConfigStore::fallback()
        });
        Self { config_store }
    }

    pub fn new_locked_vault() -> Self {
        let config_store = SyncConfigStore::load_with_locked_vault().unwrap_or_else(|err| {
            log::warn!("failed to load locked vault sync config: {err:?}");
            SyncConfigStore::fallback_with_locked_vault()
        });
        Self { config_store }
    }

    pub fn new_vault(passphrase: ProtectedPassphrase) -> Result<Self> {
        let config_store = SyncConfigStore::load_with_vault(passphrase.clone()).or_else(|err| {
            log::warn!("failed to load vault sync config: {err:?}");
            SyncConfigStore::fallback_with_vault(passphrase)
        })?;
        Ok(Self { config_store })
    }

    pub fn new_with_credentials(credentials: CredentialStore) -> Self {
        let config_store = SyncConfigStore::load_with_credentials(credentials.clone())
            .unwrap_or_else(|err| {
                log::warn!("failed to load sync config with shared credentials: {err:?}");
                SyncConfigStore::fallback_with_credentials(credentials)
            });
        Self { config_store }
    }

    /// Read data from all stores, build an encrypted payload, and push it to the
    /// configured backend. Returns `SyncStatus::Idle` when sync is disabled.
    pub async fn push(
        &mut self,
        session_store: &SessionStore,
        proxy_store: &ProxyStore,
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
        secret_store: &SecretStore,
        settings_store: &SettingsStore,
    ) -> Result<SyncStatus> {
        self.push_internal(
            session_store,
            proxy_store,
            snippet_store,
            key_store,
            secret_store,
            settings_store,
            false,
        )
        .await
    }

    pub async fn push_force(
        &mut self,
        session_store: &SessionStore,
        proxy_store: &ProxyStore,
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
        secret_store: &SecretStore,
        settings_store: &SettingsStore,
    ) -> Result<SyncStatus> {
        self.push_internal(
            session_store,
            proxy_store,
            snippet_store,
            key_store,
            secret_store,
            settings_store,
            true,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn push_internal(
        &mut self,
        session_store: &SessionStore,
        proxy_store: &ProxyStore,
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
        secret_store: &SecretStore,
        settings_store: &SettingsStore,
        force: bool,
    ) -> Result<SyncStatus> {
        if !self.sync_enabled_for_provider() {
            return Ok(SyncStatus::Idle);
        }

        self.config_store.sync_from_disk();
        let remote = self.remote_payload_state(true).await?;
        let start_config_revision = self.config_store.config.config_revision;
        let (condition, parent_payload_id, observed_remote_at) = match remote {
            RemotePayloadState::BindingRequired(provider) => {
                if provider == SyncProvider::GithubGist
                    && self.config_store.config.gist_id.is_none()
                {
                    (PushCondition::Unconditional, None, None)
                } else {
                    return Ok(SyncStatus::RemoteBindingRequired { provider });
                }
            }
            RemotePayloadState::Missing { etag } => (
                etag.map_or(PushCondition::MustNotExist, PushCondition::IfMatch),
                None,
                None,
            ),
            RemotePayloadState::NotModified => (
                self.config_store
                    .config
                    .remote_etag
                    .clone()
                    .map_or(PushCondition::Unconditional, PushCondition::IfMatch),
                self.config_store.config.remote_payload_id.clone(),
                Some(self.config_store.config.last_sync_at),
            ),
            RemotePayloadState::Current(payload, etag) => (
                etag.map_or(PushCondition::Unconditional, PushCondition::IfMatch),
                non_empty_payload_id(&payload),
                Some(payload.synced_at),
            ),
            RemotePayloadState::Changed(payload, etag) => {
                if !force {
                    return Ok(SyncStatus::PullRequired {
                        remote_at: Some(payload.synced_at),
                        reason: SyncInterventionReason::RemoteChangedBeforePush,
                    });
                }
                (
                    etag.map_or(PushCondition::Unconditional, PushCondition::IfMatch),
                    non_empty_payload_id(&payload),
                    Some(payload.synced_at),
                )
            }
        };
        // A marker followed by an unconditional write is still racy: another
        // device may write between the final GET and our PUT/PATCH. Automatic
        // sync therefore refuses to overwrite an existing remote when the
        // provider supplies no atomic write precondition. The explicit force
        // action is the user-confirmed escape hatch for such providers.
        if automatic_push_requires_confirmation(&condition, observed_remote_at, force) {
            return Ok(SyncStatus::PullRequired {
                remote_at: observed_remote_at,
                reason: SyncInterventionReason::UnsafeProviderWrite,
            });
        }
        let passphrase = self.sync_passphrase()?;

        let local = self.local_snapshot(
            session_store,
            proxy_store,
            snippet_store,
            key_store,
            secret_store,
            settings_store,
        )?;

        let payload = build_payload(
            &self.config_store.config.device_id,
            parent_payload_id.clone(),
            &local.plaintext,
            &passphrase,
        )?;
        let payload_json =
            serde_json::to_string(&payload).context("failed to serialize sync payload")?;
        let synced_at = payload.synced_at;

        let mut backend = match RemoteBackend::build(&self.config_store)? {
            Some(backend) => backend,
            None => return Ok(SyncStatus::Idle),
        };
        let outcome = backend.push(&payload_json, &condition).await?;
        let PushOutcome::Pushed {
            provider_resource_id,
            etag,
        } = outcome
        else {
            return Ok(SyncStatus::PullRequired {
                remote_at: observed_remote_at,
                reason: SyncInterventionReason::RemoteChangedBeforePush,
            });
        };
        let payload_id = payload.payload_id.clone();
        let persisted = self
            .config_store
            .update_if_revision(start_config_revision, |c| {
                if let Some(resource_id) = provider_resource_id {
                    c.gist_id = Some(resource_id);
                }
                c.last_sync_at = synced_at;
                c.remote_etag = etag;
                c.remote_payload_id = Some(payload_id);
                // Confirm only the exact local snapshot that was uploaded. A
                // concurrent edit produces a different current revision and stays
                // dirty for the next auto-sync pass.
                c.last_synced_local_revision = Some(local.revision);
            })?;
        if !persisted {
            return Ok(SyncStatus::PullRequired {
                remote_at: Some(synced_at),
                reason: SyncInterventionReason::SyncConfigurationChanged,
            });
        }

        Ok(SyncStatus::Pushed { at: synced_at })
    }

    /// Check whether the configured remote has a newer payload without applying
    /// it locally. This is the polling entry point used by auto-sync.
    pub async fn remote_state(&mut self) -> Result<RemoteSyncState> {
        if !self.sync_enabled_for_provider() {
            return Ok(RemoteSyncState::Disabled);
        }
        self.config_store.sync_from_disk();
        Ok(match self.remote_payload_state(true).await? {
            RemotePayloadState::BindingRequired(provider) => {
                RemoteSyncState::BindingRequired(provider)
            }
            RemotePayloadState::Missing { .. } => RemoteSyncState::Missing,
            RemotePayloadState::NotModified => RemoteSyncState::NotModified,
            RemotePayloadState::Current(_, _) => RemoteSyncState::UpToDate,
            RemotePayloadState::Changed(payload, etag) => RemoteSyncState::Updated {
                synced_at: payload.synced_at,
                etag,
                payload_id: non_empty_payload_id(&payload),
            },
        })
    }

    /// Pull a payload from the configured backend and apply it locally using
    /// last-write-wins: only overwrites local data when the remote `synced_at`
    /// is strictly newer than the last local sync timestamp.
    pub async fn pull(
        &mut self,
        session_store: &SessionStore,
        proxy_store: &ProxyStore,
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
        secret_store: &SecretStore,
        settings_store: &mut SettingsStore,
    ) -> Result<SyncStatus> {
        self.pull_internal(
            session_store,
            proxy_store,
            snippet_store,
            key_store,
            secret_store,
            settings_store,
        )
        .await
    }

    async fn pull_internal(
        &mut self,
        session_store: &SessionStore,
        proxy_store: &ProxyStore,
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
        secret_store: &SecretStore,
        settings_store: &mut SettingsStore,
    ) -> Result<SyncStatus> {
        if !self.sync_enabled_for_provider() {
            return Ok(SyncStatus::Idle);
        }

        self.config_store.sync_from_disk();
        let start_config_revision = self.config_store.config.config_revision;
        let start_revision = self.local_revision(
            session_store,
            proxy_store,
            snippet_store,
            key_store,
            secret_store,
            settings_store,
        )?;
        // A real pull must fetch the representation even when a preceding
        // poll, or a legacy config without the new revision baseline, already
        // has a matching ETag.
        let (payload, etag) = match self.remote_payload_state(false).await? {
            RemotePayloadState::BindingRequired(provider) => {
                return Ok(SyncStatus::RemoteBindingRequired { provider });
            }
            RemotePayloadState::Missing { .. }
            | RemotePayloadState::NotModified
            | RemotePayloadState::Current(_, _) => {
                return Ok(SyncStatus::UpToDate {
                    at: self.config_store.config.last_sync_at,
                });
            }
            RemotePayloadState::Changed(payload, etag) => (payload, etag),
        };

        let passphrase = self.sync_passphrase()?;
        let remote_synced_at = payload.synced_at;
        let plaintext = decrypt_remote_payload(&payload, &passphrase)?;
        let _sync_guard = miaominal_secrets::lock_sync_data();
        let current_revision = self.local_revision(
            session_store,
            proxy_store,
            snippet_store,
            key_store,
            secret_store,
            settings_store,
        )?;
        if current_revision != start_revision {
            return Ok(SyncStatus::PullRequired {
                remote_at: Some(remote_synced_at),
                reason: SyncInterventionReason::LocalChangedDuringPull,
            });
        }
        settings_store.reload_from_disk()?;
        let applied_revision = local_data_revision(&plaintext)?;
        let remote_payload_id = non_empty_payload_id(&payload);

        let apply_result = apply_plaintext_payload(
            &plaintext,
            session_store,
            proxy_store,
            snippet_store,
            key_store,
            secret_store,
            settings_store,
            || {
                let persisted =
                    self.config_store
                        .update_if_revision(start_config_revision, |c| {
                            c.last_sync_at = remote_synced_at;
                            c.remote_etag = etag.clone();
                            c.remote_payload_id = remote_payload_id.clone();
                            c.last_synced_local_revision = Some(applied_revision.clone());
                        })?;
                if !persisted {
                    return Err(SyncConfigurationChangedDuringPull.into());
                }
                Ok(())
            },
        );
        if let Err(error) = apply_result {
            self.config_store.sync_from_disk();
            if error
                .downcast_ref::<SyncConfigurationChangedDuringPull>()
                .is_some()
            {
                return Ok(SyncStatus::PullRequired {
                    remote_at: Some(remote_synced_at),
                    reason: SyncInterventionReason::SyncConfigurationChanged,
                });
            }
            return Err(error);
        }

        Ok(SyncStatus::Pulled {
            at: remote_synced_at,
        })
    }

    async fn remote_payload_state(&mut self, conditional: bool) -> Result<RemotePayloadState> {
        self.config_store.sync_from_disk();
        let config_revision = self.config_store.config.config_revision;
        let backend = match RemoteBackend::build(&self.config_store)? {
            Some(backend) => backend,
            None => return Ok(RemotePayloadState::Missing { etag: None }),
        };
        let etag = conditional
            .then(|| self.config_store.config.remote_etag.clone())
            .flatten();
        let outcome = backend.pull(etag.as_deref()).await?;
        self.config_store.sync_from_disk();
        anyhow::ensure!(
            self.config_store.config.config_revision == config_revision,
            "sync configuration changed during the remote check"
        );
        match outcome {
            PullOutcome::BindingRequired { provider } => {
                Ok(RemotePayloadState::BindingRequired(provider))
            }
            PullOutcome::Missing { etag } => Ok(RemotePayloadState::Missing { etag }),
            PullOutcome::NotModified => Ok(RemotePayloadState::NotModified),
            PullOutcome::Payload(payload) => {
                let parsed = parse_remote_payload(&payload.json)?;
                if self.remote_payload_is_current(&parsed) {
                    if self.config_store.config.remote_etag != payload.etag {
                        let updated =
                            self.config_store.update_if_revision(config_revision, |c| {
                                c.remote_etag = payload.etag.clone();
                            })?;
                        anyhow::ensure!(
                            updated,
                            "sync configuration changed while refreshing the remote ETag"
                        );
                    }
                    Ok(RemotePayloadState::Current(parsed, payload.etag))
                } else {
                    Ok(RemotePayloadState::Changed(parsed, payload.etag))
                }
            }
        }
    }

    pub fn sync_enabled_for_provider(&self) -> bool {
        self.config_store.config.provider != SyncProvider::None
    }

    #[allow(clippy::too_many_arguments)]
    pub fn local_revision(
        &self,
        session_store: &SessionStore,
        proxy_store: &ProxyStore,
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
        secret_store: &SecretStore,
        settings_store: &SettingsStore,
    ) -> Result<String> {
        Ok(self
            .local_snapshot(
                session_store,
                proxy_store,
                snippet_store,
                key_store,
                secret_store,
                settings_store,
            )?
            .revision)
    }

    #[allow(clippy::too_many_arguments)]
    fn local_snapshot(
        &self,
        session_store: &SessionStore,
        proxy_store: &ProxyStore,
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
        secret_store: &SecretStore,
        settings_store: &SettingsStore,
    ) -> Result<LocalSyncSnapshot> {
        let _sync_guard = miaominal_secrets::lock_sync_data();
        let sessions = session_store
            .read_sessions_content()?
            .map(|content| session_store.parse_sessions(&content))
            .transpose()?
            .unwrap_or_default();
        let proxies = proxy_store.load(secret_store)?;
        let snippets = snippet_store.load()?;
        let managed_keys = key_store.load()?;
        let settings = settings_store.read_current()?.synced_settings();
        let plaintext = build_plaintext_payload(
            &sessions,
            &proxies,
            &snippets,
            &managed_keys,
            &settings,
            secret_store,
        )?;
        let revision = local_data_revision(&plaintext)?;
        Ok(LocalSyncSnapshot {
            plaintext,
            revision,
        })
    }

    fn remote_payload_is_current(&self, payload: &SyncPayload) -> bool {
        if payload.payload_id.is_empty() {
            payload.synced_at <= self.config_store.config.last_sync_at
        } else {
            self.config_store.config.remote_payload_id.as_deref()
                == Some(payload.payload_id.as_str())
        }
    }

    fn sync_passphrase(&self) -> Result<String> {
        let passphrase = self
            .config_store
            .get_passphrase()?
            .filter(|passphrase| !passphrase.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("sync passphrase not configured"))?;
        Ok(passphrase)
    }
}

fn non_empty_payload_id(payload: &SyncPayload) -> Option<String> {
    (!payload.payload_id.is_empty()).then(|| payload.payload_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_configuration_conflict_marker_survives_rollback_context() {
        let error = anyhow::Error::new(SyncConfigurationChangedDuringPull)
            .context("sync pull failed; local changes were rolled back");
        assert!(
            error
                .downcast_ref::<SyncConfigurationChangedDuringPull>()
                .is_some()
        );
    }

    #[test]
    fn ordinary_pull_io_errors_are_not_configuration_conflicts() {
        let error = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "simulated write failure",
        ))
        .context("sync pull failed; local changes were rolled back");
        assert!(
            error
                .downcast_ref::<SyncConfigurationChangedDuringPull>()
                .is_none()
        );
    }

    #[test]
    fn automatic_push_rejects_existing_remote_without_atomic_precondition() {
        assert!(automatic_push_requires_confirmation(
            &PushCondition::Unconditional,
            Some(42),
            false,
        ));
        assert!(!automatic_push_requires_confirmation(
            &PushCondition::IfMatch("\"etag-v1\"".into()),
            Some(42),
            false,
        ));
        assert!(!automatic_push_requires_confirmation(
            &PushCondition::Unconditional,
            None,
            false,
        ));
        assert!(!automatic_push_requires_confirmation(
            &PushCondition::Unconditional,
            Some(42),
            true,
        ));
    }
}
