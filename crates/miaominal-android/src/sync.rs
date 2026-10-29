//! Sync coordination for the mobile backend.
//!
//! The shared [`miaominal_sync`] engine owns protocol behaviour; this module
//! owns *mobile* concerns only:
//!
//! - one execution gate shared by manual actions, the debounced local-change
//!   sync, the foreground check, and the WorkManager entry point;
//! - explicit cancellation that does not rely on dropping a Kotlin coroutine;
//! - a bounded, pull-based runtime snapshot (progress is never queued);
//! - reliable control events published under the backend lifecycle lock;
//! - a bounded registry of parsed import previews awaiting user confirmation.
//!
//! Secrets never enter this module's state: snapshots carry presence flags and
//! sanitized failure text only.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use miaominal_core::profile::ImportedBatch;
use miaominal_services::{SyncService, SyncTaskResult};
use miaominal_storage::SettingsStore;
use miaominal_sync::capability::{
    CapabilityError, CapabilityReport, CapabilityState, ProbeCancellation,
};
use miaominal_sync::engine::SyncEngine;
use miaominal_sync::{
    RemoteSyncState, SyncContentRelation, SyncStatus, classify_content_revisions,
};
use tokio::sync::{Notify, watch};

use crate::backend::LifecycleState;
use crate::data::MobileData;
use crate::error::BackendError;
use crate::events::{BackendEvent, ConfigArea, EventBus};
use crate::runtime;
use crate::sync_dto::{
    ImportPreviewDto, SyncCapabilityDto, SyncConfigDto, SyncConflictDto, SyncFailureDto,
    SyncFailureKindDto, SyncInterventionReasonDto, SyncPhaseDto, SyncRemoteStateDto,
    SyncResolution, SyncRunResult, SyncSnapshotDto, SyncStatusDto, entry_preview, status_dto,
    warning_dto,
};

/// Automatic synchronization debounce after the last local configuration write.
pub(crate) const AUTO_SYNC_DEBOUNCE: Duration = Duration::from_secs(5);
/// Minimum spacing between automatic checks triggered by foreground/WorkManager.
pub(crate) const AUTO_CHECK_COOLDOWN: Duration = Duration::from_secs(15);
/// Retained parsed import previews; each batch is one user-selected file.
const MAX_PENDING_IMPORTS: usize = 4;
/// Upper bound for one imported document, so a provider cannot exhaust memory.
pub(crate) const MAX_IMPORT_BYTES: usize = 8 * 1024 * 1024;

/// Everything one sync operation needs, cloned out of `MobileData` while its
/// mutex is held. The stores are path handles, so later replaces of the data
/// generation still target the same files.
pub(crate) struct SyncHandles {
    pub(crate) service: SyncService,
    pub(crate) engine: SyncEngine,
    pub(crate) settings_store: SettingsStore,
    pub(crate) secrets: miaominal_secrets::SecretStore,
    pub(crate) vault_locked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoTrigger {
    /// Local configuration changed; wait for the debounce and push.
    Debounced,
    /// The app returned to the foreground; run the normal remote check.
    Foreground,
    /// WorkManager periodic check.
    Worker,
}

#[derive(Debug)]
struct SyncRuntimeState {
    phase: SyncPhaseDto,
    status: SyncStatusDto,
    capability: CapabilityReport,
    pending_conflict: Option<SyncConflictDto>,
    last_failure: Option<SyncFailureDto>,
    last_auto_check_at: u64,
}

struct ImportRegistry {
    next_id: u64,
    entries: VecDeque<(u64, ImportedBatch)>,
}

pub(crate) struct SyncCoordinator {
    lifecycle: Arc<Mutex<LifecycleState>>,
    events: Arc<EventBus>,
    state: Mutex<SyncRuntimeState>,
    /// Serializes every sync operation (manual, debounced, foreground, worker).
    op_gate: tokio::sync::Mutex<()>,
    /// Cancellation generations: a new operation takes the next generation and
    /// observes only its own; `cancel()` invalidates the running one.
    cancel_tx: watch::Sender<u64>,
    cancel_rx: watch::Receiver<u64>,
    probe_cancel: ProbeCancellation,
    debounce_wake: Arc<Notify>,
    /// Set once the backend owns a data layer; the debounce watcher resolves it
    /// lazily so it survives a restart that replaces the data generation.
    data: Mutex<Option<Arc<MobileData>>>,
    imports: Mutex<ImportRegistry>,
}

impl SyncCoordinator {
    pub(crate) fn new(
        lifecycle: Arc<Mutex<LifecycleState>>,
        events: Arc<EventBus>,
        mut close_rx: watch::Receiver<bool>,
    ) -> Arc<Self> {
        let (cancel_tx, cancel_rx) = watch::channel(0u64);
        let debounce_wake = Arc::new(Notify::new());
        let coordinator = Arc::new(Self {
            lifecycle,
            events,
            state: Mutex::new(SyncRuntimeState {
                phase: SyncPhaseDto::Disabled,
                status: SyncStatusDto::Idle,
                capability: CapabilityReport::default(),
                pending_conflict: None,
                last_failure: None,
                last_auto_check_at: 0,
            }),
            op_gate: tokio::sync::Mutex::new(()),
            cancel_tx,
            cancel_rx,
            probe_cancel: ProbeCancellation::default(),
            debounce_wake: Arc::clone(&debounce_wake),
            data: Mutex::new(None),
            imports: Mutex::new(ImportRegistry {
                next_id: 1,
                entries: VecDeque::new(),
            }),
        });
        // One bounded watcher task owns the debounce; every publish goes through
        // the lifecycle lock exactly like the other control events.
        let watcher = Arc::clone(&coordinator);
        runtime::runtime().spawn(async move {
            loop {
                tokio::select! {
                    () = watcher.debounce_wake.notified() => {}
                    result = close_rx.changed() => {
                        let _ = result;
                        return;
                    }
                }
                // Coalesce a burst of local writes: restart the timer on every
                // new notification, then run one automatic pass.
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(AUTO_SYNC_DEBOUNCE) => break,
                        () = watcher.debounce_wake.notified() => {}
                        result = close_rx.changed() => {
                            let _ = result;
                            return;
                        }
                    }
                }
                if *close_rx.borrow() {
                    return;
                }
                let Some(data) = watcher.data_handle() else {
                    continue;
                };
                let handles = match runtime::offload(move || data.sync_handles()).await {
                    Ok(handles) => handles,
                    Err(error) => {
                        log::warn!("automatic sync skipped: {error}");
                        continue;
                    }
                };
                let _ = watcher
                    .run_auto_check(handles, AutoTrigger::Debounced)
                    .await;
            }
        });
        coordinator
    }

    pub(crate) fn attach_data(&self, data: Arc<MobileData>) {
        *self
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(data);
    }

    fn data_handle(&self) -> Option<Arc<MobileData>> {
        self.data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn lock_state(&self) -> MutexGuard<'_, SyncRuntimeState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Publish one reliable control event under the lifecycle lock.
    pub(crate) fn publish(&self, event: BackendEvent) {
        let state = self
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.closed {
            return;
        }
        self.events.publish(event);
    }

    /// Mark local configuration as changed so a debounced sync will follow.
    pub(crate) fn note_local_change(&self) {
        self.debounce_wake.notify_one();
    }

    pub(crate) fn cancel(&self) {
        self.probe_cancel.cancel();
        self.set_phase(SyncPhaseDto::Cancelling);
        self.cancel_tx.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
    }

    /// Read the last capability verdict (used by the auto-sync enable gate).
    pub(crate) fn capability_report(&self) -> CapabilityReport {
        self.capability()
    }

    /// Reflect an explicit auto-sync toggle without waiting for the next pass.
    pub(crate) fn set_auto_enabled(&self, enabled: bool) {
        let phase = self.lock_state().phase;
        if enabled {
            if matches!(
                phase,
                SyncPhaseDto::Disabled
                    | SyncPhaseDto::PausedVaultLocked
                    | SyncPhaseDto::PausedCredentials
            ) {
                self.set_phase(SyncPhaseDto::Idle);
            }
        } else if phase != SyncPhaseDto::Conflict {
            self.set_phase(SyncPhaseDto::Disabled);
        }
    }

    pub(crate) fn snapshot(&self, config: &SyncConfigDto, vault_locked: bool) -> SyncSnapshotDto {
        let state = self.lock_state();
        SyncSnapshotDto {
            phase: state.phase,
            status: state.status.clone(),
            config: config.clone(),
            capability: SyncCapabilityDto::from(&state.capability),
            vault_locked,
            pending_conflict: state.pending_conflict.clone(),
            last_failure: state.last_failure.clone(),
            last_auto_check_at_ms: state.last_auto_check_at,
        }
    }

    fn set_phase(&self, phase: SyncPhaseDto) {
        let changed = {
            let mut state = self.lock_state();
            if state.phase == phase {
                false
            } else {
                state.phase = phase;
                true
            }
        };
        if changed {
            self.publish(BackendEvent::SyncPhaseChanged { phase });
        }
    }

    fn set_status(&self, status: SyncStatusDto) {
        self.lock_state().status = status;
    }

    fn set_failure(&self, failure: Option<SyncFailureDto>) {
        self.lock_state().last_failure = failure;
    }

    /// A conflict is reliable state: it stays in the snapshot until resolved.
    fn set_conflict(&self, conflict: Option<SyncConflictDto>) -> bool {
        let changed = {
            let mut state = self.lock_state();
            if state.pending_conflict == conflict {
                false
            } else {
                state.pending_conflict = conflict.clone();
                true
            }
        };
        if let Some(conflict) = conflict
            && changed
        {
            self.publish(BackendEvent::SyncConflictDetected {
                remote_at_ms: conflict.remote_at_ms,
                reason: conflict.reason,
            });
        }
        changed
    }

    pub(crate) fn set_vault_locked(&self, locked: bool) {
        if locked {
            self.set_phase(SyncPhaseDto::PausedVaultLocked);
            self.publish(BackendEvent::SyncAuthenticationRequired {
                detail: "vault_locked".to_string(),
            });
        } else if self.lock_state().phase == SyncPhaseDto::PausedVaultLocked {
            self.set_phase(SyncPhaseDto::Idle);
        }
    }

    /// Called with `op_gate` held. Starting a new cancellation generation must
    /// commit under the same lifecycle lock as shutdown, including for work
    /// whose handles were captured before it waited for the execution gate.
    fn begin_operation(&self, handles: &SyncHandles) -> Result<u64, BackendError> {
        let state = self
            .lifecycle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.closed {
            return Err(BackendError::Closed);
        }
        // All entry points, including conflict resolution and local revision
        // checks, must use the credential backend belonging to these handles.
        handles.service.replace_secrets(handles.secrets.clone());
        self.cancel_tx.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
        Ok(*self.cancel_rx.borrow())
    }

    /// Await `future`, but report `Cancelled` as soon as the generation moves.
    async fn run_cancellable<T, F>(&self, generation: u64, future: F) -> Result<T, BackendError>
    where
        F: std::future::Future<Output = Result<T, BackendError>>,
    {
        let mut cancel_rx = self.cancel_rx.clone();
        tokio::select! {
            biased;
            _ = async move {
                loop {
                    if *cancel_rx.borrow_and_update() != generation {
                        break;
                    }
                    if cancel_rx.changed().await.is_err() {
                        break;
                    }
                }
            } => Err(BackendError::Cancelled),
            result = future => result,
        }
    }

    // ------------------------------------------------------------------
    // Operations
    // ------------------------------------------------------------------

    pub(crate) async fn run_capability_check(
        &self,
        handles: SyncHandles,
    ) -> Result<SyncRunResult, BackendError> {
        if handles.vault_locked {
            return Err(BackendError::VaultLocked);
        }
        let _gate = self.op_gate.lock().await;
        let generation = self.begin_operation(&handles)?;
        self.set_phase(SyncPhaseDto::CheckingCapability);
        let engine = self.engine_for(&handles);
        let cancel = self.probe_cancel.fresh();
        let report = match self
            .run_cancellable(generation, async {
                Ok::<_, BackendError>(handles.service.check_capability(engine, cancel).await)
            })
            .await
        {
            Ok(report) => report,
            Err(error) => return Err(self.record_operation_error(error)),
        };
        self.store_capability(&report);
        let status = if report.state == CapabilityState::Supported {
            SyncStatusDto::Idle
        } else {
            SyncStatusDto::Failed {
                failure: SyncFailureDto {
                    kind: SyncFailureKindDto::Capability,
                    detail: format!("{:?}:{}", report.state, report.step),
                },
            }
        };
        self.finish(&handles, status, &report, &[ConfigArea::Sync]);
        Ok(self.into_run_result(&handles))
    }

    pub(crate) async fn run_pull(
        &self,
        handles: SyncHandles,
        force: bool,
    ) -> Result<SyncRunResult, BackendError> {
        if handles.vault_locked {
            return Err(BackendError::VaultLocked);
        }
        let _gate = self.op_gate.lock().await;
        let generation = self.begin_operation(&handles)?;
        self.set_phase(SyncPhaseDto::Pulling);
        let result = self.pull_inner(&handles, generation, force, None).await;
        self.apply_task_result(&handles, result).await
    }

    pub(crate) async fn run_push(
        &self,
        handles: SyncHandles,
        manual: bool,
    ) -> Result<SyncRunResult, BackendError> {
        if handles.vault_locked {
            return Err(BackendError::VaultLocked);
        }
        let _gate = self.op_gate.lock().await;
        let generation = self.begin_operation(&handles)?;
        self.set_phase(SyncPhaseDto::Pushing);
        let result = self.push_inner(&handles, generation, manual).await;
        self.apply_task_result(&handles, result).await
    }

    pub(crate) async fn run_remote_state(
        &self,
        handles: SyncHandles,
    ) -> Result<SyncRemoteStateDto, BackendError> {
        if handles.vault_locked {
            return Err(BackendError::VaultLocked);
        }
        let _gate = self.op_gate.lock().await;
        let generation = self.begin_operation(&handles)?;
        self.set_phase(SyncPhaseDto::CheckingRemote);
        let engine = self.engine_for(&handles);
        let remote = match self
            .run_cancellable(generation, async {
                handles
                    .service
                    .remote_state(engine)
                    .await
                    .map_err(sync_error)
            })
            .await
        {
            Ok(remote) => remote,
            Err(error) => return Err(self.record_operation_error(error)),
        };
        let capability = self.capability();
        self.finish(&handles, SyncStatusDto::Idle, &capability, &[]);
        Ok(remote_state_dto(&remote))
    }

    /// Resolve a pending conflict explicitly: local wins (force push) or remote
    /// wins (force pull). Never called automatically.
    pub(crate) async fn run_conflict_resolution(
        &self,
        handles: SyncHandles,
        resolution: SyncResolution,
    ) -> Result<SyncRunResult, BackendError> {
        if handles.vault_locked {
            return Err(BackendError::VaultLocked);
        }
        let _gate = self.op_gate.lock().await;
        let generation = self.begin_operation(&handles)?;
        let result = match resolution {
            SyncResolution::KeepLocal => {
                self.set_phase(SyncPhaseDto::Pushing);
                let engine = self.engine_for(&handles);
                let settings = handles.settings_store.clone();
                self.run_cancellable(generation, async {
                    handles
                        .service
                        .push_force(engine, settings)
                        .await
                        .map_err(sync_error)
                })
                .await
            }
            SyncResolution::KeepRemote => {
                self.set_phase(SyncPhaseDto::Pulling);
                let engine = self.engine_for(&handles);
                let settings = handles.settings_store.clone();
                self.run_cancellable(generation, async {
                    handles
                        .service
                        .pull_force(engine, settings)
                        .await
                        .map_err(sync_error)
                })
                .await
            }
        };
        self.apply_task_result(&handles, result).await
    }

    /// Entry point for the foreground check, the WorkManager periodic job, and
    /// the debounced local-change sync.
    ///
    /// Every failure exit runs through [`SyncCoordinator::record_operation_error`]
    /// so a network error or a cancellation can never leave the phase in a
    /// running state (which would keep the manual buttons disabled).
    pub(crate) async fn run_auto_check(
        &self,
        handles: SyncHandles,
        trigger: AutoTrigger,
    ) -> Result<SyncRunResult, BackendError> {
        let _gate = self.op_gate.lock().await;
        let generation = self.begin_operation(&handles)?;
        let result = self.auto_check_inner(&handles, generation, trigger).await;
        result.map_err(|error| self.record_operation_error(error))
    }

    async fn auto_check_inner(
        &self,
        handles: &SyncHandles,
        generation: u64,
        trigger: AutoTrigger,
    ) -> Result<SyncRunResult, BackendError> {
        let config = handles.engine.config_store.config.clone();
        if !config.auto_sync_enabled {
            self.set_phase(SyncPhaseDto::Disabled);
            self.set_status(SyncStatusDto::Idle);
            return Ok(self.into_run_result(handles));
        }
        if handles.vault_locked {
            self.publish(BackendEvent::SyncAuthenticationRequired {
                detail: "vault_locked".to_string(),
            });
            self.finish_paused(
                SyncPhaseDto::PausedVaultLocked,
                SyncFailureKindDto::Locked,
                "vault_locked",
            );
            return Ok(self.into_run_result(handles));
        }
        if !has_provider_credentials(&config) {
            self.publish(BackendEvent::SyncAuthenticationRequired {
                detail: "sync_credentials_missing".to_string(),
            });
            self.finish_paused(
                SyncPhaseDto::PausedCredentials,
                SyncFailureKindDto::NotConfigured,
                "sync_credentials_missing",
            );
            return Ok(self.into_run_result(handles));
        }
        if trigger == AutoTrigger::Debounced {
            self.set_phase(SyncPhaseDto::Pushing);
            let result = self.push_inner(handles, generation, false).await;
            return self.apply_task_result(handles, result).await;
        }
        // Foreground and WorkManager run the same check; the operation gate
        // already prevents overlap with a manual action. A short cooldown keeps
        // the two callers from repeating the same network round trip when both
        // fire within seconds. The state guard is released before building the
        // result: the mutex is not reentrant.
        let now = unix_millis();
        let cooling_down = {
            let state = self.lock_state();
            state.last_auto_check_at != 0
                && now.saturating_sub(state.last_auto_check_at)
                    < AUTO_CHECK_COOLDOWN.as_millis() as u64
        };
        if cooling_down {
            return Ok(self.into_run_result(handles));
        }
        self.lock_state().last_auto_check_at = now;
        if let Some(result) = self.ensure_capability(handles, generation, false).await? {
            return Ok(result);
        }
        self.set_phase(SyncPhaseDto::CheckingRemote);
        let engine = self.engine_for(handles);
        let remote = self
            .run_cancellable(generation, async {
                handles
                    .service
                    .remote_state(engine)
                    .await
                    .map_err(sync_error)
            })
            .await?;
        let baseline = handles
            .engine
            .config_store
            .config
            .last_synced_local_revision
            .clone();
        match remote {
            RemoteSyncState::Disabled => {
                self.set_phase(SyncPhaseDto::Disabled);
                self.set_status(SyncStatusDto::Idle);
            }
            RemoteSyncState::BindingRequired(provider) => {
                self.set_status(SyncStatusDto::RemoteBindingRequired {
                    provider: provider.into(),
                });
            }
            RemoteSyncState::Missing | RemoteSyncState::NotModified | RemoteSyncState::UpToDate => {
                // The remote did not move, but local edits may still be
                // waiting: a debounced push that failed while offline leaves
                // the local revision ahead of the recorded baseline. Without
                // this check a 304 would skip the upload forever.
                let local_revision = self.local_revision(handles, generation).await?;
                if baseline.as_deref() != Some(local_revision.as_str()) {
                    self.set_phase(SyncPhaseDto::Pushing);
                    let result = self.push_inner(handles, generation, false).await;
                    return self.apply_task_result(handles, result).await;
                }
                self.set_status(SyncStatusDto::UpToDate {
                    at_ms: handles
                        .engine
                        .config_store
                        .config
                        .last_sync_at
                        .saturating_mul(1_000),
                });
            }
            RemoteSyncState::Updated {
                synced_at,
                content_revision,
                ..
            } => {
                let local_revision = self.local_revision(handles, generation).await?;
                let (Some(remote_revision), Some(baseline)) = (content_revision, baseline.clone())
                else {
                    self.record_conflict_ms(
                        Some(synced_at.saturating_mul(1_000)),
                        SyncInterventionReasonDto::MissingSyncBaseline,
                    );
                    self.set_status(SyncStatusDto::PullRequired {
                        remote_at_ms: Some(synced_at.saturating_mul(1_000)),
                        reason: SyncInterventionReasonDto::MissingSyncBaseline,
                    });
                    self.set_phase(SyncPhaseDto::Conflict);
                    self.publish(BackendEvent::ConfigChanged {
                        area: ConfigArea::Sync,
                    });
                    return Ok(self.into_run_result(handles));
                };
                match classify_content_revisions(&local_revision, &remote_revision, Some(&baseline))
                {
                    SyncContentRelation::Identical => {
                        self.set_status(SyncStatusDto::UpToDate {
                            at_ms: synced_at.saturating_mul(1_000),
                        });
                    }
                    SyncContentRelation::LocalChanged => {
                        self.set_phase(SyncPhaseDto::Pushing);
                        let result = self.push_inner(handles, generation, false).await;
                        return self.apply_task_result(handles, result).await;
                    }
                    SyncContentRelation::RemoteChanged => {
                        self.set_phase(SyncPhaseDto::Pulling);
                        let result = self
                            .pull_inner(handles, generation, false, Some(local_revision))
                            .await;
                        return self.apply_task_result(handles, result).await;
                    }
                    SyncContentRelation::Diverged => {
                        self.record_conflict_ms(
                            Some(synced_at.saturating_mul(1_000)),
                            SyncInterventionReasonDto::BothSidesChanged,
                        );
                        self.set_status(SyncStatusDto::PullRequired {
                            remote_at_ms: Some(synced_at.saturating_mul(1_000)),
                            reason: SyncInterventionReasonDto::BothSidesChanged,
                        });
                        self.set_phase(SyncPhaseDto::Conflict);
                        self.publish(BackendEvent::ConfigChanged {
                            area: ConfigArea::Sync,
                        });
                        return Ok(self.into_run_result(handles));
                    }
                    SyncContentRelation::MissingBaseline => {
                        self.record_conflict_ms(
                            Some(synced_at.saturating_mul(1_000)),
                            SyncInterventionReasonDto::MissingSyncBaseline,
                        );
                        self.set_status(SyncStatusDto::PullRequired {
                            remote_at_ms: Some(synced_at.saturating_mul(1_000)),
                            reason: SyncInterventionReasonDto::MissingSyncBaseline,
                        });
                        self.set_phase(SyncPhaseDto::Conflict);
                        self.publish(BackendEvent::ConfigChanged {
                            area: ConfigArea::Sync,
                        });
                        return Ok(self.into_run_result(handles));
                    }
                }
            }
        }
        self.set_failure(None);
        self.set_phase(SyncPhaseDto::Idle);
        self.publish(BackendEvent::ConfigChanged {
            area: ConfigArea::Sync,
        });
        Ok(self.into_run_result(handles))
    }

    /// Runs a capability probe when the provider still needs one. Returns a
    /// result when the check stopped the automatic pass.
    async fn ensure_capability(
        &self,
        handles: &SyncHandles,
        generation: u64,
        manual: bool,
    ) -> Result<Option<SyncRunResult>, BackendError> {
        let config = handles.engine.config_store.config.clone();
        if config.provider != miaominal_sync::SyncProvider::WebDav || config.legacy_auto_sync_compat
        {
            return Ok(None);
        }
        if self.capability().state == CapabilityState::Supported {
            return Ok(None);
        }
        self.set_phase(SyncPhaseDto::CheckingCapability);
        let engine = self.engine_for(handles);
        let cancel = self.probe_cancel.fresh();
        let report = self
            .run_cancellable(generation, async {
                Ok::<_, BackendError>(handles.service.check_capability(engine, cancel).await)
            })
            .await?;
        self.store_capability(&report);
        if report.permits_unpreconditioned_write(config.webdav_unsafe_write_consent) {
            return Ok(None);
        }
        if manual {
            return Err(BackendError::SyncCapability {
                capability_detail: format!("{:?}:{}", report.state, report.step),
            });
        }
        self.finish_paused(
            SyncPhaseDto::PausedCapability,
            SyncFailureKindDto::Capability,
            &format!("{:?}:{}", report.state, report.step),
        );
        Ok(Some(self.into_run_result(handles)))
    }

    async fn push_inner(
        &self,
        handles: &SyncHandles,
        generation: u64,
        manual: bool,
    ) -> Result<SyncTaskResult, BackendError> {
        let engine = self.engine_for(handles);
        let settings = handles.settings_store.clone();
        self.run_cancellable(generation, async move {
            let outcome = if manual {
                handles.service.push_manual(engine, settings).await
            } else {
                handles.service.push(engine, settings).await
            };
            outcome.map_err(sync_error)
        })
        .await
    }

    async fn pull_inner(
        &self,
        handles: &SyncHandles,
        generation: u64,
        force: bool,
        expected_revision: Option<String>,
    ) -> Result<SyncTaskResult, BackendError> {
        let engine = self.engine_for(handles);
        let settings = handles.settings_store.clone();
        self.run_cancellable(generation, async move {
            let outcome = match expected_revision {
                Some(revision) => {
                    handles
                        .service
                        .pull_if_unchanged(engine, settings, revision)
                        .await
                }
                None if force => handles.service.pull_force(engine, settings).await,
                None => handles.service.pull(engine, settings).await,
            };
            outcome.map_err(sync_error)
        })
        .await
    }

    async fn local_revision(
        &self,
        handles: &SyncHandles,
        generation: u64,
    ) -> Result<String, BackendError> {
        let engine = self.engine_for(handles);
        let settings = handles.settings_store.clone();
        self.run_cancellable(generation, async move {
            handles
                .service
                .local_revision(engine, settings)
                .await
                .map_err(sync_error)
        })
        .await
    }

    async fn apply_task_result(
        &self,
        handles: &SyncHandles,
        result: Result<SyncTaskResult, BackendError>,
    ) -> Result<SyncRunResult, BackendError> {
        let result = match result {
            Ok(result) => result,
            Err(error) => return Err(self.record_operation_error(error)),
        };
        let mut areas = vec![ConfigArea::Sync];
        let status = status_dto(&result.status);
        if matches!(result.status, SyncStatus::Pulled { .. }) {
            areas = vec![
                ConfigArea::Hosts,
                ConfigArea::Forwards,
                ConfigArea::Proxies,
                ConfigArea::Snippets,
                ConfigArea::Settings,
                ConfigArea::Credentials,
                ConfigArea::Sync,
            ];
        }
        match &status {
            SyncStatusDto::PullRequired {
                remote_at_ms,
                reason,
            } => {
                self.record_conflict_ms(*remote_at_ms, *reason);
            }
            _ => {
                if matches!(
                    status,
                    SyncStatusDto::Pulled { .. }
                        | SyncStatusDto::UpToDate { .. }
                        | SyncStatusDto::Pushed { .. }
                ) {
                    self.set_conflict(None);
                }
            }
        }
        if let Some(reload) = result.reload.as_ref()
            && reload.any_failed()
        {
            log::warn!("sync pull applied but one or more snapshots failed to reload");
        }
        // The engine applied settings through its own store clone; refresh the
        // data layer's cache before any ConfigChanged event is published so a
        // host that refreshes on the event never reads pre-pull values.
        if matches!(result.status, SyncStatus::Pulled { .. }) {
            self.reload_live_settings().await;
        }
        self.set_failure(None);
        self.finish(handles, status, &self.capability(), &areas);
        Ok(SyncRunResult {
            status: self.lock_state().status.clone(),
            config: SyncConfigDto::from(&result.updated_config),
            changed_areas: areas,
        })
    }

    /// Re-read the data layer's settings cache through the attached handle.
    async fn reload_live_settings(&self) {
        let Some(data) = self.data_handle() else {
            return;
        };
        let reload = runtime::offload(move || data.reload_settings_after_sync()).await;
        if let Err(error) = reload {
            log::warn!("failed to reload live settings after a sync pull: {error}");
        }
    }

    /// Terminal bookkeeping for any operation error: the phase must never stay
    /// in a running state, and the failure stays visible for the UI.
    fn record_operation_error(&self, error: BackendError) -> BackendError {
        let failure = failure_from_backend_error(&error);
        let (phase, status) = failure_phase_and_status(&failure);
        self.set_failure(Some(failure));
        self.set_phase(phase);
        self.set_status(status);
        self.publish(BackendEvent::ConfigChanged {
            area: ConfigArea::Sync,
        });
        error
    }

    fn record_conflict_ms(&self, remote_at_ms: Option<u64>, reason: SyncInterventionReasonDto) {
        self.set_conflict(Some(SyncConflictDto {
            remote_at_ms,
            reason,
        }));
    }

    fn store_capability(&self, report: &CapabilityReport) {
        let mut state = self.lock_state();
        let previous = state.capability.clone();
        let changed = previous.state != report.state || previous.reason != report.reason;
        state.capability = report.clone();
        drop(state);
        if changed {
            self.publish(BackendEvent::SyncCapabilityChanged {
                state: report.state.into(),
                reason: report.reason.map(Into::into),
            });
        }
    }

    fn capability(&self) -> CapabilityReport {
        self.lock_state().capability.clone()
    }

    fn engine_for(&self, handles: &SyncHandles) -> SyncEngine {
        let mut engine = handles.engine.clone();
        engine.last_capability_report = Some(self.capability());
        engine
    }

    fn finish(
        &self,
        handles: &SyncHandles,
        status: SyncStatusDto,
        report: &CapabilityReport,
        areas: &[ConfigArea],
    ) {
        self.store_capability(report);
        self.set_status(status);
        let next_phase = match self.lock_state().pending_conflict {
            Some(_) => SyncPhaseDto::Conflict,
            None => {
                if handles.engine.config_store.config.auto_sync_enabled {
                    SyncPhaseDto::Idle
                } else {
                    SyncPhaseDto::Disabled
                }
            }
        };
        self.set_phase(next_phase);
        for area in areas {
            self.publish(BackendEvent::ConfigChanged { area: *area });
        }
    }

    fn finish_paused(&self, phase: SyncPhaseDto, kind: SyncFailureKindDto, detail: &str) {
        let failure = SyncFailureDto {
            kind,
            detail: detail.to_string(),
        };
        self.set_failure(Some(failure.clone()));
        self.set_status(SyncStatusDto::Failed { failure });
        self.set_phase(phase);
        self.publish(BackendEvent::ConfigChanged {
            area: ConfigArea::Sync,
        });
    }

    fn into_run_result(&self, handles: &SyncHandles) -> SyncRunResult {
        SyncRunResult {
            status: self.lock_state().status.clone(),
            config: SyncConfigDto::from(&handles.engine.config_store.config),
            changed_areas: vec![ConfigArea::Sync],
        }
    }

    // ------------------------------------------------------------------
    // Import previews
    // ------------------------------------------------------------------

    pub(crate) fn insert_import(
        &self,
        batch: ImportedBatch,
        source_label: &str,
    ) -> ImportPreviewDto {
        let mut registry = self
            .imports
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while registry.entries.len() >= MAX_PENDING_IMPORTS {
            registry.entries.pop_front();
        }
        let preview_id = registry.next_id;
        registry.next_id = registry.next_id.wrapping_add(1);
        let entries = batch.sessions.iter().map(entry_preview).collect::<Vec<_>>();
        let warnings = batch
            .issues
            .iter()
            .map(|issue| warning_dto(issue.kind, &issue.entry_name, &issue.reason))
            .collect::<Vec<_>>();
        registry.entries.push_back((preview_id, batch));
        ImportPreviewDto {
            preview_id,
            source_label: source_label.to_string(),
            entries,
            warnings,
        }
    }

    pub(crate) fn take_import(&self, preview_id: u64) -> Option<ImportedBatch> {
        let mut registry = self
            .imports
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let index = registry
            .entries
            .iter()
            .position(|(id, _)| *id == preview_id)?;
        registry.entries.remove(index).map(|(_, batch)| batch)
    }

    pub(crate) fn discard_import(&self, preview_id: u64) -> bool {
        let mut registry = self
            .imports
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(index) = registry
            .entries
            .iter()
            .position(|(id, _)| *id == preview_id)
        else {
            return false;
        };
        registry.entries.remove(index);
        true
    }

    #[cfg(test)]
    pub(crate) fn clear_conflict(&self) {
        self.set_conflict(None);
    }
}

pub(crate) fn remote_state_dto(remote: &RemoteSyncState) -> SyncRemoteStateDto {
    match remote {
        RemoteSyncState::Disabled => SyncRemoteStateDto::Disabled,
        RemoteSyncState::BindingRequired(provider) => SyncRemoteStateDto::BindingRequired {
            provider: (*provider).into(),
        },
        RemoteSyncState::Missing => SyncRemoteStateDto::Missing,
        RemoteSyncState::NotModified => SyncRemoteStateDto::NotModified,
        RemoteSyncState::UpToDate => SyncRemoteStateDto::UpToDate,
        RemoteSyncState::Updated {
            synced_at,
            content_revision,
            ..
        } => SyncRemoteStateDto::Updated {
            synced_at_ms: synced_at.saturating_mul(1_000),
            content_revision: content_revision.clone(),
        },
    }
}

pub(crate) fn has_provider_credentials(config: &miaominal_sync::SyncConfig) -> bool {
    let provider_ready = match config.provider {
        miaominal_sync::SyncProvider::None => false,
        miaominal_sync::SyncProvider::GithubGist => config.has_github_token,
        miaominal_sync::SyncProvider::WebDav => config.has_webdav_password,
    };
    provider_ready && config.has_passphrase
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// The phase and status a terminal failure must leave behind. A cancellation
/// and every non-auth failure return to `Idle`, so `isRunning` never sticks.
fn failure_phase_and_status(failure: &SyncFailureDto) -> (SyncPhaseDto, SyncStatusDto) {
    match failure.kind {
        SyncFailureKindDto::Locked => (
            SyncPhaseDto::PausedVaultLocked,
            SyncStatusDto::Failed {
                failure: failure.clone(),
            },
        ),
        SyncFailureKindDto::NotConfigured => (
            SyncPhaseDto::PausedCredentials,
            SyncStatusDto::Failed {
                failure: failure.clone(),
            },
        ),
        SyncFailureKindDto::Cancelled => (SyncPhaseDto::Idle, SyncStatusDto::Cancelled),
        _ => (
            SyncPhaseDto::Idle,
            SyncStatusDto::Failed {
                failure: failure.clone(),
            },
        ),
    }
}

/// Classify an engine/service failure into the stable sync failure kinds.
pub(crate) fn failure_from_anyhow(error: &anyhow::Error) -> SyncFailureDto {
    if miaominal_secrets::SecretStore::is_locked_error(error) {
        return SyncFailureDto {
            kind: SyncFailureKindDto::Locked,
            detail: "vault_locked".to_string(),
        };
    }
    if let Some(capability) = error.downcast_ref::<CapabilityError>() {
        return SyncFailureDto {
            kind: SyncFailureKindDto::Capability,
            detail: format!("{:?}:{}", capability.0.state, capability.0.step),
        };
    }
    let message = error
        .chain()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ");
    let lower = message.to_ascii_lowercase();
    let kind = if lower.contains("not configured") {
        SyncFailureKindDto::NotConfigured
    } else if lower.contains("decryption failed")
        || lower.contains("wrong passphrase")
        || lower.contains("authentication failed")
        || lower.contains("401")
    {
        SyncFailureKindDto::Authentication
    } else if lower.contains("failed to get")
        || lower.contains("failed to put")
        || lower.contains("failed to post")
        || lower.contains("failed to patch")
        || lower.contains("failed to fetch")
        || lower.contains("failed to communicate")
        || lower.contains("error sending request")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connection")
        || lower.contains("dns")
    {
        SyncFailureKindDto::Network
    } else if lower.contains("412") || lower.contains("conflict") {
        SyncFailureKindDto::Conflict
    } else {
        SyncFailureKindDto::Other
    };
    SyncFailureDto {
        kind,
        detail: crate::sync_dto::sanitize_detail(&message),
    }
}

fn sync_error(error: anyhow::Error) -> BackendError {
    let failure = failure_from_anyhow(&error);
    match failure.kind {
        SyncFailureKindDto::Locked => BackendError::VaultLocked,
        SyncFailureKindDto::Cancelled => BackendError::Cancelled,
        SyncFailureKindDto::Network => BackendError::Network {
            network_failure: failure.detail,
        },
        SyncFailureKindDto::Conflict => BackendError::SyncConflict {
            conflict_detail: failure.detail,
        },
        SyncFailureKindDto::Capability => BackendError::SyncCapability {
            capability_detail: failure.detail,
        },
        SyncFailureKindDto::Authentication => BackendError::SyncAuthentication {
            sync_detail: failure.detail,
        },
        SyncFailureKindDto::NotConfigured => BackendError::SyncNotConfigured {
            sync_detail: failure.detail,
        },
        _ => BackendError::Storage {
            storage_failure: failure.detail,
        },
    }
}

fn failure_from_backend_error(error: &BackendError) -> SyncFailureDto {
    match error {
        BackendError::VaultLocked => SyncFailureDto {
            kind: SyncFailureKindDto::Locked,
            detail: "vault_locked".to_string(),
        },
        BackendError::Cancelled => SyncFailureDto {
            kind: SyncFailureKindDto::Cancelled,
            detail: "cancelled".to_string(),
        },
        BackendError::Network { network_failure } => SyncFailureDto {
            kind: SyncFailureKindDto::Network,
            detail: network_failure.clone(),
        },
        BackendError::SyncConflict { conflict_detail } => SyncFailureDto {
            kind: SyncFailureKindDto::Conflict,
            detail: conflict_detail.clone(),
        },
        BackendError::SyncCapability { capability_detail } => SyncFailureDto {
            kind: SyncFailureKindDto::Capability,
            detail: capability_detail.clone(),
        },
        BackendError::SyncAuthentication { sync_detail } => SyncFailureDto {
            kind: SyncFailureKindDto::Authentication,
            detail: sync_detail.clone(),
        },
        BackendError::SyncNotConfigured { sync_detail } => SyncFailureDto {
            kind: SyncFailureKindDto::NotConfigured,
            detail: sync_detail.clone(),
        },
        other => SyncFailureDto {
            kind: SyncFailureKindDto::Other,
            detail: crate::sync_dto::sanitize_detail(&other.to_string()),
        },
    }
}

/// Keep the import entry point total: parse errors are caller errors.
pub(crate) fn import_parse_error(error: anyhow::Error) -> BackendError {
    BackendError::InvalidArgument {
        detail: crate::sync_dto::sanitize_detail(&error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;
    use crate::sync_dto::{ImportSourceDto, SyncConfigDto};
    use miaominal_core::profile::ImportSourceKind;
    use std::sync::Mutex;

    fn test_coordinator() -> Arc<SyncCoordinator> {
        let lifecycle = Arc::new(Mutex::new(LifecycleState::default()));
        let events = EventBus::new();
        let (_close_tx, close_rx) = watch::channel(false);
        SyncCoordinator::new(lifecycle, events, close_rx)
    }

    fn default_config() -> SyncConfigDto {
        SyncConfigDto::from(&miaominal_sync::SyncConfig::default())
    }

    #[tokio::test]
    async fn closed_coordinator_rejects_every_sync_entry_point() {
        let _guard = crate::test_support::shared_test_lock();
        crate::test_support::clear_test_root();
        let data = MobileData::new(crate::test_support::test_host()).unwrap();
        let coordinator = test_coordinator();
        let [
            capability,
            pull,
            push,
            remote,
            keep_local,
            keep_remote,
            auto,
        ] = std::array::from_fn(|_| data.sync_handles().unwrap());
        coordinator.lifecycle.lock().unwrap().closed = true;
        coordinator.cancel();

        assert!(matches!(
            coordinator.run_capability_check(capability).await,
            Err(BackendError::Closed)
        ));
        assert!(matches!(
            coordinator.run_pull(pull, true).await,
            Err(BackendError::Closed)
        ));
        assert!(matches!(
            coordinator.run_push(push, true).await,
            Err(BackendError::Closed)
        ));
        assert!(matches!(
            coordinator.run_remote_state(remote).await,
            Err(BackendError::Closed)
        ));
        assert!(matches!(
            coordinator
                .run_conflict_resolution(keep_local, SyncResolution::KeepLocal)
                .await,
            Err(BackendError::Closed)
        ));
        assert!(matches!(
            coordinator
                .run_conflict_resolution(keep_remote, SyncResolution::KeepRemote)
                .await,
            Err(BackendError::Closed)
        ));
        assert!(matches!(
            coordinator
                .run_auto_check(auto, AutoTrigger::Foreground)
                .await,
            Err(BackendError::Closed)
        ));
    }

    #[tokio::test]
    async fn queued_sync_cannot_start_after_shutdown() {
        let _guard = crate::test_support::shared_test_lock();
        crate::test_support::clear_test_root();
        let data = MobileData::new(crate::test_support::test_host()).unwrap();
        let coordinator = test_coordinator();
        let gate = coordinator.op_gate.lock().await;
        let operation = coordinator.run_push(data.sync_handles().unwrap(), true);
        tokio::pin!(operation);
        assert!(futures::poll!(operation.as_mut()).is_pending());

        coordinator.lifecycle.lock().unwrap().closed = true;
        coordinator.cancel();
        drop(gate);
        assert!(matches!(operation.await, Err(BackendError::Closed)));
    }

    #[tokio::test]
    async fn cancellation_is_observed_before_polling_ready_work() {
        let coordinator = test_coordinator();
        let generation = *coordinator.cancel_rx.borrow();
        coordinator.cancel();
        let polled = std::cell::Cell::new(false);
        let result = coordinator
            .run_cancellable(generation, async {
                polled.set(true);
                Ok(())
            })
            .await;
        assert!(matches!(result, Err(BackendError::Cancelled)));
        assert!(
            !polled.get(),
            "cancelled work must not perform synchronous side effects"
        );
    }

    #[test]
    fn snapshot_starts_disabled_and_conflict_free() {
        let coordinator = test_coordinator();
        let snapshot = coordinator.snapshot(&default_config(), false);

        assert_eq!(snapshot.phase, SyncPhaseDto::Disabled);
        assert_eq!(snapshot.status, SyncStatusDto::Idle);
        assert!(snapshot.pending_conflict.is_none());
        assert!(!snapshot.vault_locked);
    }

    #[test]
    fn conflict_state_is_reliable_and_clears_explicitly() {
        let coordinator = test_coordinator();
        coordinator.record_conflict_ms(Some(42), SyncInterventionReasonDto::BothSidesChanged);
        let snapshot = coordinator.snapshot(&default_config(), false);
        assert_eq!(
            snapshot.pending_conflict,
            Some(SyncConflictDto {
                remote_at_ms: Some(42),
                reason: SyncInterventionReasonDto::BothSidesChanged,
            })
        );

        coordinator.clear_conflict();
        assert!(
            coordinator
                .snapshot(&default_config(), false)
                .pending_conflict
                .is_none()
        );
    }

    #[test]
    fn import_previews_are_bounded_and_ids_stay_unique() {
        let coordinator = test_coordinator();
        let mut last_id = 0;
        for _ in 0..(MAX_PENDING_IMPORTS + 2) {
            let preview = coordinator.insert_import(
                ImportedBatch::default(),
                ImportSourceKind::OpenSshConfig.label(),
            );
            last_id = preview.preview_id;
        }
        assert_eq!(last_id as usize, MAX_PENDING_IMPORTS + 2);
        assert_eq!(
            coordinator
                .imports
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .entries
                .len(),
            MAX_PENDING_IMPORTS
        );
        assert!(coordinator.discard_import(last_id));
        assert!(!coordinator.discard_import(last_id));
        assert!(
            coordinator.take_import(1).is_none(),
            "the oldest previews were evicted"
        );
        assert!(
            coordinator.take_import(last_id).is_none(),
            "already discarded"
        );
        assert!(coordinator.take_import(2).is_none(), "evicted");
        assert!(coordinator.take_import(5).is_some());
    }

    #[test]
    fn import_source_maps_to_the_shared_kind() {
        assert_eq!(
            ImportSourceKind::from(ImportSourceDto::FinalShell),
            ImportSourceKind::FinalShellJson
        );
        assert_eq!(
            ImportSourceKind::from(ImportSourceDto::OpenSsh),
            ImportSourceKind::OpenSshConfig
        );
    }
}
