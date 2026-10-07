//! Mobile backend handle: lifecycle, snapshot, and event stream.
//!
//! `Backend` is the single entry point Kotlin owns. It initializes the embedded
//! runtime (paths), publishes sequenced control events, and shuts down
//! idempotently. Later phases add the configuration, session, and transfer
//! interfaces on top of this lifecycle.
//!
//! Lifecycle mutations and snapshot reads share one mutex, and every event is
//! published while that mutex is held. A snapshot can therefore never combine
//! an old lifecycle state with a newer event cursor; a subscriber started from
//! `BackendSnapshot::sequence` cannot miss an event published before the read.

use std::sync::Arc;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::watch;

use crate::build_info::BuildInfo;
use crate::data::{MobileData, OperationGuard};
use crate::dto::{
    CredentialAction, CredentialHealth, CredentialKindDto, ForwardRuleDto, HostDto, HostUpdate,
    KnownHostDto, ManagedKeyAlgorithmDto, ManagedKeyDto, ProxyDto, ProxyUpdate, SettingsDto,
    SettingsUpdate, SnippetDto, SnippetUpdate, VaultSnapshot,
};
use crate::error::BackendError;
use crate::agent::{AgentSessionRegistry, AgentSshSession};
use crate::events::{BackendEvent, ConfigArea, EventBus, EventSubscription};
use crate::forward::{ForwardSession, ForwardSessionRegistry, ForwardSnapshot};
use crate::local_file::LocalFileHandle;
use crate::platform::{LogLevel, PlatformHost};
use crate::runtime;
use crate::session::{SessionRegistry, SshSession, SshSessionSnapshot};
use crate::sftp::{SftpSession, SftpSessionRegistry, SftpSessionSnapshot};
use crate::sync::{MAX_IMPORT_BYTES, SyncCoordinator, has_provider_credentials};
use crate::sync_dto::{
    ImportCommitResult, ImportPreviewDto, ImportSourceDto, SyncConfigDto, SyncConfigUpdate,
    SyncCredentialKind, SyncRemoteStateDto, SyncResolution, SyncRunResult, SyncSnapshotDto,
    SyncStatusDto,
};

/// Host-provided directories for the embedded runtime.
///
/// Android passes `noBackupFilesDir` for business data and `cacheDir` for
/// temporary data; the native library never derives directories on its own.
#[derive(Debug, Clone, uniffi::Record)]
pub struct BackendConfig {
    pub data_dir: String,
    pub cache_dir: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct BackendSnapshot {
    pub initialized: bool,
    pub closed: bool,
    /// Latest published event sequence; pass it to `subscribe_from` to make
    /// sure no event between the snapshot and the subscription is missed.
    pub sequence: u64,
    pub data_dir: String,
    pub cache_dir: String,
}

#[derive(Debug, Default)]
pub(crate) struct LifecycleState {
    pub(crate) initialized: bool,
    pub(crate) closed: bool,
}

#[derive(uniffi::Object)]
pub struct Backend {
    host: Arc<dyn PlatformHost>,
    config: BackendConfig,
    events: Arc<EventBus>,
    sessions: Arc<SessionRegistry>,
    agent_sessions: Arc<AgentSessionRegistry>,
    sftp_sessions: Arc<SftpSessionRegistry>,
    forward_sessions: Arc<ForwardSessionRegistry>,
    sync: Arc<SyncCoordinator>,
    lifecycle: Arc<Mutex<LifecycleState>>,
    data: Mutex<Option<Arc<MobileData>>>,
    close_tx: watch::Sender<bool>,
    close_rx: watch::Receiver<bool>,
}

#[uniffi::export(async_runtime = "tokio")]
impl Backend {
    #[uniffi::constructor]
    pub fn new(host: Arc<dyn PlatformHost>, config: BackendConfig) -> Arc<Self> {
        crate::platform::install_logger();
        let (close_tx, close_rx) = watch::channel(false);
        let events = EventBus::new();
        let lifecycle = Arc::new(Mutex::new(LifecycleState::default()));
        let sessions = SessionRegistry::new(Arc::clone(&lifecycle), Arc::clone(&events));
        let agent_sessions = AgentSessionRegistry::new(Arc::clone(&lifecycle));
        let sftp_sessions = SftpSessionRegistry::new(Arc::clone(&lifecycle), Arc::clone(&events));
        let forward_sessions = ForwardSessionRegistry::new(Arc::clone(&lifecycle), Arc::clone(&events));
        let sync = SyncCoordinator::new(
            Arc::clone(&lifecycle),
            Arc::clone(&events),
            close_rx.clone(),
        );
        let backend = Arc::new(Self {
            host,
            config,
            events,
            sessions,
            agent_sessions,
            sftp_sessions,
            forward_sessions,
            sync,
            lifecycle,
            data: Mutex::new(None),
            close_tx,
            close_rx,
        });
        backend
            .host
            .log(LogLevel::Info, "native backend created".to_string());
        backend
    }

    pub fn build_info(&self) -> BuildInfo {
        BuildInfo::current()
    }

    pub fn device_label(&self) -> String {
        self.host.device_label()
    }

    pub fn probe_secret(&self, key: String) -> Result<Option<String>, BackendError> {
        self.ensure_open()?;
        self.host.probe_secret(key)
    }

    /// Initialize the embedded runtime. Repeated calls are idempotent; a
    /// failure leaves the backend uninitialized and returns the error instead
    /// of falling back to another directory.
    ///
    /// The commit is serialized with [`Backend::shutdown`]: if the backend was
    /// closed while the directories were being prepared, this returns `Closed`
    /// and no `Ready` event is published after `Shutdown`.
    pub async fn initialize(&self) -> Result<BackendSnapshot, BackendError> {
        {
            let state = self.lock_lifecycle();
            if state.closed {
                return Err(BackendError::Closed);
            }
            if state.initialized {
                return Ok(self.build_snapshot(&state));
            }
        }
        let data_dir = self.config.data_dir.clone();
        let cache_dir = self.config.cache_dir.clone();
        runtime::offload(move || {
            miaominal_paths::initialize_embedded_runtime(data_dir, cache_dir)
        })
        .await
        .map_err(|error| BackendError::RuntimeUnavailable {
            runtime_failure: error.to_string(),
        })?;

        let host = Arc::clone(&self.host);
        let (data, vault_locked) = runtime::offload(move || {
            let data = MobileData::new(host)?;
            let locked = data.vault_snapshot()?.mode == crate::dto::VaultModeDto::Locked;
            Ok::<_, anyhow::Error>((data, locked))
        })
        .await
        .map_err(BackendError::from_storage)?;

        let snapshot = {
            let mut state = self.lock_lifecycle();
            if state.closed {
                return Err(BackendError::Closed);
            }
            if !state.initialized {
                state.initialized = true;
                let data = Arc::new(data);
                self.set_data(Arc::clone(&data));
                self.sync.attach_data(Arc::clone(&data));
                self.host.log(
                    LogLevel::Info,
                    format!(
                        "native backend initialized (data_dir={}, cache_dir={})",
                        self.config.data_dir, self.config.cache_dir
                    ),
                );
                self.events.publish(BackendEvent::Ready);
            }
            self.build_snapshot(&state)
        };
        // Outside the lifecycle lock: the coordinator publishes its own events.
        if snapshot.initialized {
            self.sync.set_vault_locked(vault_locked);
        }
        Ok(snapshot)
    }

    /// Consistent view of the lifecycle state and the matching event cursor.
    pub fn snapshot(&self) -> BackendSnapshot {
        let state = self.lock_lifecycle();
        self.build_snapshot(&state)
    }

    /// Subscribe from the cursor returned by [`Backend::snapshot`]; events with
    /// a higher sequence are delivered in order.
    pub fn subscribe_from(&self, since_sequence: u64) -> Arc<EventSubscription> {
        self.events.subscribe(since_sequence)
    }

    /// Idempotent shutdown: publishes `Shutdown`, wakes event waiters, drains
    /// in-flight data operations, cancels running backend tasks, and rejects
    /// further lifecycle calls. The loaded runtime stays alive so the process
    /// can create a fresh backend handle for the same business root.
    pub fn shutdown(&self) {
        {
            let mut state = self.lock_lifecycle();
            if state.closed {
                return;
            }
            state.closed = true;
            self.events.publish(BackendEvent::Shutdown);
            self.events.close();
        }
        // Sessions observe the same close signal through their watch receiver;
        // ask them to stop so no SSH task outlives the handle, then drop the
        // retained sessions so the registry cannot keep them (or their parser
        // threads) alive after a restart.
        self.sync.cancel();
        self.sessions.request_close_all();
        self.sessions.clear();
        // AI channel sessions own SSH connections and listening sockets; close
        // them before the storage generation retires so no worker outlives the
        // handle and no tunnel keeps a remote loopback port reachable.
        self.agent_sessions.request_close_all();
        let agent_registry = Arc::clone(&self.agent_sessions);
        runtime::runtime().spawn(async move {
            agent_registry
                .wait_all(std::time::Duration::from_secs(5))
                .await;
            agent_registry.clear();
        });
        // SFTP workers cancel running transfers (and remove incomplete local
        // documents) before they exit; give them a bounded moment so a restart
        // cannot race the old generation's cleanup.
        self.sftp_sessions.request_close_all();
        self.sftp_sessions
            .wait_all(std::time::Duration::from_secs(5));
        self.sftp_sessions.clear();
        // Forwarding sessions own listening sockets and their SSH routes; the
        // close signal already asked them to stop, and the bounded wait proves
        // no forward worker outlives the handle.
        self.forward_sessions.request_close_all();
        self.forward_sessions
            .wait_all(std::time::Duration::from_secs(5));
        self.forward_sessions.clear();
        // Wait for storage writes started by this generation to finish before
        // a restart may clear or replace the business root; operations that
        // arrive later fail with `Closed` instead of writing stale data.
        let data = self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(data) = data {
            data.retire();
        }
        let _ = self.close_tx.send(true);
        self.host
            .log(LogLevel::Info, "native backend closed".to_string());
    }

    pub fn is_closed(&self) -> bool {
        self.lock_lifecycle().closed
    }

    /// Asynchronous round trip executed on the shared native runtime. The call
    /// is cancelled as soon as the backend shuts down.
    pub async fn echo(&self, payload: String, delay_ms: u32) -> Result<String, BackendError> {
        self.ensure_open()?;
        let mut closed = self.close_rx.clone();
        runtime::run_task(async move {
            tokio::select! {
                result = async move {
                    if delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(u64::from(delay_ms))).await;
                    }
                    payload
                } => Ok(result),
                () = wait_for_close(&mut closed) => Err(BackendError::Closed),
            }
        })
        .await?
    }

    // ------------------------------------------------------------------
    // Settings
    // ------------------------------------------------------------------

    pub async fn settings_snapshot(&self) -> Result<SettingsDto, BackendError> {
        self.read_data(|data| data.settings()).await
    }

    pub async fn update_settings(
        &self,
        update: SettingsUpdate,
    ) -> Result<SettingsDto, BackendError> {
        let snapshot = self
            .write_data(ConfigArea::Settings, move |data| data.update_settings(update))
            .await?;
        // The settings store installs the new palette globally, but live
        // sessions captured the old theme when they were created; refresh them
        // so a theme or seed edit reaches open terminals immediately.
        self.sessions.refresh_terminal_themes();
        Ok(snapshot)
    }

    // ------------------------------------------------------------------
    // Hosts and forwarding rules
    // ------------------------------------------------------------------

    pub async fn hosts_snapshot(&self) -> Result<Vec<HostDto>, BackendError> {
        self.read_data(|data| data.hosts()).await
    }

    pub async fn save_host(&self, update: HostUpdate) -> Result<HostDto, BackendError> {
        self.write_data(ConfigArea::Hosts, move |data| data.save_host(update))
            .await
    }

    /// Replace the reusable entry proxy and ProxyJump chain of one host.
    ///
    /// Unknown proxies/hosts and chaining loops are rejected before anything is
    /// written; every later connection (SSH, SFTP, forwarding, key deployment)
    /// uses the same resolved route.
    pub async fn set_host_routing(
        &self,
        profile_id: String,
        entry_proxy_id: String,
        proxy_jump_profile_ids: Vec<String>,
    ) -> Result<HostDto, BackendError> {
        self.write_data(ConfigArea::Hosts, move |data| {
            data.set_host_routing(&profile_id, entry_proxy_id, proxy_jump_profile_ids)
        })
        .await
    }

    pub async fn delete_host(&self, id: String) -> Result<bool, BackendError> {        let profile_id = id.clone();
        let removed = self
            .write_data_all(&[ConfigArea::Hosts, ConfigArea::Forwards], move |data| {
                data.delete_host(&id)
            })
            .await?;
        if removed {
            // A deleted host must not keep a forwarding route alive.
            self.forward_sessions
                .request_close_matching(&profile_id, None);
        }
        Ok(removed)
    }

    pub async fn forwards_snapshot(&self) -> Result<Vec<ForwardRuleDto>, BackendError> {
        self.read_data(|data| data.forwards()).await
    }

    pub async fn upsert_forward(
        &self,
        forward: ForwardRuleDto,
    ) -> Result<ForwardRuleDto, BackendError> {
        self.write_data(ConfigArea::Forwards, move |data| {
            data.upsert_forward(forward)
        })
        .await
    }

    pub async fn delete_forward(
        &self,
        profile_id: String,
        rule_id: String,
    ) -> Result<bool, BackendError> {
        let (profile, rule) = (profile_id.clone(), rule_id.clone());
        let removed = self
            .write_data(ConfigArea::Forwards, move |data| {
                data.delete_forward(&profile_id, &rule_id)
            })
            .await?;
        if removed {
            self.forward_sessions
                .request_close_matching(&profile, Some(&rule));
        }
        Ok(removed)
    }

    pub async fn set_forward_enabled(
        &self,
        profile_id: String,
        rule_id: String,
        enabled: bool,
    ) -> Result<bool, BackendError> {
        self.write_data(ConfigArea::Forwards, move |data| {
            data.set_forward_enabled(&profile_id, &rule_id, enabled)
        })
        .await
    }

    // ------------------------------------------------------------------
    // Snippets
    // ------------------------------------------------------------------

    pub async fn snippets_snapshot(&self) -> Result<Vec<SnippetDto>, BackendError> {
        self.read_data(|data| data.snippets()).await
    }

    pub async fn save_snippet(&self, update: SnippetUpdate) -> Result<SnippetDto, BackendError> {
        self.write_data(ConfigArea::Snippets, move |data| data.save_snippet(update))
            .await
    }

    pub async fn delete_snippet(&self, id: String) -> Result<bool, BackendError> {
        self.write_data(ConfigArea::Snippets, move |data| data.delete_snippet(&id))
            .await
    }

    // ------------------------------------------------------------------
    // Proxies
    // ------------------------------------------------------------------

    pub async fn proxies_snapshot(&self) -> Result<Vec<ProxyDto>, BackendError> {
        self.read_data(|data| data.proxies()).await
    }

    pub async fn save_proxy(&self, update: ProxyUpdate) -> Result<ProxyDto, BackendError> {
        self.write_data(ConfigArea::Proxies, move |data| data.save_proxy(update))
            .await
    }

    pub async fn delete_proxy(&self, id: String) -> Result<bool, BackendError> {
        self.write_data(ConfigArea::Proxies, move |data| data.delete_proxy(&id))
            .await
    }

    // ------------------------------------------------------------------
    // Credentials
    // ------------------------------------------------------------------

    pub async fn host_credential_health(
        &self,
        profile_id: String,
        kind: CredentialKindDto,
    ) -> Result<CredentialHealth, BackendError> {
        self.read_data(move |data| Ok(data.host_credential_health(&profile_id, kind)))
            .await
    }

    pub async fn set_host_credential(
        &self,
        profile_id: String,
        kind: CredentialKindDto,
        action: CredentialAction,
    ) -> Result<CredentialHealth, BackendError> {
        self.write_data(ConfigArea::Credentials, move |data| {
            data.set_host_credential(&profile_id, kind, action)
        })
        .await
    }

    pub async fn proxy_password_health(
        &self,
        proxy_id: String,
    ) -> Result<CredentialHealth, BackendError> {
        self.read_data(move |data| Ok(data.proxy_password_health(&proxy_id)))
            .await
    }

    pub async fn set_proxy_password(
        &self,
        proxy_id: String,
        action: CredentialAction,
    ) -> Result<CredentialHealth, BackendError> {
        self.write_data(ConfigArea::Credentials, move |data| {
            data.set_proxy_password(&proxy_id, action)
        })
        .await
    }

    pub async fn vault_snapshot(&self) -> Result<VaultSnapshot, BackendError> {
        self.read_data(|data| data.vault_snapshot()).await
    }

    pub async fn enable_vault(&self, passphrase: String) -> Result<VaultSnapshot, BackendError> {
        let snapshot = self
            .write_data_all(
                &[ConfigArea::Credentials, ConfigArea::Settings],
                move |data| data.enable_vault(passphrase),
            )
            .await?;
        self.sync.set_vault_locked(false);
        Ok(snapshot)
    }

    pub async fn disable_vault(&self) -> Result<VaultSnapshot, BackendError> {
        let snapshot = self
            .write_data_all(
                &[ConfigArea::Credentials, ConfigArea::Settings],
                |data| data.disable_vault(),
            )
            .await?;
        self.sync.set_vault_locked(false);
        Ok(snapshot)
    }

    pub async fn lock_vault(&self) -> Result<VaultSnapshot, BackendError> {
        let snapshot = self
            .write_data(ConfigArea::Credentials, |data| data.lock_vault())
            .await?;
        self.sync.set_vault_locked(true);
        Ok(snapshot)
    }

    pub async fn unlock_vault(&self, passphrase: String) -> Result<VaultSnapshot, BackendError> {
        let snapshot = self
            .write_data(ConfigArea::Credentials, move |data| {
                data.unlock_vault(passphrase)
            })
            .await?;
        self.sync.set_vault_locked(false);
        Ok(snapshot)
    }

    pub async fn change_vault_passphrase(
        &self,
        current: String,
        new: String,
    ) -> Result<VaultSnapshot, BackendError> {
        let snapshot = self
            .write_data(ConfigArea::Credentials, move |data| {
                data.change_vault_passphrase(current, new)
            })
            .await?;
        self.sync.set_vault_locked(false);
        Ok(snapshot)
    }

    // ------------------------------------------------------------------
    // Managed keys
    // ------------------------------------------------------------------

    pub async fn keys_snapshot(&self) -> Result<Vec<ManagedKeyDto>, BackendError> {
        self.read_data(|data| data.keys()).await
    }

    pub async fn generate_key(
        &self,
        algorithm: ManagedKeyAlgorithmDto,
        name: String,
    ) -> Result<ManagedKeyDto, BackendError> {
        self.write_data_all(
            &[ConfigArea::Credentials],
            move |data| data.generate_key(algorithm, name),
        )
        .await
    }

    pub async fn import_key(
        &self,
        name: String,
        private_key: String,
        passphrase: Option<String>,
    ) -> Result<ManagedKeyDto, BackendError> {
        self.write_data_all(&[ConfigArea::Credentials], move |data| {
            data.import_key(name, private_key, passphrase)
        })
        .await
    }

    pub async fn delete_key(&self, key_id: String) -> Result<bool, BackendError> {
        self.write_data_all(
            &[ConfigArea::Credentials, ConfigArea::Hosts],
            move |data| data.delete_key(&key_id),
        )
        .await
    }

    pub async fn public_key(&self, key_id: String) -> Result<String, BackendError> {
        self.read_data(move |data| data.public_key(&key_id)).await
    }

    pub async fn rename_key(
        &self,
        key_id: String,
        name: String,
    ) -> Result<ManagedKeyDto, BackendError> {
        self.write_data(ConfigArea::Credentials, move |data| {
            data.rename_key(&key_id, name)
        })
        .await
    }

    // ------------------------------------------------------------------
    // Trust
    // ------------------------------------------------------------------

    pub async fn known_hosts_snapshot(&self) -> Result<Vec<KnownHostDto>, BackendError> {
        self.read_data(|data| data.known_hosts()).await
    }

    pub async fn delete_known_host(
        &self,
        host: String,
        port: u16,
    ) -> Result<bool, BackendError> {
        self.write_data(ConfigArea::Trust, move |data| {
            data.delete_known_host(&host, port)
        })
        .await
    }

    // ------------------------------------------------------------------
    // Sync
    // ------------------------------------------------------------------

    /// Pull-based sync state: phase, last result, capability verdict, pending
    /// conflict. Progress rows are never queued; the host re-reads this.
    pub async fn sync_snapshot(&self) -> Result<SyncSnapshotDto, BackendError> {
        let data = self.data_handle()?;
        let probe = Arc::clone(&data);
        let (config, vault_locked) = runtime::offload(move || {
            Ok::<_, BackendError>((
                probe.sync_config_snapshot()?,
                probe.vault_snapshot()?.mode == crate::dto::VaultModeDto::Locked,
            ))
        })
        .await?;
        Ok(self.sync.snapshot(&config, vault_locked))
    }

    pub async fn update_sync_config(
        &self,
        update: SyncConfigUpdate,
    ) -> Result<SyncConfigDto, BackendError> {
        self.write_data(ConfigArea::Sync, move |data| data.update_sync_config(update))
            .await
    }

    pub async fn set_sync_credential(
        &self,
        kind: SyncCredentialKind,
        action: CredentialAction,
    ) -> Result<CredentialHealth, BackendError> {
        self.write_data(ConfigArea::Sync, move |data| {
            data.set_sync_credential(kind, action)
        })
        .await
    }

    pub async fn sync_credential_health(
        &self,
        kind: SyncCredentialKind,
    ) -> Result<CredentialHealth, BackendError> {
        self.read_data(move |data| Ok(data.sync_credential_health(kind)))
            .await
    }

    /// Copy the legacy Kotlin sync entries into the Rust credential store. The
    /// device entries are removed only after the copy verified.
    pub async fn sync_migrate_legacy_credentials(
        &self,
    ) -> Result<crate::sync_dto::SyncMigrationResult, BackendError> {
        self.write_data(ConfigArea::Sync, |data| {
            data.migrate_legacy_sync_credentials()
        })
        .await
    }

    pub async fn sync_check_capability(&self) -> Result<SyncRunResult, BackendError> {
        self.ensure_open()?;
        let data = self.data_handle()?;
        let probe = Arc::clone(&data);
        let handles = runtime::offload(move || probe.sync_handles()).await?;
        let result = self.sync.run_capability_check(handles).await?;
        Ok(result)
    }

    pub async fn sync_pull(&self, force: bool) -> Result<SyncRunResult, BackendError> {
        self.ensure_open()?;
        let data = self.data_handle()?;
        let probe = Arc::clone(&data);
        let handles = runtime::offload(move || probe.sync_handles()).await?;
        let result = self.sync.run_pull(handles, force).await?;
        Ok(result)
    }

    pub async fn sync_push(&self, manual: bool) -> Result<SyncRunResult, BackendError> {
        self.ensure_open()?;
        let data = self.data_handle()?;
        let probe = Arc::clone(&data);
        let handles = runtime::offload(move || probe.sync_handles()).await?;
        let result = self.sync.run_push(handles, manual).await?;
        Ok(result)
    }

    pub async fn sync_remote_state(&self) -> Result<SyncRemoteStateDto, BackendError> {
        self.ensure_open()?;
        let data = self.data_handle()?;
        let probe = Arc::clone(&data);
        let handles = runtime::offload(move || probe.sync_handles()).await?;
        self.sync.run_remote_state(handles).await
    }

    /// Explicit conflict resolution: keep local (force push) or remote (force
    /// pull). Never applied automatically.
    pub async fn sync_resolve_conflict(
        &self,
        resolution: SyncResolution,
    ) -> Result<SyncRunResult, BackendError> {
        self.ensure_open()?;
        let data = self.data_handle()?;
        let probe = Arc::clone(&data);
        let handles = runtime::offload(move || probe.sync_handles()).await?;
        let result = self.sync.run_conflict_resolution(handles, resolution).await?;
        Ok(result)
    }

    /// Explicit cancellation of the in-flight sync operation (pull, push,
    /// capability check, auto pass). Dropping a Kotlin coroutine is not enough.
    pub fn sync_cancel(&self) {
        self.sync.cancel();
    }

    /// Automatic pass used by the foreground check and the WorkManager job.
    pub async fn sync_auto_check(&self, from_worker: bool) -> Result<SyncRunResult, BackendError> {
        self.ensure_open()?;
        let data = self.data_handle()?;
        let probe = Arc::clone(&data);
        let handles = runtime::offload(move || probe.sync_handles()).await?;
        let trigger = if from_worker {
            crate::sync::AutoTrigger::Worker
        } else {
            crate::sync::AutoTrigger::Foreground
        };
        let result = self.sync.run_auto_check(handles, trigger).await?;
        Ok(result)
    }

    /// Turn automatic sync on or off. Enabling a WebDAV configuration first
    /// requires a capability verdict (or the legacy exemption); a remote that
    /// cannot prove safe writes is reported as a typed error and stays off.
    pub async fn sync_set_auto_sync(&self, enabled: bool) -> Result<SyncRunResult, BackendError> {
        self.ensure_open()?;
        let data = self.data_handle()?;
        let probe = Arc::clone(&data);
        let handles = runtime::offload(move || probe.sync_handles()).await?;
        let engine_config = handles.engine.config_store.config.clone();
        if enabled {
            if !has_provider_credentials(&engine_config) {
                return Err(BackendError::SyncNotConfigured {
                    sync_detail:
                        "provider credentials or the encryption passphrase are missing".to_string(),
                });
            }
            if engine_config.provider == miaominal_sync::SyncProvider::WebDav
                && !engine_config.legacy_auto_sync_compat
            {
                self.sync.run_capability_check(handles).await?;
                let report = self.sync.capability_report();
                if !report.permits_unpreconditioned_write(
                    engine_config.webdav_unsafe_write_consent,
                ) {
                    return Err(BackendError::SyncCapability {
                        capability_detail: format!("{:?}:{}", report.state, report.step),
                    });
                }
            }
        }
        let config = self
            .write_data(ConfigArea::Sync, move |data| {
                data.update_sync_config(SyncConfigUpdate {
                    auto_sync_enabled: Some(enabled),
                    ..SyncConfigUpdate::default()
                })
            })
            .await?;
        self.sync.set_auto_enabled(enabled);
        Ok(SyncRunResult {
            status: SyncStatusDto::Idle,
            config,
            changed_areas: vec![ConfigArea::Sync],
        })
    }

    // ------------------------------------------------------------------
    // Configuration import
    // ------------------------------------------------------------------
    /// Parse one user-selected document from bytes (SAF `content://` URIs are
    /// never treated as paths) and keep the complete batch in a bounded
    /// registry until the user confirms or discards it.
    pub async fn preview_import(
        &self,
        source: ImportSourceDto,
        file_name: String,
        bytes: Vec<u8>,
    ) -> Result<ImportPreviewDto, BackendError> {
        self.ensure_open()?;
        if bytes.len() > MAX_IMPORT_BYTES {
            return Err(BackendError::InvalidArgument {
                detail: format!("the selected file exceeds the {MAX_IMPORT_BYTES} byte import limit"),
            });
        }
        let kind: miaominal_core::profile::ImportSourceKind = source.into();
        let batch = runtime::offload(move || {
            miaominal_storage::config_store::import::import_profiles_from_bytes(
                kind, &file_name, &bytes,
            )
        })
        .await
        .map_err(crate::sync::import_parse_error)?;
        Ok(self.sync.insert_import(batch, kind.label()))
    }

    /// Commit a confirmed preview into the session store and the credential
    /// backend. The preview is consumed even when the commit fails.
    pub async fn commit_import(
        &self,
        preview_id: u64,
    ) -> Result<ImportCommitResult, BackendError> {
        self.ensure_open()?;
        let Some(batch) = self.sync.take_import(preview_id) else {
            return Err(BackendError::InvalidArgument {
                detail: "the import preview is no longer available".to_string(),
            });
        };
        let warnings = batch
            .issues
            .iter()
            .map(|issue| crate::sync_dto::warning_dto(issue.kind, &issue.entry_name, &issue.reason))
            .collect::<Vec<_>>();
        let result = self
            .write_data_all(&[ConfigArea::Hosts, ConfigArea::Credentials], move |data| {
                data.commit_import(batch)
            })
            .await?;
        Ok(ImportCommitResult {
            imported: result.imported_count as u32,
            warnings,
        })
    }

    /// Drop a preview the user cancelled. Returns `false` when it expired.
    pub fn discard_import(&self, preview_id: u64) -> bool {
        self.sync.discard_import(preview_id)
    }

    // ------------------------------------------------------------------
    // Reset
    // ------------------------------------------------------------------

    /// Delete all business files. The host must close running tasks before
    /// calling this and delete its platform credential entries afterwards.
    pub async fn clear_local_data(&self) -> Result<(), BackendError> {
        self.write_data_all(
            &[
                ConfigArea::Hosts,
                ConfigArea::Proxies,
                ConfigArea::Snippets,
                ConfigArea::Settings,
                ConfigArea::Forwards,
                ConfigArea::Credentials,
                ConfigArea::Trust,
            ],
            |data| data.clear_local_data(),
        )
        .await
    }

    // ------------------------------------------------------------------
    // SSH sessions
    // ------------------------------------------------------------------

    /// Open one interactive SSH session for a saved host.
    ///
    /// The session starts in `Connecting`; every later change is visible
    /// through [`Backend::ssh_session_snapshots`] or the event stream. A locked
    /// vault returns `VaultLocked` without starting any connection.
    pub async fn open_ssh_session(
        &self,
        host_id: String,
        columns: u32,
        lines: u32,
    ) -> Result<Arc<SshSession>, BackendError> {
        self.ensure_open()?;
        let data = self.data_handle()?;
        let registry = Arc::clone(&self.sessions);
        let host = Arc::clone(&self.host);
        let close_rx = self.close_rx.clone();
        let session = runtime::run_task(async move {
            crate::session::start_session(host, data, registry, close_rx, host_id, columns, lines)
                .await
        })
        .await??;
        Ok(Arc::new(SshSession::new(session)))
    }

    /// Current state of every session owned by this backend handle, including
    /// sessions that already closed (so a rebuilt page can show them as ended).
    pub fn ssh_session_snapshots(&self) -> Vec<SshSessionSnapshot> {
        self.sessions.snapshots()
    }

    /// Remove one session from the registry and wait for its task to finish.
    /// Idempotent: an unknown id returns `false`.
    pub async fn close_ssh_session(&self, connection_id: u64) -> Result<bool, BackendError> {
        let Some(session) = self.sessions.get(connection_id) else {
            return Ok(false);
        };
        session.request_close();
        let inner = Arc::clone(&session);
        runtime::run_task(async move {
            inner.wait_for_task().await;
            Ok::<(), BackendError>(())
        })
        .await??;
        self.sessions.remove(connection_id);
        Ok(true)
    }

    // ------------------------------------------------------------------
    // AI CLI channel sessions
    // ------------------------------------------------------------------

    /// Open one dedicated SSH channel session for an AI CLI.
    ///
    /// The session carries `exec` channels (stdin/stdout/stderr/exit status
    /// stay separate) and temporary loopback tunnels to a remote loopback port
    /// (for HTTP/SSE CLIs such as `opencode serve`). Events are pulled through
    /// [`AgentSshSession::read_events`]; a locked vault returns `VaultLocked`
    /// without starting any connection.
    pub async fn open_agent_session(
        &self,
        host_id: String,
    ) -> Result<Arc<AgentSshSession>, BackendError> {
        self.ensure_open()?;
        let data = self.data_handle()?;
        let registry = Arc::clone(&self.agent_sessions);
        let session = runtime::run_task(async move {
            crate::agent::start_agent_session(data, registry, host_id).await
        })
        .await??;
        Ok(Arc::new(AgentSshSession::new(session)))
    }

    /// Close one agent session and wait for its SSH worker to finish.
    /// Idempotent: an unknown id returns `false`.
    pub async fn close_agent_session(&self, connection_id: u64) -> Result<bool, BackendError> {
        let Some(session) = self.agent_sessions.get(connection_id) else {
            return Ok(false);
        };
        session.request_close();
        let inner = Arc::clone(&session);
        runtime::run_task(async move {
            inner.wait_for_thread().await;
            Ok::<(), BackendError>(())
        })
        .await??;
        self.agent_sessions.remove(connection_id);
        Ok(true)
    }

    // ------------------------------------------------------------------
    // SFTP sessions and local file descriptors
    // ------------------------------------------------------------------

    /// Open one interactive SFTP session for a saved host.
    ///
    /// The session starts in `Connecting`; host trust and keyboard-interactive
    /// challenges arrive as `BackendEvent::SftpRequest` values carrying request
    /// ids, and every later change is visible through
    /// [`Backend::sftp_session_snapshots`]. A locked vault returns
    /// `VaultLocked` without starting any connection.
    pub async fn open_sftp_session(
        &self,
        host_id: String,
    ) -> Result<Arc<SftpSession>, BackendError> {
        self.ensure_open()?;
        let data = self.data_handle()?;
        let registry = Arc::clone(&self.sftp_sessions);
        let host = Arc::clone(&self.host);
        let close_rx = self.close_rx.clone();
        let session = runtime::run_task(async move {
            crate::sftp::start_sftp_session(host, data, registry, close_rx, host_id).await
        })
        .await??;
        Ok(Arc::new(SftpSession::new(session)))
    }

    /// Current state of every SFTP session owned by this backend handle,
    /// including sessions that already closed.
    pub fn sftp_session_snapshots(&self) -> Vec<SftpSessionSnapshot> {
        self.sftp_sessions.snapshots()
    }

    /// Remove one SFTP session from the registry and wait for its worker.
    /// Idempotent: an unknown id returns `false`.
    pub async fn close_sftp_session(&self, connection_id: u64) -> Result<bool, BackendError> {
        let Some(session) = self.sftp_sessions.get(connection_id) else {
            return Ok(false);
        };
        session.request_close();
        let inner = Arc::clone(&session);
        runtime::run_task(async move {
            inner.wait_for_task().await;
            Ok::<(), BackendError>(())
        })
        .await??;
        self.sftp_sessions.remove(connection_id);
        Ok(true)
    }

    /// Register a Kotlin-owned file descriptor for the transfer engine.
    ///
    /// Rust duplicates the descriptor before returning and owns the duplicate;
    /// Kotlin must close its original descriptor afterwards. The returned
    /// handle is idempotently closable and is also released when the owning
    /// transfer finishes, fails, or is cancelled.
    pub fn register_local_file(
        &self,
        fd: i32,
        name: String,
    ) -> Result<Arc<LocalFileHandle>, BackendError> {
        self.ensure_open()?;
        if name.trim().is_empty() {
            return Err(BackendError::InvalidArgument {
                detail: "the local file needs a display name".to_string(),
            });
        }
        let file = crate::local_file::open_owned_fd(fd).map_err(|error| match error.kind() {
            std::io::ErrorKind::Unsupported => BackendError::Unsupported {
                capability: "file descriptor registration is unavailable on this platform"
                    .to_string(),
            },
            std::io::ErrorKind::PermissionDenied => BackendError::PermissionDenied {
                permission_detail: format!("failed to duplicate the file descriptor: {error}"),
            },
            _ => BackendError::Storage {
                storage_failure: format!("failed to duplicate the file descriptor: {error}"),
            },
        })?;
        LocalFileHandle::from_file(file, name)
    }

    // ------------------------------------------------------------------
    // Port forwarding sessions
    // ------------------------------------------------------------------

    /// Start one dedicated forwarding session for a saved rule.
    ///
    /// Starting a rule that is already active returns the running session
    /// (idempotent), so a rebuilt page reconnects to the same connection. The
    /// stored `enabled` flag is never used to start anything automatically: a
    /// process restart leaves every rule stopped until it is asked for.
    ///
    /// Starts of the same rule are serialized by a per-rule lock: the
    /// active-check and the registration are one atomic step, so two fast taps
    /// cannot create two SSH connections for one rule.
    pub async fn start_forward_session(
        &self,
        profile_id: String,
        rule_id: String,
    ) -> Result<Arc<ForwardSession>, BackendError> {
        self.ensure_open()?;
        let data = self.data_handle()?;
        let registry = Arc::clone(&self.forward_sessions);
        let close_rx = self.close_rx.clone();
        let session = runtime::run_task(async move {
            let key = crate::forward::ForwardKey::new(profile_id.clone(), rule_id.clone());
            let start_lock = registry.start_lock(&key);
            let _start_guard = start_lock.lock().await;
            if let Some(existing) = registry.active(&key) {
                return Ok(existing);
            }
            crate::forward::start_forward_session(data, registry, close_rx, profile_id, rule_id)
                .await
        })
        .await??;
        Ok(Arc::new(ForwardSession::new(session)))
    }

    /// Live handle for one forwarding rule, or `None` when it was never
    /// started in this backend generation.
    pub fn forward_session(&self, profile_id: String, rule_id: String) -> Option<Arc<ForwardSession>> {
        self.forward_sessions
            .get(&crate::forward::ForwardKey::new(profile_id, rule_id))
            .map(ForwardSession::new)
            .map(Arc::new)
    }

    /// Runtime state of every forwarding session owned by this handle,
    /// including stopped and failed rules (so a rebuilt page can show them).
    pub fn forward_session_snapshots(&self) -> Vec<ForwardSnapshot> {
        self.forward_sessions.snapshots()
    }

    /// Stop one forwarding session and wait (bounded) for its worker to finish.
    /// Idempotent: an unknown or already stopped rule returns `false`. The stop
    /// shares the per-rule start lock, so it always stops the session that a
    /// concurrent start just created instead of observing an empty registry.
    pub async fn close_forward_session(
        &self,
        profile_id: String,
        rule_id: String,
    ) -> Result<bool, BackendError> {
        let registry = Arc::clone(&self.forward_sessions);
        let key = crate::forward::ForwardKey::new(profile_id, rule_id);
        runtime::run_task(async move {
            let start_lock = registry.start_lock(&key);
            let _start_guard = start_lock.lock().await;
            let Some(session) = registry.get(&key) else {
                return Ok(false);
            };
            session.request_close();
            let inner = Arc::clone(&session);
            inner.wait_for_task().await;
            registry.remove(&key);
            Ok(true)
        })
        .await?
    }
}

impl Backend {
    fn ensure_open(&self) -> Result<(), BackendError> {
        if self.is_closed() {
            return Err(BackendError::Closed);
        }
        Ok(())
    }

    fn lock_lifecycle(&self) -> MutexGuard<'_, LifecycleState> {
        self.lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Callers must hold the lifecycle lock so the cursor belongs to the state.
    fn build_snapshot(&self, state: &LifecycleState) -> BackendSnapshot {
        BackendSnapshot {
            initialized: state.initialized,
            closed: state.closed,
            sequence: self.events.latest_sequence(),
            data_dir: self.config.data_dir.clone(),
            cache_dir: self.config.cache_dir.clone(),
        }
    }

    fn set_data(&self, data: Arc<MobileData>) {
        *self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(data);
    }

    fn data_handle(&self) -> Result<Arc<MobileData>, BackendError> {
        if self.is_closed() {
            return Err(BackendError::Closed);
        }
        self.data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or_else(|| BackendError::RuntimeUnavailable {
                runtime_failure: "the backend is not initialized".to_string(),
            })
    }

    async fn read_data<T, F>(&self, operation: F) -> Result<T, BackendError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<MobileData>) -> Result<T, BackendError> + Send + 'static,
    {
        let data = self.data_handle()?;
        runtime::offload(move || {
            let _operation = OperationGuard::new(Arc::clone(&data))?;
            // Read under the same gate as sync apply so a snapshot never mixes
            // pre- and post-apply content.
            let _sync_guard = miaominal_secrets::lock_sync_data();
            operation(data)
        })
        .await
    }

    async fn write_data<T, F>(&self, area: ConfigArea, operation: F) -> Result<T, BackendError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<MobileData>) -> Result<T, BackendError> + Send + 'static,
    {
        self.write_data_all(&[area], operation).await
    }

    /// Run one serialized data mutation and publish `ConfigChanged` for every
    /// affected area **while holding the lifecycle lock**, matching the P1
    /// invariant that snapshots and cursors are read under the same lock.
    ///
    /// The mutation runs under the process-wide sync data gate, the same lock
    /// the sync engine holds across `apply_plaintext_payload` and
    /// `local_revision`. A local edit therefore either happens completely
    /// before a pull (the pull then reports `LocalChangedDuringPull`) or
    /// completely after it, and can never be overwritten by stale remote data.
    async fn write_data_all<T, F>(&self, areas: &[ConfigArea], operation: F) -> Result<T, BackendError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<MobileData>) -> Result<T, BackendError> + Send + 'static,
    {
        let data = self.data_handle()?;
        let result = runtime::offload(move || {
            let _operation = OperationGuard::new(Arc::clone(&data))?;
            let _sync_guard = miaominal_secrets::lock_sync_data();
            operation(data)
        })
        .await?;
        {
            let state = self.lock_lifecycle();
            if state.closed {
                // The bus drops events after close, so nothing more to do.
                return Err(BackendError::Closed);
            }
            for area in areas {
                self.events.publish(BackendEvent::ConfigChanged { area: *area });
            }
        }
        if areas.iter().copied().any(area_participates_in_sync) {
            // Local content changed: schedule the debounced automatic push.
            self.sync.note_local_change();
        }
        Ok(result)
    }
}

/// Resolve once the backend close signal is set.
async fn wait_for_close(closed: &mut watch::Receiver<bool>) {
    loop {
        if *closed.borrow_and_update() {
            return;
        }
        if closed.changed().await.is_err() {
            return;
        }
    }
}

/// Areas whose content is part of a sync payload: a local write schedules the
/// debounced automatic push. `Sync` and `Trust` are device-local.
fn area_participates_in_sync(area: ConfigArea) -> bool {
    matches!(
        area,
        ConfigArea::Hosts
            | ConfigArea::Proxies
            | ConfigArea::Snippets
            | ConfigArea::Settings
            | ConfigArea::Forwards
            | ConfigArea::Credentials
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventBatch, SequencedEvent};
    use crate::test_support::{
        business_root, cache_root, clear_test_root, shared_test_lock, test_host,
    };
    use std::path::PathBuf;

    fn test_backend() -> Arc<Backend> {
        Backend::new(
            test_host(),
            BackendConfig {
                data_dir: business_root().to_string_lossy().into_owned(),
                cache_dir: cache_root().to_string_lossy().into_owned(),
            },
        )
    }

    #[tokio::test]
    async fn initialize_is_idempotent() {
        let _guard = shared_test_lock();
        let backend = test_backend();

        let first = backend.initialize().await.unwrap();
        let second = backend.initialize().await.unwrap();

        assert!(first.initialized);
        assert!(!first.closed);
        assert_eq!(first.sequence, 1);
        assert_eq!(second.sequence, first.sequence);
        assert_eq!(second.data_dir, first.data_dir);
        assert!(PathBuf::from(&first.data_dir).is_dir());
        backend.shutdown();
    }

    #[tokio::test]
    async fn shutdown_is_idempotent_and_closes_subscriptions() {
        let _guard = shared_test_lock();
        clear_test_root();
        let backend = test_backend();
        backend.initialize().await.unwrap();
        let subscription = backend.subscribe_from(1);

        backend.shutdown();
        backend.shutdown();

        assert!(backend.is_closed());
        let batch = subscription.next_batch(16, 100).await.unwrap();
        assert_eq!(
            batch.events,
            vec![SequencedEvent {
                sequence: 2,
                event: BackendEvent::Shutdown,
            }]
        );
        let closed = subscription.next_batch(16, 100).await;
        assert!(matches!(closed, Err(BackendError::Closed)));
    }

    #[tokio::test]
    async fn a_new_backend_handle_can_restart_on_the_same_root() {
        let _guard = shared_test_lock();
        let first = test_backend();
        first.initialize().await.unwrap();
        first.shutdown();

        let second = test_backend();
        let snapshot = second.initialize().await.unwrap();

        assert!(snapshot.initialized);
        assert_eq!(snapshot.sequence, 1);
        assert!(!snapshot.closed);
        second.shutdown();
    }

    #[tokio::test]
    async fn lifecycle_calls_fail_after_shutdown() {
        let _guard = shared_test_lock();
        let backend = test_backend();
        backend.shutdown();

        let initialized = backend.initialize().await;
        assert!(matches!(initialized, Err(BackendError::Closed)));
        let echo = backend.echo("payload".to_string(), 0).await;
        assert!(matches!(echo, Err(BackendError::Closed)));
        assert!(matches!(
            backend.probe_secret("key".to_string()),
            Err(BackendError::Closed)
        ));
        assert!(backend.snapshot().closed);
    }

    #[tokio::test]
    async fn subscribing_from_zero_receives_ready() {
        let _guard = shared_test_lock();
        clear_test_root();
        let backend = test_backend();
        let subscription = backend.subscribe_from(0);
        backend.initialize().await.unwrap();

        let batch = subscription.next_batch(16, 100).await.unwrap();

        assert_eq!(
            batch,
            EventBatch {
                events: vec![SequencedEvent {
                    sequence: 1,
                    event: BackendEvent::Ready,
                }],
                resync_required: false,
            }
        );
        backend.shutdown();
    }

    #[tokio::test]
    async fn config_writes_publish_events_without_a_snapshot_subscription_gap() {
        let _guard = shared_test_lock();
        crate::test_support::clear_test_root();
        let backend = test_backend();
        let snapshot = backend.initialize().await.unwrap();
        // Snapshot first, subscribe from the snapshot cursor, then write: the
        // write must not fall into the gap between read and subscription.
        let subscription = backend.subscribe_from(snapshot.sequence);

        backend
            .save_host(crate::dto::HostUpdate {
                id: "host-1".to_string(),
                name: Some("Host".to_string()),
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
            })
            .await
            .unwrap();

        let batch = subscription.next_batch(16, 1_000).await.unwrap();
        assert_eq!(
            batch.events,
            vec![SequencedEvent {
                sequence: snapshot.sequence + 1,
                event: BackendEvent::ConfigChanged {
                    area: ConfigArea::Hosts
                },
            }]
        );
        backend.shutdown();
    }

    #[tokio::test]
    async fn config_writes_after_a_snapshot_are_not_redelivered_on_subscribe() {
        let _guard = shared_test_lock();
        crate::test_support::clear_test_root();
        let backend = test_backend();
        backend.initialize().await.unwrap();
        backend
            .save_host(crate::dto::HostUpdate {
                id: "host-1".to_string(),
                name: Some("Host".to_string()),
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
            })
            .await
            .unwrap();

        let snapshot = backend.snapshot();
        let subscription = backend.subscribe_from(snapshot.sequence);

        let batch = subscription.next_batch(16, 50).await.unwrap();
        assert!(batch.events.is_empty(), "no event may be redelivered");
        backend.shutdown();
    }

    #[tokio::test]
    async fn shutdown_drains_in_flight_data_writes() {
        let _guard = shared_test_lock();
        crate::test_support::clear_test_root();
        let backend = test_backend();
        backend.initialize().await.unwrap();

        let writer = Arc::clone(&backend);
        let task = crate::runtime::runtime().spawn(async move {
            writer
                .write_data_all(&[ConfigArea::Hosts], |data| {
                    // Simulate a slow storage write (for example key material
                    // generation) that must not outlive the shutdown.
                    std::thread::sleep(Duration::from_millis(300));
                    data.save_host(crate::dto::HostUpdate {
                        id: "host-slow".to_string(),
                        name: Some("Slow".to_string()),
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
                    })
                })
                .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;

        let started = std::time::Instant::now();
        backend.shutdown();
        let drained = started.elapsed();

        assert!(
            drained >= Duration::from_millis(200),
            "shutdown must wait for the in-flight write, waited {drained:?}"
        );
        // The write completed before shutdown returned; the caller may have
        // observed `Closed` because the event publish was rejected after the
        // bus closed, which is fine as long as the bytes are on disk.
        let _ = task.await.unwrap();

        let second = test_backend();
        second.initialize().await.unwrap();
        assert!(
            second
                .hosts_snapshot()
                .await
                .unwrap()
                .iter()
                .any(|host| host.id == "host-slow"),
            "the drained write must be visible before the next generation clears"
        );
        second.clear_local_data().await.unwrap();
        assert!(second.hosts_snapshot().await.unwrap().is_empty());
        second.shutdown();
    }

    #[tokio::test]
    async fn retired_data_layer_rejects_new_operations() {
        let _guard = shared_test_lock();
        crate::test_support::clear_test_root();
        let backend = test_backend();
        backend.initialize().await.unwrap();
        backend.shutdown();

        let data = backend
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .expect("initialized backend should own data");
        let error = match OperationGuard::new(data) {
            Err(error) => error,
            Ok(_guard) => panic!("a retired data layer must reject new operations"),
        };
        assert!(matches!(error, BackendError::Closed));
    }

    #[tokio::test]
    async fn a_settings_update_never_restores_values_a_pull_just_replaced() {
        let _guard = shared_test_lock();
        clear_test_root();
        let backend = test_backend();
        backend.initialize().await.unwrap();

        // Simulate the sync engine applying a pulled payload through its own
        // SettingsStore clone: the live data layer cache still holds the
        // pre-pull value until it reads through.
        let settings_path =
            miaominal_paths::config_file("settings.toml").expect("settings path should resolve");
        let mut writer = miaominal_storage::SettingsStore::load_with_path(settings_path.clone())
            .expect("settings writer should load");
        writer.update(|settings| settings.terminal_font_size = 21.5);

        let snapshot = backend
            .settings_snapshot()
            .await
            .expect("the snapshot must read through to disk");
        assert_eq!(
            snapshot.terminal_font_size, 21.5,
            "the bridge must observe settings written by the sync engine"
        );

        // A local edit of a *different* field must be based on the pulled
        // state, not on a stale cache.
        backend
            .update_settings(SettingsUpdate {
                language: None,
                recent_host_count: Some(7),
                auto_monitoring: None,
                monitor_history: None,
                close_last_session_behavior: None,
                vault_auto_lock_minutes: None,
                onboarding_completed: None,
                terminal_font: None,
                cjk_fallback_font: None,
                terminal_font_size: None,
                terminal_line_height: None,
                theme_id: None,
                seed_color: None,
            })
            .await
            .unwrap();

        let reloaded = miaominal_storage::SettingsStore::load_with_path(settings_path)
            .expect("settings should reload");
        assert_eq!(
            reloaded.settings().terminal_font_size, 21.5,
            "a local update must not restore the stale pre-pull value"
        );
        assert_eq!(reloaded.settings().recent_connections_count, 7);
        backend.shutdown();
    }

    #[tokio::test]
    async fn relative_directories_are_rejected() {
        let _guard = shared_test_lock();
        let backend = Backend::new(
            test_host(),
            BackendConfig {
                data_dir: "relative-data".to_string(),
                cache_dir: "relative-cache".to_string(),
            },
        );

        let error = backend
            .initialize()
            .await
            .expect_err("relative directories must be rejected");

        assert!(matches!(error, BackendError::RuntimeUnavailable { .. }));
        assert!(!backend.snapshot().initialized);
        backend.shutdown();
    }

    #[tokio::test]
    async fn shutdown_cancels_running_tasks() {
        let _guard = shared_test_lock();
        let backend = test_backend();
        backend.initialize().await.unwrap();
        let worker = Arc::clone(&backend);
        let echo = crate::runtime::runtime()
            .spawn(async move { worker.echo("payload".to_string(), 30_000).await });
        tokio::time::sleep(Duration::from_millis(50)).await;

        backend.shutdown();

        let result = tokio::time::timeout(Duration::from_secs(2), echo)
            .await
            .expect("shutdown must cancel the running task")
            .expect("task should not panic");
        assert!(matches!(result, Err(BackendError::Closed)));
    }

    #[tokio::test]
    async fn concurrent_shutdown_and_initialize_stay_ordered() {
        let _guard = shared_test_lock();
        for _ in 0..25 {
            let backend = test_backend();
            let subscription = backend.subscribe_from(0);

            let (initialized, ()) = tokio::join!(
                backend.initialize(),
                async { backend.shutdown() },
            );

            let batch = subscription.next_batch(16, 50).await.unwrap();
            let sequences = batch
                .events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>();
            assert!(sequences.windows(2).all(|window| window[0] < window[1]));
            if let Some(position) = batch
                .events
                .iter()
                .position(|event| event.event == BackendEvent::Shutdown)
            {
                assert_eq!(
                    position,
                    batch.events.len() - 1,
                    "no event may follow Shutdown: {:?}",
                    batch.events
                );
            }
            if let Ok(snapshot) = initialized {
                assert!(snapshot.initialized);
                assert!(
                    batch
                        .events
                        .iter()
                        .any(|event| event.event == BackendEvent::Ready),
                    "a successful initialize must publish Ready"
                );
            }
        }
    }

    #[tokio::test]
    async fn snapshots_never_combine_uninitialized_state_with_a_cursor() {
        let _guard = shared_test_lock();
        for _ in 0..10 {
            let backend = test_backend();
            let observer = Arc::clone(&backend);
            let watcher = crate::runtime::runtime().spawn(async move {
                for _ in 0..500 {
                    let snapshot = observer.snapshot();
                    assert!(
                        !snapshot.initialized || snapshot.sequence >= 1,
                        "initialized snapshot without a cursor: {snapshot:?}"
                    );
                    assert!(
                        snapshot.sequence == 0 || snapshot.initialized,
                        "cursor without initialized state: {snapshot:?}"
                    );
                }
            });
            backend.initialize().await.unwrap();
            watcher.await.unwrap();
            backend.shutdown();
        }
    }
}
