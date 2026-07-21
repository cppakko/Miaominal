use super::payload::{
    apply_plaintext_payload, build_payload, decrypt_remote_payload, parse_remote_payload,
};
use super::providers::{PullOutcome, RemoteBackend};
use super::store::SyncConfigStore;
use crate::{SyncPayload, SyncProvider, SyncStatus};
use anyhow::{Context, Result};
use miaominal_secrets::{CredentialStore, ProtectedPassphrase, SecretStore};
use miaominal_storage::SettingsStore;
use miaominal_storage::config_store::store::{SessionStore, SnippetStore};
use miaominal_storage::keychain_store::ManagedKeyStore;

enum RemotePayloadState {
    BindingRequired(SyncProvider),
    Missing,
    NotNewer,
    Newer(SyncPayload),
}

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
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
        secret_store: &SecretStore,
        settings_store: &SettingsStore,
    ) -> Result<SyncStatus> {
        self.push_internal(
            session_store,
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
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
        secret_store: &SecretStore,
        settings_store: &SettingsStore,
    ) -> Result<SyncStatus> {
        self.push_internal(
            session_store,
            snippet_store,
            key_store,
            secret_store,
            settings_store,
            true,
        )
        .await
    }

    async fn push_internal(
        &mut self,
        session_store: &SessionStore,
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
        if !force && let RemotePayloadState::Newer(payload) = self.remote_payload_state().await? {
            return Ok(SyncStatus::PullRequired {
                remote_at: payload.synced_at,
            });
        }
        let passphrase = self.sync_passphrase()?;

        let sessions = session_store
            .read_sessions_content()?
            .map(|c| session_store.parse_sessions(&c))
            .transpose()?
            .unwrap_or_default();
        let snippets = snippet_store.load()?;
        let managed_keys = key_store.load()?;
        let settings = settings_store.settings().synced_settings();

        let payload = build_payload(
            &self.config_store.config.device_id,
            &sessions,
            &snippets,
            &managed_keys,
            &settings,
            secret_store,
            &passphrase,
        )?;
        let payload_json =
            serde_json::to_string(&payload).context("failed to serialize sync payload")?;
        let synced_at = payload.synced_at;

        let mut backend = match RemoteBackend::build(&self.config_store)? {
            Some(backend) => backend,
            None => return Ok(SyncStatus::Idle),
        };
        let outcome = backend.push(&payload_json).await?;
        self.config_store.update(|c| {
            if let Some(resource_id) = outcome.provider_resource_id {
                c.gist_id = Some(resource_id);
            }
            c.last_sync_at = synced_at;
        })?;

        Ok(SyncStatus::Pushed { at: synced_at })
    }

    /// Pull a payload from the configured backend and apply it locally using
    /// last-write-wins: only overwrites local data when the remote `synced_at`
    /// is strictly newer than the last local sync timestamp.
    pub async fn pull(
        &mut self,
        session_store: &SessionStore,
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
        secret_store: &SecretStore,
        settings_store: &mut SettingsStore,
    ) -> Result<SyncStatus> {
        self.pull_internal(
            session_store,
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
        snippet_store: &SnippetStore,
        key_store: &ManagedKeyStore,
        secret_store: &SecretStore,
        settings_store: &mut SettingsStore,
    ) -> Result<SyncStatus> {
        if !self.sync_enabled_for_provider() {
            return Ok(SyncStatus::Idle);
        }

        self.config_store.sync_from_disk();
        let payload = match self.remote_payload_state().await? {
            RemotePayloadState::BindingRequired(provider) => {
                return Ok(SyncStatus::RemoteBindingRequired { provider });
            }
            RemotePayloadState::Missing | RemotePayloadState::NotNewer => {
                return Ok(SyncStatus::UpToDate {
                    at: self.config_store.config.last_sync_at,
                });
            }
            RemotePayloadState::Newer(payload) => payload,
        };

        let passphrase = self.sync_passphrase()?;
        let remote_synced_at = payload.synced_at;
        let plaintext = decrypt_remote_payload(&payload, &passphrase)?;

        apply_plaintext_payload(
            &plaintext,
            session_store,
            snippet_store,
            key_store,
            secret_store,
            settings_store,
            || {
                self.config_store.update(|c| {
                    c.last_sync_at = remote_synced_at;
                })
            },
        )?;

        Ok(SyncStatus::Pulled {
            at: remote_synced_at,
        })
    }

    async fn remote_payload_state(&self) -> Result<RemotePayloadState> {
        let backend = match RemoteBackend::build(&self.config_store)? {
            Some(backend) => backend,
            None => return Ok(RemotePayloadState::Missing),
        };
        let payload_json = match backend.pull().await? {
            PullOutcome::BindingRequired { provider } => {
                return Ok(RemotePayloadState::BindingRequired(provider));
            }
            PullOutcome::Missing => return Ok(RemotePayloadState::Missing),
            PullOutcome::Payload(payload_json) => payload_json,
        };
        let payload = parse_remote_payload(&payload_json)?;

        if payload.synced_at > self.config_store.config.last_sync_at {
            Ok(RemotePayloadState::Newer(payload))
        } else {
            Ok(RemotePayloadState::NotNewer)
        }
    }

    pub fn sync_enabled_for_provider(&self) -> bool {
        self.config_store.config.provider != SyncProvider::None
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
