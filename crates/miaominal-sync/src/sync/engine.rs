use super::payload::{
    apply_plaintext_payload, build_payload, build_plaintext_payload, decrypt_remote_payload,
    local_data_revision, normalize_remote_payload, parse_remote_payload,
};
use super::providers::{PullOutcome, PushCondition, PushOutcome, RemoteBackend};
use super::store::SyncConfigStore;
use crate::capability::{
    CapabilityError, CapabilityReason, CapabilityReport, EtagKind, classify_etag,
};
use crate::{SyncInterventionReason, SyncPayload, SyncPlaintextPayload, SyncProvider, SyncStatus};
use anyhow::{Context, Result};
use miaominal_secrets::{CredentialStore, ProtectedPassphrase, SecretStore};
use miaominal_storage::config_store::store::{SessionStore, SnippetStore};
use miaominal_storage::keychain_store::ManagedKeyStore;
use miaominal_storage::{ProxyStore, SettingsStore};
use std::{error::Error as StdError, fmt};

/// Result of a lightweight remote check. The payload is fetched (or answered
/// with 304 when the persisted ETag matches) but never applied locally. An
/// `Updated` result describes a changed remote representation, not necessarily
/// changed synchronized content.
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
        content_revision: Option<String>,
    },
}

/// Three-way relationship between the current local content, the fetched
/// remote content, and the content recorded by the last successful sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncContentRelation {
    Identical,
    LocalChanged,
    RemoteChanged,
    Diverged,
    MissingBaseline,
}

pub fn classify_content_revisions(
    local_revision: &str,
    remote_revision: &str,
    baseline_revision: Option<&str>,
) -> SyncContentRelation {
    if local_revision == remote_revision {
        return SyncContentRelation::Identical;
    }

    let Some(baseline_revision) = baseline_revision else {
        return SyncContentRelation::MissingBaseline;
    };
    match (
        local_revision == baseline_revision,
        remote_revision == baseline_revision,
    ) {
        (false, true) => SyncContentRelation::LocalChanged,
        (true, false) => SyncContentRelation::RemoteChanged,
        (false, false) => SyncContentRelation::Diverged,
        (true, true) => unreachable!("different revisions cannot both equal the baseline"),
    }
}

enum RemotePayloadState {
    BindingRequired(SyncProvider),
    Missing { etag: Option<String> },
    NotModified,
    Current(SyncPayload, Option<String>),
    Changed(SyncPayload, Option<String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PushMode {
    Automatic,
    Manual,
    Force,
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
            PushMode::Automatic,
        )
        .await
    }

    /// Push after an explicit user action. WebDAV uses unconditional HTTP
    /// requests while retaining content conflict checks before uploading.
    pub async fn push_manual(
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
            PushMode::Manual,
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
            PushMode::Force,
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
        mode: PushMode,
    ) -> Result<SyncStatus> {
        if !self.sync_enabled_for_provider() {
            return Ok(SyncStatus::Idle);
        }

        self.config_store.sync_from_disk();
        let force = mode == PushMode::Force;
        let manual_webdav = mode != PushMode::Automatic
            && self.config_store.config.provider == SyncProvider::WebDav;
        let remote = self
            .remote_payload_state(!manual_webdav, mode == PushMode::Automatic)
            .await?;
        let start_config_revision = self.config_store.config.config_revision;
        let passphrase = self.sync_passphrase()?;
        let local = self.local_snapshot(
            session_store,
            proxy_store,
            snippet_store,
            key_store,
            secret_store,
            settings_store,
        )?;
        let (mut condition, parent_payload_id, observed_remote_at) = match remote {
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
            RemotePayloadState::NotModified => {
                if !force
                    && self
                        .config_store
                        .config
                        .last_synced_local_revision
                        .as_deref()
                        == Some(local.revision.as_str())
                {
                    return Ok(SyncStatus::UpToDate {
                        at: self.config_store.config.last_sync_at,
                    });
                }
                (
                    self.config_store
                        .config
                        .remote_etag
                        .clone()
                        .map_or(PushCondition::Unconditional, PushCondition::IfMatch),
                    self.config_store.config.remote_payload_id.clone(),
                    Some(self.config_store.config.last_sync_at),
                )
            }
            RemotePayloadState::Current(payload, etag) => {
                if !force {
                    match self
                        .config_store
                        .config
                        .last_synced_local_revision
                        .as_deref()
                    {
                        Some(baseline_revision) if baseline_revision == local.revision => {
                            return Ok(SyncStatus::UpToDate {
                                at: payload.synced_at,
                            });
                        }
                        None => {
                            let remote_revision =
                                match decrypt_normalized_remote_payload(&payload, &passphrase) {
                                    Ok((_, revision)) => revision,
                                    Err(_) => {
                                        return Ok(SyncStatus::PullRequired {
                                            remote_at: Some(payload.synced_at),
                                            reason: SyncInterventionReason::MissingSyncBaseline,
                                        });
                                    }
                                };
                            if local.revision == remote_revision {
                                return self.acknowledge_remote_payload(
                                    start_config_revision,
                                    &payload,
                                    etag,
                                    local.revision,
                                );
                            }
                            return Ok(SyncStatus::PullRequired {
                                remote_at: Some(payload.synced_at),
                                reason: SyncInterventionReason::MissingSyncBaseline,
                            });
                        }
                        Some(_) => {}
                    }
                }
                (
                    etag.map_or(PushCondition::Unconditional, PushCondition::IfMatch),
                    non_empty_payload_id(&payload),
                    Some(payload.synced_at),
                )
            }
            RemotePayloadState::Changed(payload, etag) => {
                if !force {
                    let remote_revision =
                        match decrypt_normalized_remote_payload(&payload, &passphrase) {
                            Ok((_, revision)) => revision,
                            Err(_)
                                if self
                                    .config_store
                                    .config
                                    .last_synced_local_revision
                                    .is_none() =>
                            {
                                return Ok(SyncStatus::PullRequired {
                                    remote_at: Some(payload.synced_at),
                                    reason: SyncInterventionReason::MissingSyncBaseline,
                                });
                            }
                            Err(error) => return Err(error),
                        };
                    match classify_content_revisions(
                        &local.revision,
                        &remote_revision,
                        self.config_store
                            .config
                            .last_synced_local_revision
                            .as_deref(),
                    ) {
                        SyncContentRelation::Identical => {
                            return self.acknowledge_remote_payload(
                                start_config_revision,
                                &payload,
                                etag,
                                local.revision,
                            );
                        }
                        SyncContentRelation::LocalChanged => {}
                        SyncContentRelation::RemoteChanged => {
                            return Ok(SyncStatus::PullRequired {
                                remote_at: Some(payload.synced_at),
                                reason: SyncInterventionReason::RemoteChangedBeforePush,
                            });
                        }
                        SyncContentRelation::Diverged => {
                            return Ok(SyncStatus::PullRequired {
                                remote_at: Some(payload.synced_at),
                                reason: SyncInterventionReason::BothSidesChanged,
                            });
                        }
                        SyncContentRelation::MissingBaseline => {
                            return Ok(SyncStatus::PullRequired {
                                remote_at: Some(payload.synced_at),
                                reason: SyncInterventionReason::MissingSyncBaseline,
                            });
                        }
                    }
                }
                (
                    etag.map_or(PushCondition::Unconditional, PushCondition::IfMatch),
                    non_empty_payload_id(&payload),
                    Some(payload.synced_at),
                )
            }
        };
        // Explicit WebDAV uploads must also work with weak ETags or servers
        // that reject conditional writes. Content conflicts above still need
        // confirmation; only the HTTP preconditions are omitted here.
        if manual_webdav {
            condition = PushCondition::Unconditional;
        }
        if mode == PushMode::Automatic && self.config_store.config.provider == SyncProvider::WebDav
        {
            let kind = match &condition {
                PushCondition::MustNotExist => EtagKind::Strong,
                PushCondition::IfMatch(tag) => classify_etag(Some(tag)),
                PushCondition::Unconditional => EtagKind::Missing,
            };
            if kind != EtagKind::Strong {
                return Err(CapabilityError(CapabilityReport::issue(
                    CapabilityReason::VersionUnavailable,
                    "upload-precondition",
                    None,
                    kind,
                ))
                .into());
            }
        }
        // A marker followed by an unconditional write is still racy: another
        // device may write between the final GET and our PUT/PATCH. Automatic
        // sync therefore refuses to overwrite an existing remote when the
        // provider supplies no atomic write precondition. The explicit force
        // action is the user-confirmed escape hatch for such providers.
        if !manual_webdav
            && automatic_push_requires_confirmation(&condition, observed_remote_at, force)
        {
            return Ok(SyncStatus::PullRequired {
                remote_at: observed_remote_at,
                reason: SyncInterventionReason::UnsafeProviderWrite,
            });
        }
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
            if matches!(outcome, PushOutcome::Conflict)
                && mode == PushMode::Automatic
                && self.config_store.config.provider == SyncProvider::WebDav
            {
                let observed_tag = match &condition {
                    PushCondition::IfMatch(tag) => Some(tag.as_str()),
                    _ => None,
                };
                let after = backend.pull_checked(None, true).await?;
                let unchanged = match &after {
                    PullOutcome::Payload(payload) => observed_tag == payload.etag.as_deref(),
                    PullOutcome::Missing { .. } => observed_tag.is_none(),
                    _ => false,
                };
                if unchanged {
                    return Err(CapabilityError(CapabilityReport::issue(
                        CapabilityReason::ConditionalWrite,
                        "upload-412-unchanged",
                        Some(412),
                        EtagKind::Strong,
                    ))
                    .into());
                }
            }
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

    /// Check whether the configured remote representation changed without
    /// applying it locally. This is the polling entry point used by auto-sync.
    pub async fn remote_state(&mut self) -> Result<RemoteSyncState> {
        if !self.sync_enabled_for_provider() {
            return Ok(RemoteSyncState::Disabled);
        }
        self.config_store.sync_from_disk();
        Ok(match self.remote_payload_state(true, true).await? {
            RemotePayloadState::BindingRequired(provider) => {
                RemoteSyncState::BindingRequired(provider)
            }
            RemotePayloadState::Missing { .. } => RemoteSyncState::Missing,
            RemotePayloadState::NotModified => RemoteSyncState::NotModified,
            RemotePayloadState::Current(payload, etag)
                if self
                    .config_store
                    .config
                    .last_synced_local_revision
                    .is_none() =>
            {
                let passphrase = self.sync_passphrase()?;
                let content_revision = decrypt_normalized_remote_payload(&payload, &passphrase)
                    .ok()
                    .map(|(_, revision)| revision);
                RemoteSyncState::Updated {
                    synced_at: payload.synced_at,
                    etag,
                    payload_id: non_empty_payload_id(&payload),
                    content_revision,
                }
            }
            RemotePayloadState::Current(_, _) => RemoteSyncState::UpToDate,
            RemotePayloadState::Changed(payload, etag) => {
                let passphrase = self.sync_passphrase()?;
                let content_revision =
                    match decrypt_normalized_remote_payload(&payload, &passphrase) {
                        Ok((_, revision)) => Some(revision),
                        Err(_)
                            if self
                                .config_store
                                .config
                                .last_synced_local_revision
                                .is_none() =>
                        {
                            None
                        }
                        Err(error) => return Err(error),
                    };
                RemoteSyncState::Updated {
                    synced_at: payload.synced_at,
                    etag,
                    payload_id: non_empty_payload_id(&payload),
                    content_revision,
                }
            }
        })
    }

    /// Pull a payload from the configured backend. Identical content only
    /// refreshes the synchronization baseline; different content is applied
    /// after the caller's normal confirmation flow.
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
            None,
            false,
        )
        .await
    }

    /// Pull and apply the remote payload even when a missing synchronization
    /// baseline prevents proving that the current local data is unchanged.
    /// Callers must obtain explicit user confirmation before using this path.
    pub async fn pull_force(
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
            None,
            true,
        )
        .await
    }

    /// Pull only if the synchronized local data still matches the revision
    /// observed by the caller. Auto-sync uses this to close the gap between
    /// deciding that the working copy is clean and applying the remote payload.
    #[allow(clippy::too_many_arguments)]
    pub async fn pull_if_unchanged(
        &mut self,
        session_store: &SessionStore,
        proxy_store: &ProxyStore,
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
        secret_store: &SecretStore,
        settings_store: &mut SettingsStore,
        expected_local_revision: &str,
    ) -> Result<SyncStatus> {
        self.pull_internal(
            session_store,
            proxy_store,
            snippet_store,
            key_store,
            secret_store,
            settings_store,
            Some(expected_local_revision),
            false,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn pull_internal(
        &mut self,
        session_store: &SessionStore,
        proxy_store: &ProxyStore,
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
        secret_store: &SecretStore,
        settings_store: &mut SettingsStore,
        expected_local_revision: Option<&str>,
        force: bool,
    ) -> Result<SyncStatus> {
        if !self.sync_enabled_for_provider() {
            return Ok(SyncStatus::Idle);
        }

        self.config_store.sync_from_disk();
        let observed_revision = self.local_revision(
            session_store,
            proxy_store,
            snippet_store,
            key_store,
            secret_store,
            settings_store,
        )?;
        if expected_local_revision.is_some_and(|expected| expected != observed_revision) {
            return Ok(SyncStatus::PullRequired {
                remote_at: None,
                reason: SyncInterventionReason::LocalChangedDuringPull,
            });
        }
        let start_revision = expected_local_revision
            .map(str::to_owned)
            .unwrap_or(observed_revision);
        // A real pull must fetch the representation even when a preceding
        // poll, or a legacy config without the new revision baseline, already
        // has a matching ETag.
        let remote = self
            .remote_payload_state(false, expected_local_revision.is_some())
            .await?;
        // remote_payload_state already rejects configuration changes made while
        // the request is in flight. Capture its resulting revision (including a
        // refreshed ETag) so the final apply guard only rejects later changes.
        let start_config_revision = self.config_store.config.config_revision;
        let (payload, etag) = match remote {
            RemotePayloadState::BindingRequired(provider) => {
                return Ok(SyncStatus::RemoteBindingRequired { provider });
            }
            RemotePayloadState::Missing { .. } | RemotePayloadState::NotModified => {
                return Ok(SyncStatus::UpToDate {
                    at: self.config_store.config.last_sync_at,
                });
            }
            RemotePayloadState::Current(payload, etag)
            | RemotePayloadState::Changed(payload, etag) => (payload, etag),
        };

        let passphrase = self.sync_passphrase()?;
        let remote_synced_at = payload.synced_at;
        let (plaintext, remote_revision) =
            decrypt_normalized_remote_payload(&payload, &passphrase)?;
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
        if current_revision == remote_revision {
            return self.acknowledge_remote_payload(
                start_config_revision,
                &payload,
                etag,
                current_revision,
            );
        }

        if self
            .config_store
            .config
            .last_synced_local_revision
            .is_none()
            && !force
        {
            return Ok(SyncStatus::PullRequired {
                remote_at: Some(remote_synced_at),
                reason: SyncInterventionReason::MissingSyncBaseline,
            });
        }

        if expected_local_revision.is_some() {
            match classify_content_revisions(
                &current_revision,
                &remote_revision,
                self.config_store
                    .config
                    .last_synced_local_revision
                    .as_deref(),
            ) {
                SyncContentRelation::RemoteChanged => {}
                SyncContentRelation::LocalChanged => {
                    return Ok(SyncStatus::PullRequired {
                        remote_at: Some(remote_synced_at),
                        reason: SyncInterventionReason::LocalChangedDuringPull,
                    });
                }
                SyncContentRelation::Diverged => {
                    return Ok(SyncStatus::PullRequired {
                        remote_at: Some(remote_synced_at),
                        reason: SyncInterventionReason::BothSidesChanged,
                    });
                }
                SyncContentRelation::MissingBaseline => {
                    return Ok(SyncStatus::PullRequired {
                        remote_at: Some(remote_synced_at),
                        reason: SyncInterventionReason::MissingSyncBaseline,
                    });
                }
                SyncContentRelation::Identical => unreachable!("equality handled above"),
            }
        }

        settings_store.reload_from_disk()?;
        let applied_revision = remote_revision;
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

    async fn remote_payload_state(
        &mut self,
        conditional: bool,
        automatic: bool,
    ) -> Result<RemotePayloadState> {
        self.config_store.sync_from_disk();
        let config_revision = self.config_store.config.config_revision;
        let backend = match RemoteBackend::build(&self.config_store)? {
            Some(backend) => backend,
            None => return Ok(RemotePayloadState::Missing { etag: None }),
        };
        // A conditional 304 only proves that the remote representation still
        // matches its stored ETag. Without a local content baseline we still
        // need the encrypted payload body to determine whether both sides are
        // actually identical.
        let etag = (conditional
            && self
                .config_store
                .config
                .last_synced_local_revision
                .is_some())
        .then(|| self.config_store.config.remote_etag.clone())
        .flatten();
        let outcome = if automatic {
            backend.pull_checked(etag.as_deref(), true).await?
        } else {
            backend.pull(etag.as_deref()).await?
        };
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
            false
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

    fn acknowledge_remote_payload(
        &mut self,
        expected_config_revision: u64,
        payload: &SyncPayload,
        etag: Option<String>,
        content_revision: String,
    ) -> Result<SyncStatus> {
        let synced_at = payload.synced_at;
        let remote_payload_id = non_empty_payload_id(payload);
        let persisted =
            self.config_store
                .update_if_revision(expected_config_revision, |config| {
                    config.last_sync_at = synced_at;
                    config.remote_etag = etag;
                    config.remote_payload_id = remote_payload_id;
                    config.last_synced_local_revision = Some(content_revision);
                })?;
        if !persisted {
            self.config_store.sync_from_disk();
            return Ok(SyncStatus::PullRequired {
                remote_at: Some(synced_at),
                reason: SyncInterventionReason::SyncConfigurationChanged,
            });
        }
        Ok(SyncStatus::UpToDate { at: synced_at })
    }
}

fn decrypt_normalized_remote_payload(
    payload: &SyncPayload,
    passphrase: &str,
) -> Result<(SyncPlaintextPayload, String)> {
    let mut plaintext = decrypt_remote_payload(payload, passphrase)?;
    // Applying a payload sanitizes these same fields before persisting them;
    // compare and store the canonical representation used by local snapshots.
    normalize_remote_payload(&mut plaintext);
    let revision = local_data_revision(&plaintext)?;
    Ok((plaintext, revision))
}

fn non_empty_payload_id(payload: &SyncPayload) -> Option<String> {
    (!payload.payload_id.is_empty()).then(|| payload.payload_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::keychain::{ManagedKeyRecord, ManagedKeySource};
    use miaominal_core::profile::SessionProfile;
    use miaominal_core::proxy::ProxyProfile;
    use miaominal_core::snippet::SnippetRecord;
    use miaominal_secrets::APP_CREDENTIAL_SERVICE;
    use miaominal_secrets::credential_backend::{CredentialBackend, CredentialStore};
    use miaominal_settings::AppSettings;
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::sync::Mutex;
    use tempfile::tempdir;

    #[derive(Default)]
    struct MemoryCredentialBackend(Mutex<BTreeMap<String, String>>);

    impl CredentialBackend for MemoryCredentialBackend {
        fn name(&self) -> &'static str {
            "sync-engine-test-memory"
        }

        fn get(&self, service: &str, account: &str) -> Result<Option<String>> {
            Ok(self
                .0
                .lock()
                .expect("memory backend should lock")
                .get(&format!("{service}/{account}"))
                .cloned())
        }

        fn set(&self, service: &str, account: &str, value: &str) -> Result<()> {
            self.0
                .lock()
                .expect("memory backend should lock")
                .insert(format!("{service}/{account}"), value.to_string());
            Ok(())
        }

        fn delete(&self, service: &str, account: &str) -> Result<()> {
            self.0
                .lock()
                .expect("memory backend should lock")
                .remove(&format!("{service}/{account}"));
            Ok(())
        }
    }

    fn memory_credentials() -> CredentialStore {
        CredentialStore::with_backend(APP_CREDENTIAL_SERVICE, MemoryCredentialBackend::default())
    }

    /// A WebDAV server whose weak ETag cannot satisfy conditional PUTs.
    /// Read the complete upload so tests can verify the actual remote content.
    fn weak_etag_server(
        payload: String,
        request_count: usize,
        put_status: &'static str,
        remote_etag: &'static str,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..request_count {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < deadline, "request timed out");
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        Err(error) => panic!("accept failed: {error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let read = stream.read(&mut buffer).expect("request should read");
                    assert_ne!(read, 0, "request ended before its body was complete");
                    request.extend_from_slice(&buffer[..read]);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        let length = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .map_or(0, |value| value.trim().parse::<usize>().unwrap());
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8(request).unwrap();
                let headers = request
                    .split_once("\r\n\r\n")
                    .unwrap()
                    .0
                    .to_ascii_lowercase();
                let (status, body, etag) = if headers.starts_with("get ") {
                    if remote_etag.starts_with('"') && headers.contains("if-none-match:") {
                        ("304 Not Modified", "", remote_etag)
                    } else {
                        ("200 OK", payload.as_str(), remote_etag)
                    }
                } else {
                    assert!(headers.starts_with("put "));
                    let conditional =
                        headers.contains("\r\nif-match:") || headers.contains("\r\nif-none-match:");
                    (
                        if conditional {
                            "412 Precondition Failed"
                        } else {
                            put_status
                        },
                        "",
                        "W/\"uploaded-etag\"",
                    )
                };
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nETag: {etag}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                )
                .expect("response should write");
                requests.push(request);
            }
            requests
        });
        (format!("http://{address}/sync.json"), handle)
    }

    struct WebDavPushFixture {
        temp: tempfile::TempDir,
        engine: SyncEngine,
        sessions: SessionStore,
        proxies: ProxyStore,
        snippets: SnippetStore,
        keys: ManagedKeyStore,
        secrets: SecretStore,
        settings: SettingsStore,
        server: std::thread::JoinHandle<Vec<String>>,
    }

    impl WebDavPushFixture {
        fn new(request_count: usize, put_status: &'static str) -> Self {
            Self::with_etag(request_count, put_status, "W/\"remote-etag\"")
        }

        fn with_etag(
            request_count: usize,
            put_status: &'static str,
            remote_etag: &'static str,
        ) -> Self {
            let temp = tempdir().unwrap();
            let settings =
                SettingsStore::load_with_path(temp.path().join("settings.toml")).unwrap();
            let plaintext = empty_plaintext(&settings);
            let baseline = local_data_revision(&plaintext).unwrap();
            let payload =
                build_payload("remote-device", None, &plaintext, "test-passphrase").unwrap();
            let (url, server) = weak_etag_server(
                serde_json::to_string(&payload).unwrap(),
                request_count,
                put_status,
                remote_etag,
            );
            let credentials = memory_credentials();
            let mut engine = SyncEngine {
                config_store: SyncConfigStore::with_credentials(
                    temp.path().join("sync_config.toml"),
                    crate::SyncConfig {
                        provider: SyncProvider::WebDav,
                        webdav_url: url,
                        webdav_username: "user".into(),
                        last_sync_at: 42,
                        remote_etag: Some(remote_etag.into()),
                        remote_payload_id: Some(payload.payload_id),
                        last_synced_local_revision: Some(baseline),
                        ..crate::SyncConfig::default()
                    },
                    credentials.clone(),
                ),
            };
            engine.config_store.set_webdav_password("password").unwrap();
            engine
                .config_store
                .set_passphrase("test-passphrase")
                .unwrap();
            engine.config_store.update(|_| {}).unwrap();
            let sessions = SessionStore::with_path(temp.path().join("sessions.toml"));
            let mut session = SessionProfile::blank("local-session", 1);
            session.host = "latest.example.com".into();
            sessions.save(&[session]).unwrap();
            Self {
                proxies: ProxyStore::with_path(temp.path().join("proxies.toml")),
                snippets: SnippetStore::with_path(temp.path().join("snippets.toml")),
                keys: ManagedKeyStore::with_path(temp.path().join("managed_keys.toml")),
                secrets: SecretStore::with_credentials(credentials),
                settings,
                sessions,
                engine,
                server,
                temp,
            }
        }

        async fn push(&mut self, mode: PushMode) -> Result<SyncStatus> {
            match mode {
                PushMode::Force => {
                    self.engine
                        .push_force(
                            &self.sessions,
                            &self.proxies,
                            &self.snippets,
                            &self.keys,
                            &self.secrets,
                            &self.settings,
                        )
                        .await
                }
                PushMode::Manual => {
                    self.engine
                        .push_manual(
                            &self.sessions,
                            &self.proxies,
                            &self.snippets,
                            &self.keys,
                            &self.secrets,
                            &self.settings,
                        )
                        .await
                }
                PushMode::Automatic => {
                    self.engine
                        .push(
                            &self.sessions,
                            &self.proxies,
                            &self.snippets,
                            &self.keys,
                            &self.secrets,
                            &self.settings,
                        )
                        .await
                }
            }
        }

        fn assert_uploaded(self, status: SyncStatus) {
            let requests = self.server.join().unwrap();
            assert!(matches!(status, SyncStatus::Pushed { .. }), "{status:?}");
            assert!(requests.len() >= 2);
            for request in &requests {
                let headers = request
                    .split_once("\r\n\r\n")
                    .unwrap()
                    .0
                    .to_ascii_lowercase();
                assert!(!headers.contains("\r\nif-match:"), "{headers}");
                assert!(!headers.contains("\r\nif-none-match:"), "{headers}");
            }
            let uploaded =
                parse_remote_payload(requests.last().unwrap().split_once("\r\n\r\n").unwrap().1)
                    .unwrap();
            let (plaintext, revision) =
                decrypt_normalized_remote_payload(&uploaded, "test-passphrase").unwrap();
            assert_eq!(plaintext.sessions[0].host, "latest.example.com");
            assert_eq!(
                self.sessions.load(&self.secrets).unwrap()[0].host,
                "latest.example.com"
            );
            let persisted: crate::SyncConfig = toml::from_str(
                &fs::read_to_string(self.temp.path().join("sync_config.toml")).unwrap(),
            )
            .unwrap();
            assert_eq!(persisted, self.engine.config_store.config);
            assert_eq!(persisted.last_sync_at, uploaded.synced_at);
            assert_eq!(persisted.remote_payload_id, Some(uploaded.payload_id));
            assert_eq!(
                persisted.remote_etag.as_deref(),
                Some("W/\"uploaded-etag\"")
            );
            assert_eq!(persisted.last_synced_local_revision, Some(revision));
        }
    }

    #[tokio::test]
    async fn webdav_force_push_overwrites_despite_weak_etag() {
        let mut fixture = WebDavPushFixture::new(2, "204 No Content");
        let status = fixture.push(PushMode::Force).await.unwrap();
        fixture.assert_uploaded(status);
    }

    #[tokio::test]
    async fn webdav_manual_push_uploads_local_changes_despite_weak_etag() {
        let mut fixture = WebDavPushFixture::new(2, "204 No Content");
        let status = fixture.push(PushMode::Manual).await.unwrap();
        fixture.assert_uploaded(status);
    }

    #[tokio::test]
    async fn webdav_manual_push_requires_confirmation_without_baseline_then_force_push_succeeds() {
        let mut fixture = WebDavPushFixture::new(3, "204 No Content");
        fixture
            .engine
            .config_store
            .update(|config| {
                config.last_synced_local_revision = None;
                config.remote_payload_id = None;
                config.remote_etag = None;
            })
            .unwrap();
        let status = fixture.push(PushMode::Manual).await.unwrap();
        assert!(matches!(
            status,
            SyncStatus::PullRequired {
                reason: SyncInterventionReason::MissingSyncBaseline,
                ..
            }
        ));
        assert_eq!(fixture.engine.config_store.config.last_sync_at, 42);
        assert_eq!(
            fixture
                .engine
                .config_store
                .config
                .last_synced_local_revision,
            None
        );
        let status = fixture.push(PushMode::Force).await.unwrap();
        fixture.assert_uploaded(status);
    }

    #[tokio::test]
    async fn webdav_manual_push_preserves_diverged_content_until_confirmation() {
        let mut fixture = WebDavPushFixture::new(1, "204 No Content");
        fixture
            .engine
            .config_store
            .update(|config| {
                config.remote_payload_id = Some("older-payload".into());
                config.last_synced_local_revision = Some("older-content".into());
            })
            .unwrap();
        let before = fixture.engine.config_store.config.clone();
        let status = fixture.push(PushMode::Manual).await.unwrap();
        let requests = fixture.server.join().unwrap();
        assert!(matches!(
            status,
            SyncStatus::PullRequired {
                reason: SyncInterventionReason::BothSidesChanged,
                ..
            }
        ));
        assert_eq!(requests.len(), 1);
        assert_eq!(fixture.engine.config_store.config, before);
    }

    #[tokio::test]
    async fn webdav_manual_push_does_not_record_success_after_failed_upload() {
        let mut fixture = WebDavPushFixture::new(2, "500 Internal Server Error");
        let before = fixture.engine.config_store.config.clone();
        let error = fixture.push(PushMode::Manual).await.unwrap_err();
        let requests = fixture.server.join().unwrap();
        assert!(error.to_string().contains("WebDAV PUT failed: 500"));
        assert_eq!(requests.len(), 2);
        assert_eq!(fixture.engine.config_store.config, before);
        let persisted: crate::SyncConfig = toml::from_str(
            &fs::read_to_string(fixture.temp.path().join("sync_config.toml")).unwrap(),
        )
        .unwrap();
        assert_eq!(persisted, before);
    }

    #[tokio::test]
    async fn webdav_automatic_push_preserves_conditions_and_baseline_on_412() {
        let mut fixture = WebDavPushFixture::with_etag(3, "204 No Content", "\"remote-etag\"");
        let before = fixture.engine.config_store.config.clone();
        let error = fixture.push(PushMode::Automatic).await.unwrap_err();
        let requests = fixture.server.join().unwrap();
        assert_eq!(
            error.downcast_ref::<CapabilityError>().unwrap().0.reason,
            Some(CapabilityReason::ConditionalWrite)
        );
        assert!(
            requests[0]
                .to_ascii_lowercase()
                .contains("if-none-match: \"remote-etag\"")
        );
        assert!(
            requests[1]
                .to_ascii_lowercase()
                .contains("if-match: \"remote-etag\"")
        );
        assert_eq!(fixture.engine.config_store.config, before);
    }

    #[tokio::test]
    async fn webdav_automatic_push_rejects_weak_etags_before_upload() {
        let mut fixture = WebDavPushFixture::new(1, "204 No Content");
        let before = fixture.engine.config_store.config.clone();
        let error = fixture.push(PushMode::Automatic).await.unwrap_err();
        assert_eq!(
            error.downcast_ref::<CapabilityError>().unwrap().0.etag_kind,
            EtagKind::Weak
        );
        assert_eq!(fixture.server.join().unwrap().len(), 1);
        assert_eq!(fixture.engine.config_store.config, before);
    }

    fn payload_server(payload: String) -> (String, std::thread::JoinHandle<()>) {
        payload_server_with_etag(payload, Some("\"baseline-etag\""))
    }

    fn payload_server_with_etag(
        payload: String,
        etag: Option<&str>,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener.local_addr().expect("test address should resolve");
        let etag_header = etag.map_or_else(String::new, |etag| format!("ETag: {etag}\r\n"));
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should connect");
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let read = stream.read(&mut buffer).expect("request should read");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n{}",
                payload.len(),
                etag_header,
                payload
            )
            .expect("response should write");
        });
        (format!("http://{address}/sync.json"), handle)
    }

    fn conditional_payload_server(
        payload: String,
        etag: &'static str,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener.local_addr().expect("test address should resolve");
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should connect");
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let read = stream.read(&mut buffer).expect("request should read");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
            if request.contains("if-none-match:") {
                write!(
                    stream,
                    "HTTP/1.1 304 Not Modified\r\nETag: {etag}\r\nConnection: close\r\n\r\n"
                )
                .expect("304 response should write");
            } else {
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nETag: {etag}\r\nConnection: close\r\n\r\n{}",
                    payload.len(),
                    payload
                )
                .expect("payload response should write");
            }
        });
        (format!("http://{address}/sync.json"), handle)
    }

    fn not_modified_server() -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener.local_addr().expect("test address should resolve");
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should connect");
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let read = stream.read(&mut buffer).expect("request should read");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            assert!(
                String::from_utf8_lossy(&request)
                    .to_ascii_lowercase()
                    .contains("if-none-match: \"current-etag\"")
            );
            write!(
                stream,
                "HTTP/1.1 304 Not Modified\r\nETag: \"current-etag\"\r\nConnection: close\r\n\r\n"
            )
            .expect("response should write");
        });
        (format!("http://{address}/sync.json"), handle)
    }

    fn empty_plaintext(settings_store: &SettingsStore) -> SyncPlaintextPayload {
        SyncPlaintextPayload {
            sessions: Vec::new(),
            proxies: Vec::new(),
            snippets: Vec::new(),
            managed_keys: Vec::new(),
            settings: settings_store
                .read_current()
                .expect("settings should load")
                .synced_settings(),
            secrets: crate::PlaintextSecrets::default(),
        }
    }

    fn engine_for_current_payload(
        root: &Path,
        plaintext: &SyncPlaintextPayload,
        passphrase: &str,
    ) -> (
        SyncEngine,
        CredentialStore,
        std::thread::JoinHandle<()>,
        String,
    ) {
        let payload = build_payload("remote-device", None, plaintext, passphrase)
            .expect("payload should encrypt");
        let payload_revision = local_data_revision(plaintext).expect("revision should build");
        let payload_id = payload.payload_id.clone();
        let last_sync_at = payload.synced_at;
        let (url, server) =
            payload_server(serde_json::to_string(&payload).expect("payload should serialize"));
        let credentials = memory_credentials();
        let engine = SyncEngine {
            config_store: SyncConfigStore::with_credentials(
                root.join("sync_config.toml"),
                crate::SyncConfig {
                    provider: SyncProvider::WebDav,
                    webdav_url: url,
                    webdav_username: "user".into(),
                    last_sync_at,
                    remote_payload_id: Some(payload_id),
                    last_synced_local_revision: None,
                    ..crate::SyncConfig::default()
                },
                credentials.clone(),
            ),
        };
        engine
            .config_store
            .set_webdav_password("password")
            .expect("password should persist");
        engine
            .config_store
            .set_passphrase(passphrase)
            .expect("passphrase should persist");
        (engine, credentials, server, payload_revision)
    }

    fn save_local_fixture(
        session_store: &SessionStore,
        proxy_store: &ProxyStore,
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
    ) {
        let mut session = SessionProfile::blank("local-session", 1);
        session.host = "local.example.com".into();
        session_store
            .save(&[session])
            .expect("local session should persist");

        let mut proxy = ProxyProfile::blank("local-proxy", 1);
        proxy.host = "127.0.0.1".into();
        proxy_store
            .save(&[proxy])
            .expect("local proxy should persist");

        snippet_store
            .save(&[SnippetRecord {
                id: "local-snippet".into(),
                description: "Local-only snippet".into(),
                package: "Tests".into(),
                language: "bash".into(),
                script: "echo local".into(),
            }])
            .expect("local snippet should persist");

        key_store
            .save(&[ManagedKeyRecord {
                id: "local-key".into(),
                name: "Local key".into(),
                algorithm: "ssh-ed25519".into(),
                public_key: "ssh-ed25519 test".into(),
                source: ManagedKeySource::Imported,
            }])
            .expect("local key should persist");
    }

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

    #[test]
    fn content_relation_compares_current_sides_before_the_baseline() {
        assert_eq!(
            classify_content_revisions("same", "same", None),
            SyncContentRelation::Identical
        );
        assert_eq!(
            classify_content_revisions("local", "baseline", Some("baseline")),
            SyncContentRelation::LocalChanged
        );
        assert_eq!(
            classify_content_revisions("baseline", "remote", Some("baseline")),
            SyncContentRelation::RemoteChanged
        );
        assert_eq!(
            classify_content_revisions("local", "remote", Some("baseline")),
            SyncContentRelation::Diverged
        );
        assert_eq!(
            classify_content_revisions("local", "remote", None),
            SyncContentRelation::MissingBaseline
        );
    }

    #[test]
    fn legacy_payload_timestamp_is_not_content_identity() {
        let temp = tempdir().expect("temporary directory should exist");
        let mut payload = build_payload(
            "remote-device",
            None,
            &SyncPlaintextPayload {
                sessions: Vec::new(),
                proxies: Vec::new(),
                snippets: Vec::new(),
                managed_keys: Vec::new(),
                settings: AppSettings::default().synced_settings(),
                secrets: Default::default(),
            },
            "passphrase",
        )
        .expect("payload should build");
        payload.payload_id.clear();
        let engine = SyncEngine {
            config_store: SyncConfigStore::with_credentials(
                temp.path().join("sync_config.toml"),
                crate::SyncConfig {
                    last_sync_at: payload.synced_at.saturating_add(60),
                    ..crate::SyncConfig::default()
                },
                memory_credentials(),
            ),
        };

        assert!(!engine.remote_payload_is_current(&payload));
    }

    #[tokio::test]
    async fn push_acknowledges_same_content_with_a_different_payload_id() {
        let temp = tempdir().expect("temporary directory should exist");
        let session_store = SessionStore::with_path(temp.path().join("sessions.toml"));
        let proxy_store = ProxyStore::with_path(temp.path().join("proxies.toml"));
        let snippet_store = SnippetStore::with_path(temp.path().join("snippets.toml"));
        let key_store = ManagedKeyStore::with_path(temp.path().join("managed_keys.toml"));
        let settings_store = SettingsStore::load_with_path(temp.path().join("settings.toml"))
            .expect("settings store should load");
        let plaintext = empty_plaintext(&settings_store);
        let (mut engine, credentials, server, payload_revision) =
            engine_for_current_payload(temp.path(), &plaintext, "same-content-passphrase");
        engine.config_store.config.remote_payload_id = Some("previous-payload".into());
        engine.config_store.config.last_synced_local_revision = Some(payload_revision.clone());
        let secret_store = SecretStore::with_credentials(credentials);

        let status = engine
            .push(
                &session_store,
                &proxy_store,
                &snippet_store,
                &key_store,
                &secret_store,
                &settings_store,
            )
            .await
            .expect("same content should be acknowledged");

        assert!(matches!(status, SyncStatus::UpToDate { .. }));
        assert_ne!(
            engine.config_store.config.remote_payload_id.as_deref(),
            Some("previous-payload")
        );
        assert_eq!(
            engine.config_store.config.last_synced_local_revision,
            Some(payload_revision)
        );
        assert_eq!(
            engine.config_store.config.remote_etag.as_deref(),
            Some("\"baseline-etag\"")
        );
        server.join().expect("payload server should finish");
    }

    #[tokio::test]
    async fn push_acknowledges_current_payload_when_local_baseline_is_missing() {
        let temp = tempdir().expect("temporary directory should exist");
        let session_store = SessionStore::with_path(temp.path().join("sessions.toml"));
        let proxy_store = ProxyStore::with_path(temp.path().join("proxies.toml"));
        let snippet_store = SnippetStore::with_path(temp.path().join("snippets.toml"));
        let key_store = ManagedKeyStore::with_path(temp.path().join("managed_keys.toml"));
        let settings_store = SettingsStore::load_with_path(temp.path().join("settings.toml"))
            .expect("settings store should load");
        let plaintext = empty_plaintext(&settings_store);
        let (mut engine, credentials, server, payload_revision) =
            engine_for_current_payload(temp.path(), &plaintext, "missing-baseline-passphrase");
        let expected_payload_id = engine.config_store.config.remote_payload_id.clone();
        assert!(
            engine
                .config_store
                .config
                .last_synced_local_revision
                .is_none()
        );
        let secret_store = SecretStore::with_credentials(credentials);

        let status = engine
            .push(
                &session_store,
                &proxy_store,
                &snippet_store,
                &key_store,
                &secret_store,
                &settings_store,
            )
            .await
            .expect("matching remote content should restore the local baseline");

        assert!(matches!(status, SyncStatus::UpToDate { .. }));
        assert_eq!(
            engine.config_store.config.remote_payload_id,
            expected_payload_id
        );
        assert_eq!(
            engine.config_store.config.last_synced_local_revision,
            Some(payload_revision)
        );
        assert_eq!(
            engine.config_store.config.remote_etag.as_deref(),
            Some("\"baseline-etag\"")
        );
        server.join().expect("payload server should finish");
    }

    #[tokio::test]
    async fn push_fetches_content_when_etag_exists_but_local_baseline_is_missing() {
        let temp = tempdir().expect("temporary directory should exist");
        let session_store = SessionStore::with_path(temp.path().join("sessions.toml"));
        let proxy_store = ProxyStore::with_path(temp.path().join("proxies.toml"));
        let snippet_store = SnippetStore::with_path(temp.path().join("snippets.toml"));
        let key_store = ManagedKeyStore::with_path(temp.path().join("managed_keys.toml"));
        let settings_store = SettingsStore::load_with_path(temp.path().join("settings.toml"))
            .expect("settings store should load");
        let plaintext = empty_plaintext(&settings_store);
        let passphrase = "missing-baseline-with-etag-passphrase";
        let payload = build_payload("remote-device", None, &plaintext, passphrase)
            .expect("payload should build");
        let payload_id = payload.payload_id.clone();
        let payload_revision = local_data_revision(&plaintext).expect("revision should build");
        let (url, server) = conditional_payload_server(
            serde_json::to_string(&payload).expect("payload should serialize"),
            "\"current-etag\"",
        );
        let credentials = memory_credentials();
        let mut engine = SyncEngine {
            config_store: SyncConfigStore::with_credentials(
                temp.path().join("sync_config.toml"),
                crate::SyncConfig {
                    provider: SyncProvider::WebDav,
                    webdav_url: url,
                    webdav_username: "user".into(),
                    last_sync_at: payload.synced_at,
                    remote_etag: Some("\"current-etag\"".into()),
                    remote_payload_id: Some(payload_id.clone()),
                    last_synced_local_revision: None,
                    ..crate::SyncConfig::default()
                },
                credentials.clone(),
            ),
        };
        engine
            .config_store
            .set_webdav_password("password")
            .expect("password should persist");
        engine
            .config_store
            .set_passphrase(passphrase)
            .expect("passphrase should persist");
        let secret_store = SecretStore::with_credentials(credentials);

        let status = engine
            .push(
                &session_store,
                &proxy_store,
                &snippet_store,
                &key_store,
                &secret_store,
                &settings_store,
            )
            .await
            .expect("matching content should be fetched and acknowledged");

        assert!(matches!(status, SyncStatus::UpToDate { .. }));
        assert_eq!(
            engine.config_store.config.remote_payload_id,
            Some(payload_id)
        );
        assert_eq!(
            engine.config_store.config.last_synced_local_revision,
            Some(payload_revision)
        );
        server.join().expect("payload server should finish");
    }

    #[tokio::test]
    async fn manual_push_skips_upload_when_remote_without_etag_is_identical() {
        let temp = tempdir().expect("temporary directory should exist");
        let session_store = SessionStore::with_path(temp.path().join("sessions.toml"));
        let proxy_store = ProxyStore::with_path(temp.path().join("proxies.toml"));
        let snippet_store = SnippetStore::with_path(temp.path().join("snippets.toml"));
        let key_store = ManagedKeyStore::with_path(temp.path().join("managed_keys.toml"));
        let settings_store = SettingsStore::load_with_path(temp.path().join("settings.toml"))
            .expect("settings store should load");
        let plaintext = empty_plaintext(&settings_store);
        let passphrase = "no-etag-passphrase";
        let payload = build_payload("remote-device", None, &plaintext, passphrase)
            .expect("payload should build");
        let payload_id = payload.payload_id.clone();
        let payload_revision = local_data_revision(&plaintext).expect("revision should build");
        let (url, server) = payload_server_with_etag(
            serde_json::to_string(&payload).expect("payload should serialize"),
            None,
        );
        let credentials = memory_credentials();
        let mut engine = SyncEngine {
            config_store: SyncConfigStore::with_credentials(
                temp.path().join("sync_config.toml"),
                crate::SyncConfig {
                    provider: SyncProvider::WebDav,
                    webdav_url: url,
                    webdav_username: "user".into(),
                    last_sync_at: payload.synced_at,
                    remote_payload_id: Some(payload_id),
                    last_synced_local_revision: Some(payload_revision),
                    ..crate::SyncConfig::default()
                },
                credentials.clone(),
            ),
        };
        engine
            .config_store
            .set_webdav_password("password")
            .expect("password should persist");
        engine
            .config_store
            .set_passphrase(passphrase)
            .expect("passphrase should persist");
        let secret_store = SecretStore::with_credentials(credentials);

        let status = engine
            .push_manual(
                &session_store,
                &proxy_store,
                &snippet_store,
                &key_store,
                &secret_store,
                &settings_store,
            )
            .await
            .expect("manual sync should allow a remote without ETag");

        assert!(matches!(status, SyncStatus::UpToDate { .. }));
        server.join().expect("payload server should finish");
    }

    #[tokio::test]
    async fn automatic_push_skips_upload_when_remote_without_etag_is_identical() {
        let temp = tempdir().expect("temporary directory should exist");
        let session_store = SessionStore::with_path(temp.path().join("sessions.toml"));
        let proxy_store = ProxyStore::with_path(temp.path().join("proxies.toml"));
        let snippet_store = SnippetStore::with_path(temp.path().join("snippets.toml"));
        let key_store = ManagedKeyStore::with_path(temp.path().join("managed_keys.toml"));
        let settings_store = SettingsStore::load_with_path(temp.path().join("settings.toml"))
            .expect("settings store should load");
        let plaintext = empty_plaintext(&settings_store);
        let passphrase = "no-etag-passphrase";
        let payload = build_payload("remote-device", None, &plaintext, passphrase)
            .expect("payload should build");
        let payload_id = payload.payload_id.clone();
        let payload_revision = local_data_revision(&plaintext).expect("revision should build");
        let (url, server) = payload_server_with_etag(
            serde_json::to_string(&payload).expect("payload should serialize"),
            None,
        );
        let credentials = memory_credentials();
        let mut engine = SyncEngine {
            config_store: SyncConfigStore::with_credentials(
                temp.path().join("sync_config.toml"),
                crate::SyncConfig {
                    provider: SyncProvider::WebDav,
                    webdav_url: url,
                    webdav_username: "user".into(),
                    last_sync_at: payload.synced_at,
                    remote_payload_id: Some(payload_id),
                    last_synced_local_revision: Some(payload_revision),
                    ..crate::SyncConfig::default()
                },
                credentials.clone(),
            ),
        };
        engine
            .config_store
            .set_webdav_password("password")
            .expect("password should persist");
        engine
            .config_store
            .set_passphrase(passphrase)
            .expect("passphrase should persist");
        let secret_store = SecretStore::with_credentials(credentials);

        let error = engine
            .push(
                &session_store,
                &proxy_store,
                &snippet_store,
                &key_store,
                &secret_store,
                &settings_store,
            )
            .await
            .expect_err("automatic sync must refuse a remote without ETag");

        assert_eq!(
            error.downcast_ref::<CapabilityError>().unwrap().0.reason,
            Some(CapabilityReason::VersionUnavailable)
        );
        server.join().expect("payload server should finish");
    }

    #[tokio::test]
    async fn repeated_push_skips_upload_when_etag_and_local_content_are_unchanged() {
        let temp = tempdir().expect("temporary directory should exist");
        let (url, server) = not_modified_server();
        let credentials = memory_credentials();
        let mut engine = SyncEngine {
            config_store: SyncConfigStore::with_credentials(
                temp.path().join("sync_config.toml"),
                crate::SyncConfig {
                    provider: SyncProvider::WebDav,
                    webdav_url: url,
                    webdav_username: "user".into(),
                    last_sync_at: 42,
                    remote_etag: Some("\"current-etag\"".into()),
                    remote_payload_id: Some("current-payload".into()),
                    ..crate::SyncConfig::default()
                },
                credentials.clone(),
            ),
        };
        engine
            .config_store
            .set_webdav_password("password")
            .expect("password should persist");
        engine
            .config_store
            .set_passphrase("unchanged-passphrase")
            .expect("passphrase should persist");
        let session_store = SessionStore::with_path(temp.path().join("sessions.toml"));
        let proxy_store = ProxyStore::with_path(temp.path().join("proxies.toml"));
        let snippet_store = SnippetStore::with_path(temp.path().join("snippets.toml"));
        let key_store = ManagedKeyStore::with_path(temp.path().join("managed_keys.toml"));
        let secret_store = SecretStore::with_credentials(credentials);
        let settings_store = SettingsStore::load_with_path(temp.path().join("settings.toml"))
            .expect("settings store should load");
        let local_revision = engine
            .local_revision(
                &session_store,
                &proxy_store,
                &snippet_store,
                &key_store,
                &secret_store,
                &settings_store,
            )
            .expect("local revision should build");
        engine.config_store.config.last_synced_local_revision = Some(local_revision);

        let status = engine
            .push(
                &session_store,
                &proxy_store,
                &snippet_store,
                &key_store,
                &secret_store,
                &settings_store,
            )
            .await
            .expect("unchanged push should be skipped");

        assert_eq!(status, SyncStatus::UpToDate { at: 42 });
        server.join().expect("payload server should finish");
    }

    #[tokio::test]
    async fn conditional_pull_rejects_a_local_save_after_the_clean_check() {
        let temp = tempdir().expect("temporary directory should exist");
        let credentials = memory_credentials();
        let mut engine = SyncEngine {
            config_store: SyncConfigStore::with_credentials(
                temp.path().join("sync_config.toml"),
                crate::SyncConfig {
                    provider: SyncProvider::WebDav,
                    ..crate::SyncConfig::default()
                },
                credentials.clone(),
            ),
        };
        let session_store = SessionStore::with_path(temp.path().join("sessions.toml"));
        let proxy_store = ProxyStore::with_path(temp.path().join("proxies.toml"));
        let snippet_store = SnippetStore::with_path(temp.path().join("snippets.toml"));
        let key_store = ManagedKeyStore::with_path(temp.path().join("managed_keys.toml"));
        let secret_store = SecretStore::with_credentials(credentials);
        let mut settings_store = SettingsStore::load_with_path(temp.path().join("settings.toml"))
            .expect("settings store should load");
        let expected_local_revision = engine
            .local_revision(
                &session_store,
                &proxy_store,
                &snippet_store,
                &key_store,
                &secret_store,
                &settings_store,
            )
            .expect("initial revision should build");
        let mut saved_session = SessionProfile::blank("saved-session", 1);
        saved_session.host = "saved.example.com".into();
        session_store
            .save(&[saved_session])
            .expect("local save should persist");

        let status = engine
            .pull_if_unchanged(
                &session_store,
                &proxy_store,
                &snippet_store,
                &key_store,
                &secret_store,
                &mut settings_store,
                &expected_local_revision,
            )
            .await
            .expect("conditional pull should return an intervention");

        assert!(matches!(
            status,
            SyncStatus::PullRequired {
                reason: SyncInterventionReason::LocalChangedDuringPull,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn manual_pull_restores_matching_baseline_without_rewriting_local_stores() {
        let temp = tempdir().expect("temporary directory should exist");
        let passphrase = "baseline-recovery-passphrase";
        let sessions_path = temp.path().join("sessions.toml");
        let proxies_path = temp.path().join("proxies.toml");
        let snippets_path = temp.path().join("snippets.toml");
        let keys_path = temp.path().join("managed_keys.toml");
        let session_store = SessionStore::with_path(sessions_path.clone());
        let proxy_store = ProxyStore::with_path(proxies_path.clone());
        let snippet_store = SnippetStore::with_path(snippets_path.clone());
        let key_store = ManagedKeyStore::with_path(keys_path.clone());
        let mut settings_store = SettingsStore::load_with_path(temp.path().join("settings.toml"))
            .expect("settings store should load");
        let plaintext = empty_plaintext(&settings_store);
        let (mut engine, credentials, server, payload_revision) =
            engine_for_current_payload(temp.path(), &plaintext, passphrase);
        let secret_store = SecretStore::with_credentials(credentials);

        let status = engine
            .pull(
                &session_store,
                &proxy_store,
                &snippet_store,
                &key_store,
                &secret_store,
                &mut settings_store,
            )
            .await
            .expect("manual pull should restore the baseline");

        assert!(matches!(status, SyncStatus::UpToDate { .. }));
        assert_eq!(
            engine.config_store.config.last_synced_local_revision,
            Some(payload_revision)
        );
        assert_eq!(
            engine.config_store.config.remote_etag.as_deref(),
            Some("\"baseline-etag\"")
        );
        for path in [sessions_path, proxies_path, snippets_path, keys_path] {
            assert!(
                !path.exists(),
                "baseline recovery should not create {}",
                path.display()
            );
        }
        server.join().expect("payload server should finish");
    }

    #[tokio::test]
    async fn manual_pull_preserves_changed_local_data_when_baseline_is_missing() {
        let temp = tempdir().expect("temporary directory should exist");
        let passphrase = "baseline-conflict-passphrase";
        let sessions_path = temp.path().join("sessions.toml");
        let proxies_path = temp.path().join("proxies.toml");
        let snippets_path = temp.path().join("snippets.toml");
        let keys_path = temp.path().join("managed_keys.toml");
        let session_store = SessionStore::with_path(sessions_path.clone());
        let proxy_store = ProxyStore::with_path(proxies_path.clone());
        let snippet_store = SnippetStore::with_path(snippets_path.clone());
        let key_store = ManagedKeyStore::with_path(keys_path.clone());
        let mut settings_store = SettingsStore::load_with_path(temp.path().join("settings.toml"))
            .expect("settings store should load");
        let plaintext = empty_plaintext(&settings_store);
        let (mut engine, credentials, server, _) =
            engine_for_current_payload(temp.path(), &plaintext, passphrase);
        let secret_store = SecretStore::with_credentials(credentials);
        save_local_fixture(&session_store, &proxy_store, &snippet_store, &key_store);
        let original_files = [
            fs::read(&sessions_path).expect("sessions should read"),
            fs::read(&proxies_path).expect("proxies should read"),
            fs::read(&snippets_path).expect("snippets should read"),
            fs::read(&keys_path).expect("keys should read"),
        ];

        let status = engine
            .pull(
                &session_store,
                &proxy_store,
                &snippet_store,
                &key_store,
                &secret_store,
                &mut settings_store,
            )
            .await
            .expect("safe pull should require intervention");

        assert!(matches!(
            status,
            SyncStatus::PullRequired {
                reason: SyncInterventionReason::MissingSyncBaseline,
                ..
            }
        ));
        assert_eq!(engine.config_store.config.last_synced_local_revision, None);
        for (path, original) in [sessions_path, proxies_path, snippets_path, keys_path]
            .iter()
            .zip(original_files)
        {
            assert_eq!(fs::read(path).expect("local file should read"), original);
        }
        server.join().expect("payload server should finish");
    }

    #[tokio::test]
    async fn force_pull_overwrites_changed_local_data_when_baseline_is_missing() {
        let temp = tempdir().expect("temporary directory should exist");
        let passphrase = "baseline-force-passphrase";
        let session_store = SessionStore::with_path(temp.path().join("sessions.toml"));
        let proxy_store = ProxyStore::with_path(temp.path().join("proxies.toml"));
        let snippet_store = SnippetStore::with_path(temp.path().join("snippets.toml"));
        let key_store = ManagedKeyStore::with_path(temp.path().join("managed_keys.toml"));
        let mut settings_store = SettingsStore::load_with_path(temp.path().join("settings.toml"))
            .expect("settings store should load");
        let plaintext = empty_plaintext(&settings_store);
        let (mut engine, credentials, server, payload_revision) =
            engine_for_current_payload(temp.path(), &plaintext, passphrase);
        let secret_store = SecretStore::with_credentials(credentials);
        save_local_fixture(&session_store, &proxy_store, &snippet_store, &key_store);

        let status = engine
            .pull_force(
                &session_store,
                &proxy_store,
                &snippet_store,
                &key_store,
                &secret_store,
                &mut settings_store,
            )
            .await
            .expect("confirmed force pull should overwrite local data");

        assert!(matches!(status, SyncStatus::Pulled { .. }));
        assert!(
            session_store
                .read_sessions_content()
                .expect("sessions should read")
                .map(|content| session_store
                    .parse_sessions(&content)
                    .expect("sessions should parse"))
                .unwrap_or_default()
                .is_empty()
        );
        assert!(
            proxy_store
                .load(&secret_store)
                .expect("proxies should load")
                .is_empty()
        );
        assert!(
            snippet_store
                .load()
                .expect("snippets should load")
                .is_empty()
        );
        assert!(key_store.load().expect("keys should load").is_empty());
        assert_eq!(
            engine.config_store.config.last_synced_local_revision,
            Some(payload_revision)
        );
        server.join().expect("payload server should finish");
    }

    #[tokio::test]
    async fn manual_pull_applies_known_remote_when_local_content_changed() {
        let temp = tempdir().expect("temporary directory should exist");
        let passphrase = "known-remote-passphrase";
        let session_store = SessionStore::with_path(temp.path().join("sessions.toml"));
        let proxy_store = ProxyStore::with_path(temp.path().join("proxies.toml"));
        let snippet_store = SnippetStore::with_path(temp.path().join("snippets.toml"));
        let key_store = ManagedKeyStore::with_path(temp.path().join("managed_keys.toml"));
        let mut settings_store = SettingsStore::load_with_path(temp.path().join("settings.toml"))
            .expect("settings store should load");
        let plaintext = empty_plaintext(&settings_store);
        let (mut engine, credentials, server, payload_revision) =
            engine_for_current_payload(temp.path(), &plaintext, passphrase);
        engine.config_store.config.last_synced_local_revision = Some(payload_revision.clone());
        let secret_store = SecretStore::with_credentials(credentials);
        save_local_fixture(&session_store, &proxy_store, &snippet_store, &key_store);

        let status = engine
            .pull(
                &session_store,
                &proxy_store,
                &snippet_store,
                &key_store,
                &secret_store,
                &mut settings_store,
            )
            .await
            .expect("manual pull should apply the selected remote content");

        assert!(matches!(status, SyncStatus::Pulled { .. }));
        assert!(
            session_store
                .read_sessions_content()
                .expect("sessions should read")
                .map(|content| session_store
                    .parse_sessions(&content)
                    .expect("sessions should parse"))
                .unwrap_or_default()
                .is_empty()
        );
        assert_eq!(
            engine.config_store.config.last_synced_local_revision,
            Some(payload_revision)
        );
        server.join().expect("payload server should finish");
    }
}
