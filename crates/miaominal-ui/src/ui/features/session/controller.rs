use crate::ui::contracts::{CatalogQueryPort, session::PreparedSessionTab};
use std::{
    cell::{Cell, Ref, RefCell, RefMut},
    collections::{HashMap, HashSet},
    rc::Rc,
    time::SystemTime,
};

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, ScrollHandle, Subscription,
    WeakEntity, Window,
};
use miaominal_agent::{TerminalOutputReceiver, TerminalOutputTap};
use miaominal_core::profile::{AuthMethod, PortForwardKind, PortForwardRule};
use miaominal_core::proxy::ProxyProfile;
use miaominal_secrets::SecretStore;
use miaominal_services::{
    PortForwardManager, PortForwardManagerSnapshot, PortForwardPrompt, PortForwardRuntimeState,
    SshBridgeService, TerminalService,
};
use miaominal_ssh::{
    HostKeyDecision, HostKeyPrompt, KbiChallenge, SessionCommandSender, SessionConnection,
    SessionMonitorSnapshot,
};
use miaominal_storage::known_hosts_store::KnownHostsStore;
use miaominal_terminal::TerminalState;
use tokio::runtime::Handle as TokioHandle;

use crate::ui::components::animation::{TerminalSearchAnimation, WorkspaceSidePanelTransition};
use crate::ui::components::forms::{new_input_state, set_input_placeholder, set_input_value};
use crate::ui::contracts::commands::AppCommand;
use crate::ui::contracts::security::LocalVaultStatus;
use crate::ui::contracts::session::{
    SessionEventOutcome, SessionEventTabRemoval, SessionNotificationRequest,
    SessionNotificationTone, SessionPortSession, SessionPortSnapshot, SessionPurpose,
    SessionQueryPort, SessionQuerySource, SessionSidePanelView, SessionTabDescriptor,
    SessionTabSnapshot, SessionTerminalPort, SessionTerminalRuntime, SessionTerminalTarget,
    TerminalLease, TerminalLeaseError, TerminalLeaseGrant, TerminalLeaseRetirement,
    TransferredSessionTab, TransferredSessionWorkspaceUi,
};
use crate::ui::contracts::workspace::{TabId, TabKindTag, TabPlacement};
use crate::ui::features::session::SessionSftpProgressLayoutState;
use crate::ui::features::session::view::authentication::{
    host_key_overlay_snapshot, keyboard_interactive_overlay_snapshot,
};
use crate::ui::i18n;
use miaominal_core::profile::SessionProfile;

mod authentication;
mod events;
mod forwarding;
mod lifecycle;
mod local_session;
mod progress;
mod search;
mod terminal;

pub(in crate::ui) const SESSION_MONITOR_HISTORY_LIMIT: usize = 900;
pub(in crate::ui) use forwarding::PortForwardSessionStart;

#[derive(Debug, Clone)]
pub(in crate::ui) struct MonitorChartPoint {
    pub(in crate::ui) label: String,
    pub(in crate::ui) value: f64,
}

#[derive(Clone)]
pub(in crate::ui) struct SessionMonitoringState {
    pub(in crate::ui) auto_collect_enabled: bool,
    pub(in crate::ui) last_snapshot: Option<SessionMonitorSnapshot>,
    pub(in crate::ui) last_error: Option<String>,
    pub(in crate::ui) last_updated_at: Option<SystemTime>,
    pub(in crate::ui) cpu_history: Vec<MonitorChartPoint>,
    pub(in crate::ui) memory_history: Vec<MonitorChartPoint>,
    pub(in crate::ui) network_rx_history: Vec<MonitorChartPoint>,
    pub(in crate::ui) network_tx_history: Vec<MonitorChartPoint>,
    pub(in crate::ui) cpu_sample_ready: bool,
    pub(in crate::ui) network_sample_ready: bool,
    rates_warming_up: bool,
    pub(in crate::ui) sample_count: usize,
}

impl SessionMonitoringState {
    pub(in crate::ui) fn new(auto_collect_enabled: bool) -> Self {
        Self {
            auto_collect_enabled,
            last_snapshot: None,
            last_error: None,
            last_updated_at: None,
            cpu_history: Vec::new(),
            memory_history: Vec::new(),
            network_rx_history: Vec::new(),
            network_tx_history: Vec::new(),
            cpu_sample_ready: false,
            network_sample_ready: false,
            rates_warming_up: true,
            sample_count: 0,
        }
    }

    pub(in crate::ui) fn set_enabled(&mut self, enabled: bool) {
        let was_enabled = self.auto_collect_enabled;
        self.auto_collect_enabled = enabled;
        if enabled && !was_enabled {
            self.last_error = None;
        }
        if !enabled || !was_enabled {
            self.mark_rates_warming_up();
        }
    }

    pub(in crate::ui) fn apply_snapshot(&mut self, snapshot: SessionMonitorSnapshot) {
        self.sample_count = self.sample_count.saturating_add(1);
        let label = self.sample_count.to_string();
        let rates_warming_up = self.rates_warming_up;
        self.cpu_sample_ready = !rates_warming_up
            || snapshot.platform != miaominal_core::forwarding::SessionMonitorPlatform::Linux;
        self.network_sample_ready = !rates_warming_up
            || snapshot.platform == miaominal_core::forwarding::SessionMonitorPlatform::Windows;

        Self::push_history_point(&mut self.memory_history, &label, snapshot.memory_percent);
        if self.cpu_sample_ready {
            Self::push_history_point(&mut self.cpu_history, &label, snapshot.cpu_percent);
        }
        if self.network_sample_ready {
            Self::push_history_point(
                &mut self.network_rx_history,
                &label,
                snapshot.network_rx_kbps,
            );
            Self::push_history_point(
                &mut self.network_tx_history,
                &label,
                snapshot.network_tx_kbps,
            );
        }
        self.rates_warming_up = false;

        self.last_snapshot = Some(snapshot);
        self.last_error = None;
        self.last_updated_at = Some(SystemTime::now());
    }

    pub(in crate::ui) fn report_error(&mut self, error: String) {
        self.last_error = Some(error);
        self.mark_rates_warming_up();
    }

    pub(in crate::ui) fn mark_rates_warming_up(&mut self) {
        self.rates_warming_up = true;
    }

    fn push_history_point(history: &mut Vec<MonitorChartPoint>, label: &str, value: f64) {
        history.push(MonitorChartPoint {
            label: label.to_string(),
            value,
        });
        if history.len() > SESSION_MONITOR_HISTORY_LIMIT {
            let overflow = history.len() - SESSION_MONITOR_HISTORY_LIMIT;
            history.drain(0..overflow);
        }
    }
}

pub(in crate::ui::features::session) struct SessionTabState {
    pub(in crate::ui::features::session) profile_id: String,
    pub(in crate::ui::features::session) port_forward_rule_id: Option<String>,
    pub(in crate::ui::features::session) terminal: TerminalState,
    pub(in crate::ui::features::session) connection_state: SessionConnectionState,
    pub(in crate::ui::features::session) preserved_history_popup_hidden: bool,
    pub(in crate::ui::features::session) pending_profile: Option<SessionProfile>,
    pub(in crate::ui::features::session) commands: Option<SessionCommandSender>,
    pub(in crate::ui::features::session) bytes_in: u64,
    pub(in crate::ui::features::session) bytes_out: u64,
    pub(in crate::ui::features::session) pending_host_key: Option<HostKeyPrompt>,
    pub(in crate::ui::features::session) pending_host_key_request_id: u64,
    pub(in crate::ui::features::session) pending_keyboard_interactive: Option<KbiChallenge>,
    pub(in crate::ui::features::session) pending_keyboard_interactive_request_id: u64,
    pub(in crate::ui::features::session) reconnect_task: Option<gpui_kit::Task<()>>,
    pub(in crate::ui::features::session) reconnect_attempt: u32,
    pub(in crate::ui::features::session) has_activity: bool,
    pub(in crate::ui::features::session) monitoring: SessionMonitoringState,
    pub(in crate::ui::features::session) purpose: SessionPurpose,
    pub(in crate::ui::features::session) port_forward_revision: u64,
    pub(in crate::ui::features::session) port_forward_log_len: usize,
    pub(in crate::ui::features::session) sftp_progress_layout: SessionSftpProgressLayoutState,
    pub(in crate::ui::features::session) local_terminal: bool,
    pub(in crate::ui::features::session) owner_route: Option<Entity<SessionTabOwnerRoute>>,
}

impl From<SessionTabState> for PreparedSessionTab {
    fn from(state: SessionTabState) -> Self {
        PreparedSessionTab::new(state)
    }
}

pub(in crate::ui) struct SessionTabOwnerRoute {
    owner: WeakEntity<SessionController>,
}

impl SessionTabOwnerRoute {
    fn new(owner: WeakEntity<SessionController>) -> Self {
        Self { owner }
    }

    fn retarget(&mut self, owner: WeakEntity<SessionController>) {
        self.owner = owner;
    }
}

struct SessionTabHandoff {
    tab_id: TabId,
    state: SessionTabState,
    terminal_lease: Option<TerminalLeaseEntry>,
    shared_monitoring: Option<SessionMonitoringState>,
    was_monitor_source: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum SessionFailureStatus {
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) enum SessionConnectionState {
    Connecting,
    Ready,
    Reconnecting {
        error: String,
        attempt: u32,
    },
    Failed {
        error: String,
        status: Option<SessionFailureStatus>,
    },
    Exited {
        exit_code: u32,
        signal: Option<String>,
    },
    Disconnected,
}

pub(in crate::ui) struct TerminalSearchForms {
    pub(in crate::ui) input: Entity<InputState>,
    pub(in crate::ui) open: bool,
    pub(in crate::ui) visible: bool,
    pub(in crate::ui) visibility: f32,
    pub(in crate::ui) animation: Option<TerminalSearchAnimation>,
    pub(in crate::ui) total: usize,
    pub(in crate::ui) current: Option<usize>,
    pub(in crate::ui) status: Option<String>,
}

pub(in crate::ui) struct SessionWorkspaceForms {
    pub(in crate::ui) search: TerminalSearchForms,
}

impl SessionConnectionState {
    pub(in crate::ui) fn preserves_terminal_history(&self) -> bool {
        matches!(
            self,
            Self::Failed { .. } | Self::Exited { .. } | Self::Disconnected
        )
    }
}

impl SessionTabState {
    pub(in crate::ui) fn set_connection_state(&mut self, connection_state: SessionConnectionState) {
        self.connection_state = connection_state;
        self.preserved_history_popup_hidden = false;
    }

    pub(in crate::ui) fn preserves_terminal_history(&self) -> bool {
        self.purpose == SessionPurpose::Terminal
            && self.connection_state.preserves_terminal_history()
    }

    pub(in crate::ui) fn uses_blocking_placeholder(&self) -> bool {
        !matches!(self.connection_state, SessionConnectionState::Ready)
            && !self.preserves_terminal_history()
    }

    pub(in crate::ui) fn is_terminal_read_only(&self) -> bool {
        self.preserves_terminal_history()
    }

    pub(in crate::ui) fn preserved_history_popup_hidden(&self) -> bool {
        self.preserves_terminal_history() && self.preserved_history_popup_hidden
    }

    pub(in crate::ui) fn hide_preserved_history_popup(&mut self) {
        if self.preserves_terminal_history() {
            self.preserved_history_popup_hidden = true;
        }
    }
}

fn localized_port_forward_kind_label(kind: PortForwardKind) -> String {
    match kind {
        PortForwardKind::Local => i18n::string("forwarding.editor.local"),
        PortForwardKind::Remote => i18n::string("forwarding.editor.remote"),
    }
}

pub(in crate::ui) struct SessionController {
    services: SessionControllerServices,
    profiles: RefCell<Vec<SessionProfile>>,
    // Phase 2: drain only successful persistence; redraws never republish profile snapshots.
    proxies: RefCell<Vec<ProxyProfile>>,
    selected_profile: Cell<Option<usize>>,
    forms: Option<RefCell<SessionWorkspaceForms>>,
    kbi_inputs: RefCell<Vec<Entity<InputState>>>,
    kbi_input_target: Cell<Option<(TabId, u64)>>,
    search_target: Cell<Option<TabId>>,
    tabs: RefCell<HashMap<TabId, SessionTabState>>,
    shared_profile_monitoring: RefCell<HashMap<String, SessionMonitoringState>>,
    monitor_source_tabs: RefCell<HashMap<String, TabId>>,
    monitor_scroll_handle: ScrollHandle,
    reported_terminal_focus_tab_id: RefCell<Option<TabId>>,
    panel: RefCell<SessionPanelState>,
    ports: Rc<RefCell<SessionPortState>>,
    terminal_focus_subscriptions: RefCell<Option<(Subscription, Subscription)>>,
    _subscriptions: Vec<Subscription>,
}

pub(in crate::ui) struct SessionControllerArgs {
    pub(in crate::ui) runtime: TokioHandle,
    pub(in crate::ui) secrets: SecretStore,
    pub(in crate::ui) known_hosts: KnownHostsStore,
    pub(in crate::ui) profiles: Vec<SessionProfile>,
    pub(in crate::ui) proxies: Vec<ProxyProfile>,
    pub(in crate::ui) selected_profile: Option<usize>,
    pub(in crate::ui) terminal_focus: FocusHandle,
    pub(in crate::ui) local_vault_status: LocalVaultStatus,
    pub(in crate::ui) auto_collect_session_monitoring: bool,
    pub(in crate::ui) ssh_bridge_service: SshBridgeService,
    pub(in crate::ui) port_forward_manager: PortForwardManager,
}

struct SessionFormsBundle {
    workspace: SessionWorkspaceForms,
}

struct SessionControllerServices {
    runtime: Option<TokioHandle>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    local_vault_status: LocalVaultStatus,
    auto_collect_session_monitoring: Cell<bool>,
    ssh_bridge_service: Option<SshBridgeService>,
    port_forward_manager: Option<PortForwardManager>,
}

#[derive(Default)]
struct SessionPanelState {
    open: bool,
    view: SessionSidePanelView,
    visible: bool,
    transition: Option<WorkspaceSidePanelTransition>,
}

fn dedicated_port_forward_rules(
    rule_id: Option<&str>,
    rules: &[PortForwardRule],
) -> Vec<PortForwardRule> {
    let Some(rule_id) = rule_id else {
        return Vec::new();
    };

    rules
        .iter()
        .filter(|rule| rule.id == rule_id)
        .cloned()
        .map(|mut rule| {
            // The dedicated session is the source of truth while it is connecting. The
            // persisted flag is only committed after Connected, so it may still be false here.
            rule.enabled = true;
            rule
        })
        .collect()
}

impl SessionController {
    fn build_forms(window: &mut Window, cx: &mut Context<Self>) -> SessionFormsBundle {
        SessionFormsBundle {
            workspace: SessionWorkspaceForms {
                search: TerminalSearchForms {
                    input: new_input_state(
                        i18n::string("placeholders.workspace.search_scrollback"),
                        "",
                        false,
                        window,
                        cx,
                    ),
                    open: false,
                    visible: false,
                    visibility: 0.0,
                    animation: None,
                    total: 0,
                    current: None,
                    status: None,
                },
            },
        }
    }

    pub(in crate::ui) fn refresh_localized_placeholders(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        set_input_placeholder(
            &self.terminal_search_input(),
            i18n::string("placeholders.workspace.search_scrollback"),
            window,
            cx,
        );
    }

    pub(in crate::ui) fn new(
        args: SessionControllerArgs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let forms = Self::build_forms(window, cx);
        let terminal_search_input = forms.workspace.search.input.clone();
        let terminal_focus_in_subscription =
            cx.on_focus_in(&args.terminal_focus, window, |_controller, _window, cx| {
                cx.emit(AppCommand::TerminalFocusReportingRequested);
            });
        let terminal_focus_out_subscription = cx.on_focus_out(
            &args.terminal_focus,
            window,
            |_controller, _, _window, cx| {
                cx.emit(AppCommand::TerminalFocusReportingRequested);
            },
        );
        let mut subscriptions = vec![cx.subscribe(
            &terminal_search_input,
            |controller, input, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    let value = input.read(cx).value().to_string();
                    controller.update_terminal_search(value, cx);
                }
                InputEvent::PressEnter {
                    secondary,
                    shift: _,
                } => {
                    if *secondary {
                        controller.terminal_search_prev(cx);
                    } else {
                        controller.terminal_search_next(cx);
                    }
                }
                _ => {}
            },
        )];
        subscriptions.push(
            cx.observe_window_activation(window, |_controller, window, cx| {
                cx.emit(AppCommand::WindowActivationChanged {
                    active: window.is_window_active(),
                });
            }),
        );
        let mut controller = Self::with_subscriptions(
            SessionControllerServices {
                runtime: Some(args.runtime),
                secrets: args.secrets,
                known_hosts: args.known_hosts,
                local_vault_status: args.local_vault_status,
                auto_collect_session_monitoring: Cell::new(args.auto_collect_session_monitoring),
                ssh_bridge_service: Some(args.ssh_bridge_service),
                port_forward_manager: Some(args.port_forward_manager),
            },
            args.profiles,
            args.proxies,
            args.selected_profile,
            Some(forms.workspace),
            subscriptions,
        );
        controller.ports.borrow_mut().owner = Some(cx.entity().downgrade());
        *controller.terminal_focus_subscriptions.get_mut() = Some((
            terminal_focus_in_subscription,
            terminal_focus_out_subscription,
        ));
        controller
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the testable constructor receives each independently owned controller state component explicitly"
    )]
    fn with_subscriptions(
        services: SessionControllerServices,
        profiles: Vec<SessionProfile>,
        proxies: Vec<ProxyProfile>,
        selected_profile: Option<usize>,
        forms: Option<SessionWorkspaceForms>,
        subscriptions: Vec<Subscription>,
    ) -> Self {
        let port_profiles = profiles.clone();
        Self {
            services,
            profiles: RefCell::new(profiles),
            proxies: RefCell::new(proxies),
            selected_profile: Cell::new(selected_profile),
            forms: forms.map(RefCell::new),
            kbi_inputs: RefCell::new(Vec::new()),
            kbi_input_target: Cell::new(None),
            search_target: Cell::new(None),
            tabs: RefCell::new(HashMap::new()),
            shared_profile_monitoring: RefCell::new(HashMap::new()),
            monitor_source_tabs: RefCell::new(HashMap::new()),
            monitor_scroll_handle: ScrollHandle::new(),
            reported_terminal_focus_tab_id: RefCell::new(None),
            panel: RefCell::new(SessionPanelState::default()),
            ports: Rc::new(RefCell::new(SessionPortState {
                snapshot: SessionPortSnapshot::new(port_profiles, Vec::new(), None, None),
                ..SessionPortState::default()
            })),
            terminal_focus_subscriptions: RefCell::new(None),
            _subscriptions: subscriptions,
        }
    }

    pub(in crate::ui) fn rebind_terminal_focus_events(
        &mut self,
        terminal_focus: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus_in = cx.on_focus_in(&terminal_focus, window, |_controller, _window, cx| {
            cx.emit(AppCommand::TerminalFocusReportingRequested);
        });
        let focus_out = cx.on_focus_out(&terminal_focus, window, |_controller, _, _window, cx| {
            cx.emit(AppCommand::TerminalFocusReportingRequested);
        });
        *self.terminal_focus_subscriptions.borrow_mut() = Some((focus_in, focus_out));
    }

    #[cfg(test)]
    fn new_for_test() -> Self {
        Self::new_for_test_with_profiles(Vec::new())
    }

    #[cfg(test)]
    fn new_for_test_with_profiles(profiles: Vec<SessionProfile>) -> Self {
        Self::with_subscriptions(
            SessionControllerServices {
                runtime: None,
                secrets: SecretStore::new_locked_vault(),
                known_hosts: KnownHostsStore::with_path(
                    std::env::temp_dir().join("miaominal-session-controller-test-known-hosts"),
                ),
                local_vault_status: LocalVaultStatus::Locked,
                auto_collect_session_monitoring: Cell::new(false),
                ssh_bridge_service: None,
                port_forward_manager: None,
            },
            profiles,
            Vec::new(),
            None,
            None,
            Vec::new(),
        )
    }

    fn forms(&self) -> &RefCell<SessionWorkspaceForms> {
        self.forms
            .as_ref()
            .expect("session workspace forms are available in the application")
    }

    pub(in crate::ui) fn profiles(&self) -> Ref<'_, Vec<SessionProfile>> {
        self.profiles.borrow()
    }

    pub(in crate::ui) fn replace_profiles_from_application(&self, profiles: Vec<SessionProfile>) {
        *self.profiles.borrow_mut() = profiles;
        self.sync_port_profiles();
    }

    pub(in crate::ui) fn replace_proxies(&self, proxies: Vec<ProxyProfile>) {
        self.replace_proxies_from_application(proxies);
    }

    pub(in crate::ui) fn replace_proxies_from_application(&self, proxies: Vec<ProxyProfile>) {
        *self.proxies.borrow_mut() = proxies;
    }

    pub(in crate::ui) fn set_selected_profile(&self, selected: Option<usize>) {
        self.selected_profile.set(selected);
    }

    fn terminal_service(&self) -> TerminalService {
        TerminalService::new(
            self.services
                .runtime
                .as_ref()
                .expect("session runtime is available in the application")
                .clone(),
            self.services.secrets.clone(),
            self.services.known_hosts.clone(),
        )
    }

    pub(in crate::ui) fn start_terminal_session(
        &self,
        profile: SessionProfile,
        columns: usize,
        lines: usize,
        monitoring_enabled: bool,
    ) -> SessionConnection {
        if profile.is_local() {
            return local_session::start_local_session(profile, columns, lines);
        }
        self.terminal_service().start_session(
            profile,
            self.profiles.borrow().clone(),
            self.proxies.borrow().clone(),
            columns,
            lines,
            monitoring_enabled,
        )
    }

    pub(in crate::ui) fn localized_port_forward_kind_label(kind: PortForwardKind) -> String {
        localized_port_forward_kind_label(kind)
    }

    pub(in crate::ui) fn rule_summary_label(rule: &PortForwardRule) -> String {
        let label = rule.label.trim();
        if !label.is_empty() {
            return label.to_string();
        }

        format!(
            "{} {}:{} -> {}:{}",
            Self::localized_port_forward_kind_label(rule.kind),
            rule.listen_host,
            rule.listen_port,
            rule.target_host,
            rule.target_port
        )
    }

    pub(in crate::ui) fn port_forward_rule_indices(
        &self,
        profile_id: &str,
        rule_id: &str,
    ) -> Option<(usize, usize)> {
        self.profiles
            .borrow()
            .iter()
            .enumerate()
            .find_map(|(profile_index, profile)| {
                (profile.id == profile_id)
                    .then_some(profile)
                    .and_then(|profile| {
                        profile
                            .port_forwarding_rules
                            .iter()
                            .position(|rule| rule.id == rule_id)
                            .map(|rule_index| (profile_index, rule_index))
                    })
            })
    }

    pub(in crate::ui) fn sync_current_port_forward_rules_for_profile(
        &self,
        profile_id: &str,
    ) -> usize {
        let Some(rules) = self
            .profiles
            .borrow()
            .iter()
            .find(|profile| profile.id == profile_id)
            .map(|profile| profile.port_forwarding_rules.clone())
        else {
            return 0;
        };

        self.sync_port_forward_rules_for_profile(profile_id, &rules)
    }

    pub(in crate::ui) fn synced_sessions_suffix(count: usize) -> String {
        if count == 0 {
            String::new()
        } else {
            let count = count.to_string();
            if count == "1" {
                i18n::string_args(
                    "forwarding.messages.synced_suffix_one",
                    &[("count", &count)],
                )
            } else {
                i18n::string_args(
                    "forwarding.messages.synced_suffix_other",
                    &[("count", &count)],
                )
            }
        }
    }

    pub(in crate::ui) fn terminal_search(&self) -> Ref<'_, TerminalSearchForms> {
        Ref::map(self.forms().borrow(), |forms| &forms.search)
    }

    pub(in crate::ui::features::session) fn terminal_search_mut(
        &self,
    ) -> RefMut<'_, TerminalSearchForms> {
        RefMut::map(self.forms().borrow_mut(), |forms| &mut forms.search)
    }

    pub(in crate::ui) fn terminal_search_input(&self) -> Entity<InputState> {
        self.forms().borrow().search.input.clone()
    }

    pub(in crate::ui) fn terminal_search_open(&self) -> bool {
        self.forms().borrow().search.open
    }

    pub(in crate::ui) fn build_local_tab(
        id: TabId,
        profile: SessionProfile,
        terminal: TerminalState,
    ) -> (SessionTabDescriptor, PreparedSessionTab) {
        let title = if profile.name.trim().is_empty() {
            local_session::local_terminal_label(&profile.local_shell)
        } else {
            profile.name.clone()
        };
        let profile_id = profile.id.clone();

        let session = SessionTabState {
            profile_id,
            port_forward_rule_id: None,
            terminal,
            connection_state: SessionConnectionState::Connecting,
            preserved_history_popup_hidden: false,
            pending_profile: Some(profile),
            commands: None,
            bytes_in: 0,
            bytes_out: 0,
            pending_host_key: None,
            pending_host_key_request_id: 0,
            pending_keyboard_interactive: None,
            pending_keyboard_interactive_request_id: 0,
            reconnect_task: None,
            reconnect_attempt: 0,
            has_activity: false,
            monitoring: SessionMonitoringState::new(false),
            purpose: SessionPurpose::Terminal,
            port_forward_revision: 0,
            port_forward_log_len: 0,
            sftp_progress_layout: SessionSftpProgressLayoutState::default(),
            local_terminal: true,
            owner_route: None,
        };
        (
            SessionTabDescriptor::new(
                id,
                title,
                i18n::string("tabs.initial.local_terminal_starting"),
                TabKindTag::LocalTerminal,
                TabPlacement::TopLevel,
            ),
            session.into(),
        )
    }

    pub(in crate::ui) fn build_pending_tab(
        id: TabId,
        profile: SessionProfile,
        terminal: TerminalState,
        auto_collect_monitoring: bool,
    ) -> (SessionTabDescriptor, PreparedSessionTab) {
        let title = profile.name.clone();
        let connection = profile.connection_label();
        let status = i18n::string_args(
            "tabs.initial.session_connecting_to",
            &[("connection", &connection)],
        );
        let profile_id = profile.id.clone();

        let session = SessionTabState {
            profile_id,
            port_forward_rule_id: None,
            terminal,
            connection_state: SessionConnectionState::Connecting,
            preserved_history_popup_hidden: false,
            pending_profile: Some(profile),
            commands: None,
            bytes_in: 0,
            bytes_out: 0,
            pending_host_key: None,
            pending_host_key_request_id: 0,
            pending_keyboard_interactive: None,
            pending_keyboard_interactive_request_id: 0,
            reconnect_task: None,
            reconnect_attempt: 0,
            has_activity: false,
            monitoring: SessionMonitoringState::new(auto_collect_monitoring),
            purpose: SessionPurpose::Terminal,
            port_forward_revision: 0,
            port_forward_log_len: 0,
            sftp_progress_layout: SessionSftpProgressLayoutState::default(),
            local_terminal: false,
            owner_route: None,
        };
        (
            SessionTabDescriptor::new(
                id,
                title,
                status,
                TabKindTag::Session,
                TabPlacement::TopLevel,
            ),
            session.into(),
        )
    }

    pub(in crate::ui) fn build_port_forwarding_tab(
        id: TabId,
        profile: &SessionProfile,
        rule: &PortForwardRule,
        commands: Option<SessionCommandSender>,
    ) -> (SessionTabDescriptor, PreparedSessionTab) {
        let kind = localized_port_forward_kind_label(rule.kind);
        let listen_port = rule.listen_port.to_string();
        let target_port = rule.target_port.to_string();
        let title = if rule.label.trim().is_empty() {
            i18n::string_args(
                "tabs.initial.port_forward_title",
                &[
                    ("kind", &kind),
                    ("listen_host", &rule.listen_host),
                    ("listen_port", &listen_port),
                    ("target_host", &rule.target_host),
                    ("target_port", &target_port),
                ],
            )
        } else {
            i18n::string_args(
                "tabs.initial.port_forward_named_title",
                &[("label", &rule.label)],
            )
        };
        let profile_summary = profile.summary();

        let session = SessionTabState {
            profile_id: profile.id.clone(),
            port_forward_rule_id: Some(rule.id.clone()),
            terminal: TerminalState::default(),
            connection_state: SessionConnectionState::Connecting,
            preserved_history_popup_hidden: false,
            pending_profile: None,
            commands,
            bytes_in: 0,
            bytes_out: 0,
            pending_host_key: None,
            pending_host_key_request_id: 0,
            pending_keyboard_interactive: None,
            pending_keyboard_interactive_request_id: 0,
            reconnect_task: None,
            reconnect_attempt: 0,
            has_activity: false,
            monitoring: SessionMonitoringState::new(false),
            purpose: SessionPurpose::PortForwarding,
            port_forward_revision: 0,
            port_forward_log_len: 0,
            sftp_progress_layout: SessionSftpProgressLayoutState::default(),
            local_terminal: false,
            owner_route: None,
        };
        (
            SessionTabDescriptor::new(
                id,
                title,
                i18n::string_args(
                    "tabs.initial.port_forward_connecting_to",
                    &[("profile", &profile_summary)],
                ),
                TabKindTag::Session,
                TabPlacement::Background,
            ),
            session.into(),
        )
    }

    pub(in crate::ui) fn build_connection_test_tab(
        id: TabId,
        profile: &SessionProfile,
        commands: SessionCommandSender,
    ) -> (SessionTabDescriptor, PreparedSessionTab) {
        let profile_summary = profile.summary();

        let session = SessionTabState {
            profile_id: profile.id.clone(),
            port_forward_rule_id: None,
            terminal: TerminalState::default(),
            connection_state: SessionConnectionState::Connecting,
            preserved_history_popup_hidden: false,
            pending_profile: None,
            commands: Some(commands),
            bytes_in: 0,
            bytes_out: 0,
            pending_host_key: None,
            pending_host_key_request_id: 0,
            pending_keyboard_interactive: None,
            pending_keyboard_interactive_request_id: 0,
            reconnect_task: None,
            reconnect_attempt: 0,
            has_activity: false,
            monitoring: SessionMonitoringState::new(false),
            purpose: SessionPurpose::ConnectionTest,
            port_forward_revision: 0,
            port_forward_log_len: 0,
            sftp_progress_layout: SessionSftpProgressLayoutState::default(),
            local_terminal: false,
            owner_route: None,
        };
        (
            SessionTabDescriptor::new(
                id,
                i18n::string_args(
                    "tabs.initial.connection_test_title",
                    &[("profile", &profile_summary)],
                ),
                i18n::string_args(
                    "tabs.initial.connection_test_status",
                    &[("profile", &profile_summary)],
                ),
                TabKindTag::Session,
                TabPlacement::Background,
            ),
            session.into(),
        )
    }

    pub(in crate::ui) fn contains_tab(&self, tab_id: TabId) -> bool {
        self.tabs.borrow().contains_key(&tab_id)
    }
    pub(in crate::ui) fn tab_snapshot(&self, tab_id: TabId) -> Option<SessionTabSnapshot> {
        let session = self.tab(tab_id)?;
        let error = match &session.connection_state {
            SessionConnectionState::Failed { .. } | SessionConnectionState::Disconnected => true,
            SessionConnectionState::Exited { exit_code, signal } => {
                *exit_code != 0 || signal.is_some()
            }
            _ => false,
        };
        Some(
            SessionTabSnapshot::new(
                session.profile_id.clone(),
                session.pending_profile.clone(),
                session.purpose,
            )
            .with_connection(
                matches!(session.connection_state, SessionConnectionState::Ready),
                error,
                session.pending_host_key.is_some(),
            )
            .with_activity(session.has_activity)
            .with_terminal(
                session.terminal.columns(),
                session.terminal.screen_lines(),
                session.bytes_in,
                session.bytes_out,
            )
            .with_sftp_progress_visible(session.sftp_progress_layout.visible()),
        )
    }
    pub(in crate::ui) fn port_session(
        &self,
        tab_id: TabId,
        title: String,
    ) -> Option<SessionPortSession> {
        let session = self.tab(tab_id)?;
        Some(SessionPortSession::new(
            tab_id,
            title,
            session.profile_id.clone(),
            session.pending_profile.clone(),
            session.purpose,
            session.commands.clone(),
        ))
    }
    pub(in crate::ui) fn terminal_monitor_profile(&self, tab_id: TabId) -> Option<String> {
        let session = self.tab(tab_id)?;
        (session.purpose == SessionPurpose::Terminal).then(|| session.profile_id.clone())
    }

    pub(in crate::ui::features::session) fn tab(
        &self,
        tab_id: TabId,
    ) -> Option<Ref<'_, SessionTabState>> {
        Ref::filter_map(self.tabs.borrow(), |tabs| tabs.get(&tab_id)).ok()
    }

    pub(in crate::ui::features::session) fn tab_mut(
        &self,
        tab_id: TabId,
    ) -> Option<RefMut<'_, SessionTabState>> {
        RefMut::filter_map(self.tabs.borrow_mut(), |tabs| tabs.get_mut(&tab_id)).ok()
    }

    pub(super) fn ensure_tab_owner_route(
        &self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) -> Option<Entity<SessionTabOwnerRoute>> {
        let owner = cx.weak_entity();
        let mut session = self.tab_mut(tab_id)?;
        if let Some(route) = session.owner_route.as_ref() {
            route.update(cx, |route, _| route.retarget(owner));
            return Some(route.clone());
        }
        let route = cx.new(|_| SessionTabOwnerRoute::new(owner));
        session.owner_route = Some(route.clone());
        Some(route)
    }

    pub(in crate::ui) fn insert_tab(&self, tab_id: TabId, tab: impl Into<PreparedSessionTab>) {
        let tab = tab.into().into_payload::<SessionTabState>();
        assert!(
            self.tabs.borrow_mut().insert(tab_id, tab).is_none(),
            "duplicate session tab payload for {tab_id}"
        );
    }

    pub(in crate::ui) fn take_tab_for_transfer(
        &self,
        tab_id: TabId,
    ) -> Option<TransferredSessionTab> {
        let state = self.tabs.borrow_mut().remove(&tab_id)?;
        let profile_id = state.profile_id.clone();
        let is_terminal = state.purpose == SessionPurpose::Terminal;
        let shared_monitoring = is_terminal
            .then(|| {
                self.shared_profile_monitoring
                    .borrow()
                    .get(&profile_id)
                    .cloned()
            })
            .flatten();
        let was_monitor_source = is_terminal
            && self.monitor_source_tabs.borrow().get(&profile_id).copied() == Some(tab_id);
        if was_monitor_source {
            self.monitor_source_tabs.borrow_mut().remove(&profile_id);
        }
        let has_remaining_profile_tab = is_terminal
            && self.tabs.borrow().values().any(|session| {
                session.purpose == SessionPurpose::Terminal && session.profile_id == profile_id
            });
        if is_terminal && !has_remaining_profile_tab {
            self.shared_profile_monitoring
                .borrow_mut()
                .remove(&profile_id);
            self.monitor_source_tabs.borrow_mut().remove(&profile_id);
        }
        let mut ports = self.ports.borrow_mut();
        let terminal_lease = ports.terminal_leases.remove(&tab_id);
        ports.snapshot.sessions.remove(&tab_id);
        ports.snapshot.terminal_order.retain(|id| *id != tab_id);
        if ports.snapshot.active_terminal_tab_id == Some(tab_id) {
            ports.snapshot.active_terminal_tab_id = None;
        }
        Some(TransferredSessionTab::new(SessionTabHandoff {
            tab_id,
            state,
            terminal_lease,
            shared_monitoring,
            was_monitor_source,
        }))
    }

    pub(in crate::ui) fn insert_transferred_tab(
        &mut self,
        transferred: TransferredSessionTab,
        cx: &mut Context<Self>,
    ) -> TabId {
        let SessionTabHandoff {
            tab_id,
            state,
            terminal_lease,
            shared_monitoring,
            was_monitor_source,
        } = transferred.into_payload::<SessionTabHandoff>();
        let profile_id = state.profile_id.clone();
        if let Some(route) = state.owner_route.as_ref() {
            let owner = cx.weak_entity();
            route.update(cx, |route, _| route.retarget(owner));
        }
        assert!(
            self.tabs.borrow_mut().insert(tab_id, state).is_none(),
            "duplicate transferred session tab payload for {tab_id}"
        );
        if let Some(shared_monitoring) = shared_monitoring {
            self.shared_profile_monitoring
                .borrow_mut()
                .insert(profile_id.clone(), shared_monitoring);
        }
        if was_monitor_source {
            self.monitor_source_tabs
                .borrow_mut()
                .insert(profile_id, tab_id);
        }
        if let Some(terminal_lease) = terminal_lease {
            *terminal_lease.owner.borrow_mut() = Rc::downgrade(&self.ports);
            if terminal_lease.retire_requested.get() && terminal_lease.tap.can_release_lease() {
                terminal_lease.tap.close();
            } else {
                self.ports
                    .borrow_mut()
                    .terminal_leases
                    .insert(tab_id, terminal_lease);
            }
        }
        tab_id
    }

    pub(in crate::ui) fn close_tab(&self, tab_id: TabId) -> bool {
        self.remove_tab(tab_id).is_some()
    }

    pub(in crate::ui::features::session) fn remove_tab(
        &self,
        tab_id: TabId,
    ) -> Option<SessionTabState> {
        let removed = self.tabs.borrow_mut().remove(&tab_id);
        if removed.is_some() {
            self.terminal_port().close_session(tab_id);
            let mut ports = self.ports.borrow_mut();
            ports.snapshot.sessions.remove(&tab_id);
            ports.snapshot.terminal_order.retain(|id| *id != tab_id);
            if ports.snapshot.active_terminal_tab_id == Some(tab_id) {
                ports.snapshot.active_terminal_tab_id = None;
            }
        }
        removed
    }

    pub(in crate::ui) fn monitor_scroll_handle(&self) -> ScrollHandle {
        self.monitor_scroll_handle.clone()
    }

    pub(in crate::ui) fn reported_terminal_focus_tab_id(&self) -> Option<TabId> {
        *self.reported_terminal_focus_tab_id.borrow()
    }

    pub(in crate::ui) fn take_reported_terminal_focus_tab_id(&self) -> Option<TabId> {
        self.reported_terminal_focus_tab_id.borrow_mut().take()
    }

    pub(in crate::ui) fn set_reported_terminal_focus_tab_id(&self, tab_id: Option<TabId>) {
        *self.reported_terminal_focus_tab_id.borrow_mut() = tab_id;
    }

    pub(in crate::ui) fn side_panel_open(&self) -> bool {
        self.panel.borrow().open
    }

    pub(in crate::ui) fn set_side_panel_open(&self, open: bool) {
        self.panel.borrow_mut().open = open;
    }

    pub(in crate::ui) fn toggle_side_panel(&self) {
        let mut panel = self.panel.borrow_mut();
        panel.open = !panel.open;
    }

    pub(in crate::ui) fn side_panel_view(&self) -> SessionSidePanelView {
        self.panel.borrow().view
    }

    pub(in crate::ui) fn set_side_panel_view(&self, view: SessionSidePanelView) {
        self.panel.borrow_mut().view = view;
    }

    pub(in crate::ui) fn side_panel_transition_state(
        &self,
    ) -> (bool, Option<WorkspaceSidePanelTransition>) {
        let panel = self.panel.borrow();
        (panel.visible, panel.transition)
    }

    pub(in crate::ui) fn set_side_panel_transition_state(
        &self,
        visible: bool,
        transition: Option<WorkspaceSidePanelTransition>,
    ) {
        let mut panel = self.panel.borrow_mut();
        panel.visible = visible;
        panel.transition = transition;
    }

    pub(in crate::ui) fn take_workspace_ui_for_transfer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TransferredSessionWorkspaceUi {
        let (side_panel_open, side_panel_view) = {
            let mut panel = self.panel.borrow_mut();
            let state = (panel.open, panel.view);
            panel.open = false;
            panel.view = SessionSidePanelView::default();
            panel.visible = false;
            panel.transition = None;
            state
        };

        let (
            terminal_search_input,
            terminal_search_query,
            terminal_search_open,
            terminal_search_total,
            terminal_search_current,
            terminal_search_status,
        ) = {
            let mut forms = self.forms().borrow_mut();
            let terminal_search_query = forms.search.input.read(cx).value().to_string();
            let state = (
                forms.search.input.clone(),
                terminal_search_query,
                forms.search.open,
                forms.search.total,
                forms.search.current,
                forms.search.status.clone(),
            );
            forms.search.open = false;
            forms.search.visible = false;
            forms.search.visibility = 0.0;
            forms.search.animation = None;
            forms.search.total = 0;
            forms.search.current = None;
            forms.search.status = None;
            state
        };

        self.search_target.set(None);
        set_input_value(&terminal_search_input, "", window, cx);
        let keyboard_interactive_values = self
            .keyboard_interactive_inputs()
            .into_iter()
            .map(|input| input.read(cx).value().to_string())
            .collect();

        TransferredSessionWorkspaceUi {
            side_panel_open,
            side_panel_view,
            terminal_search_query,
            terminal_search_open,
            terminal_search_total,
            terminal_search_current,
            terminal_search_status,
            keyboard_interactive_values,
            keyboard_interactive_target: self.kbi_input_target.get(),
        }
    }

    pub(in crate::ui) fn restore_workspace_ui_after_transfer(
        &mut self,
        transferred: TransferredSessionWorkspaceUi,
        active_tab_id: Option<TabId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let TransferredSessionWorkspaceUi {
            side_panel_open,
            side_panel_view,
            terminal_search_query,
            terminal_search_open,
            terminal_search_total,
            terminal_search_current,
            terminal_search_status,
            keyboard_interactive_values,
            keyboard_interactive_target,
        } = transferred;

        {
            let mut panel = self.panel.borrow_mut();
            panel.open = side_panel_open;
            panel.view = side_panel_view;
            panel.visible = side_panel_open;
            panel.transition = None;
        }

        let terminal_search_input = {
            let mut forms = self.forms().borrow_mut();
            forms.search.open = terminal_search_open;
            forms.search.visible = terminal_search_open;
            forms.search.visibility = if terminal_search_open { 1.0 } else { 0.0 };
            forms.search.animation = None;
            forms.search.total = terminal_search_total;
            forms.search.current = terminal_search_current;
            forms.search.status = terminal_search_status;
            forms.search.input.clone()
        };

        self.search_target.set(None);
        set_input_value(&terminal_search_input, terminal_search_query, window, cx);
        self.search_target
            .set(terminal_search_open.then_some(active_tab_id).flatten());

        self.sync_visible_authentication_inputs(active_tab_id, &[], false, window, cx);
        for (input, value) in self
            .keyboard_interactive_inputs()
            .into_iter()
            .zip(keyboard_interactive_values)
        {
            if keyboard_interactive_target == self.kbi_input_target.get() {
                set_input_value(&input, value, window, cx);
            }
        }
    }

    pub(in crate::ui) fn keyboard_interactive_inputs(&self) -> Vec<Entity<InputState>> {
        self.kbi_inputs.borrow().clone()
    }

    pub(in crate::ui) fn resolve_host_key_prompt(
        &self,
        tab_id: TabId,
        decision: HostKeyDecision,
        cx: &mut Context<Self>,
    ) {
        let Some((prompt, request_id, commands, port_forward_key)) =
            self.tab_mut(tab_id).and_then(|mut session| {
                let prompt = session.pending_host_key.take()?;
                let request_id = session.pending_host_key_request_id;
                session.pending_host_key_request_id = 0;
                let port_forward_key = (session.purpose == SessionPurpose::PortForwarding)
                    .then(|| {
                        session
                            .port_forward_rule_id
                            .clone()
                            .map(|rule_id| (session.profile_id.clone(), rule_id))
                    })
                    .flatten();
                Some((
                    prompt,
                    request_id,
                    session.commands.clone(),
                    port_forward_key,
                ))
            })
        else {
            return;
        };

        cx.emit(AppCommand::OverlayDismissed(host_key_overlay_snapshot(
            cx.entity(),
            tab_id,
            request_id,
            prompt.clone(),
        )));
        let result = if let Some((profile_id, rule_id)) = port_forward_key
            && let Some(manager) = self.services.port_forward_manager.as_ref()
        {
            manager.respond_host_key(&profile_id, &rule_id, decision)
        } else if let Some(commands) = commands {
            commands.respond_host_key(request_id, decision)
        } else {
            return;
        };
        if let Err(error) = result {
            log::warn!("failed to deliver host key decision: {error:?}");
        }

        let message = match decision {
            HostKeyDecision::AcceptOnce => i18n::string_args(
                "session.messages.accepted_host_key_session_only",
                &[("host", &prompt.host)],
            ),
            HostKeyDecision::AcceptAndSave => {
                self.publish_authenticated_known_hosts(cx);
                i18n::string_args(
                    "session.messages.trusting_host_key",
                    &[("host", &prompt.host)],
                )
            }
            HostKeyDecision::Reject => i18n::string_args(
                "session.messages.rejected_host_key",
                &[("host", &prompt.host)],
            ),
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    fn publish_authenticated_known_hosts(&self, cx: &mut Context<Self>) {
        match self.services.known_hosts.list() {
            Ok(_) => cx.emit(AppCommand::KnownHostsChanged),
            Err(error) => log::warn!("failed to read authenticated known hosts: {error:?}"),
        }
    }

    pub(in crate::ui) fn submit_keyboard_interactive(
        &self,
        tab_id: TabId,
        responses: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let Some((challenge, request_id, commands, port_forward_key)) =
            self.tab_mut(tab_id).and_then(|mut session| {
                let challenge = session.pending_keyboard_interactive.take()?;
                let request_id = session.pending_keyboard_interactive_request_id;
                session.pending_keyboard_interactive_request_id = 0;
                let port_forward_key = (session.purpose == SessionPurpose::PortForwarding)
                    .then(|| {
                        session
                            .port_forward_rule_id
                            .clone()
                            .map(|rule_id| (session.profile_id.clone(), rule_id))
                    })
                    .flatten();
                Some((
                    challenge,
                    request_id,
                    session.commands.clone(),
                    port_forward_key,
                ))
            })
        else {
            return;
        };

        if let Some((profile_id, rule_id)) = port_forward_key
            && let Some(manager) = self.services.port_forward_manager.as_ref()
        {
            let _ = manager.respond_keyboard_interactive(&profile_id, &rule_id, responses);
        } else if let Some(commands) = commands {
            let _ = commands.respond_keyboard_interactive(request_id, responses);
        }
        cx.emit(AppCommand::OverlayDismissed(
            keyboard_interactive_overlay_snapshot(
                cx.entity(),
                tab_id,
                request_id,
                challenge,
                self.keyboard_interactive_inputs(),
                cx,
            ),
        ));
        cx.notify();
    }

    pub(in crate::ui) fn cancel_keyboard_interactive(&self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some((challenge, request_id, commands, port_forward_key)) =
            self.tab_mut(tab_id).and_then(|mut session| {
                let challenge = session.pending_keyboard_interactive.take()?;
                let request_id = session.pending_keyboard_interactive_request_id;
                session.pending_keyboard_interactive_request_id = 0;
                let port_forward_key = (session.purpose == SessionPurpose::PortForwarding)
                    .then(|| {
                        session
                            .port_forward_rule_id
                            .clone()
                            .map(|rule_id| (session.profile_id.clone(), rule_id))
                    })
                    .flatten();
                Some((
                    challenge,
                    request_id,
                    session.commands.clone(),
                    port_forward_key,
                ))
            })
        else {
            return;
        };

        if let Some((profile_id, rule_id)) = port_forward_key
            && let Some(manager) = self.services.port_forward_manager.as_ref()
        {
            manager.stop(&profile_id, &rule_id);
        } else if let Some(commands) = commands {
            let _ = commands.close();
        }
        cx.emit(AppCommand::OverlayDismissed(
            keyboard_interactive_overlay_snapshot(
                cx.entity(),
                tab_id,
                request_id,
                challenge,
                self.keyboard_interactive_inputs(),
                cx,
            ),
        ));
        cx.notify();
    }

    pub(in crate::ui) fn online_session_count_for_profile(&self, profile_id: &str) -> usize {
        self.tabs
            .borrow()
            .values()
            .filter(|session| session.profile_id == profile_id && session.commands.is_some())
            .count()
    }

    fn port_forward_rule_session_id(&self, profile_id: &str, rule_id: &str) -> Option<TabId> {
        self.tabs.borrow().iter().find_map(|(tab_id, session)| {
            (session.profile_id == profile_id
                && session.port_forward_rule_id.as_deref() == Some(rule_id)
                && session.purpose == SessionPurpose::PortForwarding
                && session.commands.is_some())
            .then_some(*tab_id)
        })
    }

    pub(in crate::ui) fn has_port_forward_rule_session(
        &self,
        profile_id: &str,
        rule_id: &str,
    ) -> bool {
        self.port_forward_rule_session_id(profile_id, rule_id)
            .is_some()
    }

    pub(in crate::ui) fn terminal_profile_ids(&self) -> Vec<String> {
        self.tabs
            .borrow()
            .values()
            .filter(|&session| session.purpose == SessionPurpose::Terminal)
            .map(|session| session.profile_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect()
    }

    pub(in crate::ui) fn sync_port_forward_rules_for_profile(
        &self,
        profile_id: &str,
        rules: &[PortForwardRule],
    ) -> usize {
        let mut synced_sessions = 0;
        for session in self.tabs.borrow().values() {
            if session.profile_id != profile_id {
                continue;
            }
            let Some(commands) = session.commands.as_ref() else {
                continue;
            };
            let counts_as_synced = session.purpose == SessionPurpose::PortForwarding;
            let rules_for_session = if counts_as_synced {
                dedicated_port_forward_rules(session.port_forward_rule_id.as_deref(), rules)
            } else {
                Vec::new()
            };
            if commands.sync_port_forward_rules(rules_for_session).is_ok() && counts_as_synced {
                synced_sessions += 1;
            }
        }
        synced_sessions
    }

    pub(in crate::ui) fn monitoring_state_for_profile(
        &self,
        profile_id: &str,
        fallback: &SessionMonitoringState,
    ) -> SessionMonitoringState {
        self.shared_profile_monitoring
            .borrow()
            .get(profile_id)
            .cloned()
            .unwrap_or_else(|| fallback.clone())
    }

    pub(in crate::ui) fn shared_monitoring_enabled(&self, profile_id: &str) -> Option<bool> {
        self.shared_profile_monitoring
            .borrow()
            .get(profile_id)
            .map(|state| state.auto_collect_enabled)
    }

    pub(in crate::ui) fn monitoring_enabled_for_profile(
        &self,
        profile_id: &str,
        ordered_tab_ids: &[TabId],
        default_enabled: bool,
    ) -> bool {
        if let Some(enabled) = self
            .shared_profile_monitoring
            .borrow()
            .get(profile_id)
            .map(|state| state.auto_collect_enabled)
        {
            return enabled;
        }

        let tabs = self.tabs.borrow();
        ordered_tab_ids
            .iter()
            .find_map(|tab_id| {
                let session = tabs.get(tab_id)?;
                (session.purpose == SessionPurpose::Terminal && session.profile_id == profile_id)
                    .then_some(session.monitoring.auto_collect_enabled)
            })
            .unwrap_or(default_enabled)
    }

    pub(in crate::ui) fn claim_profile_monitor_source(
        &self,
        profile_id: &str,
        tab_id: TabId,
        enabled: bool,
    ) -> bool {
        self.update_shared_monitoring_state(profile_id, enabled, |state| {
            state.set_enabled(enabled)
        });

        if !enabled {
            self.monitor_source_tabs.borrow_mut().remove(profile_id);
            return false;
        }

        if let Some(source_tab_id) = self.current_monitor_source_tab_id(profile_id, None) {
            return source_tab_id == tab_id;
        }

        self.update_shared_monitoring_state(profile_id, enabled, |state| {
            state.mark_rates_warming_up()
        });
        self.monitor_source_tabs
            .borrow_mut()
            .insert(profile_id.to_string(), tab_id);
        true
    }

    pub(in crate::ui) fn set_profile_monitoring_enabled(
        &self,
        profile_id: &str,
        enabled: bool,
        ordered_tab_ids: &[TabId],
    ) -> Result<bool, String> {
        let current_source = self.current_monitor_source_tab_id(profile_id, None);

        for session in self.tabs.borrow_mut().values_mut() {
            if session.purpose == SessionPurpose::Terminal && session.profile_id == profile_id {
                session.monitoring.set_enabled(enabled);
            }
        }
        self.update_shared_monitoring_state(profile_id, enabled, |state| {
            state.set_enabled(enabled)
        });

        if !enabled {
            self.monitor_source_tabs.borrow_mut().remove(profile_id);
            if let Some(source_tab_id) = current_source
                && let Some(commands) = self.session_commands_by_tab_id(source_tab_id)
            {
                commands
                    .set_monitoring_enabled(false)
                    .map_err(|error| error.to_string())?;
                return Ok(true);
            }
            return Ok(false);
        }

        let Some(source_tab_id) = current_source
            .or_else(|| self.next_monitor_source_tab_id(profile_id, None, ordered_tab_ids))
        else {
            self.monitor_source_tabs.borrow_mut().remove(profile_id);
            return Ok(false);
        };

        if current_source.is_none() {
            self.update_shared_monitoring_state(profile_id, enabled, |state| {
                state.mark_rates_warming_up()
            });
        }
        self.monitor_source_tabs
            .borrow_mut()
            .insert(profile_id.to_string(), source_tab_id);
        if let Some(commands) = self.session_commands_by_tab_id(source_tab_id) {
            commands
                .set_monitoring_enabled(true)
                .map_err(|error| error.to_string())?;
            return Ok(true);
        }

        Ok(false)
    }

    fn ordered_terminal_tab_ids(&self) -> Vec<TabId> {
        self.ports.borrow().snapshot.terminal_order.clone()
    }

    pub(in crate::ui) fn enable_profile_monitoring(
        &self,
        profile_id: &str,
        cx: &mut Context<Self>,
    ) {
        let ordered_tab_ids = self.ordered_terminal_tab_ids();
        if let Err(error) = self.set_profile_monitoring_enabled(profile_id, true, &ordered_tab_ids)
        {
            log::debug!("failed to enable session monitoring: {error}");
        }
        cx.notify();
    }

    pub(in crate::ui) fn apply_auto_collect_monitoring_preference(
        &self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.services.auto_collect_session_monitoring.set(enabled);
        let ordered_tab_ids = self.ordered_terminal_tab_ids();
        for profile_id in self.terminal_profile_ids() {
            if let Err(error) =
                self.set_profile_monitoring_enabled(&profile_id, enabled, &ordered_tab_ids)
            {
                log::debug!("failed to toggle session monitoring: {error}");
            }
        }
        cx.notify();
    }

    pub(in crate::ui) fn apply_profile_monitor_snapshot(
        &self,
        profile_id: &str,
        tab_id: TabId,
        enabled: bool,
        snapshot: SessionMonitorSnapshot,
    ) {
        if let Some(source_tab_id) = self.current_monitor_source_tab_id(profile_id, None) {
            if source_tab_id != tab_id {
                return;
            }
        } else if enabled {
            self.update_shared_monitoring_state(profile_id, enabled, |state| {
                state.mark_rates_warming_up()
            });
            self.monitor_source_tabs
                .borrow_mut()
                .insert(profile_id.to_string(), tab_id);
        }

        self.update_shared_monitoring_state(profile_id, enabled, |state| {
            state.apply_snapshot(snapshot)
        });
    }

    pub(in crate::ui) fn apply_profile_monitor_error(
        &self,
        profile_id: &str,
        tab_id: TabId,
        enabled: bool,
        error: String,
    ) {
        if let Some(source_tab_id) = self.current_monitor_source_tab_id(profile_id, None) {
            if source_tab_id != tab_id {
                return;
            }
        } else if enabled {
            self.monitor_source_tabs
                .borrow_mut()
                .insert(profile_id.to_string(), tab_id);
        }

        self.update_shared_monitoring_state(profile_id, enabled, |state| state.report_error(error));
    }

    pub(in crate::ui) fn refresh_profile_monitoring(
        &self,
        profile_id: &str,
        excluded_tab_id: Option<TabId>,
        ordered_tab_ids: &[TabId],
        default_enabled: bool,
    ) {
        let has_terminal_tabs = self.tabs.borrow().values().any(|session| {
            session.purpose == SessionPurpose::Terminal && session.profile_id == profile_id
        });
        if !has_terminal_tabs {
            self.shared_profile_monitoring
                .borrow_mut()
                .remove(profile_id);
            self.monitor_source_tabs.borrow_mut().remove(profile_id);
            return;
        }

        if !self.monitoring_enabled_for_profile(profile_id, ordered_tab_ids, default_enabled) {
            self.monitor_source_tabs.borrow_mut().remove(profile_id);
            return;
        }

        if let Some(source_tab_id) = self.current_monitor_source_tab_id(profile_id, excluded_tab_id)
        {
            self.monitor_source_tabs
                .borrow_mut()
                .insert(profile_id.to_string(), source_tab_id);
            return;
        }

        let Some(source_tab_id) =
            self.next_monitor_source_tab_id(profile_id, excluded_tab_id, ordered_tab_ids)
        else {
            self.monitor_source_tabs.borrow_mut().remove(profile_id);
            return;
        };

        self.update_shared_monitoring_state(profile_id, true, |state| {
            state.mark_rates_warming_up()
        });
        self.monitor_source_tabs
            .borrow_mut()
            .insert(profile_id.to_string(), source_tab_id);
        if let Some(commands) = self.session_commands_by_tab_id(source_tab_id)
            && let Err(error) = commands.set_monitoring_enabled(true)
        {
            let message = error.to_string();
            self.update_shared_monitoring_state(profile_id, true, |state| {
                state.report_error(message.clone())
            });
            self.monitor_source_tabs.borrow_mut().remove(profile_id);
            log::debug!("failed to promote session monitoring source: {message}");
        }
    }

    fn update_shared_monitoring_state<R>(
        &self,
        profile_id: &str,
        enabled: bool,
        update: impl FnOnce(&mut SessionMonitoringState) -> R,
    ) -> R {
        let mut states = self.shared_profile_monitoring.borrow_mut();
        let state = states
            .entry(profile_id.to_string())
            .or_insert_with(|| SessionMonitoringState::new(enabled));
        update(state)
    }

    fn session_commands_by_tab_id(&self, tab_id: TabId) -> Option<SessionCommandSender> {
        self.tabs
            .borrow()
            .get(&tab_id)
            .and_then(|session| session.commands.clone())
    }

    fn can_source_monitoring(session: &SessionTabState) -> bool {
        session.purpose == SessionPurpose::Terminal
            && session.commands.is_some()
            && matches!(
                session.connection_state,
                SessionConnectionState::Connecting | SessionConnectionState::Ready
            )
    }

    fn current_monitor_source_tab_id(
        &self,
        profile_id: &str,
        excluded_tab_id: Option<TabId>,
    ) -> Option<TabId> {
        let source_tab_id = self.monitor_source_tabs.borrow().get(profile_id).copied()?;
        if Some(source_tab_id) == excluded_tab_id {
            return None;
        }

        self.tabs.borrow().get(&source_tab_id).and_then(|session| {
            (session.profile_id == profile_id && Self::can_source_monitoring(session))
                .then_some(source_tab_id)
        })
    }

    fn next_monitor_source_tab_id(
        &self,
        profile_id: &str,
        excluded_tab_id: Option<TabId>,
        ordered_tab_ids: &[TabId],
    ) -> Option<TabId> {
        let tabs = self.tabs.borrow();
        ordered_tab_ids.iter().find_map(|tab_id| {
            if Some(*tab_id) == excluded_tab_id {
                return None;
            }
            let session = tabs.get(tab_id)?;
            (session.profile_id == profile_id && Self::can_source_monitoring(session))
                .then_some(*tab_id)
        })
    }

    pub(in crate::ui) fn emit(&mut self, command: AppCommand, cx: &mut Context<Self>) {
        cx.emit(command);
    }

    pub(in crate::ui) fn credentials_changed(
        &mut self,
        secrets: SecretStore,
        local_vault_status: LocalVaultStatus,
        cx: &mut Context<Self>,
    ) {
        self.services.secrets = secrets;
        if let Some(service) = &self.services.ssh_bridge_service {
            service.replace_secrets(self.services.secrets.clone());
        }
        self.services.local_vault_status = local_vault_status;
        cx.notify();
    }

    pub(in crate::ui) fn sync_port_snapshot(&self, snapshot: SessionPortSnapshot) {
        let mut state = self.ports.borrow_mut();
        let removed_tabs = state
            .terminal_leases
            .keys()
            .copied()
            .filter(|tab_id| !snapshot.sessions.contains_key(tab_id))
            .collect::<Vec<_>>();
        for tab_id in removed_tabs {
            SessionPortState::close_terminal_lease(&mut state, tab_id);
        }
        state.snapshot = snapshot;
    }

    fn sync_port_profiles(&self) {
        self.ports.borrow_mut().snapshot.profiles = self.profiles.borrow().clone();
    }

    pub(in crate::ui) fn query_port(&self) -> SessionQueryPort {
        SessionQueryPort::new(Rc::new(SessionPortsAdapter {
            state: self.ports.clone(),
        }))
    }

    pub(in crate::ui) fn terminal_port(&self) -> SessionTerminalPort {
        SessionTerminalPort::new(Rc::new(SessionPortsAdapter {
            state: self.ports.clone(),
        }))
    }
}

impl EventEmitter<AppCommand> for SessionController {}

#[derive(Clone)]
struct SessionPortsAdapter {
    state: Rc<RefCell<SessionPortState>>,
}

impl SessionQuerySource for SessionPortsAdapter {
    fn active_profile_id(&self) -> Option<String> {
        self.state.borrow().snapshot.active_profile_id.clone()
    }

    fn resolved_profile_for_session(
        &self,
        tab_id: TabId,
        catalog: &CatalogQueryPort,
    ) -> Option<SessionProfile> {
        let state = self.state.borrow();
        let session = state.snapshot.sessions.get(&tab_id)?;
        session
            .pending_profile
            .clone()
            .or_else(|| catalog.profile(&session.profile_id))
    }
}

impl SessionTerminalRuntime for SessionPortsAdapter {
    /// Paste through the runtime owner so bracketed paste, free-type selection and
    /// terminal history use the same path as keyboard input.
    fn paste_text(&self, tab_id: TabId, text: String, cx: &mut App) -> bool {
        let owner = self
            .state
            .borrow()
            .owner
            .clone()
            .and_then(|owner| owner.upgrade());
        owner.is_some_and(|owner| {
            owner.update(cx, |controller, cx| {
                controller.paste_terminal_text(tab_id, text, cx)
            })
        })
    }

    fn targets(&self) -> Vec<SessionTerminalTarget> {
        let state = self.state.borrow();
        state
            .snapshot
            .terminal_order
            .iter()
            .filter_map(|tab_id| {
                state
                    .snapshot
                    .sessions
                    .get(tab_id)
                    .map(|session| terminal_target(&state.snapshot, session))
            })
            .collect()
    }

    fn target(&self, tab_id: TabId) -> Option<SessionTerminalTarget> {
        let state = self.state.borrow();
        let session = state.snapshot.sessions.get(&tab_id)?;
        (session.purpose == SessionPurpose::Terminal)
            .then(|| terminal_target(&state.snapshot, session))
    }

    fn active_target(&self) -> Option<SessionTerminalTarget> {
        let tab_id = self.state.borrow().snapshot.active_terminal_tab_id?;
        self.target(tab_id)
    }

    fn acquire(&self, tab_id: TabId) -> Result<TerminalLeaseGrant, TerminalLeaseError> {
        let commands = {
            let state = self.state.borrow();
            let Some(session) = state.snapshot.sessions.get(&tab_id) else {
                return Err(TerminalLeaseError::Missing);
            };
            if session.purpose != SessionPurpose::Terminal {
                return Err(TerminalLeaseError::Missing);
            }
            session
                .commands
                .clone()
                .ok_or(TerminalLeaseError::Disconnected)?
        };
        let (lease, output) = self.acquire_output_lease(tab_id)?;
        Ok(TerminalLeaseGrant {
            commands,
            output,
            lease,
        })
    }

    fn forward_output(&self, tab_id: TabId, bytes: Vec<u8>) -> bool {
        let tap = self
            .state
            .borrow()
            .terminal_leases
            .get(&tab_id)
            .map(|entry| entry.tap.clone());
        let Some(tap) = tap else {
            return false;
        };
        let delivered = tap.try_send(bytes).is_ok();
        release_retired_lease_if_ready(&self.state, tab_id, &tap);
        delivered
    }

    fn close_session(&self, tab_id: TabId) {
        SessionPortState::close_terminal_lease(&mut self.state.borrow_mut(), tab_id);
    }
    #[cfg(test)]
    fn acquire_output_lease_for_test(
        &self,
        tab_id: TabId,
    ) -> Result<(TerminalLease, TerminalOutputReceiver), TerminalLeaseError> {
        self.acquire_output_lease(tab_id)
    }
}

impl SessionPortsAdapter {
    fn acquire_output_lease(
        &self,
        tab_id: TabId,
    ) -> Result<(TerminalLease, TerminalOutputReceiver), TerminalLeaseError> {
        let mut state = self.state.borrow_mut();
        let Some(session) = state.snapshot.sessions.get(&tab_id) else {
            return Err(TerminalLeaseError::Missing);
        };
        if session.purpose != SessionPurpose::Terminal {
            return Err(TerminalLeaseError::Missing);
        }
        if state
            .terminal_leases
            .get(&tab_id)
            .is_some_and(|entry| !entry.tap.can_release_lease())
        {
            return Err(TerminalLeaseError::Busy);
        }
        SessionPortState::close_terminal_lease(&mut state, tab_id);

        let (tap, output) = TerminalOutputTap::channel();
        let owner = Rc::new(RefCell::new(Rc::downgrade(&self.state)));
        let retire_requested = Rc::new(Cell::new(false));
        state.terminal_leases.insert(
            tab_id,
            TerminalLeaseEntry {
                tap: tap.clone(),
                owner: owner.clone(),
                retire_requested: retire_requested.clone(),
            },
        );
        Ok((
            TerminalLease::new(Rc::new(SessionTerminalLease {
                tab_id,
                tap,
                owner,
                retire_requested,
            })),
            output,
        ))
    }
}
fn terminal_target(
    snapshot: &SessionPortSnapshot,
    session: &SessionPortSession,
) -> SessionTerminalTarget {
    let profile = snapshot
        .profiles
        .iter()
        .find(|profile| profile.id == session.profile_id)
        .cloned()
        .or_else(|| session.pending_profile.clone());
    SessionTerminalTarget {
        tab_id: session.tab_id,
        title: session.title.clone(),
        profile_id: session.profile_id.clone(),
        profile,
        command_available: session.commands.is_some(),
    }
}

#[derive(Clone)]
struct SessionTerminalLease {
    tab_id: TabId,
    tap: TerminalOutputTap,
    owner: Rc<RefCell<std::rc::Weak<RefCell<SessionPortState>>>>,
    retire_requested: Rc<Cell<bool>>,
}

impl TerminalLeaseRetirement for SessionTerminalLease {
    fn retire(&self) {
        self.retire_requested.set(true);
        let Some(owner) = self.owner.borrow().upgrade() else {
            self.tap.retire_lease();
            return;
        };
        let should_close = {
            let mut state = owner.borrow_mut();
            let Some(entry) = state.terminal_leases.get_mut(&self.tab_id) else {
                self.tap.retire_lease();
                return;
            };
            if !entry.tap.same_channel(&self.tap) {
                self.tap.retire_lease();
                return;
            }
            entry.retire_requested.set(true);
            entry.tap.retire_lease();
            entry.tap.can_release_lease()
        };
        if should_close {
            SessionPortState::close_terminal_lease(&mut owner.borrow_mut(), self.tab_id);
        }
    }
}

#[derive(Default)]
struct SessionPortState {
    owner: Option<gpui_kit::WeakEntity<SessionController>>,
    snapshot: SessionPortSnapshot,
    terminal_leases: HashMap<TabId, TerminalLeaseEntry>,
}

impl SessionPortState {
    fn close_terminal_lease(state: &mut Self, tab_id: TabId) {
        if let Some(entry) = state.terminal_leases.remove(&tab_id) {
            entry.tap.retire_lease();
            entry.tap.close();
        }
    }
}

struct TerminalLeaseEntry {
    tap: TerminalOutputTap,
    owner: Rc<RefCell<std::rc::Weak<RefCell<SessionPortState>>>>,
    retire_requested: Rc<Cell<bool>>,
}

fn release_retired_lease_if_ready(
    state: &Rc<RefCell<SessionPortState>>,
    tab_id: TabId,
    tap: &TerminalOutputTap,
) {
    let should_close = state
        .borrow()
        .terminal_leases
        .get(&tab_id)
        .is_some_and(|entry| {
            entry.tap.same_channel(tap)
                && entry.retire_requested.get()
                && entry.tap.can_release_lease()
        });
    if should_close {
        SessionPortState::close_terminal_lease(&mut state.borrow_mut(), tab_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::contracts::CatalogReader;
    use futures::FutureExt as _;
    use miaominal_ssh::SessionEvent;

    pub(super) fn session_payload(profile_id: &str) -> SessionTabState {
        SessionTabState {
            profile_id: profile_id.to_string(),
            port_forward_rule_id: None,
            terminal: TerminalState::default(),
            connection_state: SessionConnectionState::Ready,
            preserved_history_popup_hidden: false,
            pending_profile: None,
            commands: None,
            bytes_in: 0,
            bytes_out: 0,
            pending_host_key: None,
            pending_host_key_request_id: 0,
            pending_keyboard_interactive: None,
            pending_keyboard_interactive_request_id: 0,
            reconnect_task: None,
            reconnect_attempt: 0,
            has_activity: false,
            monitoring: SessionMonitoringState::new(false),
            purpose: SessionPurpose::Terminal,
            port_forward_revision: 0,
            port_forward_log_len: 0,
            sftp_progress_layout: SessionSftpProgressLayoutState::default(),
            local_terminal: false,
            owner_route: None,
        }
    }

    fn profile(id: &str, name: &str) -> SessionProfile {
        let mut profile = SessionProfile::blank(id, 1);
        profile.name = name.to_string();
        profile.host = "example.test".to_string();
        profile.username = "user".to_string();
        profile
    }

    fn port_forward_rule(id: &str, enabled: bool) -> PortForwardRule {
        PortForwardRule {
            id: id.to_string(),
            label: id.to_string(),
            kind: PortForwardKind::Local,
            listen_host: "127.0.0.1".to_string(),
            listen_port: 8080,
            target_host: "127.0.0.1".to_string(),
            target_port: 80,
            enabled,
        }
    }

    fn terminal(tab_id: TabId, profile_id: &str, title: &str) -> SessionPortSession {
        SessionPortSession::new(
            tab_id,
            title.to_string(),
            profile_id.to_string(),
            None,
            SessionPurpose::Terminal,
            None,
        )
    }

    #[derive(Default)]
    struct TestCatalog {
        profiles: RefCell<Vec<SessionProfile>>,
    }

    impl CatalogReader for TestCatalog {
        fn profiles(&self) -> Vec<SessionProfile> {
            self.profiles.borrow().clone()
        }

        fn profile(&self, id: &str) -> Option<SessionProfile> {
            self.profiles
                .borrow()
                .iter()
                .find(|profile| profile.id == id)
                .cloned()
        }

        fn proxies(&self) -> Vec<ProxyProfile> {
            Vec::new()
        }
    }

    #[test]
    fn pending_session_profile_wins_over_catalog_updates_and_deletion() {
        let controller = SessionController::new_for_test();
        let tab_id = TabId::new(7);
        let pending_profile = profile("profile-a", "Connection draft");
        let mut session = terminal(tab_id, "profile-a", "terminal-a");
        session.pending_profile = Some(pending_profile.clone());
        controller.sync_port_snapshot(SessionPortSnapshot::new(
            vec![profile("profile-a", "Runtime snapshot")],
            vec![session],
            Some("profile-a".into()),
            Some(tab_id),
        ));
        let source = Rc::new(TestCatalog {
            profiles: RefCell::new(vec![profile("profile-a", "Committed edit")]),
        });
        let catalog = CatalogQueryPort::new(source.clone());
        let query = controller.query_port();

        assert_eq!(
            query.resolved_profile_for_session(tab_id, &catalog),
            Some(pending_profile.clone())
        );
        source.profiles.borrow_mut().clear();
        assert_eq!(
            query.resolved_profile_for_session(tab_id, &catalog),
            Some(pending_profile)
        );
        assert!(
            query
                .resolved_profile_for_session(TabId::new(99), &catalog)
                .is_none()
        );
    }

    #[test]
    fn catalog_resolution_and_terminal_profile_snapshots_have_separate_owners() {
        let controller = SessionController::new_for_test();
        let tab_id = TabId::new(7);
        let mut runtime_profile = SessionProfile::blank_local("profile-a", 1);
        runtime_profile.name = "Runtime local terminal".into();
        controller.sync_port_snapshot(SessionPortSnapshot::new(
            vec![runtime_profile.clone()],
            vec![terminal(tab_id, "profile-a", "terminal-a")],
            Some("profile-a".into()),
            Some(tab_id),
        ));
        let source = Rc::new(TestCatalog {
            profiles: RefCell::new(vec![profile("profile-a", "New committed profile")]),
        });
        let catalog = CatalogQueryPort::new(source.clone());
        let query = controller.query_port();
        let terminal = controller.terminal_port();

        assert_eq!(
            query.resolved_profile_for_session(tab_id, &catalog),
            catalog.profile("profile-a")
        );
        assert_eq!(
            terminal.target(tab_id).unwrap().profile,
            Some(runtime_profile.clone())
        );
        source.profiles.borrow_mut().clear();
        assert!(
            query
                .resolved_profile_for_session(tab_id, &catalog)
                .is_none()
        );
        assert_eq!(
            terminal.target(tab_id).unwrap().profile,
            Some(runtime_profile)
        );
    }

    #[test]
    fn transferring_monitor_source_carries_shared_history_and_clears_source_ownership() {
        let controller = SessionController::new_for_test();
        let tab_id = TabId::new(17);
        controller.insert_tab(tab_id, session_payload("profile-a"));
        let mut monitoring = SessionMonitoringState::new(true);
        monitoring.last_error = Some("previous sample failed".to_string());
        controller
            .shared_profile_monitoring
            .borrow_mut()
            .insert("profile-a".to_string(), monitoring);
        controller
            .monitor_source_tabs
            .borrow_mut()
            .insert("profile-a".to_string(), tab_id);

        let transferred = controller
            .take_tab_for_transfer(tab_id)
            .expect("session transfer exists");

        let transferred = transferred.into_payload::<SessionTabHandoff>();
        assert!(transferred.was_monitor_source);
        assert_eq!(
            transferred
                .shared_monitoring
                .as_ref()
                .and_then(|state| state.last_error.as_deref()),
            Some("previous sample failed")
        );
        assert!(
            !controller
                .shared_profile_monitoring
                .borrow()
                .contains_key("profile-a")
        );
        assert!(
            !controller
                .monitor_source_tabs
                .borrow()
                .contains_key("profile-a")
        );
    }

    fn ports() -> (SessionQueryPort, SessionTerminalPort) {
        let controller = SessionController::new_for_test();
        controller.sync_port_snapshot(SessionPortSnapshot::new(
            vec![profile("profile-a", "A"), profile("profile-b", "B")],
            vec![
                terminal(TabId::new(7), "profile-a", "terminal-a"),
                terminal(TabId::new(9), "profile-b", "terminal-b"),
            ],
            Some("profile-b".to_string()),
            Some(TabId::new(9)),
        ));
        (controller.query_port(), controller.terminal_port())
    }

    #[test]
    fn dedicated_forward_sync_enables_rule_while_session_is_connecting() {
        let rules = vec![
            port_forward_rule("starting", false),
            port_forward_rule("other", true),
        ];

        let synced = dedicated_port_forward_rules(Some("starting"), &rules);

        assert_eq!(synced.len(), 1);
        assert_eq!(synced[0].id, "starting");
        assert!(synced[0].enabled);
    }

    #[test]
    fn payloads_remain_bound_to_stable_tab_ids() {
        let controller = SessionController::new_for_test();
        let first = TabId::new(7);
        let second = TabId::new(9);
        controller.insert_tab(first, session_payload("profile-a"));
        controller.insert_tab(second, session_payload("profile-b"));

        let removed = controller
            .remove_tab(first)
            .expect("first payload should be removed");

        assert_eq!(removed.profile_id, "profile-a");
        assert!(controller.tab(first).is_none());
        assert_eq!(
            controller
                .tab(second)
                .expect("second payload should remain")
                .profile_id,
            "profile-b"
        );
    }

    #[test]
    fn connection_test_state_ignores_other_session_purposes() {
        let controller = SessionController::new_for_test();
        let terminal_id = TabId::new(7);
        let forwarding_id = TabId::new(8);
        controller.insert_tab(terminal_id, session_payload("profile-a"));
        let mut forwarding = session_payload("profile-b");
        forwarding.purpose = SessionPurpose::PortForwarding;
        controller.insert_tab(forwarding_id, forwarding);

        assert!(!controller.connection_test_in_progress());
    }

    #[test]
    fn retiring_connection_tests_removes_only_test_sessions_and_ignores_late_events() {
        let controller = SessionController::new_for_test();
        let terminal_id = TabId::new(7);
        let test_id = TabId::new(9);
        controller.insert_tab(terminal_id, session_payload("profile-a"));
        let mut connection_test = session_payload("profile-b");
        connection_test.purpose = SessionPurpose::ConnectionTest;
        connection_test.connection_state = SessionConnectionState::Connecting;
        controller.insert_tab(test_id, connection_test);

        assert!(controller.connection_test_in_progress());
        assert_eq!(controller.retire_connection_tests(), vec![test_id]);
        assert!(!controller.connection_test_in_progress());
        assert!(controller.tab(terminal_id).is_some());
        assert!(controller.tab(test_id).is_none());
        assert!(
            controller
                .apply_session_event(test_id, SessionEvent::Closed, false, "Test profile-b")
                .is_none()
        );
    }

    #[test]
    fn removed_tab_id_rejects_late_access_after_reopen() {
        let controller = SessionController::new_for_test();
        let closed = TabId::new(7);
        let reopened = TabId::new(10);
        controller.insert_tab(closed, session_payload("old-profile"));
        controller
            .remove_tab(closed)
            .expect("closed payload should exist");
        controller.insert_tab(reopened, session_payload("new-profile"));

        assert!(controller.tab_mut(closed).is_none());
        assert_eq!(
            controller
                .tab(reopened)
                .expect("reopened payload should use its new id")
                .profile_id,
            "new-profile"
        );
    }

    #[gpui_kit::test]
    fn transfer_and_rollback_preserve_identity_history_routes_and_lease_owner(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(|cx| {
            let source = cx.new(|_| SessionController::new_for_test());
            let destination = cx.new(|_| SessionController::new_for_test());
            let id = TabId::new(71);
            let mut payload = session_payload("profile-a");
            payload.bytes_in = 42;
            payload.terminal.push_text("retained history");
            let generation = payload.terminal.generation();
            payload.owner_route = Some(cx.new(|_| SessionTabOwnerRoute::new(source.downgrade())));
            let route = payload.owner_route.clone().unwrap();
            source.read(cx).insert_tab(id, payload);
            source.read(cx).sync_port_snapshot(SessionPortSnapshot::new(
                vec![],
                vec![terminal(id, "profile-a", "terminal")],
                None,
                Some(id),
            ));
            let (lease, _output) = source
                .read(cx)
                .terminal_port()
                .acquire_output_lease(id)
                .unwrap();
            let transferred = source.read(cx).take_tab_for_transfer(id).unwrap();
            assert!(source.read(cx).tab(id).is_none());
            assert!(!source.read(cx).terminal_port().forward_output(id, vec![0]));
            assert_eq!(
                destination.update(cx, |owner, cx| owner
                    .insert_transferred_tab(transferred, cx)),
                id
            );
            assert_eq!(
                route.read(cx).owner.upgrade().unwrap().entity_id(),
                destination.entity_id()
            );
            assert_eq!(destination.read(cx).tab(id).unwrap().bytes_in, 42);
            assert_eq!(
                destination.read(cx).tab(id).unwrap().terminal.generation(),
                generation
            );
            assert!(
                destination
                    .read(cx)
                    .terminal_port()
                    .forward_output(id, vec![1])
            );
            // Restore the same opaque resource bundle on a failed handoff; no reconnect/new ID.
            let rollback = destination.read(cx).take_tab_for_transfer(id).unwrap();
            assert_eq!(
                source.update(cx, |owner, cx| owner.insert_transferred_tab(rollback, cx)),
                id
            );
            assert_eq!(
                route.read(cx).owner.upgrade().unwrap().entity_id(),
                source.entity_id()
            );
            assert!(source.read(cx).terminal_port().forward_output(id, vec![2]));
            assert!(
                !destination
                    .read(cx)
                    .terminal_port()
                    .forward_output(id, vec![3])
            );
            // Move once more, then release using the original lease handle.
            let transferred = source.read(cx).take_tab_for_transfer(id).unwrap();
            destination.update(cx, |owner, cx| {
                owner.insert_transferred_tab(transferred, cx)
            });
            lease.retire();
            assert!(
                !destination
                    .read(cx)
                    .ports
                    .borrow()
                    .terminal_leases
                    .contains_key(&id)
            );
            assert!(
                !destination
                    .read(cx)
                    .terminal_port()
                    .forward_output(id, vec![4])
            );
            assert!(source.read(cx).ports.borrow().terminal_leases.is_empty());
        });
    }

    struct SearchHarness(Entity<SessionController>);
    impl gpui_kit::Render for SearchHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui_kit::IntoElement {
            gpui_kit::div()
        }
    }

    #[gpui_kit::test]
    fn search_target_changes_recompute_matches_without_render_and_ignore_closed_tabs(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            SearchHarness(cx.new(|cx| {
                let mut controller = SessionController::new_for_test();
                controller.forms = Some(RefCell::new(
                    SessionController::build_forms(window, cx).workspace,
                ));
                controller
            }))
        });
        let owner = view.read_with(cx, |view, _| view.0.clone());
        cx.update(|window, cx| {
            owner.update(cx, |controller, cx| {
                let first = TabId::new(80);
                let second = TabId::new(81);
                let a = session_payload("profile-a");
                a.terminal.push_text("needle needle");
                let b = session_payload("profile-b");
                b.terminal.push_text("needle");
                controller.insert_tab(first, a);
                controller.insert_tab(second, b);
                controller.open_terminal_search(first, window, cx);
                set_input_value(&controller.terminal_search_input(), "needle", window, cx);
                controller.update_terminal_search("needle".into(), cx);
                assert_eq!(controller.terminal_search().total, 2);
                controller.sync_terminal_search_target(Some(second), cx);
                assert_eq!(controller.terminal_search().total, 1);
                assert_eq!(
                    controller
                        .tab(first)
                        .unwrap()
                        .terminal
                        .snapshot(false)
                        .search_total,
                    0
                );
                assert_eq!(
                    controller
                        .tab(second)
                        .unwrap()
                        .terminal
                        .snapshot(false)
                        .search_total,
                    1
                );
                controller.remove_tab(second);
                controller.sync_terminal_search_target(Some(second), cx);
                assert_eq!(controller.search_target.get(), None);
                controller.update_terminal_search("late input".into(), cx);
                assert_eq!(controller.terminal_search().total, 0);
                assert_eq!(
                    controller
                        .tab(first)
                        .unwrap()
                        .terminal
                        .snapshot(false)
                        .search_total,
                    0
                );
            });
        });
    }

    #[gpui_kit::test]
    fn retirement_during_prepared_transfer_does_not_revive_the_lease(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(|cx| {
            let source = cx.new(|_| SessionController::new_for_test());
            let destination = cx.new(|_| SessionController::new_for_test());
            let id = TabId::new(72);
            source.read(cx).insert_tab(id, session_payload("profile-a"));
            source.read(cx).sync_port_snapshot(SessionPortSnapshot::new(
                vec![],
                vec![terminal(id, "profile-a", "terminal")],
                None,
                Some(id),
            ));
            let (lease, _output) = source
                .read(cx)
                .terminal_port()
                .acquire_output_lease(id)
                .unwrap();
            let transferred = source.read(cx).take_tab_for_transfer(id).unwrap();
            lease.retire();
            destination.update(cx, |owner, cx| {
                owner.insert_transferred_tab(transferred, cx)
            });
            assert!(destination.read(cx).tab(id).is_some());
            assert!(
                destination
                    .read(cx)
                    .ports
                    .borrow()
                    .terminal_leases
                    .is_empty()
            );
            assert!(
                !destination
                    .read(cx)
                    .terminal_port()
                    .forward_output(id, vec![1])
            );
            assert!(
                !destination
                    .read(cx)
                    .terminal_port()
                    .forward_output(id, vec![2])
            );
            destination
                .read(cx)
                .sync_port_snapshot(SessionPortSnapshot::new(
                    vec![],
                    vec![terminal(id, "profile-a", "terminal")],
                    None,
                    Some(id),
                ));
            let (replacement, _replacement_output) = destination
                .read(cx)
                .terminal_port()
                .acquire_output_lease(id)
                .unwrap();
            lease.retire();
            assert!(matches!(
                destination
                    .read(cx)
                    .terminal_port()
                    .acquire_output_lease(id),
                Err(TerminalLeaseError::Busy)
            ));
            replacement.retire();
        });
    }
    #[test]
    fn snapshot_exposes_active_profile_and_ordered_terminal_targets() {
        let (query, terminal) = ports();

        assert_eq!(query.active_profile_id(), Some("profile-b".to_string()));
        let targets = terminal.targets();
        assert_eq!(
            targets
                .iter()
                .map(|target| {
                    (
                        target.tab_id,
                        target.title.as_str(),
                        target.command_available,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (TabId::new(7), "terminal-a", false),
                (TabId::new(9), "terminal-b", false),
            ]
        );
        assert_eq!(
            terminal.active_target().map(|target| target.tab_id),
            Some(TabId::new(9))
        );
    }

    #[test]
    fn terminal_lease_is_exclusive_until_explicitly_retired() {
        let (_, terminal) = ports();
        let (lease, _output) = terminal
            .acquire_output_lease(TabId::new(7))
            .expect("first lease should be acquired");

        assert!(matches!(
            terminal.acquire_output_lease(TabId::new(7)),
            Err(TerminalLeaseError::Busy)
        ));

        lease.retire();
        assert!(terminal.acquire_output_lease(TabId::new(7)).is_ok());
    }

    #[test]
    fn late_retire_does_not_release_a_new_terminal_lease() {
        let (_, terminal) = ports();
        let (old, _old_output) = terminal
            .acquire_output_lease(TabId::new(7))
            .expect("old lease should be acquired");
        old.retire();
        let (new, _new_output) = terminal
            .acquire_output_lease(TabId::new(7))
            .expect("new lease should be acquired");

        old.retire();
        assert!(matches!(
            terminal.acquire_output_lease(TabId::new(7)),
            Err(TerminalLeaseError::Busy)
        ));

        new.retire();
        new.retire();
        assert!(terminal.acquire_output_lease(TabId::new(7)).is_ok());
    }

    #[test]
    fn closing_session_closes_and_removes_its_terminal_lease() {
        let (_, terminal) = ports();
        let (_lease, mut output) = terminal
            .acquire_output_lease(TabId::new(7))
            .expect("lease should be acquired");

        terminal.close_session(TabId::new(7));

        assert_eq!(output.recv().now_or_never(), Some(None));
        assert!(terminal.acquire_output_lease(TabId::new(7)).is_ok());
    }

    #[test]
    fn output_is_forwarded_only_to_the_current_terminal_lease() {
        let (_, terminal) = ports();
        let (old, mut old_output) = terminal
            .acquire_output_lease(TabId::new(7))
            .expect("old lease should be acquired");
        old.retire();
        let (_new, mut new_output) = terminal
            .acquire_output_lease(TabId::new(7))
            .expect("new lease should be acquired");

        assert!(terminal.forward_output(TabId::new(7), vec![1]));
        assert_eq!(old_output.recv().now_or_never(), Some(None));
        assert_eq!(new_output.recv().now_or_never(), Some(Some(vec![1])));
        assert!(!terminal.forward_output(TabId::new(99), vec![4]));
    }
    fn local_profile(id: &str, name: &str) -> SessionProfile {
        let mut profile = SessionProfile::blank_local(id, 1);
        profile.name = name.to_string();
        profile
    }

    #[test]
    fn local_terminal_tab_uses_profile_name_and_local_kind() {
        let (tab, session) = SessionController::build_local_tab(
            TabId::new(11),
            local_profile("local-a", "Dev box"),
            TerminalState::default(),
        );

        assert_eq!(tab.title(), "Dev box");
        assert_eq!(
            tab.status(),
            i18n::string("tabs.initial.local_terminal_starting")
        );
        assert_eq!(tab.kind(), TabKindTag::LocalTerminal);
        assert_eq!(tab.kind(), TabKindTag::LocalTerminal);
        let session = session.into_payload::<SessionTabState>();
        assert!(session.local_terminal);
        assert!(!session.monitoring.auto_collect_enabled);
    }

    #[test]
    fn local_terminal_tab_without_name_falls_back_to_the_configured_shell() {
        let mut profile = SessionProfile::blank_local("local-a", 1);
        profile.name = String::new();
        profile.local_shell = "/bin/zsh".into();

        let (tab, _) =
            SessionController::build_local_tab(TabId::new(12), profile, TerminalState::default());

        assert_eq!(tab.title(), "zsh");
    }

    #[test]
    fn local_terminal_tab_without_shell_uses_the_generic_title() {
        let mut profile = SessionProfile::blank_local("local-a", 1);
        profile.name = String::new();

        let (tab, _) =
            SessionController::build_local_tab(TabId::new(13), profile, TerminalState::default());

        assert_eq!(
            tab.title(),
            i18n::string("tabs.initial.local_terminal_title")
        );
    }

    #[test]
    fn local_terminal_profile_never_requires_a_locked_vault() {
        let controller = SessionController::new_for_test();
        let mut local = local_profile("local-a", "Local");
        local.auth_method = Some(AuthMethod::Password);
        local.has_stored_password = true;

        assert!(!controller.profile_requires_local_vault_unlock(&local));

        let mut ssh = profile("ssh-a", "A");
        ssh.auth_method = Some(AuthMethod::Password);
        ssh.password = String::new();
        ssh.has_stored_password = true;

        assert!(controller.profile_requires_local_vault_unlock(&ssh));
    }
}

#[cfg(test)]
mod input_integration;

#[cfg(test)]
mod model_tests;
