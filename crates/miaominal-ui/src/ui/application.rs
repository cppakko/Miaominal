use gpui::{App, AppContext as _, Context, Entity, Global};
use miaominal_core::keychain::ManagedKeyRecord;
use miaominal_core::known_host::KnownHostEntry;
use miaominal_core::profile::SessionProfile;
use miaominal_core::proxy::ProxyProfile;
use miaominal_core::snippet::SnippetRecord;
use miaominal_core::ssh_bridge_security::BridgeSecuritySnapshot;
use miaominal_secrets::{ProtectedPassphrase, SecretStore, VaultCredentialBackend};
use miaominal_services::{
    AgentService, AppServices, ChatService, LoadedAppData, LocalVaultMode, LocalVaultTransition,
    PortForwardManagerSnapshot, SettingsService,
};
use miaominal_ssh::{SshBridgeStatus, SshBridgeSyncResult};
use miaominal_storage::SettingsStore;
use miaominal_storage::chat_store::ChatSessionRecord;
use miaominal_sync::SyncEngine;
use std::rc::Rc;
use std::time::Instant;
use tokio::runtime::Handle as TokioHandle;

use super::i18n;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ApplicationGenerations {
    pub(crate) catalogs: u64,
    pub(crate) settings: u64,
    pub(crate) vault: u64,
    pub(crate) chat: u64,
    pub(crate) port_forwards: u64,
    pub(crate) bridge: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApplicationVaultStatus {
    Disabled,
    Locked,
    Unlocked,
}

impl From<LocalVaultMode> for ApplicationVaultStatus {
    fn from(mode: LocalVaultMode) -> Self {
        match mode {
            LocalVaultMode::Disabled => Self::Disabled,
            LocalVaultMode::Locked => Self::Locked,
            LocalVaultMode::Unlocked => Self::Unlocked,
        }
    }
}

fn initial_application_vault_status(
    local_vault_enabled: bool,
    credential_policy: Option<miaominal_paths::CredentialPolicy>,
    vault_store_exists: Result<bool, ()>,
) -> ApplicationVaultStatus {
    if !local_vault_enabled {
        return ApplicationVaultStatus::Disabled;
    }

    if credential_policy == Some(miaominal_paths::CredentialPolicy::LocalVaultRequired)
        && vault_store_exists == Ok(false)
    {
        ApplicationVaultStatus::Disabled
    } else {
        ApplicationVaultStatus::Locked
    }
}

#[derive(Clone)]
pub(crate) struct ApplicationVaultSnapshot {
    pub(crate) secrets: SecretStore,
    pub(crate) agent_service: AgentService,
    pub(crate) sync_engine: SyncEngine,
    pub(crate) status: ApplicationVaultStatus,
    pub(crate) session_passphrase: Option<ProtectedPassphrase>,
    pub(crate) automatic_lock: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApplicationVaultTransitionSource {
    User,
    AutoLock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VaultChatRefreshAction {
    Preserve,
    RefreshLocalVault,
    ClearUnavailable,
}

fn vault_chat_refresh_action(
    policy: Result<miaominal_paths::CredentialPolicy, ()>,
) -> VaultChatRefreshAction {
    match policy {
        Ok(miaominal_paths::CredentialPolicy::LocalVaultRequired) => {
            VaultChatRefreshAction::RefreshLocalVault
        }
        Ok(miaominal_paths::CredentialPolicy::SystemKeyring) => VaultChatRefreshAction::Preserve,
        Err(()) => VaultChatRefreshAction::ClearUnavailable,
    }
}

#[derive(Clone)]
pub(crate) struct ApplicationBootstrapSnapshot {
    pub(crate) services: AppServices,
    pub(crate) settings_store: SettingsStore,
    pub(crate) profiles: Vec<SessionProfile>,
    pub(crate) proxies: Vec<ProxyProfile>,
    pub(crate) selected_profile: Option<usize>,
    pub(crate) known_hosts_entries: Vec<KnownHostEntry>,
    pub(crate) snippets: Vec<SnippetRecord>,
    pub(crate) managed_keys: Vec<ManagedKeyRecord>,
    pub(crate) chat_service: Option<Rc<ChatService>>,
    pub(crate) chat_sessions: Vec<ChatSessionRecord>,
    pub(crate) chat_status_message: Option<String>,
    pub(crate) status_message: String,
    pub(crate) vault: ApplicationVaultSnapshot,
    pub(crate) port_forwards: PortForwardManagerSnapshot,
    pub(crate) bridge_status: SshBridgeStatus,
    pub(crate) bridge_sync_result: Option<SshBridgeSyncResult>,
    pub(crate) bridge_security: BridgeSecuritySnapshot,
    pub(crate) generations: ApplicationGenerations,
}

pub(crate) struct ApplicationState {
    services: AppServices,
    settings_store: SettingsStore,
    profiles: Vec<SessionProfile>,
    proxies: Vec<ProxyProfile>,
    selected_profile: Option<usize>,
    known_hosts_entries: Vec<KnownHostEntry>,
    snippets: Vec<SnippetRecord>,
    managed_keys: Vec<ManagedKeyRecord>,
    chat_service: Option<Rc<ChatService>>,
    chat_sessions: Vec<ChatSessionRecord>,
    chat_status_message: Option<String>,
    status_message: String,
    sync_engine: SyncEngine,
    vault_status: ApplicationVaultStatus,
    session_passphrase: Option<ProtectedPassphrase>,
    last_vault_lock_was_automatic: bool,
    vault_unlocked_at: Option<Instant>,
    vault_auto_lock_task: Option<gpui::Task<()>>,
    port_forward_snapshot: PortForwardManagerSnapshot,
    runtime_observer_task: Option<gpui::Task<()>>,
    bridge_observer_task: Option<gpui::Task<()>>,
    bridge_status: SshBridgeStatus,
    bridge_sync_result: Option<SshBridgeSyncResult>,
    bridge_security: BridgeSecuritySnapshot,
    generations: ApplicationGenerations,
}

#[derive(Clone)]
struct GlobalApplicationState(Entity<ApplicationState>);

impl Global for GlobalApplicationState {}

impl ApplicationState {
    fn load(runtime: TokioHandle) -> Self {
        let settings_store = match SettingsStore::load() {
            Ok(store) => store,
            Err(error) => {
                log::warn!("settings unavailable, using defaults: {error:?}");
                SettingsStore::fallback()
            }
        };
        i18n::set_language(settings_store.settings().language);

        let local_vault_enabled = settings_store.settings().local_vault_enabled;
        let credential_policy = match miaominal_paths::credential_policy() {
            Ok(policy) => Some(policy),
            Err(error) => {
                log::warn!("credential policy unavailable while resolving vault state: {error:?}");
                None
            }
        };
        let vault_store_exists = VaultCredentialBackend::default_store_exists().map_err(|error| {
            log::warn!("failed to inspect local vault file: {error:?}");
        });
        let vault_status = initial_application_vault_status(
            local_vault_enabled,
            credential_policy,
            vault_store_exists,
        );
        let LoadedAppData {
            services,
            known_hosts_entries,
            managed_keys,
            chat_service,
            chat_sessions,
            sessions,
            proxies,
            snippets,
            selected_profile,
            status_message,
        } = AppServices::load(
            runtime,
            local_vault_enabled,
            settings_store.settings().open_ssh_integration_mode,
            settings_store.settings().ssh_bridge.clone(),
        );
        let sync_engine = if local_vault_enabled {
            SyncEngine::new_locked_vault()
        } else {
            SyncEngine::new()
        };
        let port_forward_snapshot = services.port_forward_manager.snapshot();
        let bridge_status = services.ssh_bridge_service.status();
        let bridge_sync_result = services.open_ssh_integration_service.last_sync_result();
        let bridge_security = services.ssh_bridge_service.security_snapshot();

        Self {
            services,
            settings_store,
            profiles: sessions,
            proxies,
            selected_profile,
            known_hosts_entries,
            snippets,
            managed_keys,
            chat_service,
            chat_sessions,
            chat_status_message: None,
            status_message,
            sync_engine,
            vault_status,
            session_passphrase: None,
            last_vault_lock_was_automatic: false,
            vault_unlocked_at: None,
            vault_auto_lock_task: None,
            port_forward_snapshot,
            runtime_observer_task: None,
            bridge_observer_task: None,
            generations: ApplicationGenerations {
                catalogs: 1,
                settings: 1,
                vault: 1,
                chat: 1,
                port_forwards: 1,
                bridge: 1,
            },
            bridge_status,
            bridge_sync_result,
            bridge_security,
        }
    }

    fn reload(&mut self, runtime: TokioHandle, cx: &mut Context<Self>) {
        let mut replacement = Self::load(runtime);
        replacement.generations = generations_after_reload(self.generations);
        let previous = std::mem::replace(self, replacement);
        drop(previous);
        self.start_runtime_observers(cx);
        cx.notify();
    }

    pub(crate) fn snapshot(&self) -> ApplicationBootstrapSnapshot {
        ApplicationBootstrapSnapshot {
            services: self.services.clone(),
            settings_store: self.settings_store.clone(),
            profiles: self.profiles.clone(),
            proxies: self.proxies.clone(),
            selected_profile: self.selected_profile,
            known_hosts_entries: self.known_hosts_entries.clone(),
            snippets: self.snippets.clone(),
            managed_keys: self.managed_keys.clone(),
            chat_service: self.chat_service.clone(),
            chat_sessions: self.chat_sessions.clone(),
            chat_status_message: self.chat_status_message.clone(),
            status_message: self.status_message.clone(),
            vault: ApplicationVaultSnapshot {
                secrets: self.services.secrets.clone(),
                agent_service: self.services.agent_service.clone(),
                sync_engine: self.sync_engine.clone(),
                status: self.vault_status,
                session_passphrase: self.session_passphrase.clone(),
                automatic_lock: self.last_vault_lock_was_automatic,
            },
            port_forwards: self.port_forward_snapshot.clone(),
            bridge_status: self.bridge_status.clone(),
            bridge_sync_result: self.bridge_sync_result.clone(),
            bridge_security: self.bridge_security.clone(),
            generations: self.generations,
        }
    }

    pub(crate) fn publish_catalogs(
        &mut self,
        profiles: Vec<SessionProfile>,
        proxies: Vec<ProxyProfile>,
        snippets: Vec<SnippetRecord>,
        known_hosts_entries: Vec<KnownHostEntry>,
        cx: &mut Context<Self>,
    ) {
        if self.profiles == profiles
            && self.proxies == proxies
            && self.snippets == snippets
            && self.known_hosts_entries == known_hosts_entries
        {
            return;
        }
        self.profiles = profiles;
        self.proxies = proxies;
        self.snippets = snippets;
        self.known_hosts_entries = known_hosts_entries;
        self.services
            .ssh_bridge_service
            .refresh_routes(self.profiles.clone(), self.proxies.clone());
        self.services
            .port_forward_manager
            .replace_catalogs(self.profiles.clone(), self.proxies.clone());
        self.generations.catalogs = next_generation(self.generations.catalogs);
        cx.notify();
    }

    pub(crate) fn publish_settings(
        &mut self,
        settings_store: SettingsStore,
        proxies: Vec<ProxyProfile>,
        sync_engine: SyncEngine,
        cx: &mut Context<Self>,
    ) {
        let settings_changed =
            miaominal_settings::changed(self.settings_store.settings(), settings_store.settings());
        let proxies_changed = self.proxies != proxies;
        let sync_changed = self.sync_engine.config_store.config != sync_engine.config_store.config;
        if !settings_changed && !proxies_changed && !sync_changed {
            return;
        }

        let auto_lock_duration_changed = self
            .settings_store
            .settings()
            .local_vault_auto_lock_duration
            != settings_store.settings().local_vault_auto_lock_duration;
        self.settings_store = settings_store;
        self.proxies = proxies;
        self.sync_engine = sync_engine;
        self.generations.settings = next_generation(self.generations.settings);
        if proxies_changed {
            self.generations.catalogs = next_generation(self.generations.catalogs);
            self.services
                .ssh_bridge_service
                .refresh_routes(self.profiles.clone(), self.proxies.clone());
            self.services
                .port_forward_manager
                .replace_catalogs(self.profiles.clone(), self.proxies.clone());
        }
        if auto_lock_duration_changed {
            self.sync_vault_auto_lock_task(cx);
        }
        cx.notify();
    }

    pub(crate) fn publish_managed_keys(
        &mut self,
        managed_keys: Vec<ManagedKeyRecord>,
        cx: &mut Context<Self>,
    ) {
        if self.managed_keys == managed_keys {
            return;
        }
        self.managed_keys = managed_keys;
        self.generations.catalogs = next_generation(self.generations.catalogs);
        cx.notify();
    }

    pub(crate) fn publish_chat_sessions(
        &mut self,
        chat_sessions: Vec<ChatSessionRecord>,
        cx: &mut Context<Self>,
    ) {
        if self.chat_sessions == chat_sessions {
            return;
        }
        self.chat_sessions = chat_sessions;
        self.generations.chat = next_generation(self.generations.chat);
        cx.notify();
    }

    pub(crate) fn commit_local_vault_transition(
        &mut self,
        transition: LocalVaultTransition,
        cx: &mut Context<Self>,
    ) {
        self.commit_local_vault_transition_with_source(
            transition,
            ApplicationVaultTransitionSource::User,
            cx,
        );
    }

    fn commit_local_vault_transition_with_source(
        &mut self,
        transition: LocalVaultTransition,
        source: ApplicationVaultTransitionSource,
        cx: &mut Context<Self>,
    ) {
        let was_unlocked = self.vault_status == ApplicationVaultStatus::Unlocked;
        let LocalVaultTransition {
            mode,
            secrets,
            sync_engine,
            session_passphrase,
        } = transition;
        let next_status = ApplicationVaultStatus::from(mode);

        self.services.secrets = secrets.clone();
        self.services.agent_service = AgentService::new(
            self.services.runtime.clone(),
            secrets.clone(),
            self.services.known_hosts.clone(),
        );
        self.services.ssh_bridge_service.replace_secrets(secrets);
        self.services
            .port_forward_manager
            .replace_secrets(self.services.secrets.clone());
        self.sync_engine = sync_engine;
        self.vault_status = next_status;
        self.session_passphrase = session_passphrase;
        self.last_vault_lock_was_automatic = next_status == ApplicationVaultStatus::Locked
            && source == ApplicationVaultTransitionSource::AutoLock;
        let credential_policy = miaominal_paths::credential_policy();
        let chat_refresh_action =
            vault_chat_refresh_action(credential_policy.as_ref().copied().map_err(|_| ()));
        match chat_refresh_action {
            VaultChatRefreshAction::RefreshLocalVault => {
                if next_status == ApplicationVaultStatus::Unlocked {
                    match ChatService::open(&self.services.secrets.credentials()) {
                        Ok(service) => {
                            let sessions = service.list_sessions().unwrap_or_else(|error| {
                                log::warn!(
                                    "failed to refresh application chat sessions after vault unlock: {error:?}"
                                );
                                Vec::new()
                            });
                            self.chat_service = Some(Rc::new(service));
                            self.chat_sessions = sessions;
                            self.chat_status_message = None;
                        }
                        Err(error) => {
                            log::warn!(
                                "chat service unavailable after vault transition: {error:?}"
                            );
                            self.chat_service = None;
                            self.chat_sessions.clear();
                            self.chat_status_message = Some(
                                if error.chain().any(|cause| {
                                    cause.to_string().contains("encryption key is unavailable")
                                }) {
                                    i18n::string("status.chat_history_key_missing")
                                } else {
                                    i18n::string("status.chat_history_unavailable")
                                },
                            );
                        }
                    }
                } else {
                    self.chat_service = None;
                    self.chat_sessions.clear();
                    self.chat_status_message = None;
                }
                self.generations.chat = next_generation(self.generations.chat);
            }
            VaultChatRefreshAction::Preserve => {}
            VaultChatRefreshAction::ClearUnavailable => {
                let error =
                    credential_policy.expect_err("unavailable credential policy retains its error");
                log::warn!(
                    "failed to determine credential policy during vault transition: {error:?}"
                );
                self.chat_service = None;
                self.chat_sessions.clear();
                self.chat_status_message = Some(i18n::string("status.chat_history_unavailable"));
                self.generations.chat = next_generation(self.generations.chat);
            }
        }
        self.vault_unlocked_at = vault_unlocked_at_after_transition(
            was_unlocked,
            next_status,
            self.vault_unlocked_at,
            Instant::now(),
        );
        self.generations.vault = next_generation(self.generations.vault);
        self.sync_vault_auto_lock_task(cx);
        cx.notify();
    }

    fn start_runtime_observers(&mut self, cx: &mut Context<Self>) {
        if self.runtime_observer_task.is_some() {
            return;
        }
        let mut port_forwards = self.services.port_forward_manager.subscribe();
        self.runtime_observer_task = Some(cx.spawn(async move |this, cx| {
            loop {
                if port_forwards.changed().await.is_err() {
                    break;
                }
                let snapshot = port_forwards.borrow_and_update().clone();
                if this
                    .update(cx, |state, cx| {
                        if state.port_forward_snapshot.revision == snapshot.revision {
                            return;
                        }
                        state.port_forward_snapshot = snapshot;
                        let catalogs_changed = reconcile_port_forward_enabled(
                            &mut state.profiles,
                            &state.port_forward_snapshot,
                        );
                        if catalogs_changed {
                            if let Some(store) = &state.services.session_store
                                && let Err(error) = store.save(&state.profiles)
                            {
                                log::warn!(
                                    "failed to persist terminal port-forward runtime state: {error:?}"
                                );
                            }
                            state.generations.catalogs =
                                next_generation(state.generations.catalogs);
                        }
                        state.generations.port_forwards =
                            next_generation(state.generations.port_forwards);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));

        let bridge = self.services.ssh_bridge_service.clone();
        let integration = self.services.open_ssh_integration_service.clone();
        self.bridge_observer_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                let status = bridge.status();
                let sync_result = integration.last_sync_result();
                let security = bridge.security_snapshot();
                if this
                    .update(cx, |state, cx| {
                        if state.bridge_status == status
                            && state.bridge_sync_result == sync_result
                            && state.bridge_security == security
                        {
                            return;
                        }
                        state.bridge_status = status;
                        state.bridge_sync_result = sync_result;
                        state.bridge_security = security;
                        state.generations.bridge = next_generation(state.generations.bridge);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));

        let bridge = self.services.ssh_bridge_service.clone();
        cx.spawn(async move |_this, _cx| {
            let available = crate::ui::bridge_security_platform::system_auth_available().await;
            bridge.set_system_auth_available(available);
        })
        .detach();
    }

    fn sync_vault_auto_lock_task(&mut self, cx: &mut Context<Self>) {
        self.vault_auto_lock_task = None;
        if self.vault_status != ApplicationVaultStatus::Unlocked {
            return;
        }
        let Some(duration) = self
            .settings_store
            .settings()
            .local_vault_auto_lock_duration
            .duration()
        else {
            return;
        };
        let remaining =
            remaining_vault_auto_lock_duration(duration, self.vault_unlocked_at, Instant::now());

        self.vault_auto_lock_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(remaining).await;
            if let Err(error) = this.update(cx, |state, cx| {
                state.vault_auto_lock_task = None;
                if state.vault_status != ApplicationVaultStatus::Unlocked {
                    return;
                }
                let transition =
                    SettingsService::local_vault_lock_transition(&state.settings_store);
                state.commit_local_vault_transition_with_source(
                    transition,
                    ApplicationVaultTransitionSource::AutoLock,
                    cx,
                );
            }) {
                log::debug!("failed to apply application vault auto-lock: {error:?}");
            }
        }));
    }
}

fn vault_unlocked_at_after_transition(
    was_unlocked: bool,
    next_status: ApplicationVaultStatus,
    current: Option<Instant>,
    now: Instant,
) -> Option<Instant> {
    match next_status {
        ApplicationVaultStatus::Unlocked if was_unlocked => current,
        ApplicationVaultStatus::Unlocked => Some(now),
        ApplicationVaultStatus::Disabled | ApplicationVaultStatus::Locked => None,
    }
}

fn remaining_vault_auto_lock_duration(
    duration: std::time::Duration,
    unlocked_at: Option<Instant>,
    now: Instant,
) -> std::time::Duration {
    let elapsed = unlocked_at
        .map(|unlocked_at| now.saturating_duration_since(unlocked_at))
        .unwrap_or_default();
    duration.saturating_sub(elapsed)
}

fn reconcile_port_forward_enabled(
    profiles: &mut [SessionProfile],
    snapshot: &PortForwardManagerSnapshot,
) -> bool {
    let mut changed = false;
    for profile in profiles {
        for rule in &mut profile.port_forwarding_rules {
            let active = snapshot
                .session(&profile.id, &rule.id)
                .is_some_and(|runtime| runtime.state.is_active());
            if rule.enabled && !active {
                rule.enabled = false;
                changed = true;
            }
        }
    }
    changed
}

fn next_generation(current: u64) -> u64 {
    let next = current.wrapping_add(1);
    if next == 0 { 1 } else { next }
}

fn generations_after_reload(current: ApplicationGenerations) -> ApplicationGenerations {
    ApplicationGenerations {
        catalogs: next_generation(current.catalogs),
        settings: next_generation(current.settings),
        vault: next_generation(current.vault),
        chat: next_generation(current.chat),
        port_forwards: next_generation(current.port_forwards),
        bridge: next_generation(current.bridge),
    }
}

pub fn initialize_application_state(runtime: TokioHandle, cx: &mut App) {
    if cx.has_global::<GlobalApplicationState>() {
        return;
    }
    let state = cx.new(|_| ApplicationState::load(runtime));
    state.update(cx, |state, cx| state.start_runtime_observers(cx));
    cx.set_global(GlobalApplicationState(state));
    miaominal_settings::sync_component_theme(cx);
}

pub fn reload_application_state(runtime: TokioHandle, cx: &mut App) {
    if !cx.has_global::<GlobalApplicationState>() {
        initialize_application_state(runtime, cx);
        return;
    }
    let state = application_state(cx);
    state.update(cx, |state, cx| state.reload(runtime, cx));
    miaominal_settings::sync_component_theme(cx);
}

impl Drop for ApplicationState {
    fn drop(&mut self) {
        self.services.port_forward_manager.stop_all();
    }
}

pub(crate) fn application_state(cx: &App) -> Entity<ApplicationState> {
    cx.global::<GlobalApplicationState>().0.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_core::profile::{PortForwardKind, PortForwardRule};
    use miaominal_services::{PortForwardKey, PortForwardRuntimeSnapshot, PortForwardRuntimeState};
    use std::time::Duration;

    fn profile_with_enabled_forward() -> SessionProfile {
        let mut profile = SessionProfile::blank("profile", 1);
        profile.port_forwarding_rules.push(PortForwardRule {
            id: "rule".into(),
            label: "Forward".into(),
            kind: PortForwardKind::Local,
            listen_host: "127.0.0.1".into(),
            listen_port: 1000,
            target_host: "127.0.0.1".into(),
            target_port: 2000,
            enabled: true,
        });
        profile
    }

    fn port_forward_snapshot(state: PortForwardRuntimeState) -> PortForwardManagerSnapshot {
        PortForwardManagerSnapshot {
            sessions: vec![PortForwardRuntimeSnapshot {
                key: PortForwardKey::new("profile", "rule"),
                profile_name: "Profile".into(),
                rule: profile_with_enabled_forward()
                    .port_forwarding_rules
                    .remove(0),
                state,
                status_message: String::new(),
                log: Vec::new(),
                prompt: None,
                revision: 1,
            }],
            revision: 1,
        }
    }

    #[test]
    fn vault_unlock_timestamp_is_fixed_until_the_vault_locks() {
        let first_unlock = Instant::now();
        let later = first_unlock + Duration::from_secs(60);

        assert_eq!(
            vault_unlocked_at_after_transition(
                true,
                ApplicationVaultStatus::Unlocked,
                Some(first_unlock),
                later,
            ),
            Some(first_unlock)
        );
        assert_eq!(
            vault_unlocked_at_after_transition(
                false,
                ApplicationVaultStatus::Unlocked,
                None,
                later,
            ),
            Some(later)
        );
        assert_eq!(
            vault_unlocked_at_after_transition(
                true,
                ApplicationVaultStatus::Locked,
                Some(first_unlock),
                later,
            ),
            None
        );
    }

    #[test]
    fn initial_vault_status_requires_creation_for_missing_portable_vault() {
        assert_eq!(
            initial_application_vault_status(
                true,
                Some(miaominal_paths::CredentialPolicy::LocalVaultRequired),
                Ok(false),
            ),
            ApplicationVaultStatus::Disabled
        );
    }

    #[test]
    fn initial_vault_status_locks_existing_portable_vault() {
        assert_eq!(
            initial_application_vault_status(
                true,
                Some(miaominal_paths::CredentialPolicy::LocalVaultRequired),
                Ok(true),
            ),
            ApplicationVaultStatus::Locked
        );
    }

    #[test]
    fn initial_vault_status_preserves_non_portable_behavior() {
        assert_eq!(
            initial_application_vault_status(
                false,
                Some(miaominal_paths::CredentialPolicy::SystemKeyring),
                Ok(false),
            ),
            ApplicationVaultStatus::Disabled
        );
        assert_eq!(
            initial_application_vault_status(
                true,
                Some(miaominal_paths::CredentialPolicy::SystemKeyring),
                Ok(false),
            ),
            ApplicationVaultStatus::Locked
        );
    }

    #[test]
    fn initial_vault_status_fails_closed_when_store_inspection_fails() {
        assert_eq!(
            initial_application_vault_status(
                true,
                Some(miaominal_paths::CredentialPolicy::LocalVaultRequired),
                Err(()),
            ),
            ApplicationVaultStatus::Locked
        );
    }

    #[test]
    fn auto_lock_reschedule_uses_time_remaining_from_first_unlock() {
        let unlocked_at = Instant::now();
        let now = unlocked_at + Duration::from_secs(90);

        assert_eq!(
            remaining_vault_auto_lock_duration(Duration::from_secs(300), Some(unlocked_at), now,),
            Duration::from_secs(210)
        );
    }

    #[test]
    fn terminal_port_forward_state_clears_persisted_enabled_flag() {
        let mut profiles = vec![profile_with_enabled_forward()];
        let snapshot = port_forward_snapshot(PortForwardRuntimeState::Stopped);

        assert!(reconcile_port_forward_enabled(&mut profiles, &snapshot));
        assert!(!profiles[0].port_forwarding_rules[0].enabled);
    }

    #[test]
    fn active_port_forward_state_preserves_enabled_flag() {
        let mut profiles = vec![profile_with_enabled_forward()];
        let snapshot = port_forward_snapshot(PortForwardRuntimeState::Running);

        assert!(!reconcile_port_forward_enabled(&mut profiles, &snapshot));
        assert!(profiles[0].port_forwarding_rules[0].enabled);
    }

    #[test]
    fn reload_advances_every_application_generation() {
        let current = ApplicationGenerations {
            catalogs: 10,
            settings: 20,
            vault: 30,
            chat: 40,
            port_forwards: 50,
            bridge: 60,
        };

        assert_eq!(
            generations_after_reload(current),
            ApplicationGenerations {
                catalogs: 11,
                settings: 21,
                vault: 31,
                chat: 41,
                port_forwards: 51,
                bridge: 61,
            }
        );
    }

    #[test]
    fn credential_policy_errors_select_clear_unavailable_chat_action() {
        assert_eq!(
            vault_chat_refresh_action(Ok(miaominal_paths::CredentialPolicy::LocalVaultRequired)),
            VaultChatRefreshAction::RefreshLocalVault
        );
        assert_eq!(
            vault_chat_refresh_action(Ok(miaominal_paths::CredentialPolicy::SystemKeyring)),
            VaultChatRefreshAction::Preserve
        );
        assert_eq!(
            vault_chat_refresh_action(Err(())),
            VaultChatRefreshAction::ClearUnavailable
        );
    }
}
