use anyhow::{Result, anyhow};
use miaominal_core::profile::{PortForwardRule, SessionProfile};
use miaominal_core::proxy::ProxyProfile;
use miaominal_secrets::SecretStore;
use miaominal_ssh::{
    HostKeyDecision, HostKeyPrompt, KbiChallenge, SessionCommandSender, SessionEvent,
    start_port_forward_session,
};
use miaominal_storage::known_hosts_store::KnownHostsStore;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::watch;

const MAX_LOG_ENTRIES: usize = 64;
const TERMINAL_SESSION_RETENTION: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PortForwardKey {
    pub profile_id: String,
    pub rule_id: String,
}

impl PortForwardKey {
    pub fn new(profile_id: impl Into<String>, rule_id: impl Into<String>) -> Self {
        Self {
            profile_id: profile_id.into(),
            rule_id: rule_id.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortForwardRuntimeState {
    Starting,
    Running,
    Reconnecting {
        error: String,
        attempt: u32,
        max_attempts: u32,
        retry_after_secs: u64,
    },
    Stopping,
    Stopped,
    Failed(String),
}

impl PortForwardRuntimeState {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Running | Self::Reconnecting { .. } | Self::Stopping
        )
    }
}

#[derive(Clone, Debug)]
pub enum PortForwardPrompt {
    HostKey(HostKeyPrompt),
    KeyboardInteractive(KbiChallenge),
}

#[derive(Clone, Debug)]
pub struct PortForwardRuntimeSnapshot {
    pub key: PortForwardKey,
    pub profile_name: String,
    pub rule: PortForwardRule,
    pub state: PortForwardRuntimeState,
    pub status_message: String,
    pub log: Vec<String>,
    pub prompt: Option<PortForwardPrompt>,
    pub revision: u64,
}

#[derive(Clone, Debug, Default)]
pub struct PortForwardManagerSnapshot {
    pub sessions: Vec<PortForwardRuntimeSnapshot>,
    pub revision: u64,
}

impl PortForwardManagerSnapshot {
    pub fn session(&self, profile_id: &str, rule_id: &str) -> Option<&PortForwardRuntimeSnapshot> {
        self.sessions
            .iter()
            .find(|session| session.key.profile_id == profile_id && session.key.rule_id == rule_id)
    }
}

#[derive(Clone)]
pub struct PortForwardManager {
    inner: Arc<Inner>,
}

struct Inner {
    runtime: TokioHandle,
    known_hosts: KnownHostsStore,
    secrets: RwLock<SecretStore>,
    profiles: RwLock<Vec<SessionProfile>>,
    proxies: RwLock<Vec<ProxyProfile>>,
    state: Mutex<ManagerState>,
    snapshots: watch::Sender<PortForwardManagerSnapshot>,
}

#[derive(Default)]
struct ManagerState {
    sessions: HashMap<PortForwardKey, ManagedSession>,
    revision: u64,
    next_generation: u64,
}

struct ManagedSession {
    commands: Option<SessionCommandSender>,
    generation: u64,
    restart_requested: bool,
    snapshot: PortForwardRuntimeSnapshot,
}

enum StartDecision {
    Existing(PortForwardRuntimeSnapshot),
    Launch {
        generation: u64,
        snapshot: PortForwardRuntimeSnapshot,
    },
}

impl PortForwardManager {
    pub fn new(runtime: TokioHandle, secrets: SecretStore, known_hosts: KnownHostsStore) -> Self {
        let (snapshots, _) = watch::channel(PortForwardManagerSnapshot::default());
        Self {
            inner: Arc::new(Inner {
                runtime,
                known_hosts,
                secrets: RwLock::new(secrets),
                profiles: RwLock::new(Vec::new()),
                proxies: RwLock::new(Vec::new()),
                state: Mutex::new(ManagerState::default()),
                snapshots,
            }),
        }
    }

    pub fn replace_secrets(&self, secrets: SecretStore) {
        if let Ok(mut current) = self.inner.secrets.write() {
            *current = secrets;
        }
    }

    pub fn replace_catalogs(&self, mut profiles: Vec<SessionProfile>, proxies: Vec<ProxyProfile>) {
        let mut updates = Vec::new();
        let mut stops = Vec::new();
        let mut preserved = Vec::new();
        if let Ok(mut state) = self.inner.state.lock() {
            let mut snapshots_changed = false;
            for (key, session) in &mut state.sessions {
                let rule = profiles
                    .iter()
                    .find(|profile| profile.id == key.profile_id)
                    .and_then(|profile| {
                        profile
                            .port_forwarding_rules
                            .iter()
                            .find(|rule| rule.id == key.rule_id)
                    })
                    .cloned();
                match (rule, session.commands.clone()) {
                    (Some(mut rule), commands)
                        if should_preserve_catalog_rule(&session.snapshot.state) =>
                    {
                        let restored_runtime_state = !rule.enabled;
                        rule.enabled = true;
                        preserved.push(key.clone());
                        if session.snapshot.rule != rule || restored_runtime_state {
                            session.snapshot.rule = rule.clone();
                            session.snapshot.revision = next_revision(session.snapshot.revision);
                            snapshots_changed = true;
                        }
                        if let Some(commands) = commands {
                            updates.push((key.clone(), rule, commands));
                        }
                    }
                    (Some(rule), _) if !rule.enabled => stops.push(key.clone()),
                    (None, _) => stops.push(key.clone()),
                    _ => {}
                }
            }
            if snapshots_changed {
                self.publish_locked(&mut state);
            }
        }
        for key in preserved {
            if let Some(rule) = profiles
                .iter_mut()
                .find(|profile| profile.id == key.profile_id)
                .and_then(|profile| {
                    profile
                        .port_forwarding_rules
                        .iter_mut()
                        .find(|rule| rule.id == key.rule_id)
                })
            {
                rule.enabled = true;
            }
        }
        if let Ok(mut current) = self.inner.profiles.write() {
            *current = profiles;
        }
        if let Ok(mut current) = self.inner.proxies.write() {
            *current = proxies;
        }
        for (_, rule, commands) in updates {
            let _ = commands.sync_port_forward_rules(vec![rule]);
        }
        for key in stops {
            if !self.stop(&key.profile_id, &key.rule_id) {
                self.remove_terminal_session(&key, None);
            }
        }
    }

    pub fn start(&self, profile_id: &str, rule_id: &str) -> Result<PortForwardRuntimeSnapshot> {
        let key = PortForwardKey::new(profile_id, rule_id);
        let profiles = self
            .inner
            .profiles
            .read()
            .map(|profiles| profiles.clone())
            .map_err(|_| {
                log::error!("port-forward manager profile catalog lock is poisoned");
                anyhow!("port-forward manager profile catalog is unavailable")
            })?;
        let proxies = self
            .inner
            .proxies
            .read()
            .map(|proxies| proxies.clone())
            .map_err(|_| {
                log::error!("port-forward manager proxy catalog lock is poisoned");
                anyhow!("port-forward manager proxy catalog is unavailable")
            })?;
        let profile = profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
            .ok_or_else(|| anyhow!("port-forward profile not found: {profile_id}"))?;
        let mut rule = profile
            .port_forwarding_rules
            .iter()
            .find(|rule| rule.id == rule_id)
            .cloned()
            .ok_or_else(|| anyhow!("port-forward rule not found: {profile_id}/{rule_id}"))?;
        rule.enabled = true;
        let (generation, snapshot) =
            match self.reserve_start(key.clone(), profile.name.clone(), rule.clone())? {
                StartDecision::Existing(snapshot) => return Ok(snapshot),
                StartDecision::Launch {
                    generation,
                    snapshot,
                } => (generation, snapshot),
            };

        let mut connection_profile = profile.clone();
        connection_profile.port_forwarding_rules = vec![rule.clone()];
        let secrets = self
            .inner
            .secrets
            .read()
            .map(|secrets| secrets.clone())
            .map_err(|_| {
                log::error!("port-forward manager secret store lock is poisoned");
                anyhow!("port-forward manager secret store is unavailable")
            })?;
        let connection = start_port_forward_session(
            &self.inner.runtime,
            connection_profile,
            profiles,
            proxies,
            secrets,
            self.inner.known_hosts.clone(),
        );
        let commands = connection.commands;
        let mut events = connection.events;

        let (should_close, current_snapshot) = {
            let mut state = self
                .inner
                .state
                .lock()
                .map_err(|_| anyhow!("port-forward manager state is poisoned"))?;
            match state.sessions.get_mut(&key) {
                Some(session) if session.generation == generation => {
                    let should_close =
                        matches!(session.snapshot.state, PortForwardRuntimeState::Stopping);
                    if !should_close {
                        session.commands = Some(commands.clone());
                    }
                    (should_close, Some(session.snapshot.clone()))
                }
                _ => (true, None),
            }
        };
        if should_close {
            let _ = commands.close();
        }

        let manager = self.clone();
        self.inner.runtime.spawn(async move {
            while let Some(event) = events.recv().await {
                manager.handle_event(&key, generation, event);
            }
            manager.handle_event(&key, generation, SessionEvent::Closed);
        });
        Ok(current_snapshot.unwrap_or(snapshot))
    }

    fn reserve_start(
        &self,
        key: PortForwardKey,
        profile_name: String,
        rule: PortForwardRule,
    ) -> Result<StartDecision> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| anyhow!("port-forward manager state is poisoned"))?;
        if let Some(session) = state.sessions.get_mut(&key)
            && session.snapshot.state.is_active()
        {
            let changed = if matches!(session.snapshot.state, PortForwardRuntimeState::Stopping)
                && !session.restart_requested
            {
                session.restart_requested = true;
                session.snapshot.status_message = "Restarting after current session stops".into();
                session.snapshot.revision = next_revision(session.snapshot.revision);
                true
            } else {
                false
            };
            let snapshot = session.snapshot.clone();
            if changed {
                self.publish_locked(&mut state);
            }
            return Ok(StartDecision::Existing(snapshot));
        }

        state.next_generation = next_revision(state.next_generation);
        let generation = state.next_generation;
        let snapshot = PortForwardRuntimeSnapshot {
            key: key.clone(),
            profile_name,
            rule,
            state: PortForwardRuntimeState::Starting,
            status_message: "Connecting".to_string(),
            log: Vec::new(),
            prompt: None,
            revision: 1,
        };
        state.sessions.insert(
            key,
            ManagedSession {
                commands: None,
                generation,
                restart_requested: false,
                snapshot: snapshot.clone(),
            },
        );
        self.publish_locked(&mut state);
        Ok(StartDecision::Launch {
            generation,
            snapshot,
        })
    }

    pub fn stop(&self, profile_id: &str, rule_id: &str) -> bool {
        let key = PortForwardKey::new(profile_id, rule_id);
        let commands = {
            let Ok(mut state) = self.inner.state.lock() else {
                return false;
            };
            let Some(session) = state.sessions.get_mut(&key) else {
                return false;
            };
            session.restart_requested = false;
            if !session.snapshot.state.is_active() {
                return false;
            }
            session.snapshot.state = PortForwardRuntimeState::Stopping;
            session.snapshot.status_message = "Stopping".to_string();
            session.snapshot.revision = next_revision(session.snapshot.revision);
            let commands = session.commands.clone();
            self.publish_locked(&mut state);
            commands
        };
        if let Some(commands) = commands {
            let _ = commands.close();
        }
        true
    }

    pub fn stop_all(&self) {
        let keys = self
            .snapshot()
            .sessions
            .into_iter()
            .filter(|session| session.state.is_active())
            .map(|session| session.key)
            .collect::<Vec<_>>();
        for key in keys {
            self.stop(&key.profile_id, &key.rule_id);
        }
    }

    pub fn command_sender(&self, profile_id: &str, rule_id: &str) -> Option<SessionCommandSender> {
        let key = PortForwardKey::new(profile_id, rule_id);
        self.inner
            .state
            .lock()
            .ok()?
            .sessions
            .get(&key)?
            .commands
            .clone()
    }

    pub fn respond_host_key(
        &self,
        profile_id: &str,
        rule_id: &str,
        decision: HostKeyDecision,
    ) -> Result<()> {
        let key = PortForwardKey::new(profile_id, rule_id);
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| anyhow!("port-forward manager state is poisoned"))?;
        let session = state
            .sessions
            .get_mut(&key)
            .ok_or_else(|| anyhow!("port-forward session is not available"))?;
        if !matches!(session.snapshot.prompt, Some(PortForwardPrompt::HostKey(_))) {
            return Err(anyhow!("host-key prompt is no longer active"));
        }
        let commands = session
            .commands
            .as_ref()
            .ok_or_else(|| anyhow!("port-forward session is not available"))?;
        commands.respond_host_key(decision)?;
        session.snapshot.prompt = None;
        session.snapshot.revision = next_revision(session.snapshot.revision);
        self.publish_locked(&mut state);
        Ok(())
    }

    pub fn respond_keyboard_interactive(
        &self,
        profile_id: &str,
        rule_id: &str,
        responses: Vec<String>,
    ) -> Result<()> {
        let key = PortForwardKey::new(profile_id, rule_id);
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| anyhow!("port-forward manager state is poisoned"))?;
        let session = state
            .sessions
            .get_mut(&key)
            .ok_or_else(|| anyhow!("port-forward session is not available"))?;
        if !matches!(
            session.snapshot.prompt,
            Some(PortForwardPrompt::KeyboardInteractive(_))
        ) {
            return Err(anyhow!("keyboard-interactive prompt is no longer active"));
        }
        let commands = session
            .commands
            .as_ref()
            .ok_or_else(|| anyhow!("port-forward session is not available"))?;
        commands.respond_keyboard_interactive(responses)?;
        session.snapshot.prompt = None;
        session.snapshot.revision = next_revision(session.snapshot.revision);
        self.publish_locked(&mut state);
        Ok(())
    }

    pub fn snapshot(&self) -> PortForwardManagerSnapshot {
        self.inner.snapshots.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<PortForwardManagerSnapshot> {
        self.inner.snapshots.subscribe()
    }

    fn handle_event(&self, key: &PortForwardKey, generation: u64, event: SessionEvent) {
        let Ok(mut state) = self.inner.state.lock() else {
            return;
        };
        let Some(session) = state.sessions.get_mut(key) else {
            return;
        };
        if session.generation != generation {
            return;
        }

        let mut terminal = false;
        match event {
            SessionEvent::Connected(message) => {
                session.snapshot.state = PortForwardRuntimeState::Running;
                session.snapshot.status_message = message.clone();
                session.snapshot.prompt = None;
                push_log(&mut session.snapshot.log, message);
            }
            SessionEvent::PortForwardReconnecting {
                error,
                attempt,
                max_attempts,
                retry_after_secs,
            } => {
                session.snapshot.prompt = None;
                session.snapshot.status_message = error.clone();
                session.snapshot.state = PortForwardRuntimeState::Reconnecting {
                    error: error.clone(),
                    attempt,
                    max_attempts,
                    retry_after_secs,
                };
                push_log(&mut session.snapshot.log, error);
            }
            SessionEvent::Status(message) | SessionEvent::PortForwardNotice(message) => {
                session.snapshot.status_message = message.clone();
                push_log(&mut session.snapshot.log, message);
            }
            SessionEvent::Error(error) => {
                session.snapshot.state = PortForwardRuntimeState::Failed(error.clone());
                session.snapshot.status_message = error.clone();
                session.snapshot.prompt = None;
                push_log(&mut session.snapshot.log, error);
                terminal = true;
            }
            SessionEvent::HostKeyPrompt(prompt) => {
                session.snapshot.prompt = Some(PortForwardPrompt::HostKey(prompt));
                session.snapshot.status_message = "Host key verification required".to_string();
            }
            SessionEvent::KeyboardInteractivePrompt(challenge) => {
                session.snapshot.prompt = Some(PortForwardPrompt::KeyboardInteractive(challenge));
                session.snapshot.status_message = "Authentication response required".to_string();
            }
            SessionEvent::Closed => {
                if !matches!(session.snapshot.state, PortForwardRuntimeState::Failed(_)) {
                    session.snapshot.state = PortForwardRuntimeState::Stopped;
                    session.snapshot.status_message = "Stopped".to_string();
                }
                session.commands = None;
                session.snapshot.prompt = None;
                terminal = true;
            }
            SessionEvent::Output(_)
            | SessionEvent::MonitorUpdated(_)
            | SessionEvent::MonitorFailed(_) => return,
        }
        let restart_requested = terminal && session.restart_requested;
        if terminal {
            session.restart_requested = false;
        }
        session.snapshot.revision = next_revision(session.snapshot.revision);
        if !restart_requested {
            self.publish_locked(&mut state);
        }
        drop(state);
        if restart_requested {
            if let Err(error) = self.start(&key.profile_id, &key.rule_id) {
                log::warn!(
                    "failed to restart port-forward session {}/{}: {error:?}",
                    key.profile_id,
                    key.rule_id
                );
                self.publish_session_generation(key, generation);
                self.schedule_terminal_cleanup(key.clone(), generation);
            }
        } else if terminal {
            self.schedule_terminal_cleanup(key.clone(), generation);
        }
    }

    fn publish_session_generation(&self, key: &PortForwardKey, generation: u64) {
        let Ok(mut state) = self.inner.state.lock() else {
            return;
        };
        if state
            .sessions
            .get(key)
            .is_some_and(|session| session.generation == generation)
        {
            self.publish_locked(&mut state);
        }
    }

    fn schedule_terminal_cleanup(&self, key: PortForwardKey, generation: u64) {
        let manager = self.clone();
        self.inner.runtime.spawn(async move {
            tokio::time::sleep(TERMINAL_SESSION_RETENTION).await;
            manager.remove_terminal_session(&key, Some(generation));
        });
    }

    fn remove_terminal_session(
        &self,
        key: &PortForwardKey,
        expected_generation: Option<u64>,
    ) -> bool {
        let Ok(mut state) = self.inner.state.lock() else {
            return false;
        };
        let should_remove = state.sessions.get(key).is_some_and(|session| {
            !session.snapshot.state.is_active()
                && expected_generation.is_none_or(|generation| session.generation == generation)
        });
        if !should_remove {
            return false;
        }
        state.sessions.remove(key);
        self.publish_locked(&mut state);
        true
    }

    fn publish_locked(&self, state: &mut ManagerState) {
        state.revision = next_revision(state.revision);
        let mut sessions = state
            .sessions
            .values()
            .map(|session| session.snapshot.clone())
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| left.key.cmp(&right.key));
        self.inner
            .snapshots
            .send_replace(PortForwardManagerSnapshot {
                sessions,
                revision: state.revision,
            });
    }
}

fn push_log(log: &mut Vec<String>, message: String) {
    log.push(message);
    if log.len() > MAX_LOG_ENTRIES {
        log.drain(0..(log.len() - MAX_LOG_ENTRIES));
    }
}

fn should_preserve_catalog_rule(state: &PortForwardRuntimeState) -> bool {
    matches!(
        state,
        PortForwardRuntimeState::Starting
            | PortForwardRuntimeState::Running
            | PortForwardRuntimeState::Reconnecting { .. }
    )
}

fn next_revision(current: u64) -> u64 {
    let next = current.wrapping_add(1);
    if next == 0 { 1 } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::thread;
    use tokio::runtime::Runtime;

    fn test_manager() -> (Runtime, PortForwardManager) {
        let runtime = Runtime::new().expect("test runtime");
        let manager = PortForwardManager::new(
            runtime.handle().clone(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(
                std::env::temp_dir().join("miaominal-port-forward-manager-test-known-hosts"),
            ),
        );
        (runtime, manager)
    }

    fn test_rule() -> PortForwardRule {
        PortForwardRule {
            id: "rule".into(),
            label: String::new(),
            kind: Default::default(),
            listen_host: "127.0.0.1".into(),
            listen_port: 1000,
            target_host: "127.0.0.1".into(),
            target_port: 2000,
            enabled: true,
        }
    }

    fn insert_test_session(
        manager: &PortForwardManager,
        key: PortForwardKey,
        generation: u64,
        state: PortForwardRuntimeState,
    ) {
        let mut rule = test_rule();
        rule.id = key.rule_id.clone();
        let snapshot = PortForwardRuntimeSnapshot {
            key: key.clone(),
            profile_name: "Profile".into(),
            rule,
            state,
            status_message: String::new(),
            log: Vec::new(),
            prompt: None,
            revision: 1,
        };
        let mut manager_state = manager.inner.state.lock().expect("manager state");
        manager_state.sessions.insert(
            key,
            ManagedSession {
                commands: None,
                generation,
                restart_requested: false,
                snapshot,
            },
        );
        manager.publish_locked(&mut manager_state);
    }

    #[test]
    fn active_states_exclude_terminal_states() {
        assert!(PortForwardRuntimeState::Starting.is_active());
        assert!(PortForwardRuntimeState::Running.is_active());
        assert!(PortForwardRuntimeState::Stopping.is_active());
        assert!(!PortForwardRuntimeState::Stopped.is_active());
        assert!(!PortForwardRuntimeState::Failed("failed".into()).is_active());
    }

    #[test]
    fn catalog_runtime_preservation_excludes_stopping_and_terminal_rules() {
        assert!(should_preserve_catalog_rule(
            &PortForwardRuntimeState::Starting
        ));
        assert!(should_preserve_catalog_rule(
            &PortForwardRuntimeState::Running
        ));
        assert!(should_preserve_catalog_rule(
            &PortForwardRuntimeState::Reconnecting {
                error: "offline".into(),
                attempt: 1,
                max_attempts: 3,
                retry_after_secs: 1,
            }
        ));
        assert!(!should_preserve_catalog_rule(
            &PortForwardRuntimeState::Stopping
        ));
        assert!(!should_preserve_catalog_rule(
            &PortForwardRuntimeState::Stopped
        ));
    }

    #[test]
    fn disabled_catalog_does_not_stop_a_running_forward() {
        let (_runtime, manager) = test_manager();
        let key = PortForwardKey::new("profile", "rule");
        insert_test_session(&manager, key, 1, PortForwardRuntimeState::Running);
        let mut profile = SessionProfile::blank("profile", 1);
        let mut rule = test_rule();
        rule.enabled = false;
        profile.port_forwarding_rules.push(rule);

        manager.replace_catalogs(vec![profile], Vec::new());

        let snapshot = manager.snapshot();
        let session = snapshot
            .session("profile", "rule")
            .expect("running session");
        assert!(matches!(session.state, PortForwardRuntimeState::Running));
        assert!(session.rule.enabled);
        let profiles = manager.inner.profiles.read().expect("profile catalog");
        assert!(profiles[0].port_forwarding_rules[0].enabled);
    }

    #[test]
    fn catalog_changes_hot_update_a_running_forward_without_stopping_it() {
        let (_runtime, manager) = test_manager();
        let key = PortForwardKey::new("profile", "rule");
        insert_test_session(&manager, key, 1, PortForwardRuntimeState::Running);
        let mut profile = SessionProfile::blank("profile", 1);
        let mut rule = test_rule();
        rule.enabled = false;
        rule.target_port = 2222;
        profile.port_forwarding_rules.push(rule);

        manager.replace_catalogs(vec![profile], Vec::new());

        let snapshot = manager.snapshot();
        let session = snapshot
            .session("profile", "rule")
            .expect("running session");
        assert!(matches!(session.state, PortForwardRuntimeState::Running));
        assert_eq!(session.rule.target_port, 2222);
        assert!(session.rule.enabled);
    }

    #[test]
    fn disabled_catalog_rule_cancels_a_queued_restart() {
        let (_runtime, manager) = test_manager();
        let key = PortForwardKey::new("profile", "rule");
        insert_test_session(&manager, key.clone(), 1, PortForwardRuntimeState::Stopping);
        {
            manager
                .inner
                .state
                .lock()
                .expect("manager state")
                .sessions
                .get_mut(&key)
                .expect("stopping session")
                .restart_requested = true;
        }
        let mut profile = SessionProfile::blank("profile", 1);
        let mut rule = test_rule();
        rule.enabled = false;
        profile.port_forwarding_rules.push(rule);

        manager.replace_catalogs(vec![profile], Vec::new());

        let state = manager.inner.state.lock().expect("manager state");
        let session = state.sessions.get(&key).expect("stopping session");
        assert!(matches!(
            session.snapshot.state,
            PortForwardRuntimeState::Stopping
        ));
        assert!(!session.restart_requested);
    }

    #[test]
    fn poisoned_profile_catalog_returns_an_explicit_manager_error() {
        let (_runtime, manager) = test_manager();
        let poisoned_manager = manager.clone();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _profiles = poisoned_manager
                .inner
                .profiles
                .write()
                .expect("profile catalog lock");
            panic!("poison profile catalog");
        }));

        let error = manager
            .start("profile", "rule")
            .expect_err("poisoned catalog must fail explicitly");
        assert!(error.to_string().contains("profile catalog is unavailable"));
    }

    #[test]
    fn concurrent_start_reservations_launch_only_one_connection() {
        let (_runtime, manager) = test_manager();
        let barrier = Arc::new(Barrier::new(3));
        let launches = thread::scope(|scope| {
            let mut handles = Vec::new();
            for _ in 0..2 {
                let manager = manager.clone();
                let barrier = barrier.clone();
                handles.push(scope.spawn(move || {
                    barrier.wait();
                    matches!(
                        manager
                            .reserve_start(
                                PortForwardKey::new("profile", "rule"),
                                "Profile".into(),
                                test_rule(),
                            )
                            .expect("start reservation"),
                        StartDecision::Launch { .. }
                    )
                }));
            }
            barrier.wait();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("reservation thread"))
                .collect::<Vec<_>>()
        });

        assert_eq!(launches.into_iter().filter(|launch| *launch).count(), 1);
    }

    #[test]
    fn start_while_stopping_queues_restart_and_explicit_stop_cancels_it() {
        let (_runtime, manager) = test_manager();
        let key = PortForwardKey::new("profile", "rule");
        insert_test_session(&manager, key.clone(), 1, PortForwardRuntimeState::Stopping);

        let decision = manager
            .reserve_start(key.clone(), "Profile".into(), test_rule())
            .expect("start reservation");
        assert!(matches!(decision, StartDecision::Existing(_)));
        {
            let state = manager.inner.state.lock().expect("manager state");
            let session = state.sessions.get(&key).expect("queued session");
            assert!(session.restart_requested);
            assert_eq!(
                session.snapshot.status_message,
                "Restarting after current session stops"
            );
        }

        assert!(manager.stop("profile", "rule"));
        let state = manager.inner.state.lock().expect("manager state");
        assert!(
            !state
                .sessions
                .get(&key)
                .expect("stopping session")
                .restart_requested
        );
    }

    #[test]
    fn closed_stopping_session_with_queued_restart_uses_a_new_generation() {
        let (_runtime, manager) = test_manager();
        let key = PortForwardKey::new("profile", "rule");
        let mut profile = SessionProfile::blank("profile", 1);
        profile.host = "127.0.0.1".into();
        profile.port = 1;
        profile.port_forwarding_rules.push(test_rule());
        manager.replace_catalogs(vec![profile], Vec::new());
        insert_test_session(&manager, key.clone(), 1, PortForwardRuntimeState::Stopping);
        {
            let mut state = manager.inner.state.lock().expect("manager state");
            state.next_generation = 1;
            state
                .sessions
                .get_mut(&key)
                .expect("stopping session")
                .restart_requested = true;
        }

        manager.handle_event(&key, 1, SessionEvent::Closed);

        let state = manager.inner.state.lock().expect("manager state");
        assert_eq!(
            state
                .sessions
                .get(&key)
                .expect("restarted session")
                .generation,
            2
        );
    }

    #[test]
    fn snapshot_finds_session_by_stable_key() {
        let snapshot = PortForwardManagerSnapshot {
            sessions: vec![PortForwardRuntimeSnapshot {
                key: PortForwardKey::new("profile", "rule"),
                profile_name: "Profile".into(),
                rule: PortForwardRule {
                    id: "rule".into(),
                    label: String::new(),
                    kind: Default::default(),
                    listen_host: "127.0.0.1".into(),
                    listen_port: 1000,
                    target_host: "127.0.0.1".into(),
                    target_port: 2000,
                    enabled: true,
                },
                state: PortForwardRuntimeState::Running,
                status_message: String::new(),
                log: Vec::new(),
                prompt: None,
                revision: 1,
            }],
            revision: 1,
        };
        assert!(snapshot.session("profile", "rule").is_some());
        assert!(snapshot.session("profile", "missing").is_none());
    }

    #[test]
    fn terminal_failure_clears_shared_authentication_prompt() {
        let (_runtime, manager) = test_manager();
        let key = PortForwardKey::new("profile", "rule");
        let snapshot = PortForwardRuntimeSnapshot {
            key: key.clone(),
            profile_name: "Profile".into(),
            rule: PortForwardRule {
                id: "rule".into(),
                label: String::new(),
                kind: Default::default(),
                listen_host: "127.0.0.1".into(),
                listen_port: 1000,
                target_host: "127.0.0.1".into(),
                target_port: 2000,
                enabled: true,
            },
            state: PortForwardRuntimeState::Starting,
            status_message: String::new(),
            log: Vec::new(),
            prompt: Some(PortForwardPrompt::HostKey(HostKeyPrompt {
                host: "example.test".into(),
                port: 22,
                algorithm: "ssh-ed25519".into(),
                fingerprint: "SHA256:test".into(),
                previous_fingerprint: None,
            })),
            revision: 1,
        };
        {
            let mut state = manager.inner.state.lock().expect("manager state");
            state.sessions.insert(
                key.clone(),
                ManagedSession {
                    commands: None,
                    generation: 1,
                    restart_requested: false,
                    snapshot,
                },
            );
            manager.publish_locked(&mut state);
        }

        manager.handle_event(&key, 1, SessionEvent::Error("authentication failed".into()));

        let snapshot = manager.snapshot();
        let session = snapshot.session("profile", "rule").expect("session");
        assert!(session.prompt.is_none());
        assert_eq!(
            session.state,
            PortForwardRuntimeState::Failed("authentication failed".into())
        );
    }

    #[test]
    fn stale_host_key_response_does_not_clear_a_newer_keyboard_prompt() {
        let (_runtime, manager) = test_manager();
        let key = PortForwardKey::new("profile", "rule");
        insert_test_session(&manager, key.clone(), 1, PortForwardRuntimeState::Starting);
        {
            let mut state = manager.inner.state.lock().expect("manager state");
            state
                .sessions
                .get_mut(&key)
                .expect("session")
                .snapshot
                .prompt = Some(PortForwardPrompt::KeyboardInteractive(KbiChallenge {
                name: "Authentication".into(),
                instructions: String::new(),
                prompts: Vec::new(),
            }));
            manager.publish_locked(&mut state);
        }

        assert!(
            manager
                .respond_host_key("profile", "rule", HostKeyDecision::AcceptOnce)
                .is_err()
        );
        assert!(matches!(
            manager
                .snapshot()
                .session("profile", "rule")
                .and_then(|session| session.prompt.as_ref()),
            Some(PortForwardPrompt::KeyboardInteractive(_))
        ));
    }

    #[test]
    fn manager_event_path_reconnects_and_restores_running_state() {
        let (_runtime, manager) = test_manager();
        let key = PortForwardKey::new("profile", "rule");
        insert_test_session(&manager, key.clone(), 1, PortForwardRuntimeState::Running);

        manager.handle_event(
            &key,
            1,
            SessionEvent::PortForwardReconnecting {
                error: "transport closed".into(),
                attempt: 2,
                max_attempts: 10,
                retry_after_secs: 4,
            },
        );
        let snapshot = manager.snapshot();
        assert!(matches!(
            snapshot
                .session("profile", "rule")
                .map(|session| &session.state),
            Some(PortForwardRuntimeState::Reconnecting { attempt: 2, .. })
        ));

        manager.handle_event(&key, 1, SessionEvent::Connected("profile-a".into()));
        let snapshot = manager.snapshot();
        assert!(matches!(
            snapshot
                .session("profile", "rule")
                .map(|session| &session.state),
            Some(PortForwardRuntimeState::Running)
        ));
    }

    #[test]
    fn terminal_session_is_removed_only_for_the_expected_generation() {
        let (_runtime, manager) = test_manager();
        let key = PortForwardKey::new("profile", "rule");
        insert_test_session(&manager, key.clone(), 7, PortForwardRuntimeState::Stopped);

        assert!(manager.remove_terminal_session(&key, Some(7)));
        assert!(manager.snapshot().session("profile", "rule").is_none());
    }

    #[test]
    fn stale_cleanup_does_not_remove_a_restarted_generation() {
        let (_runtime, manager) = test_manager();
        let key = PortForwardKey::new("profile", "rule");
        insert_test_session(
            &manager,
            key.clone(),
            8,
            PortForwardRuntimeState::Failed("new generation failed".into()),
        );

        assert!(!manager.remove_terminal_session(&key, Some(7)));
        assert!(manager.snapshot().session("profile", "rule").is_some());
    }

    #[test]
    fn removing_a_rule_evicts_its_terminal_session_immediately() {
        let (_runtime, manager) = test_manager();
        let key = PortForwardKey::new("profile", "rule");
        insert_test_session(&manager, key, 1, PortForwardRuntimeState::Stopped);

        manager.replace_catalogs(Vec::new(), Vec::new());

        assert!(manager.snapshot().session("profile", "rule").is_none());
    }
}
