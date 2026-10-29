//! Mobile configuration, credential, key, and trust data layer.
//!
//! Every operation runs through this type, serialized by one mutex so config
//! writes cannot interleave. Credentials always go through the active
//! [`SecretStore`] backend: either the Android Kaastore adapter injected by the
//! host, or the encrypted local vault file.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use anyhow::{Context, Result, anyhow};
use miaominal_core::keychain::ManagedKeySource;
use miaominal_core::profile::SessionProfile;
use miaominal_core::proxy::ProxyProfile;
use miaominal_core::snippet::SnippetRecord;
use miaominal_paths as paths;
use miaominal_secrets::{
    CredentialStore, ProtectedPassphrase, SecretKind, SecretStore, VaultCredentialBackend,
    credential_backend::APP_CREDENTIAL_SERVICE,
};
use miaominal_services::{KeychainService, ProfileService, SyncService};
use miaominal_settings::AppSettings;
use miaominal_storage::SettingsStore;
use miaominal_storage::config_store::store::{SessionStore, SnippetStore};
use miaominal_storage::keychain_store::ManagedKeyStore;
use miaominal_storage::known_hosts_store::KnownHostsStore;
use miaominal_storage::proxy_store::ProxyStore;
use miaominal_sync::engine::SyncEngine;
use miaominal_sync::{SyncConfig, SyncConfigStore};

use crate::credentials::{PlatformCredentialBackend, classify_credential_error};
use crate::dto::{
    CredentialAction, CredentialHealth, CredentialKindDto, ForwardRuleDto, HostDto, HostUpdate,
    KnownHostDto, ManagedKeyAlgorithmDto, ManagedKeyDto, ProxyDto, ProxyUpdate, SettingsDto,
    SettingsUpdate, SnippetDto, SnippetUpdate, VaultModeDto, VaultSnapshot, apply_host_update,
    forward_rule_from_dto, forward_rule_to_dto, host_to_dto, key_to_dto, proxy_protocol_from_name,
    settings_to_dto,
};
use crate::error::BackendError;
use crate::platform::PlatformHost;
use crate::runtime;
use crate::sync::SyncHandles;
use crate::sync_dto::{SyncConfigUpdate, SyncCredentialKind};
/// Data layer for the embedded runtime. Constructed after the runtime roots
/// exist; the stores resolve their files from the embedded config directory.
pub(crate) struct MobileData {
    host: Arc<dyn PlatformHost>,
    inner: Mutex<DataInner>,
    /// Process-wide sync facade. Created once so capability-probe cleanup and
    /// the process sync lock survive data-generation replacements.
    sync_service: SyncService,
    retired: AtomicBool,
    active_operations: Mutex<usize>,
    idle: Condvar,
}

/// Snapshot of the stores needed to start one SSH session.
pub(crate) struct SessionLaunchContext {
    pub(crate) host_label: String,
    pub(crate) profile: SessionProfile,
    pub(crate) all_profiles: Vec<SessionProfile>,
    pub(crate) all_proxies: Vec<ProxyProfile>,
    pub(crate) secrets: SecretStore,
    pub(crate) known_hosts: KnownHostsStore,
    pub(crate) vault_locked: bool,
}

/// Keeps one in-flight data operation visible to [`MobileData::retire`].
///
/// `shutdown` retires the data layer and waits for the counter to reach zero
/// before the caller (restart or clear) is allowed to continue; an operation
/// that arrives after retirement starts fails with `Closed` instead of
/// writing into a business root that a newer generation already cleared.
pub(crate) struct OperationGuard {
    data: Arc<MobileData>,
}

impl OperationGuard {
    pub(crate) fn new(data: Arc<MobileData>) -> Result<Self, BackendError> {
        data.enter_operation()?;
        Ok(Self { data })
    }
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.data.leave_operation();
    }
}

struct DataInner {
    secrets: SecretStore,
    vault_passphrase: Option<ProtectedPassphrase>,
    vault_mode: VaultModeDto,
    session_store: SessionStore,
    snippet_store: SnippetStore,
    managed_key_store: ManagedKeyStore,
    proxy_store: ProxyStore,
    known_hosts: KnownHostsStore,
    settings_store: SettingsStore,
}

impl MobileData {
    pub(crate) fn new(host: Arc<dyn PlatformHost>) -> Result<Self> {
        let inner = DataInner::new(Arc::clone(&host))?;
        let sync_service = SyncService::new(
            runtime::handle(),
            Some(inner.session_store.clone()),
            Some(inner.proxy_store.clone()),
            Some(inner.snippet_store.clone()),
            Some(inner.managed_key_store.clone()),
            inner.secrets.clone(),
        )
        .map_err(|error| anyhow!("failed to build the sync service: {error}"))?;
        Ok(Self {
            host,
            inner: Mutex::new(inner),
            sync_service,
            retired: AtomicBool::new(false),
            active_operations: Mutex::new(0),
            idle: Condvar::new(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, DataInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn enter_operation(&self) -> Result<(), BackendError> {
        let mut active = self
            .active_operations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.retired.load(Ordering::SeqCst) {
            return Err(BackendError::Closed);
        }
        *active += 1;
        Ok(())
    }

    fn leave_operation(&self) {
        let mut active = self
            .active_operations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *active = active.saturating_sub(1);
        if *active == 0 {
            self.idle.notify_all();
        }
    }

    /// Refuse new operations and block until every in-flight operation has
    /// finished. Called from `Backend::shutdown` before a restart may clear or
    /// replace the business root.
    pub(crate) fn retire(&self) {
        let mut active = self
            .active_operations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.retired.store(true, Ordering::SeqCst);
        while *active > 0 {
            active = self
                .idle
                .wait(active)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    fn platform_store(&self) -> SecretStore {
        platform_secret_store(&self.host)
    }

    // ------------------------------------------------------------------
    // Settings
    // ------------------------------------------------------------------

    /// Always reads the persisted settings and refreshes the cached copy.
    ///
    /// The sync engine applies a pulled payload through its own `SettingsStore`
    /// clone while holding the process sync data gate; the cache can therefore
    /// lag behind disk until the next read. Reading through closes that window
    /// for both UI snapshots and the update base below.
    pub(crate) fn settings(&self) -> Result<SettingsDto, BackendError> {
        let mut inner = self.lock();
        inner
            .settings_store
            .reload_from_disk()
            .map_err(BackendError::from_storage)?;
        Ok(settings_to_dto(inner.settings_store.settings()))
    }

    pub(crate) fn update_settings(
        &self,
        update: SettingsUpdate,
    ) -> Result<SettingsDto, BackendError> {
        let mut inner = self.lock();
        // Base the field update on the latest persisted settings, not on a
        // possibly stale cache: a pull may have written other fields moments
        // ago, and `replace` would otherwise restore their old values.
        inner
            .settings_store
            .reload_from_disk()
            .map_err(BackendError::from_storage)?;
        let mut settings = inner.settings_store.settings().clone();
        apply_settings_update(&mut settings, &update)?;
        inner
            .settings_store
            .replace(settings)
            .map_err(BackendError::from_storage)?;
        Ok(settings_to_dto(inner.settings_store.settings()))
    }

    // ------------------------------------------------------------------
    // Hosts and forwarding rules
    // ------------------------------------------------------------------

    pub(crate) fn hosts(&self) -> Result<Vec<HostDto>, BackendError> {
        let inner = self.lock();
        let sessions = inner
            .session_store
            .load(&inner.secrets)
            .map_err(BackendError::from_storage)?;
        Ok(sessions.iter().map(host_to_dto).collect())
    }

    pub(crate) fn save_host(&self, update: HostUpdate) -> Result<HostDto, BackendError> {
        let inner = self.lock();
        if update.id.trim().is_empty() {
            return Err(BackendError::InvalidArgument {
                detail: "host id must not be empty".to_string(),
            });
        }
        let mut sessions = inner
            .session_store
            .load(&inner.secrets)
            .map_err(BackendError::from_storage)?;
        let index = sessions.iter().position(|profile| profile.id == update.id);
        let mut profile = match index {
            Some(index) => sessions[index].clone(),
            None => SessionProfile::blank(update.id.clone(), sessions.len() + 1),
        };
        apply_host_update(&mut profile, &update);
        profile.ensure_auth_method();
        match index {
            Some(index) => sessions[index] = profile.clone(),
            None => sessions.push(profile.clone()),
        }
        inner
            .session_store
            .save(&sessions)
            .map_err(BackendError::from_storage)?;
        Ok(host_to_dto(&profile))
    }

    /// Replace the routing configuration of one host (entry proxy + jump
    /// chain). Validation happens here: unknown jump hosts and self references
    /// are rejected before anything is persisted, and the connect path still
    /// re-validates the resolved chain.
    pub(crate) fn set_host_routing(
        &self,
        profile_id: &str,
        entry_proxy_id: String,
        proxy_jump_profile_ids: Vec<String>,
    ) -> Result<HostDto, BackendError> {
        let inner = self.lock();
        let mut sessions = inner
            .session_store
            .load(&inner.secrets)
            .map_err(BackendError::from_storage)?;
        if !entry_proxy_id.trim().is_empty() {
            let proxies = inner
                .proxy_store
                .load(&inner.secrets)
                .map_err(BackendError::from_storage)?;
            if !proxies.iter().any(|proxy| proxy.id == entry_proxy_id) {
                return Err(BackendError::InvalidArgument {
                    detail: format!("proxy {entry_proxy_id} does not exist"),
                });
            }
        }
        let mut seen = std::collections::HashSet::new();
        for jump_id in &proxy_jump_profile_ids {
            if jump_id == profile_id {
                return Err(BackendError::InvalidArgument {
                    detail: "the jump chain cannot reference the host being connected".to_string(),
                });
            }
            if !seen.insert(jump_id.clone()) {
                return Err(BackendError::InvalidArgument {
                    detail: "the jump chain cannot include the same host twice".to_string(),
                });
            }
            if !sessions.iter().any(|profile| profile.id == *jump_id) {
                return Err(BackendError::InvalidArgument {
                    detail: format!("jump host {jump_id} does not exist"),
                });
            }
        }
        let Some(profile) = sessions.iter_mut().find(|profile| profile.id == profile_id) else {
            return Err(BackendError::InvalidArgument {
                detail: format!("host {profile_id} does not exist"),
            });
        };
        profile.entry_proxy_id = if entry_proxy_id.trim().is_empty() {
            None
        } else {
            Some(entry_proxy_id)
        };
        profile.proxy_jump_profile_ids = proxy_jump_profile_ids;
        let updated = profile.clone();
        inner
            .session_store
            .save(&sessions)
            .map_err(BackendError::from_storage)?;
        Ok(host_to_dto(&updated))
    }

    pub(crate) fn delete_host(&self, id: &str) -> Result<bool, BackendError> {
        let inner = self.lock();
        let mut sessions = inner
            .session_store
            .load(&inner.secrets)
            .map_err(BackendError::from_storage)?;
        let Some(index) = sessions.iter().position(|profile| profile.id == id) else {
            return Ok(false);
        };
        sessions.remove(index);
        // Keep jump-chain references coherent: a deleted host must not be
        // referenced by remaining profiles.
        for profile in &mut sessions {
            profile
                .proxy_jump_profile_ids
                .retain(|candidate| candidate != id);
        }
        inner
            .session_store
            .save(&sessions)
            .map_err(BackendError::from_storage)?;
        // Credentials for the removed host are deleted best-effort; a locked
        // vault keeps its ciphertext until the next unlock.
        inner.secrets.delete_all(id);
        Ok(true)
    }

    pub(crate) fn set_host_credential(
        &self,
        profile_id: &str,
        kind: CredentialKindDto,
        action: CredentialAction,
    ) -> Result<CredentialHealth, BackendError> {
        let mut inner = self.lock();
        let secret_kind = credential_secret_kind(kind);
        match action {
            CredentialAction::Keep => {}
            CredentialAction::Set { value } => {
                inner
                    .secrets
                    .set(profile_id, secret_kind, &value)
                    .map_err(BackendError::from_storage)?;
                set_host_stored_flag(&mut inner, profile_id, kind, true)?;
            }
            CredentialAction::Clear => {
                inner
                    .secrets
                    .delete(profile_id, secret_kind)
                    .map_err(BackendError::from_storage)?;
                set_host_stored_flag(&mut inner, profile_id, kind, false)?;
            }
        }
        Ok(credential_health_for(&inner.secrets, profile_id, kind))
    }

    pub(crate) fn host_credential_health(
        &self,
        profile_id: &str,
        kind: CredentialKindDto,
    ) -> CredentialHealth {
        let inner = self.lock();
        credential_health_for(&inner.secrets, profile_id, kind)
    }

    pub(crate) fn forwards(&self) -> Result<Vec<ForwardRuleDto>, BackendError> {
        let inner = self.lock();
        let sessions = inner
            .session_store
            .load(&inner.secrets)
            .map_err(BackendError::from_storage)?;
        Ok(sessions
            .iter()
            .flat_map(|profile| {
                profile
                    .port_forwarding_rules
                    .iter()
                    .map(|rule| forward_rule_to_dto(&profile.id, rule))
            })
            .collect())
    }

    pub(crate) fn upsert_forward(
        &self,
        forward: ForwardRuleDto,
    ) -> Result<ForwardRuleDto, BackendError> {
        let inner = self.lock();
        if forward.id.trim().is_empty() || forward.profile_id.trim().is_empty() {
            return Err(BackendError::InvalidArgument {
                detail: "forward id and profile id must not be empty".to_string(),
            });
        }
        let mut sessions = inner
            .session_store
            .load(&inner.secrets)
            .map_err(BackendError::from_storage)?;
        let Some(profile) = sessions
            .iter_mut()
            .find(|profile| profile.id == forward.profile_id)
        else {
            return Err(BackendError::InvalidArgument {
                detail: format!("host {} does not exist", forward.profile_id),
            });
        };
        let rule = forward_rule_from_dto(&forward);
        match profile
            .port_forwarding_rules
            .iter_mut()
            .find(|existing| existing.id == rule.id)
        {
            Some(existing) => *existing = rule.clone(),
            None => profile.port_forwarding_rules.push(rule.clone()),
        }
        inner
            .session_store
            .save(&sessions)
            .map_err(BackendError::from_storage)?;
        Ok(forward_rule_to_dto(&forward.profile_id, &rule))
    }

    pub(crate) fn delete_forward(
        &self,
        profile_id: &str,
        rule_id: &str,
    ) -> Result<bool, BackendError> {
        let inner = self.lock();
        let mut sessions = inner
            .session_store
            .load(&inner.secrets)
            .map_err(BackendError::from_storage)?;
        let Some(profile) = sessions.iter_mut().find(|profile| profile.id == profile_id) else {
            return Ok(false);
        };
        let previous = profile.port_forwarding_rules.len();
        profile
            .port_forwarding_rules
            .retain(|rule| rule.id != rule_id);
        if profile.port_forwarding_rules.len() == previous {
            return Ok(false);
        }
        inner
            .session_store
            .save(&sessions)
            .map_err(BackendError::from_storage)?;
        Ok(true)
    }

    pub(crate) fn set_forward_enabled(
        &self,
        profile_id: &str,
        rule_id: &str,
        enabled: bool,
    ) -> Result<bool, BackendError> {
        let inner = self.lock();
        let mut sessions = inner
            .session_store
            .load(&inner.secrets)
            .map_err(BackendError::from_storage)?;
        let Some(profile) = sessions.iter_mut().find(|profile| profile.id == profile_id) else {
            return Ok(false);
        };
        let Some(rule) = profile
            .port_forwarding_rules
            .iter_mut()
            .find(|rule| rule.id == rule_id)
        else {
            return Ok(false);
        };
        rule.enabled = enabled;
        inner
            .session_store
            .save(&sessions)
            .map_err(BackendError::from_storage)?;
        Ok(true)
    }

    // ------------------------------------------------------------------
    // Snippets
    // ------------------------------------------------------------------

    pub(crate) fn snippets(&self) -> Result<Vec<SnippetDto>, BackendError> {
        let inner = self.lock();
        let snippets = inner
            .snippet_store
            .load()
            .map_err(BackendError::from_storage)?;
        Ok(snippets.iter().map(snippet_to_dto).collect())
    }

    pub(crate) fn save_snippet(&self, update: SnippetUpdate) -> Result<SnippetDto, BackendError> {
        let inner = self.lock();
        if update.id.trim().is_empty() {
            return Err(BackendError::InvalidArgument {
                detail: "snippet id must not be empty".to_string(),
            });
        }
        let mut snippets = inner
            .snippet_store
            .load()
            .map_err(BackendError::from_storage)?;
        let index = snippets.iter().position(|snippet| snippet.id == update.id);
        let mut snippet = match index {
            Some(index) => snippets[index].clone(),
            None => SnippetRecord {
                id: update.id.clone(),
                description: String::new(),
                package: String::new(),
                language: "bash".to_string(),
                script: String::new(),
            },
        };
        if let Some(description) = &update.description {
            snippet.description = description.clone();
        }
        if let Some(package_name) = &update.package_name {
            snippet.package = package_name.clone();
        }
        if let Some(language) = &update.language {
            snippet.language = language.clone();
        }
        if let Some(script) = &update.script {
            snippet.script = script.clone();
        }
        match index {
            Some(index) => snippets[index] = snippet.clone(),
            None => snippets.push(snippet.clone()),
        }
        inner
            .snippet_store
            .save(&snippets)
            .map_err(BackendError::from_storage)?;
        Ok(snippet_to_dto(&snippet))
    }

    pub(crate) fn delete_snippet(&self, id: &str) -> Result<bool, BackendError> {
        let inner = self.lock();
        let mut snippets = inner
            .snippet_store
            .load()
            .map_err(BackendError::from_storage)?;
        let previous = snippets.len();
        snippets.retain(|snippet| snippet.id != id);
        if snippets.len() == previous {
            return Ok(false);
        }
        inner
            .snippet_store
            .save(&snippets)
            .map_err(BackendError::from_storage)?;
        Ok(true)
    }

    // ------------------------------------------------------------------
    // Proxies
    // ------------------------------------------------------------------

    pub(crate) fn proxies(&self) -> Result<Vec<ProxyDto>, BackendError> {
        let inner = self.lock();
        let proxies = inner
            .proxy_store
            .load(&inner.secrets)
            .map_err(BackendError::from_storage)?;
        Ok(proxies.iter().map(crate::dto::proxy_to_dto).collect())
    }

    /// Everything a new interactive session needs: the target profile, the
    /// full profile/proxy lists for jump resolution, and the active secret and
    /// trust stores. Also reports whether the active vault is locked so the
    /// caller can return `VaultLocked` instead of a misleading auth failure.
    pub(crate) fn session_context(
        &self,
        host_id: &str,
    ) -> Result<SessionLaunchContext, BackendError> {
        let inner = self.lock();
        let sessions = inner
            .session_store
            .load(&inner.secrets)
            .map_err(BackendError::from_storage)?;
        let profile = sessions
            .iter()
            .find(|profile| profile.id == host_id)
            .cloned()
            .ok_or_else(|| BackendError::InvalidArgument {
                detail: format!("host {host_id} is not available"),
            })?;
        let proxies = inner
            .proxy_store
            .load(&inner.secrets)
            .map_err(BackendError::from_storage)?;
        let vault_locked = inner.vault_mode == VaultModeDto::Locked;
        Ok(SessionLaunchContext {
            host_label: profile.connection_label(),
            profile,
            all_profiles: sessions,
            all_proxies: proxies,
            secrets: inner.secrets.clone(),
            known_hosts: inner.known_hosts.clone(),
            vault_locked,
        })
    }

    pub(crate) fn save_proxy(&self, update: ProxyUpdate) -> Result<ProxyDto, BackendError> {
        let inner = self.lock();
        if update.id.trim().is_empty() {
            return Err(BackendError::InvalidArgument {
                detail: "proxy id must not be empty".to_string(),
            });
        }
        let mut proxies = inner
            .proxy_store
            .load(&inner.secrets)
            .map_err(BackendError::from_storage)?;
        let index = proxies.iter().position(|proxy| proxy.id == update.id);
        let mut proxy = match index {
            Some(index) => proxies[index].clone(),
            None => ProxyProfile::blank(update.id.clone(), proxies.len() + 1),
        };
        if let Some(name) = &update.name {
            proxy.name = name.clone();
        }
        if let Some(protocol) = &update.protocol {
            proxy.protocol = proxy_protocol_from_name(protocol);
        }
        if let Some(host) = &update.host {
            proxy.host = host.clone();
        }
        if let Some(port) = update.port {
            proxy.port = port;
        }
        if let Some(auth_mode) = &update.auth_mode {
            proxy.auth_mode = crate::dto::proxy_auth_mode_from_name(auth_mode);
        }
        if let Some(username) = &update.username {
            proxy.username = username.clone();
        }
        if let Some(resolve) = update.resolve_dns_through_proxy {
            proxy.resolve_dns_through_proxy = resolve;
        }
        match index {
            Some(index) => proxies[index] = proxy.clone(),
            None => proxies.push(proxy.clone()),
        }
        inner
            .proxy_store
            .save(&proxies)
            .map_err(BackendError::from_storage)?;
        Ok(crate::dto::proxy_to_dto(&proxy))
    }

    pub(crate) fn delete_proxy(&self, id: &str) -> Result<bool, BackendError> {
        let inner = self.lock();
        let mut proxies = inner
            .proxy_store
            .load(&inner.secrets)
            .map_err(BackendError::from_storage)?;
        let previous = proxies.len();
        proxies.retain(|proxy| proxy.id != id);
        if proxies.len() == previous {
            return Ok(false);
        }
        inner
            .proxy_store
            .save(&proxies)
            .map_err(BackendError::from_storage)?;
        inner.secrets.delete_proxy_password(id);
        Ok(true)
    }

    pub(crate) fn set_proxy_password(
        &self,
        proxy_id: &str,
        action: CredentialAction,
    ) -> Result<CredentialHealth, BackendError> {
        let mut inner = self.lock();
        match action {
            CredentialAction::Keep => {}
            CredentialAction::Set { value } => {
                inner
                    .secrets
                    .set(proxy_id, SecretKind::ProxyPassword, &value)
                    .map_err(BackendError::from_storage)?;
                set_proxy_stored_flag(&mut inner, proxy_id, true)?;
            }
            CredentialAction::Clear => {
                inner
                    .secrets
                    .delete(proxy_id, SecretKind::ProxyPassword)
                    .map_err(BackendError::from_storage)?;
                set_proxy_stored_flag(&mut inner, proxy_id, false)?;
            }
        }
        Ok(credential_health_for(
            &inner.secrets,
            proxy_id,
            CredentialKindDto::ProxyPassword,
        ))
    }

    pub(crate) fn proxy_password_health(&self, proxy_id: &str) -> CredentialHealth {
        let inner = self.lock();
        credential_health_for(&inner.secrets, proxy_id, CredentialKindDto::ProxyPassword)
    }

    // ------------------------------------------------------------------
    // Managed keys
    // ------------------------------------------------------------------

    pub(crate) fn keys(&self) -> Result<Vec<ManagedKeyDto>, BackendError> {
        let inner = self.lock();
        let keys = inner
            .managed_key_store
            .load()
            .map_err(BackendError::from_storage)?;
        Ok(keys.iter().map(key_to_dto).collect())
    }

    pub(crate) fn generate_key(
        &self,
        algorithm: ManagedKeyAlgorithmDto,
        name: String,
    ) -> Result<ManagedKeyDto, BackendError> {
        let inner = self.lock();
        let service = keychain_service(&inner);
        let existing = inner
            .managed_key_store
            .load()
            .map_err(BackendError::from_storage)?;
        let (private_key, public_key) =
            ManagedKeyStore::generate_material(crate::dto::key_algorithm_from_dto(algorithm))
                .map_err(BackendError::from_storage)?;
        let imported = service
            .import_and_persist_key(
                &existing,
                name,
                ManagedKeySource::Generated,
                &private_key,
                Some(&public_key),
                None,
            )
            .map_err(BackendError::from_storage)?;
        Ok(key_to_dto(&imported.record))
    }

    pub(crate) fn import_key(
        &self,
        name: String,
        private_key: String,
        passphrase: Option<String>,
    ) -> Result<ManagedKeyDto, BackendError> {
        let inner = self.lock();
        let service = keychain_service(&inner);
        let existing = inner
            .managed_key_store
            .load()
            .map_err(BackendError::from_storage)?;
        let imported = service
            .import_and_persist_key(
                &existing,
                name,
                ManagedKeySource::Imported,
                &private_key,
                None,
                passphrase.as_deref(),
            )
            .map_err(BackendError::from_storage)?;
        Ok(key_to_dto(&imported.record))
    }

    pub(crate) fn delete_key(&self, key_id: &str) -> Result<bool, BackendError> {
        let inner = self.lock();
        let mut keys = inner
            .managed_key_store
            .load()
            .map_err(BackendError::from_storage)?;
        let Some(index) = keys.iter().position(|key| key.id == key_id) else {
            return Ok(false);
        };
        keys.remove(index);
        // Clear references from hosts so a deleted key never dangles.
        let mut sessions = inner
            .session_store
            .load(&inner.secrets)
            .map_err(BackendError::from_storage)?;
        for profile in &mut sessions {
            if profile.managed_key_id == key_id {
                profile.managed_key_id.clear();
                if profile.auth_method == Some(miaominal_core::profile::AuthMethod::ManagedKey) {
                    profile.auth_method = Some(miaominal_core::profile::AuthMethod::Password);
                }
            }
        }
        inner
            .session_store
            .save(&sessions)
            .map_err(BackendError::from_storage)?;
        inner
            .managed_key_store
            .save(&keys)
            .map_err(BackendError::from_storage)?;
        inner.secrets.delete_managed_key(key_id);
        Ok(true)
    }

    pub(crate) fn public_key(&self, key_id: &str) -> Result<String, BackendError> {
        let inner = self.lock();
        let keys = inner
            .managed_key_store
            .load()
            .map_err(BackendError::from_storage)?;
        keys.iter()
            .find(|key| key.id == key_id)
            .map(|key| key.public_key.clone())
            .ok_or_else(|| BackendError::InvalidArgument {
                detail: format!("managed key {key_id} does not exist"),
            })
    }

    pub(crate) fn rename_key(
        &self,
        key_id: &str,
        name: String,
    ) -> Result<ManagedKeyDto, BackendError> {
        let inner = self.lock();
        let mut keys = inner
            .managed_key_store
            .load()
            .map_err(BackendError::from_storage)?;
        let Some(key) = keys.iter_mut().find(|key| key.id == key_id) else {
            return Err(BackendError::InvalidArgument {
                detail: format!("managed key {key_id} does not exist"),
            });
        };
        let name = name.trim();
        if !name.is_empty() {
            key.name = name.to_string();
        }
        inner
            .managed_key_store
            .save(&keys)
            .map_err(BackendError::from_storage)?;
        keys.iter()
            .find(|key| key.id == key_id)
            .map(key_to_dto)
            .ok_or_else(|| BackendError::InvalidArgument {
                detail: format!("managed key {key_id} does not exist"),
            })
    }

    // ------------------------------------------------------------------
    // Trust (known hosts)
    // ------------------------------------------------------------------

    pub(crate) fn known_hosts(&self) -> Result<Vec<KnownHostDto>, BackendError> {
        let inner = self.lock();
        let entries = inner
            .known_hosts
            .list()
            .map_err(BackendError::from_storage)?;
        Ok(entries
            .iter()
            .map(|entry| KnownHostDto {
                host: entry.host.clone(),
                port: entry.port,
                algorithm: entry.algorithm.clone(),
                fingerprint: entry.fingerprint.clone(),
                added_at_ms: None,
            })
            .collect())
    }

    pub(crate) fn delete_known_host(&self, host: &str, port: u16) -> Result<bool, BackendError> {
        let inner = self.lock();
        inner
            .known_hosts
            .remove(host, port)
            .map_err(BackendError::from_storage)
    }

    // ------------------------------------------------------------------
    // Sync configuration and credentials
    // ------------------------------------------------------------------

    /// Everything one sync operation needs: the process sync service, an
    /// engine bound to the active credential backend, and the live stores.
    pub(crate) fn sync_handles(&self) -> Result<SyncHandles, BackendError> {
        let inner = self.lock();
        let config_store = sync_config_store(inner.secrets.credentials().clone())?;
        Ok(SyncHandles {
            service: self.sync_service.clone(),
            engine: SyncEngine {
                config_store,
                last_capability_report: None,
            },
            settings_store: inner.settings_store.clone(),
            secrets: inner.secrets.clone(),
            vault_locked: inner.vault_mode == VaultModeDto::Locked,
        })
    }

    pub(crate) fn sync_config_snapshot(
        &self,
    ) -> Result<crate::sync_dto::SyncConfigDto, BackendError> {
        let inner = self.lock();
        let store = sync_config_store(inner.secrets.credentials().clone())?;
        Ok(crate::sync_dto::SyncConfigDto::from(&store.config))
    }

    /// Apply a typed field update; the store's merge rules keep unrelated
    /// fields (including desktop-only ones) intact.
    pub(crate) fn update_sync_config(
        &self,
        update: SyncConfigUpdate,
    ) -> Result<crate::sync_dto::SyncConfigDto, BackendError> {
        let inner = self.lock();
        let mut store = sync_config_store(inner.secrets.credentials().clone())?;
        store
            .update(|config| apply_sync_config_update(config, &update))
            .map_err(BackendError::from_storage)?;
        Ok(crate::sync_dto::SyncConfigDto::from(&store.config))
    }

    pub(crate) fn set_sync_credential(
        &self,
        kind: SyncCredentialKind,
        action: CredentialAction,
    ) -> Result<CredentialHealth, BackendError> {
        let inner = self.lock();
        let mut store = sync_config_store(inner.secrets.credentials().clone())?;
        if !matches!(action, CredentialAction::Keep) {
            match kind {
                SyncCredentialKind::GithubToken => match &action {
                    CredentialAction::Set { value } => {
                        store
                            .set_github_token(value)
                            .map_err(BackendError::from_storage)?;
                        store
                            .update(|config| config.has_github_token = !value.trim().is_empty())
                            .map_err(BackendError::from_storage)?;
                    }
                    CredentialAction::Clear => {
                        store
                            .delete_github_token()
                            .map_err(BackendError::from_storage)?;
                        store
                            .update(|config| config.has_github_token = false)
                            .map_err(BackendError::from_storage)?;
                    }
                    CredentialAction::Keep => {}
                },
                SyncCredentialKind::WebdavPassword => match &action {
                    CredentialAction::Set { value } => {
                        store
                            .set_webdav_password(value)
                            .map_err(BackendError::from_storage)?;
                        store
                            .update(|config| config.has_webdav_password = !value.trim().is_empty())
                            .map_err(BackendError::from_storage)?;
                    }
                    CredentialAction::Clear => {
                        store
                            .delete_webdav_password()
                            .map_err(BackendError::from_storage)?;
                        store
                            .update(|config| config.has_webdav_password = false)
                            .map_err(BackendError::from_storage)?;
                    }
                    CredentialAction::Keep => {}
                },
                SyncCredentialKind::Passphrase => match &action {
                    CredentialAction::Set { value } => {
                        let previous =
                            store.get_passphrase().map_err(BackendError::from_storage)?;
                        store
                            .set_passphrase(value)
                            .map_err(BackendError::from_storage)?;
                        if let Err(error) = store.update(|config| {
                            config.has_passphrase = !value.trim().is_empty();
                            if previous.as_deref() != Some(value.as_str()) {
                                // A new encryption passphrase requires a fresh
                                // conditional push so the remote can be read
                                // with the new key.
                                config.last_synced_local_revision = None;
                            }
                        }) {
                            let rollback = match previous.as_deref() {
                                Some(previous) => store.set_passphrase(previous),
                                None => store.delete_passphrase(),
                            };
                            return match rollback {
                                Ok(()) => Err(BackendError::from_storage(error)),
                                Err(rollback_error) => Err(BackendError::from_storage(
                                    error.context(format!(
                                        "restoring the previous passphrase also failed: {rollback_error:#}"
                                    )),
                                )),
                            };
                        }
                    }
                    CredentialAction::Clear => {
                        store
                            .delete_passphrase()
                            .map_err(BackendError::from_storage)?;
                        store
                            .update(|config| {
                                config.has_passphrase = false;
                                config.last_synced_local_revision = None;
                            })
                            .map_err(BackendError::from_storage)?;
                    }
                    CredentialAction::Keep => {}
                },
            }
        }
        Ok(sync_credential_health(&store, kind))
    }

    pub(crate) fn sync_credential_health(&self, kind: SyncCredentialKind) -> CredentialHealth {
        let inner = self.lock();
        match sync_config_store(inner.secrets.credentials().clone()) {
            Ok(store) => sync_credential_health(&store, kind),
            Err(error) => classify_credential_error(&anyhow!("{error}")),
        }
    }

    /// Copy the legacy Kotlin device entries (`sync:gist-token`,
    /// `sync:webdav-password`, `sync:sync-passphrase`) into the Rust credential
    /// store: read, write, verify, then delete the device entry. A failed
    /// migration keeps the legacy entry and reports a typed error.
    pub(crate) fn migrate_legacy_sync_credentials(
        &self,
    ) -> Result<crate::sync_dto::SyncMigrationResult, BackendError> {
        let inner = self.lock();
        let mut store = sync_config_store(inner.secrets.credentials().clone())?;
        let host = Arc::clone(&self.host);

        let migrated_github_token = migrate_legacy_credential(
            &host,
            "sync:gist-token",
            store
                .get_github_token()
                .map_err(BackendError::from_storage)?,
            |value| {
                store
                    .set_github_token(value)
                    .map_err(BackendError::from_storage)
            },
            || store.get_github_token().map_err(BackendError::from_storage),
            || {
                store
                    .delete_github_token()
                    .map_err(BackendError::from_storage)
            },
        )?;
        let migrated_webdav_password = {
            // The legacy Kotlin account for the WebDAV password is the same
            // account the shared store uses (`sync:webdav-password`), so there
            // is nothing to copy: the value is already available to Rust. The
            // flag is reconciled and the entry is left in place.
            let present = store
                .get_webdav_password()
                .map_err(BackendError::from_storage)?
                .is_some_and(|value| !value.is_empty());
            if present {
                store
                    .update(|config| config.has_webdav_password = true)
                    .map_err(BackendError::from_storage)?;
            }
            present
        };
        let migrated_passphrase = migrate_legacy_credential(
            &host,
            "sync:sync-passphrase",
            store.get_passphrase().map_err(BackendError::from_storage)?,
            |value| {
                store
                    .set_passphrase(value)
                    .map_err(BackendError::from_storage)
            },
            || store.get_passphrase().map_err(BackendError::from_storage),
            || {
                store
                    .delete_passphrase()
                    .map_err(BackendError::from_storage)
            },
        )?;

        if migrated_github_token {
            store
                .update(|config| config.has_github_token = true)
                .map_err(BackendError::from_storage)?;
        }
        if migrated_passphrase {
            store
                .update(|config| {
                    config.has_passphrase = true;
                    config.last_synced_local_revision = None;
                })
                .map_err(BackendError::from_storage)?;
        }

        Ok(crate::sync_dto::SyncMigrationResult {
            migrated_github_token,
            migrated_webdav_password,
            migrated_passphrase,
        })
    }

    /// Re-read settings after a sync pull wrote them through an engine clone.
    pub(crate) fn reload_settings_after_sync(&self) -> Result<(), BackendError> {
        let mut inner = self.lock();
        inner
            .settings_store
            .reload_from_disk()
            .map_err(BackendError::from_storage)
    }

    // ------------------------------------------------------------------
    // Configuration import
    // ------------------------------------------------------------------

    /// Commit one confirmed import batch into the session store and the active
    /// credential backend, with the same rollback rules as the desktop import.
    pub(crate) fn commit_import(
        &self,
        batch: miaominal_core::profile::ImportedBatch,
    ) -> Result<miaominal_services::ImportedProfilesResult, BackendError> {
        let inner = self.lock();
        let _sync_guard = miaominal_secrets::lock_sync_data();
        let mut sessions = inner
            .session_store
            .load(&inner.secrets)
            .map_err(BackendError::from_storage)?;
        let service = ProfileService::new(Some(inner.session_store.clone()), inner.secrets.clone());
        service
            .import_profiles(&mut sessions, batch)
            .map_err(BackendError::from_storage)
    }

    // ------------------------------------------------------------------
    // Vault lifecycle
    // ------------------------------------------------------------------

    pub(crate) fn vault_snapshot(&self) -> Result<VaultSnapshot, BackendError> {
        let inner = self.lock();
        Ok(VaultSnapshot {
            mode: inner.vault_mode,
            file_present: VaultCredentialBackend::default_store_exists()
                .map_err(BackendError::from_storage)?,
        })
    }

    pub(crate) fn enable_vault(&self, passphrase: String) -> Result<VaultSnapshot, BackendError> {
        let mut inner = self.lock();
        if inner.vault_mode != VaultModeDto::Disabled {
            return Err(BackendError::InvalidArgument {
                detail: "the local vault is already enabled".to_string(),
            });
        }
        let passphrase = protect_passphrase(passphrase)?;
        let vault_store = open_vault_store(passphrase.clone())?;
        let accounts = collect_credential_accounts(&inner)?;
        // Sync secrets live in their own store (account prefix `sync:`), so
        // they are copied separately but with the same copy-verify order.
        let sync_source = sync_config_store(inner.secrets.credentials().clone())?;
        let sync_target = sync_config_store(vault_store.credentials().clone())?;

        // Copy -> verify.
        if let Err(error) = copy_and_verify(&inner.secrets, &vault_store, &accounts) {
            let _ = VaultCredentialBackend::erase_default_store_file();
            return Err(BackendError::from_storage(error.context(
                "failed to enable the local vault; existing credentials were kept",
            )));
        }
        if let Err(error) = copy_sync_secrets(&sync_source, &sync_target) {
            let _ = VaultCredentialBackend::erase_default_store_file();
            return Err(error);
        }

        // Switch, then persist the mode.
        let previous = std::mem::replace(&mut inner.secrets, vault_store);
        inner.vault_mode = VaultModeDto::Unlocked;
        if let Err(error) = set_vault_enabled_flag(&mut inner, true) {
            // Roll back to the platform backend and remove the partial vault.
            inner.secrets = previous;
            inner.vault_mode = VaultModeDto::Disabled;
            let _ = VaultCredentialBackend::erase_default_store_file();
            return Err(error);
        }

        inner.replace_vault_passphrase(Some(passphrase));

        // Cleanup: the platform copy is no longer authoritative.
        for (id, kind) in &accounts {
            if let Err(error) = previous.delete(id, *kind) {
                log::warn!("failed to remove migrated platform credential for {id}: {error:?}");
            }
        }
        delete_sync_secrets(&sync_source);
        Ok(VaultSnapshot {
            mode: inner.vault_mode,
            file_present: true,
        })
    }

    pub(crate) fn disable_vault(&self) -> Result<VaultSnapshot, BackendError> {
        let mut inner = self.lock();
        if inner.vault_mode != VaultModeDto::Unlocked {
            return Err(BackendError::VaultLocked);
        }
        let platform_store = self.platform_store();
        let accounts = collect_credential_accounts(&inner)?;
        let sync_source = sync_config_store(inner.secrets.credentials().clone())?;
        let sync_target = sync_config_store(platform_store.credentials().clone())?;

        // Copy -> verify.
        copy_and_verify(&inner.secrets, &platform_store, &accounts).map_err(|error| {
            BackendError::from_storage(
                error.context("failed to disable the local vault; the vault was kept"),
            )
        })?;
        copy_sync_secrets(&sync_source, &sync_target)?;

        // Persist the mode first: if the settings write fails, the vault stays
        // authoritative in memory and on disk instead of switching to a
        // backend that a restart would not agree with.
        set_vault_enabled_flag(&mut inner, false)?;

        // Switch only after the persistent flag changed.
        let previous = std::mem::replace(&mut inner.secrets, platform_store);
        inner.replace_vault_passphrase(None);
        inner.vault_mode = VaultModeDto::Disabled;
        drop(previous);
        if let Err(error) = VaultCredentialBackend::erase_default_store_file() {
            log::warn!("failed to erase the local vault file after disabling: {error:?}");
        }
        Ok(VaultSnapshot {
            mode: inner.vault_mode,
            file_present: false,
        })
    }

    pub(crate) fn lock_vault(&self) -> Result<VaultSnapshot, BackendError> {
        let mut inner = self.lock();
        if inner.vault_mode == VaultModeDto::Disabled {
            return Err(BackendError::InvalidArgument {
                detail: "the local vault is not enabled".to_string(),
            });
        }
        // Revoke the shared session material, including copies held by workers.
        inner.replace_vault_passphrase(None);
        inner.secrets = SecretStore::new_locked_vault();
        inner.vault_mode = VaultModeDto::Locked;
        Ok(VaultSnapshot {
            mode: inner.vault_mode,
            file_present: VaultCredentialBackend::default_store_exists()
                .map_err(BackendError::from_storage)?,
        })
    }

    pub(crate) fn unlock_vault(&self, passphrase: String) -> Result<VaultSnapshot, BackendError> {
        let mut inner = self.lock();
        if inner.vault_mode == VaultModeDto::Disabled {
            return Err(BackendError::InvalidArgument {
                detail: "the local vault is not enabled".to_string(),
            });
        }
        if !VaultCredentialBackend::default_store_exists().map_err(BackendError::from_storage)? {
            return Err(BackendError::Storage {
                storage_failure: "local vault file is missing".to_string(),
            });
        }
        let passphrase = protect_passphrase(passphrase)?;
        let vault_store = open_vault_store(passphrase.clone())?;
        inner.secrets = vault_store;
        inner.replace_vault_passphrase(Some(passphrase));
        inner.vault_mode = VaultModeDto::Unlocked;
        Ok(VaultSnapshot {
            mode: inner.vault_mode,
            file_present: true,
        })
    }

    pub(crate) fn change_vault_passphrase(
        &self,
        current: String,
        new: String,
    ) -> Result<VaultSnapshot, BackendError> {
        let mut inner = self.lock();
        if inner.vault_mode == VaultModeDto::Disabled {
            return Err(BackendError::InvalidArgument {
                detail: "the local vault is not enabled".to_string(),
            });
        }
        let current = protect_passphrase(current)?;
        let new = protect_passphrase(new)?;
        VaultCredentialBackend::rotate_default_store_passphrase(&current, &new)
            .map_err(BackendError::from_storage)?;
        inner.replace_vault_passphrase(None);
        match open_vault_store(new.clone()) {
            Ok(vault_store) => {
                inner.secrets = vault_store;
                inner.replace_vault_passphrase(Some(new));
                inner.vault_mode = VaultModeDto::Unlocked;
                Ok(VaultSnapshot {
                    mode: inner.vault_mode,
                    file_present: true,
                })
            }
            Err(error) => {
                inner.secrets = SecretStore::new_locked_vault();
                inner.vault_mode = VaultModeDto::Locked;
                Err(error)
            }
        }
    }

    // ------------------------------------------------------------------
    // Reset
    // ------------------------------------------------------------------

    /// Delete every business file and rebuild the in-memory stores. The host is
    /// responsible for closing running tasks first and for deleting the
    /// platform credential entries afterwards.
    pub(crate) fn clear_local_data(&self) -> Result<(), BackendError> {
        let mut inner = self.lock();
        paths::clear_active_data_dir().map_err(BackendError::from_storage)?;
        *inner = DataInner::new(Arc::clone(&self.host)).map_err(BackendError::from_storage)?;
        Ok(())
    }
}

impl DataInner {
    fn replace_vault_passphrase(&mut self, passphrase: Option<ProtectedPassphrase>) {
        if let Some(previous) = std::mem::replace(&mut self.vault_passphrase, passphrase) {
            previous.revoke();
        }
    }

    fn new(host: Arc<dyn PlatformHost>) -> Result<Self> {
        let settings_store = SettingsStore::load()?;
        // A vault that was enabled before the process restarted must start
        // locked: reads and writes have to fail with `VaultLocked` instead of
        // silently falling back to the platform backend (which would make the
        // just-saved credential diverge from the vault that is unlocked later).
        let (secrets, vault_mode) = if settings_store.settings().local_vault_enabled {
            (SecretStore::new_locked_vault(), VaultModeDto::Locked)
        } else {
            (platform_secret_store(&host), VaultModeDto::Disabled)
        };
        Ok(Self {
            secrets,
            vault_passphrase: None,
            vault_mode,
            session_store: SessionStore::new()?,
            snippet_store: SnippetStore::new()?,
            managed_key_store: ManagedKeyStore::new()?,
            proxy_store: ProxyStore::new()?,
            known_hosts: KnownHostsStore::new()?,
            settings_store,
        })
    }
}

impl Drop for DataInner {
    fn drop(&mut self) {
        self.replace_vault_passphrase(None);
    }
}

fn platform_secret_store(host: &Arc<dyn PlatformHost>) -> SecretStore {
    SecretStore::with_credentials(CredentialStore::with_backend(
        APP_CREDENTIAL_SERVICE,
        PlatformCredentialBackend::new(Arc::clone(host)),
    ))
}

/// Open the sync configuration against the active credential backend.
///
/// The config file itself is not secret, so it loads even while a vault is
/// locked; only reading or writing sync secrets then fails with `VaultLocked`.
fn sync_config_store(credentials: CredentialStore) -> Result<SyncConfigStore, BackendError> {
    SyncConfigStore::load_with_credentials(credentials).map_err(BackendError::from_storage)
}

fn sync_credential_health(store: &SyncConfigStore, kind: SyncCredentialKind) -> CredentialHealth {
    let value = match kind {
        SyncCredentialKind::GithubToken => store.get_github_token(),
        SyncCredentialKind::WebdavPassword => store.get_webdav_password(),
        SyncCredentialKind::Passphrase => store.get_passphrase(),
    };
    match value {
        Ok(Some(value)) if !value.is_empty() => CredentialHealth::Present,
        Ok(_) => CredentialHealth::Missing,
        Err(error) => classify_credential_error(&error),
    }
}

/// Copy one legacy device credential into the Rust account, verify it, and only
/// then delete the device entry. On failure the legacy entry is untouched.
fn migrate_legacy_credential<S, V, D>(
    host: &Arc<dyn PlatformHost>,
    legacy_account: &str,
    stored_value: Option<String>,
    set: S,
    verify: V,
    delete_target: D,
) -> Result<bool, BackendError>
where
    S: FnOnce(&str) -> Result<(), BackendError>,
    V: FnOnce() -> Result<Option<String>, BackendError>,
    D: FnOnce() -> Result<(), BackendError>,
{
    let legacy = host
        .credential_get(legacy_account.to_string())
        .map_err(|error| BackendError::Platform {
            platform_failure: format!("failed to read the legacy sync credential: {error}"),
        })?;
    let Some(legacy) = legacy.filter(|value| !value.is_empty()) else {
        return Ok(false);
    };
    match stored_value {
        Some(existing) if existing == legacy => {
            // Already migrated by an earlier run; the device copy is obsolete.
            host.credential_delete(legacy_account.to_string())
                .map_err(|error| BackendError::Platform {
                    platform_failure: format!(
                        "failed to remove the legacy sync credential: {error}"
                    ),
                })?;
            return Ok(true);
        }
        Some(_) => {
            // A different value is already configured: never overwrite it and
            // never delete the only remaining copy of the legacy secret.
            return Err(BackendError::SyncConflict {
                conflict_detail: format!(
                    "the legacy device entry {legacy_account} differs from the configured sync credential"
                ),
            });
        }
        None => {}
    }

    set(&legacy)?;
    match verify() {
        Ok(Some(copied)) if copied == legacy => {
            host.credential_delete(legacy_account.to_string())
                .map_err(|error| BackendError::Platform {
                    platform_failure: format!(
                        "failed to remove the legacy sync credential: {error}"
                    ),
                })?;
            Ok(true)
        }
        Ok(_) => {
            let _ = delete_target();
            Err(BackendError::Storage {
                storage_failure: format!(
                    "the migrated sync credential for {legacy_account} did not verify; the legacy entry was kept"
                ),
            })
        }
        Err(error) => {
            let _ = delete_target();
            Err(error)
        }
    }
}

/// Mirror of the desktop settings semantics for a field update.
fn apply_sync_config_update(config: &mut SyncConfig, update: &SyncConfigUpdate) {
    if let Some(provider) = update.provider {
        let provider: miaominal_sync::SyncProvider = provider.into();
        if config.provider != provider {
            clear_sync_baseline(config);
        }
        config.provider = provider;
        config.normalize_legacy_provider_flags();
    }
    if let Some(gist_id) = &update.gist_id {
        let next = gist_id.trim();
        let next = (!next.is_empty()).then(|| next.to_string());
        if config.gist_id != next {
            clear_sync_baseline(config);
        }
        config.gist_id = next;
    }
    if let Some(url) = &update.webdav_url {
        if config.webdav_url != *url {
            clear_sync_baseline(config);
        }
        config.webdav_url = url.clone();
    }
    if let Some(username) = &update.webdav_username {
        if config.webdav_username != *username {
            clear_sync_baseline(config);
        }
        config.webdav_username = username.clone();
    }
    if let Some(enabled) = update.auto_sync_enabled {
        config.auto_sync_enabled = enabled;
        if !enabled {
            // Turning the feature off also revokes the preconditions fallback
            // and the legacy probe exemption it was granted for.
            config.webdav_unsafe_write_consent = false;
            config.legacy_auto_sync_compat = false;
        }
    }
    if let Some(consent) = update.webdav_unsafe_write_consent {
        config.webdav_unsafe_write_consent = consent;
    }
}

fn clear_sync_baseline(config: &mut SyncConfig) {
    config.last_sync_at = 0;
    config.remote_etag = None;
    config.remote_payload_id = None;
    config.last_synced_local_revision = None;
}

/// Copy the three sync secrets between credential backends, verifying each one
/// before the caller switches or cleans up.
fn copy_sync_secrets(
    source: &SyncConfigStore,
    target: &SyncConfigStore,
) -> Result<(), BackendError> {
    if let Some(token) = source
        .get_github_token()
        .map_err(BackendError::from_storage)?
    {
        target
            .set_github_token(&token)
            .map_err(BackendError::from_storage)?;
        if target
            .get_github_token()
            .map_err(BackendError::from_storage)?
            .as_deref()
            != Some(token.as_str())
        {
            return Err(BackendError::Storage {
                storage_failure: "the sync token did not verify after copying".to_string(),
            });
        }
    }
    if let Some(password) = source
        .get_webdav_password()
        .map_err(BackendError::from_storage)?
    {
        target
            .set_webdav_password(&password)
            .map_err(BackendError::from_storage)?;
        if target
            .get_webdav_password()
            .map_err(BackendError::from_storage)?
            .as_deref()
            != Some(password.as_str())
        {
            return Err(BackendError::Storage {
                storage_failure: "the WebDAV password did not verify after copying".to_string(),
            });
        }
    }
    if let Some(passphrase) = source
        .get_passphrase()
        .map_err(BackendError::from_storage)?
    {
        target
            .set_passphrase(&passphrase)
            .map_err(BackendError::from_storage)?;
        if target
            .get_passphrase()
            .map_err(BackendError::from_storage)?
            .as_deref()
            != Some(passphrase.as_str())
        {
            return Err(BackendError::Storage {
                storage_failure: "the sync passphrase did not verify after copying".to_string(),
            });
        }
    }
    Ok(())
}

/// Best-effort removal of the sync secrets from a backend that is no longer
/// authoritative.
fn delete_sync_secrets(store: &SyncConfigStore) {
    for (label, result) in [
        ("github token", store.delete_github_token()),
        ("WebDAV password", store.delete_webdav_password()),
        ("sync passphrase", store.delete_passphrase()),
    ] {
        if let Err(error) = result {
            log::warn!("failed to remove the migrated sync {label}: {error:?}");
        }
    }
}

fn open_vault_store(passphrase: ProtectedPassphrase) -> Result<SecretStore, BackendError> {
    let backend = VaultCredentialBackend::new(passphrase).map_err(BackendError::from_storage)?;
    let store = SecretStore::with_credentials(CredentialStore::with_backend(
        APP_CREDENTIAL_SERVICE,
        backend,
    ));
    store
        .credentials()
        .initialize()
        .map_err(BackendError::from_storage)?;
    Ok(store)
}

fn protect_passphrase(passphrase: String) -> Result<ProtectedPassphrase, BackendError> {
    if passphrase.is_empty() {
        return Err(BackendError::InvalidArgument {
            detail: "local vault passphrase must not be empty".to_string(),
        });
    }
    ProtectedPassphrase::try_from_string(passphrase).map_err(|error| {
        BackendError::InvalidArgument {
            detail: error.to_string(),
        }
    })
}

fn keychain_service(inner: &DataInner) -> KeychainService {
    KeychainService::new(
        runtime::handle(),
        inner.managed_key_store.clone(),
        inner.secrets.clone(),
        inner.known_hosts.clone(),
    )
}

fn credential_secret_kind(kind: CredentialKindDto) -> SecretKind {
    match kind {
        CredentialKindDto::Password => SecretKind::Password,
        CredentialKindDto::Passphrase => SecretKind::Passphrase,
        CredentialKindDto::ManagedPrivateKey => SecretKind::ManagedPrivateKey,
        CredentialKindDto::ProxyPassword => SecretKind::ProxyPassword,
    }
}

fn credential_health_for(
    secrets: &SecretStore,
    target_id: &str,
    kind: CredentialKindDto,
) -> CredentialHealth {
    match secrets.get(target_id, credential_secret_kind(kind)) {
        Ok(Some(_)) => CredentialHealth::Present,
        Ok(None) => CredentialHealth::Missing,
        Err(error) if SecretStore::is_locked_error(&error) => CredentialHealth::Locked,
        Err(error) => classify_credential_error(&error),
    }
}

fn set_host_stored_flag(
    inner: &mut DataInner,
    profile_id: &str,
    kind: CredentialKindDto,
    stored: bool,
) -> Result<(), BackendError> {
    let mut sessions = inner
        .session_store
        .load(&inner.secrets)
        .map_err(BackendError::from_storage)?;
    let Some(profile) = sessions.iter_mut().find(|profile| profile.id == profile_id) else {
        return Ok(());
    };
    match kind {
        CredentialKindDto::Password => profile.has_stored_password = stored,
        CredentialKindDto::Passphrase => profile.has_stored_passphrase = stored,
        _ => return Ok(()),
    }
    inner
        .session_store
        .save(&sessions)
        .map_err(BackendError::from_storage)
}

fn set_proxy_stored_flag(
    inner: &mut DataInner,
    proxy_id: &str,
    stored: bool,
) -> Result<(), BackendError> {
    let mut proxies = inner
        .proxy_store
        .load(&inner.secrets)
        .map_err(BackendError::from_storage)?;
    let Some(proxy) = proxies.iter_mut().find(|proxy| proxy.id == proxy_id) else {
        return Ok(());
    };
    proxy.has_stored_password = stored;
    inner
        .proxy_store
        .save(&proxies)
        .map_err(BackendError::from_storage)
}

fn set_vault_enabled_flag(inner: &mut DataInner, enabled: bool) -> Result<(), BackendError> {
    inner
        .settings_store
        .reload_from_disk()
        .map_err(BackendError::from_storage)?;
    let mut settings = inner.settings_store.settings().clone();
    if settings.local_vault_enabled == enabled {
        return Ok(());
    }
    settings.local_vault_enabled = enabled;
    inner
        .settings_store
        .replace(settings)
        .map_err(BackendError::from_storage)?;
    Ok(())
}

/// Every credential the mobile data layer can own, including desktop-only
/// credentials received through sync. Sync tokens are copied separately.
fn collect_credential_accounts(
    inner: &DataInner,
) -> Result<Vec<(String, SecretKind)>, BackendError> {
    let sessions = inner
        .session_store
        .load(&inner.secrets)
        .map_err(BackendError::from_storage)?;
    let keys = inner
        .managed_key_store
        .load()
        .map_err(BackendError::from_storage)?;
    let proxies = inner
        .proxy_store
        .load(&inner.secrets)
        .map_err(BackendError::from_storage)?;

    let mut accounts = Vec::new();
    for profile in &sessions {
        accounts.push((profile.id.clone(), SecretKind::Password));
        accounts.push((profile.id.clone(), SecretKind::Passphrase));
    }
    for key in &keys {
        accounts.push((key.id.clone(), SecretKind::ManagedPrivateKey));
    }
    for proxy in &proxies {
        accounts.push((proxy.id.clone(), SecretKind::ProxyPassword));
    }
    // A pull may have updated settings before the live cache was refreshed.
    // Read the persisted provider list so vault migration cannot omit keys.
    let settings = inner
        .settings_store
        .read_current()
        .map_err(BackendError::from_storage)?;
    for provider in &settings.ai_providers {
        accounts.push((provider.id.clone(), SecretKind::AiProviderApiKey));
    }
    accounts.push(("web_search".to_string(), SecretKind::WebSearchApiKey));
    Ok(accounts)
}

fn copy_and_verify(
    source: &SecretStore,
    target: &SecretStore,
    accounts: &[(String, SecretKind)],
) -> Result<()> {
    for (id, kind) in accounts {
        let Some(value) = source
            .get(id, *kind)
            .with_context(|| format!("failed to read credential {id}"))?
        else {
            continue;
        };
        target
            .set(id, *kind, &value)
            .with_context(|| format!("failed to copy credential {id}"))?;
        let copied = target
            .get(id, *kind)
            .with_context(|| format!("failed to verify credential {id}"))?;
        if copied.as_deref() != Some(value.as_str()) {
            return Err(anyhow!("credential verification failed for {id}"));
        }
    }
    Ok(())
}

fn snippet_to_dto(snippet: &SnippetRecord) -> SnippetDto {
    SnippetDto {
        id: snippet.id.clone(),
        description: snippet.description.clone(),
        package_name: snippet.package.clone(),
        language: snippet.language.clone(),
        script: snippet.script.clone(),
    }
}

fn apply_settings_update(
    settings: &mut AppSettings,
    update: &SettingsUpdate,
) -> Result<(), BackendError> {
    if let Some(language) = &update.language {
        let mapped = crate::dto::language_from_name(language).ok_or_else(|| {
            BackendError::InvalidArgument {
                detail: format!("unsupported language {language}"),
            }
        })?;
        settings.language = mapped;
    }
    if let Some(recent) = update.recent_host_count {
        settings.recent_connections_count = recent.min(u32::from(u8::MAX)) as u8;
    }
    if let Some(auto_monitoring) = update.auto_monitoring {
        settings.auto_collect_session_monitoring = auto_monitoring;
    }
    if let Some(monitor_history) = &update.monitor_history {
        settings.monitor_history_duration = crate::dto::monitor_history_from_name(monitor_history)
            .ok_or_else(|| BackendError::InvalidArgument {
                detail: format!("unsupported monitor history {monitor_history}"),
            })?;
    }
    if let Some(close_behavior) = &update.close_last_session_behavior {
        settings.last_tab_close_behavior = crate::dto::close_behavior_from_name(close_behavior)
            .ok_or_else(|| BackendError::InvalidArgument {
                detail: format!("unsupported close behavior {close_behavior}"),
            })?;
    }
    if let Some(auto_lock) = update.vault_auto_lock_minutes {
        settings.local_vault_auto_lock_duration =
            crate::dto::vault_auto_lock_from_minutes(auto_lock).ok_or_else(|| {
                BackendError::InvalidArgument {
                    detail: format!("unsupported auto-lock duration {auto_lock}"),
                }
            })?;
    }
    if let Some(onboarding_completed) = update.onboarding_completed {
        if onboarding_completed {
            settings.mark_current_onboarding_completed();
        } else {
            settings.completed_onboarding_version = 0;
        }
    }
    if let Some(terminal_font) = &update.terminal_font {
        settings.terminal_font_family = terminal_font.clone();
    }
    if let Some(cjk_fallback) = &update.cjk_fallback_font {
        if cjk_fallback.trim().is_empty() {
            settings.font_fallbacks.clear();
        } else {
            settings.font_fallbacks = vec![cjk_fallback.clone()];
        }
    }
    if let Some(font_size) = update.terminal_font_size {
        settings.terminal_font_size = font_size;
    }
    if let Some(line_height) = update.terminal_line_height {
        settings.line_height = line_height;
    }
    if let Some(theme_id) = &update.theme_id {
        settings.theme_id = crate::dto::theme_id_from_name(theme_id).ok_or_else(|| {
            BackendError::InvalidArgument {
                detail: format!("unsupported theme id {theme_id}"),
            }
        })?;
    }
    if let Some(seed_color) = &update.seed_color {
        settings.seed_color = seed_color.clone();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{
        AuthMethodDto, CredentialAction, CredentialHealth, CredentialKindDto, ForwardKindDto,
        HostUpdate, ManagedKeyAlgorithmDto, ManagedKeySourceDto, SettingsUpdate, VaultModeDto,
    };
    use crate::test_support::{
        TestHost, clear_test_root, initialize_test_runtime, shared_test_lock, test_host,
    };

    fn test_data() -> (MobileData, Arc<TestHost>) {
        initialize_test_runtime();
        miaominal_secrets::set_vault_test_parameters();
        let host = test_host();
        let data = MobileData::new(host.clone()).expect("mobile data should initialize");
        (data, host)
    }

    fn blank_host_update(id: &str) -> HostUpdate {
        HostUpdate {
            id: id.to_string(),
            name: Some(format!("Host {id}")),
            group: None,
            tags: None,
            kind: None,
            host: Some("example.com".to_string()),
            port: Some(22),
            username: Some("akko".to_string()),
            auth_method: None,
            private_key_path: None,
            managed_key_id: None,
            certificate_path: None,
            startup_command: None,
            environment_variables: None,
            shell_type: None,
            is_favorite: None,
            last_connected_at_ms: None,
        }
    }

    fn set_password(data: &MobileData, id: &str, value: &str) {
        let health = data
            .set_host_credential(
                id,
                CredentialKindDto::Password,
                CredentialAction::Set {
                    value: value.to_string(),
                },
            )
            .expect("password should store");
        assert_eq!(health, CredentialHealth::Present);
    }

    #[test]
    fn host_credentials_round_trip_through_the_injected_backend() {
        let _guard = shared_test_lock();
        clear_test_root();
        let (data, host) = test_data();
        data.save_host(blank_host_update("host-1")).unwrap();
        set_password(&data, "host-1", "s3cret");
        assert_eq!(host.stored("host-1:password").as_deref(), Some("s3cret"));
        assert!(
            data.hosts()
                .unwrap()
                .iter()
                .any(|host| host.id == "host-1" && host.has_stored_password)
        );

        data.set_host_credential(
            "host-1",
            CredentialKindDto::Password,
            CredentialAction::Clear,
        )
        .unwrap();
        assert_eq!(host.stored("host-1:password"), None);
        assert!(
            !data
                .hosts()
                .unwrap()
                .iter()
                .any(|host| host.has_stored_password)
        );
    }

    #[test]
    fn vault_enable_copies_verifies_and_cleans_the_platform_copy() {
        let _guard = shared_test_lock();
        clear_test_root();
        let (data, host) = test_data();
        data.save_host(blank_host_update("host-1")).unwrap();
        set_password(&data, "host-1", "vault-secret");
        data.set_host_credential(
            "host-1",
            CredentialKindDto::Passphrase,
            CredentialAction::Set {
                value: "key-phrase".to_string(),
            },
        )
        .unwrap();

        let snapshot = data.enable_vault("correct horse".to_string()).unwrap();

        assert_eq!(snapshot.mode, VaultModeDto::Unlocked);
        assert!(snapshot.file_present);
        // Cleanup removed the platform copies only after the switch.
        assert_eq!(host.stored("host-1:password"), None);
        assert_eq!(host.stored("host-1:passphrase"), None);
        assert_eq!(
            data.host_credential_health("host-1", CredentialKindDto::Password),
            CredentialHealth::Present
        );
        assert!(data.settings().unwrap().vault_enabled);
    }

    #[test]
    fn vault_lock_hides_credentials_until_unlock() {
        let _guard = shared_test_lock();
        clear_test_root();
        let (data, _host) = test_data();
        data.save_host(blank_host_update("host-1")).unwrap();
        set_password(&data, "host-1", "vault-secret");
        data.enable_vault("correct horse".to_string()).unwrap();
        let previous = data.sync_handles().unwrap().secrets;
        // Warm the backend cache before revocation, as a background sync would.
        assert_eq!(
            previous
                .get("host-1", SecretKind::Password)
                .unwrap()
                .as_deref(),
            Some("vault-secret")
        );

        let locked = data.lock_vault().unwrap();
        assert!(previous.get("host-1", SecretKind::Password).is_err());
        assert!(
            previous
                .set("host-1", SecretKind::Password, "stale-write")
                .is_err()
        );
        assert_eq!(locked.mode, VaultModeDto::Locked);
        assert_eq!(
            data.host_credential_health("host-1", CredentialKindDto::Password),
            CredentialHealth::Locked
        );
        // Config edits stay available while the vault is locked.
        data.save_host(HostUpdate {
            name: Some("Renamed while locked".to_string()),
            ..blank_host_update("host-1")
        })
        .unwrap();

        let wrong = data.unlock_vault("wrong horse".to_string());
        assert!(matches!(
            wrong,
            Err(BackendError::AuthenticationFailed { .. })
        ));
        assert_eq!(data.vault_snapshot().unwrap().mode, VaultModeDto::Locked);

        let unlocked = data.unlock_vault("correct horse".to_string()).unwrap();
        assert!(previous.get("host-1", SecretKind::Password).is_err());
        let current = data.sync_handles().unwrap().secrets;
        assert_eq!(
            current
                .get("host-1", SecretKind::Password)
                .unwrap()
                .as_deref(),
            Some("vault-secret")
        );
        assert_eq!(unlocked.mode, VaultModeDto::Unlocked);
        assert_eq!(
            data.host_credential_health("host-1", CredentialKindDto::Password),
            CredentialHealth::Present
        );
        assert!(data.settings().unwrap().vault_enabled);
    }

    #[test]
    fn vault_disable_copies_credentials_back_and_removes_the_file() {
        let _guard = shared_test_lock();
        clear_test_root();
        let (data, host) = test_data();
        data.save_host(blank_host_update("host-1")).unwrap();
        set_password(&data, "host-1", "vault-secret");
        data.enable_vault("correct horse".to_string()).unwrap();
        assert!(VaultCredentialBackend::default_store_exists().unwrap());

        let snapshot = data.disable_vault().unwrap();

        assert_eq!(snapshot.mode, VaultModeDto::Disabled);
        assert!(!snapshot.file_present);
        assert_eq!(
            host.stored("host-1:password").as_deref(),
            Some("vault-secret")
        );
        assert!(!data.settings().unwrap().vault_enabled);
        assert_eq!(
            data.host_credential_health("host-1", CredentialKindDto::Password),
            CredentialHealth::Present
        );
    }

    #[test]
    fn vault_passphrase_change_reopens_the_vault() {
        let _guard = shared_test_lock();
        clear_test_root();
        let (data, _host) = test_data();
        data.save_host(blank_host_update("host-1")).unwrap();
        set_password(&data, "host-1", "vault-secret");
        data.enable_vault("correct horse".to_string()).unwrap();
        let previous = data.sync_handles().unwrap().secrets;

        let snapshot = data
            .change_vault_passphrase("correct horse".to_string(), "new horse".to_string())
            .unwrap();
        assert_eq!(snapshot.mode, VaultModeDto::Unlocked);
        assert!(
            previous
                .set("host-1", SecretKind::Password, "stale-write")
                .is_err()
        );

        data.lock_vault().unwrap();
        assert!(matches!(
            data.unlock_vault("correct horse".to_string()),
            Err(BackendError::AuthenticationFailed { .. })
        ));
        data.unlock_vault("new horse".to_string()).unwrap();
        assert_eq!(
            data.host_credential_health("host-1", CredentialKindDto::Password),
            CredentialHealth::Present
        );
    }

    #[test]
    fn vault_migration_preserves_desktop_credentials_from_sync() {
        let _guard = shared_test_lock();
        // Exercise both directions with settings written through another store,
        // as a pull does before the mobile settings cache has been refreshed.
        for enabling in [true, false] {
            clear_test_root();
            let (data, _) = test_data();
            if !enabling {
                data.enable_vault("correct horse".to_string()).unwrap();
            }
            let (mut synced_settings, source) = {
                let inner = data.lock();
                (inner.settings_store.clone(), inner.secrets.clone())
            };
            let mut provider = miaominal_settings::AiProviderConfig::new(
                miaominal_settings::AiProviderKind::OpenAi,
            );
            provider.id = "synced-provider".to_string();
            provider.has_api_key = true;
            assert!(synced_settings.update(|settings| {
                settings.ai_providers = vec![provider];
                settings.web_search.has_api_key = true;
            }));
            let accounts = [
                ("synced-provider", SecretKind::AiProviderApiKey, "ai-secret"),
                ("web_search", SecretKind::WebSearchApiKey, "search-secret"),
            ];
            for (id, kind, value) in accounts {
                source.set(id, kind, value).unwrap();
            }

            if enabling {
                data.enable_vault("correct horse".to_string()).unwrap();
                data.lock_vault().unwrap();
                data.unlock_vault("correct horse".to_string()).unwrap();
                for (id, kind, _) in accounts {
                    assert!(source.get(id, kind).unwrap().is_none());
                }
            } else {
                data.disable_vault().unwrap();
                assert!(!VaultCredentialBackend::default_store_exists().unwrap());
            }
            let inner = data.lock();
            for (id, kind, value) in accounts {
                assert_eq!(inner.secrets.get(id, kind).unwrap().as_deref(), Some(value));
            }
            let settings = inner.settings_store.read_current().unwrap();
            assert_eq!(settings.ai_providers[0].id, "synced-provider");
            assert!(settings.web_search.has_api_key);
            assert_eq!(settings.local_vault_enabled, enabling);
        }
    }

    #[test]
    fn enable_vault_failure_keeps_the_platform_credentials() {
        let _guard = shared_test_lock();
        clear_test_root();
        let (data, host) = test_data();
        data.save_host(blank_host_update("host-1")).unwrap();
        set_password(&data, "host-1", "platform-secret");

        // Fail every platform read: the copy step cannot verify, so enabling
        // must fail without switching backends or losing the original value.
        host.invalidate_after(0);
        let error = data
            .enable_vault("correct horse".to_string())
            .expect_err("enable must fail when the source cannot be read");
        assert!(matches!(error, BackendError::Storage { .. }));

        assert_eq!(
            host.stored("host-1:password").as_deref(),
            Some("platform-secret")
        );
        assert_eq!(data.vault_snapshot().unwrap().mode, VaultModeDto::Disabled);
        assert!(!data.settings().unwrap().vault_enabled);
        assert!(!VaultCredentialBackend::default_store_exists().unwrap());
    }

    #[test]
    fn deleting_a_host_removes_credentials_and_forward_references() {
        let _guard = shared_test_lock();
        clear_test_root();
        let (data, host) = test_data();
        data.save_host(blank_host_update("host-1")).unwrap();
        data.save_host(HostUpdate {
            auth_method: Some(AuthMethodDto::Password),
            ..blank_host_update("host-2")
        })
        .unwrap();
        set_password(&data, "host-1", "secret");
        data.upsert_forward(ForwardRuleDto {
            id: "forward-1".to_string(),
            profile_id: "host-1".to_string(),
            label: "Local API".to_string(),
            kind: ForwardKindDto::Local,
            listen_host: "127.0.0.1".to_string(),
            listen_port: 15432,
            target_host: "127.0.0.1".to_string(),
            target_port: 5432,
            enabled: false,
        })
        .unwrap();

        data.delete_host("host-1").unwrap();

        let hosts = data.hosts().unwrap();
        assert!(!hosts.iter().any(|host| host.id == "host-1"));
        assert!(data.forwards().unwrap().is_empty());
        assert_eq!(host.stored("host-1:password"), None);
    }

    #[test]
    fn host_save_preserves_fields_the_editor_never_shows() {
        let _guard = shared_test_lock();
        clear_test_root();
        let (data, _host) = test_data();
        let saved = data.save_host(blank_host_update("host-1")).unwrap();

        {
            let inner = data.lock();
            let mut sessions = inner.session_store.load(&inner.secrets).unwrap();
            sessions[0].environment_variables.push(
                miaominal_core::profile::SessionEnvironmentVariable {
                    name: "LANG".to_string(),
                    value: "C".to_string(),
                },
            );
            sessions[0].proxy_jump_profile_ids = vec!["jump-1".to_string()];
            inner.session_store.save(&sessions).unwrap();
        }

        let updated = data
            .save_host(HostUpdate {
                name: Some("Edited".to_string()),
                ..blank_host_update(&saved.id)
            })
            .unwrap();
        assert_eq!(updated.name, "Edited");

        let inner = data.lock();
        let sessions = inner.session_store.load(&inner.secrets).unwrap();
        assert_eq!(sessions[0].environment_variables.len(), 1);
        assert_eq!(sessions[0].proxy_jump_profile_ids, vec!["jump-1"]);
    }

    #[test]
    fn settings_updates_round_trip_through_the_rust_store() {
        let _guard = shared_test_lock();
        clear_test_root();
        let (data, _host) = test_data();

        let settings = data
            .update_settings(SettingsUpdate {
                language: Some("English".to_string()),
                recent_host_count: Some(9),
                auto_monitoring: Some(true),
                monitor_history: Some("TenMinutes".to_string()),
                close_last_session_behavior: Some("OpenHosts".to_string()),
                vault_auto_lock_minutes: Some(15),
                onboarding_completed: Some(true),
                terminal_font: Some("JetBrains Mono".to_string()),
                cjk_fallback_font: Some("Noto Sans CJK SC".to_string()),
                terminal_font_size: Some(16.0),
                terminal_line_height: Some(20.0),
                theme_id: Some("dark".to_string()),
                seed_color: Some("#123456".to_string()),
            })
            .unwrap();

        assert_eq!(settings.language, "English");
        assert_eq!(settings.recent_host_count, 9);
        assert!(settings.auto_monitoring);
        assert_eq!(settings.monitor_history, "TenMinutes");
        assert_eq!(settings.close_last_session_behavior, "OpenHosts");
        assert_eq!(settings.vault_auto_lock_minutes, 15);
        assert!(settings.onboarding_completed);
        assert_eq!(settings.terminal_font, "JetBrains Mono");
        assert_eq!(settings.cjk_fallback_font, "Noto Sans CJK SC");
        assert_eq!(settings.terminal_font_size, 16.0);
        assert_eq!(settings.terminal_line_height, 20.0);
        assert_eq!(settings.theme_id, "dark");
        assert_eq!(settings.seed_color, "#123456");
        assert_eq!(data.settings().unwrap(), settings);
    }

    #[test]
    fn generated_key_private_material_goes_to_the_injected_backend() {
        let _guard = shared_test_lock();
        clear_test_root();
        let (data, host) = test_data();

        let key = data
            .generate_key(ManagedKeyAlgorithmDto::Ed25519, "mobile deploy".to_string())
            .unwrap();

        assert_eq!(key.source, ManagedKeySourceDto::Generated);
        assert!(key.public_key.starts_with("ssh-ed25519 "));
        assert!(
            host.stored(&format!("{}:managed-private-key", key.id))
                .is_some()
        );
        assert_eq!(data.public_key(&key.id).unwrap(), key.public_key);
        assert!(data.delete_key(&key.id).unwrap());
        assert_eq!(
            host.stored(&format!("{}:managed-private-key", key.id)),
            None
        );
    }

    #[test]
    fn clearing_local_data_resets_files_and_vault_state() {
        let _guard = shared_test_lock();
        clear_test_root();
        let (data, _host) = test_data();
        data.save_host(blank_host_update("host-1")).unwrap();
        set_password(&data, "host-1", "secret");
        data.enable_vault("correct horse".to_string()).unwrap();

        data.clear_local_data().unwrap();

        assert!(data.hosts().unwrap().is_empty());
        assert!(!data.settings().unwrap().vault_enabled);
        assert_eq!(data.vault_snapshot().unwrap().mode, VaultModeDto::Disabled);
        assert!(!VaultCredentialBackend::default_store_exists().unwrap());
    }

    #[test]
    fn a_restarted_data_layer_starts_with_a_locked_vault() {
        let _guard = shared_test_lock();
        clear_test_root();
        let (data, host) = test_data();
        data.save_host(blank_host_update("host-1")).unwrap();
        set_password(&data, "host-1", "vault-secret");
        data.enable_vault("correct horse".to_string()).unwrap();
        drop(data);

        // New generation over the same business root: the persisted vault
        // flag must select the locked backend instead of the platform store.
        let (data, platform_host) = test_data();
        assert_eq!(data.vault_snapshot().unwrap().mode, VaultModeDto::Locked);
        assert_eq!(
            data.host_credential_health("host-1", CredentialKindDto::Password),
            CredentialHealth::Locked
        );

        let error = data
            .set_host_credential(
                "host-1",
                CredentialKindDto::Password,
                CredentialAction::Set {
                    value: "must-not-leak".to_string(),
                },
            )
            .expect_err("a locked vault must reject credential writes");
        assert!(matches!(error, BackendError::VaultLocked));
        assert_eq!(platform_host.stored("host-1:password"), None);
        assert_eq!(host.stored("host-1:password"), None);

        data.unlock_vault("correct horse".to_string()).unwrap();
        assert_eq!(
            data.host_credential_health("host-1", CredentialKindDto::Password),
            CredentialHealth::Present
        );
        // The rejected write never reached the platform backend.
        assert_eq!(platform_host.stored("host-1:password"), None);
    }

    #[test]
    fn disable_vault_keeps_the_vault_when_the_settings_write_fails() {
        use std::fs;

        let _guard = shared_test_lock();
        clear_test_root();
        let (data, _host) = test_data();
        data.save_host(blank_host_update("host-1")).unwrap();
        set_password(&data, "host-1", "vault-secret");
        data.enable_vault("correct horse".to_string()).unwrap();

        let settings_file = paths::config_file("settings.toml").unwrap();
        let mut permissions = fs::metadata(&settings_file).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&settings_file, permissions).unwrap();

        let error = data
            .disable_vault()
            .expect_err("a settings write failure must abort the switch");
        assert!(matches!(error, BackendError::Storage { .. }));
        assert_eq!(data.vault_snapshot().unwrap().mode, VaultModeDto::Unlocked);
        assert_eq!(
            data.host_credential_health("host-1", CredentialKindDto::Password),
            CredentialHealth::Present
        );
        assert!(VaultCredentialBackend::default_store_exists().unwrap());

        let mut permissions = fs::metadata(&settings_file).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&settings_file, permissions).unwrap();
        data.disable_vault().unwrap();
    }

    #[test]
    fn invalidated_platform_key_reports_an_unavailable_credential() {
        let _guard = shared_test_lock();
        clear_test_root();
        let (data, host) = test_data();
        host.invalidate_after(0);
        assert_eq!(
            data.host_credential_health("host-1", CredentialKindDto::Password),
            CredentialHealth::Invalidated
        );
    }
}
