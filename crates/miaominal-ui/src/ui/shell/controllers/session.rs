use std::{
    cell::{Cell, Ref, RefCell, RefMut},
    collections::{HashMap, HashSet},
    rc::Rc,
    time::SystemTime,
};

use gpui::{
    App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, ScrollHandle,
    Subscription, Window,
};
use gpui_component::{
    WindowExt as _,
    input::{InputEvent, InputState, TabSize},
    select::{SearchableVec, SelectEvent, SelectItem, SelectState},
};
use miaominal_agent::{TerminalOutputReceiver, TerminalOutputTap};
use miaominal_core::keychain::ManagedKeyRecord;
use miaominal_core::known_host::KnownHostEntry;
use miaominal_core::profile::{
    AuthMethod, DEFAULT_SESSION_CHARSET, ImportedBatch, PortForwardKind, PortForwardRule,
    SessionEnvironmentVariable, ShellType,
};
use miaominal_core::snippet::SnippetRecord;
use miaominal_secrets::{SecretKind, SecretStore};
use miaominal_services::{ImportedProfilesResult, ProfileService, TerminalService};
use miaominal_ssh::{
    HostKeyDecision, HostKeyPrompt, KbiChallenge, SessionCommandSender, SessionConnection,
    SessionEventReceiver, SessionMonitorSnapshot,
};
use miaominal_storage::config_store::store::{SessionStore, SnippetStore};
use miaominal_storage::known_hosts_store::KnownHostsStore;
use miaominal_terminal::TerminalState;
use tokio::runtime::Handle as TokioHandle;

use super::{AppCommand, DeferredAppCommand, SessionDeferredCommand, TabOpenRequest};
use crate::ui::shell::bootstrap_loaders::initial_profile_selection;
use crate::ui::shell::support::set_input_masked;
use crate::ui::{
    i18n,
    shell::{
        AppIcon, DialogOverlaySnapshot, ForwardProfileSelectItem, LocalVaultStatus,
        ManagedKeySelectItem, ProfileViewMode, ProxyJumpCandidateSelectItem, SessionProfile, TabId,
        TabKindTag, TabState, TerminalSearchAnimation, ValidationFailure,
        WorkspaceSidePanelTransition, error_notification, localized_secret_placeholder,
        new_input_state, set_code_editor_input_placeholder, set_input_placeholder, set_input_value,
        validation_notification,
    },
};

mod events;
mod forwarding;
mod lifecycle;
mod profile_import;
mod search;
mod snippets;
mod terminal;

pub(in crate::ui::shell) const SESSION_MONITOR_HISTORY_LIMIT: usize = 900;
pub(in crate::ui::shell) use forwarding::PortForwardSessionStart;

fn is_valid_environment_variable_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };

    if first != '_' && !first.is_ascii_alphabetic() {
        return false;
    }

    characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct MonitorChartPoint {
    pub(in crate::ui::shell) label: String,
    pub(in crate::ui::shell) value: f64,
}

#[derive(Clone)]
pub(in crate::ui::shell) struct SessionMonitoringState {
    pub(in crate::ui::shell) auto_collect_enabled: bool,
    pub(in crate::ui::shell) last_snapshot: Option<SessionMonitorSnapshot>,
    pub(in crate::ui::shell) last_error: Option<String>,
    pub(in crate::ui::shell) last_updated_at: Option<SystemTime>,
    pub(in crate::ui::shell) cpu_history: Vec<MonitorChartPoint>,
    pub(in crate::ui::shell) memory_history: Vec<MonitorChartPoint>,
    pub(in crate::ui::shell) network_rx_history: Vec<MonitorChartPoint>,
    pub(in crate::ui::shell) network_tx_history: Vec<MonitorChartPoint>,
    pub(in crate::ui::shell) cpu_sample_ready: bool,
    pub(in crate::ui::shell) network_sample_ready: bool,
    rates_warming_up: bool,
    pub(in crate::ui::shell) sample_count: usize,
}

impl SessionMonitoringState {
    pub(in crate::ui::shell) fn new(auto_collect_enabled: bool) -> Self {
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

    pub(in crate::ui::shell) fn set_enabled(&mut self, enabled: bool) {
        let was_enabled = self.auto_collect_enabled;
        self.auto_collect_enabled = enabled;
        if enabled && !was_enabled {
            self.last_error = None;
        }
        if !enabled || !was_enabled {
            self.mark_rates_warming_up();
        }
    }

    pub(in crate::ui::shell) fn apply_snapshot(&mut self, snapshot: SessionMonitorSnapshot) {
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

    pub(in crate::ui::shell) fn report_error(&mut self, error: String) {
        self.last_error = Some(error);
        self.mark_rates_warming_up();
    }

    pub(in crate::ui::shell) fn mark_rates_warming_up(&mut self) {
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

pub(in crate::ui::shell) struct SessionTabState {
    pub(in crate::ui::shell) profile_id: String,
    pub(in crate::ui::shell) port_forward_rule_id: Option<String>,
    pub(in crate::ui::shell) terminal: TerminalState,
    pub(in crate::ui::shell) connection_state: SessionConnectionState,
    pub(in crate::ui::shell) preserved_history_popup_hidden: bool,
    pub(in crate::ui::shell) pending_profile: Option<SessionProfile>,
    pub(in crate::ui::shell) commands: Option<SessionCommandSender>,
    pub(in crate::ui::shell) bytes_in: u64,
    pub(in crate::ui::shell) bytes_out: u64,
    pub(in crate::ui::shell) pending_host_key: Option<HostKeyPrompt>,
    pub(in crate::ui::shell) pending_keyboard_interactive: Option<KbiChallenge>,
    pub(in crate::ui::shell) reconnect_task: Option<gpui::Task<()>>,
    pub(in crate::ui::shell) reconnect_attempt: u32,
    pub(in crate::ui::shell) has_activity: bool,
    pub(in crate::ui::shell) monitoring: SessionMonitoringState,
    pub(in crate::ui::shell) purpose: SessionPurpose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum SessionPurpose {
    Terminal,
    PortForwarding,
    ConnectionTest,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) enum SessionNotificationTone {
    Success,
    Error,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct SessionNotificationRequest {
    pub(in crate::ui::shell) tone: SessionNotificationTone,
    pub(in crate::ui::shell) title: String,
    pub(in crate::ui::shell) message: String,
    pub(in crate::ui::shell) id: String,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) enum SessionEventTabRemoval {
    ConnectionTest {
        status_message: String,
    },
    PortForward {
        profile_id: String,
        status_message: String,
    },
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct SessionEventOutcome {
    pub(in crate::ui::shell) tab_status: Option<String>,
    pub(in crate::ui::shell) clipboard_writes: Vec<String>,
    pub(in crate::ui::shell) notification: Option<SessionNotificationRequest>,
    pub(in crate::ui::shell) removal: Option<SessionEventTabRemoval>,
    pub(in crate::ui::shell) schedule_reconnect_error: Option<String>,
    pub(in crate::ui::shell) refresh_monitoring_profile: Option<String>,
    pub(in crate::ui::shell) should_notify: bool,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingProfileDeleteState {
    pub(in crate::ui::shell) profile_id: String,
    pub(in crate::ui::shell) profile_name: String,
    pub(in crate::ui::shell) reload_inputs_after_delete: bool,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingKnownHostDeleteState {
    pub(in crate::ui::shell) host: String,
    pub(in crate::ui::shell) port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::ui::shell) enum TrustedHostFilter {
    #[default]
    All,
    Linked,
    Orphaned,
    DefaultPort,
    CustomPort,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingSnippetDeleteState {
    pub(in crate::ui::shell) snippet_id: String,
    pub(in crate::ui::shell) snippet_description: String,
}

#[derive(Debug, Clone)]
pub(in crate::ui::shell) struct PendingPortForwardRuleDeleteState {
    pub(in crate::ui::shell) profile_id: String,
    pub(in crate::ui::shell) rule_id: String,
    pub(in crate::ui::shell) profile_label: String,
    pub(in crate::ui::shell) rule_label: String,
}

struct PortForwardRuleInputValues {
    profile_id: String,
    profile_index: usize,
    kind: PortForwardKind,
    resolved_label: String,
    listen_host: String,
    listen_port: u16,
    target_host: String,
    target_port: u16,
}

struct SavePortForwardRuleAfterUnlockResult {
    profiles: Vec<SessionProfile>,
    profile_id: String,
    profile_name: String,
    rule: PortForwardRule,
    is_edit: bool,
    persist_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProfileInputPurpose {
    Save,
    ConnectionTest,
}

impl ProfileInputPurpose {
    fn requires_name(self) -> bool {
        matches!(self, Self::Save)
    }
}

struct SaveProfileAfterUnlockResult {
    profile: SessionProfile,
    profiles: Vec<SessionProfile>,
    selected_profile: Option<usize>,
}

pub(in crate::ui::shell) struct ClosedSessionTabState {
    pub(in crate::ui::shell) tab_id: TabId,
    pub(in crate::ui::shell) profile: SessionProfile,
    pub(in crate::ui::shell) hidden_from_topbar: bool,
}

#[derive(Clone)]
pub(in crate::ui::shell) struct HostEditorEnvironmentVariableRow {
    pub(in crate::ui::shell) name_input: Entity<InputState>,
    pub(in crate::ui::shell) value_input: Entity<InputState>,
}

#[derive(Clone)]
pub(in crate::ui::shell) struct HostEditorForms {
    pub(in crate::ui::shell) name_input: Entity<InputState>,
    pub(in crate::ui::shell) group_input: Entity<InputState>,
    pub(in crate::ui::shell) group_select: Entity<SelectState<SearchableVec<String>>>,
    pub(in crate::ui::shell) managed_key_select:
        Entity<SelectState<SearchableVec<ManagedKeySelectItem>>>,
    pub(in crate::ui::shell) proxy_jump_select:
        Entity<SelectState<SearchableVec<ProxyJumpCandidateSelectItem>>>,
    pub(in crate::ui::shell) charset_select: Entity<SelectState<SearchableVec<String>>>,
    pub(in crate::ui::shell) creating_new_group: bool,
    pub(in crate::ui::shell) tags_input: Entity<InputState>,
    pub(in crate::ui::shell) host_input: Entity<InputState>,
    pub(in crate::ui::shell) port_input: Entity<InputState>,
    pub(in crate::ui::shell) username_input: Entity<InputState>,
    pub(in crate::ui::shell) password_input: Entity<InputState>,
    pub(in crate::ui::shell) private_key_input: Entity<InputState>,
    pub(in crate::ui::shell) agent_identity_input: Entity<InputState>,
    pub(in crate::ui::shell) certificate_input: Entity<InputState>,
    pub(in crate::ui::shell) passphrase_input: Entity<InputState>,
    pub(in crate::ui::shell) startup_command_input: Entity<InputState>,
    pub(in crate::ui::shell) proxy_jump_profile_ids: Vec<String>,
    pub(in crate::ui::shell) selected_proxy_jump_hop: Option<usize>,
    pub(in crate::ui::shell) environment_variable_rows: Vec<HostEditorEnvironmentVariableRow>,
    pub(in crate::ui::shell) shell_type: ShellType,
    pub(in crate::ui::shell) editing_auth_method: AuthMethod,
    pub(in crate::ui::shell) agent_forwarding_enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::ui::shell) enum SessionSidePanelView {
    #[default]
    Monitor,
    Snippets,
    Sftp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum SessionFailureStatus {
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum SessionConnectionState {
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
    Disconnected,
}

pub(in crate::ui::shell) struct TerminalSearchForms {
    pub(in crate::ui::shell) input: Entity<InputState>,
    pub(in crate::ui::shell) open: bool,
    pub(in crate::ui::shell) visible: bool,
    pub(in crate::ui::shell) visibility: f32,
    pub(in crate::ui::shell) animation: Option<TerminalSearchAnimation>,
    pub(in crate::ui::shell) total: usize,
    pub(in crate::ui::shell) current: Option<usize>,
    pub(in crate::ui::shell) status: Option<String>,
}

pub(in crate::ui::shell) struct WorkspaceSnippetsForms {
    pub(in crate::ui::shell) filter_input: Entity<InputState>,
    pub(in crate::ui::shell) selected_package_filter: Option<String>,
}

pub(in crate::ui::shell) struct SessionWorkspaceForms {
    pub(in crate::ui::shell) search: TerminalSearchForms,
    pub(in crate::ui::shell) snippets_panel: WorkspaceSnippetsForms,
}

#[derive(Clone)]
pub(in crate::ui::shell) struct SnippetsForms {
    pub(in crate::ui::shell) filter_input: Entity<InputState>,
    pub(in crate::ui::shell) description_input: Entity<InputState>,
    pub(in crate::ui::shell) package_input: Entity<InputState>,
    pub(in crate::ui::shell) package_select: Entity<SelectState<SearchableVec<String>>>,
    pub(in crate::ui::shell) creating_new_package: bool,
    pub(in crate::ui::shell) script_input: Entity<InputState>,
}

#[derive(Clone)]
pub(in crate::ui::shell) struct HostsForms {
    pub(in crate::ui::shell) filter_input: Entity<InputState>,
}

#[derive(Clone)]
pub(in crate::ui::shell) struct TrustedHostsForms {
    pub(in crate::ui::shell) filter_input: Entity<InputState>,
}

#[derive(Clone)]
pub(in crate::ui::shell) struct PortForwardingForms {
    pub(in crate::ui::shell) filter_input: Entity<InputState>,
    pub(in crate::ui::shell) profile_select:
        Entity<SelectState<SearchableVec<ForwardProfileSelectItem>>>,
    pub(in crate::ui::shell) label_input: Entity<InputState>,
    pub(in crate::ui::shell) listen_host_input: Entity<InputState>,
    pub(in crate::ui::shell) listen_port_input: Entity<InputState>,
    pub(in crate::ui::shell) target_host_input: Entity<InputState>,
    pub(in crate::ui::shell) target_port_input: Entity<InputState>,
}

#[derive(Clone)]
pub(in crate::ui::shell) struct SessionPanelForms {
    pub(in crate::ui::shell) hosts: HostsForms,
    pub(in crate::ui::shell) trusted: TrustedHostsForms,
    pub(in crate::ui::shell) forwarding: PortForwardingForms,
}

#[derive(Clone)]
pub(in crate::ui::shell) struct SessionCatalogViewState {
    pub(in crate::ui::shell) hosts_view_mode: ProfileViewMode,
    pub(in crate::ui::shell) forward_view_mode: ProfileViewMode,
    pub(in crate::ui::shell) snippets_view_mode: ProfileViewMode,
    pub(in crate::ui::shell) hosts_group_filter: Option<String>,
    pub(in crate::ui::shell) trusted_host_filter: TrustedHostFilter,
    pub(in crate::ui::shell) snippets_package_filter: Option<String>,
}

#[derive(Clone)]
pub(in crate::ui::shell) struct SessionEditorState {
    pub(in crate::ui::shell) host_editor_open: bool,
    pub(in crate::ui::shell) host_editor_is_new: bool,
    pub(in crate::ui::shell) snippets_editor_open: bool,
    pub(in crate::ui::shell) port_forward_editor_open: bool,
    pub(in crate::ui::shell) port_forward_editor_profile_id: Option<String>,
    pub(in crate::ui::shell) port_forward_editor_rule_id: Option<String>,
    pub(in crate::ui::shell) port_forward_kind: PortForwardKind,
}

impl SessionConnectionState {
    pub(in crate::ui::shell) fn preserves_terminal_history(&self) -> bool {
        matches!(self, Self::Failed { .. } | Self::Disconnected)
    }
}

impl SessionTabState {
    pub(in crate::ui::shell) fn set_connection_state(
        &mut self,
        connection_state: SessionConnectionState,
    ) {
        self.connection_state = connection_state;
        self.preserved_history_popup_hidden = false;
    }

    pub(in crate::ui::shell) fn preserves_terminal_history(&self) -> bool {
        self.purpose == SessionPurpose::Terminal
            && self.connection_state.preserves_terminal_history()
    }

    pub(in crate::ui::shell) fn uses_blocking_placeholder(&self) -> bool {
        !matches!(self.connection_state, SessionConnectionState::Ready)
            && !self.preserves_terminal_history()
    }

    pub(in crate::ui::shell) fn is_terminal_read_only(&self) -> bool {
        self.preserves_terminal_history()
    }

    pub(in crate::ui::shell) fn preserved_history_popup_hidden(&self) -> bool {
        self.preserves_terminal_history() && self.preserved_history_popup_hidden
    }

    pub(in crate::ui::shell) fn hide_preserved_history_popup(&mut self) {
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

pub(in crate::ui::shell) struct SessionController {
    host_password_visible: bool,
    services: SessionControllerServices,
    profiles: RefCell<Vec<SessionProfile>>,
    selected_profile: Cell<Option<usize>>,
    forms: Option<RefCell<SessionWorkspaceForms>>,
    host_editor_forms: Option<RefCell<HostEditorForms>>,
    snippets_forms: Option<RefCell<SnippetsForms>>,
    kbi_inputs: RefCell<Vec<Entity<InputState>>>,
    snippets: RefCell<Vec<SnippetRecord>>,
    selected_snippet: Cell<Option<usize>>,
    known_hosts_entries: RefCell<Vec<KnownHostEntry>>,
    panel_forms: Option<SessionPanelForms>,
    catalog_view: RefCell<SessionCatalogViewState>,
    editor_state: RefCell<SessionEditorState>,
    search_target: Cell<Option<TabId>>,
    tabs: RefCell<HashMap<TabId, SessionTabState>>,
    shared_profile_monitoring: RefCell<HashMap<String, SessionMonitoringState>>,
    monitor_source_tabs: RefCell<HashMap<String, TabId>>,
    monitor_scroll_handle: ScrollHandle,
    reported_terminal_focus_tab_id: RefCell<Option<TabId>>,
    panel: RefCell<SessionPanelState>,
    pending_dialogs: RefCell<SessionPendingDialogs>,
    ports: Rc<RefCell<SessionPortState>>,
    terminal_focus_subscriptions: RefCell<Option<(Subscription, Subscription)>>,
    _subscriptions: Vec<Subscription>,
}

pub(in crate::ui::shell) struct SessionControllerArgs {
    pub(in crate::ui::shell) runtime: TokioHandle,
    pub(in crate::ui::shell) session_store: Option<SessionStore>,
    pub(in crate::ui::shell) snippet_store: Option<SnippetStore>,
    pub(in crate::ui::shell) secrets: SecretStore,
    pub(in crate::ui::shell) known_hosts: KnownHostsStore,
    pub(in crate::ui::shell) profiles: Vec<SessionProfile>,
    pub(in crate::ui::shell) selected_profile: Option<usize>,
    pub(in crate::ui::shell) managed_keys: Vec<ManagedKeyRecord>,
    pub(in crate::ui::shell) snippets: Vec<SnippetRecord>,
    pub(in crate::ui::shell) selected_snippet: Option<usize>,
    pub(in crate::ui::shell) known_hosts_entries: Vec<KnownHostEntry>,
    pub(in crate::ui::shell) terminal_focus: FocusHandle,
    pub(in crate::ui::shell) local_vault_status: LocalVaultStatus,
    pub(in crate::ui::shell) auto_collect_session_monitoring: bool,
}

struct SessionFormsBundle {
    workspace: SessionWorkspaceForms,
    host_editor: HostEditorForms,
    snippets: SnippetsForms,
    panel: SessionPanelForms,
}

struct SessionControllerServices {
    runtime: Option<TokioHandle>,
    session_store: Option<SessionStore>,
    snippet_store: Option<SnippetStore>,
    secrets: SecretStore,
    known_hosts: KnownHostsStore,
    local_vault_status: LocalVaultStatus,
    auto_collect_session_monitoring: Cell<bool>,
}

#[derive(Default)]
struct SessionPanelState {
    open: bool,
    view: SessionSidePanelView,
    visible: bool,
    transition: Option<WorkspaceSidePanelTransition>,
    selected_known_host: Option<(String, u16, String)>,
}

#[derive(Default)]
struct SessionPendingDialogs {
    profile_delete: Option<PendingProfileDeleteState>,
    known_host_delete: Option<PendingKnownHostDeleteState>,
    snippet_delete: Option<PendingSnippetDeleteState>,
    port_forward_rule_delete: Option<PendingPortForwardRuleDeleteState>,
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
    fn available_session_charsets() -> Vec<String> {
        [
            "UTF-8",
            "GB18030",
            "GBK",
            "GB2312",
            "Big5",
            "Shift_JIS",
            "EUC-JP",
            "EUC-KR",
            "ISO-8859-1",
            "ISO-8859-15",
            "Windows-1252",
            "KOI8-R",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    fn build_forms(
        profiles: &[SessionProfile],
        selected_profile: Option<usize>,
        managed_keys: &[ManagedKeyRecord],
        snippets: &[SnippetRecord],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> SessionFormsBundle {
        let selection = initial_profile_selection(profiles, selected_profile);
        let selected_profile_data = selection.selected_profile_data;
        let selected_group = selection.selected_group;
        let selected_existing_group = selection.selected_existing_group;
        let name_input = new_input_state(
            i18n::string("placeholders.host_editor.profile_name"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.name.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let group_input = new_input_state(
            i18n::string("placeholders.host_editor.new_group_name"),
            if selected_existing_group.is_none() {
                selected_group.clone()
            } else {
                String::new()
            },
            false,
            window,
            cx,
        );
        let tags_input = new_input_state(
            i18n::string("placeholders.host_editor.tags"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.tags.join(", "))
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let host_input = new_input_state(
            i18n::string("placeholders.host_editor.host"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.host.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let port_input = new_input_state(
            "22",
            selected_profile_data
                .as_ref()
                .map(|profile| profile.port.to_string())
                .unwrap_or_else(|| "22".to_string()),
            false,
            window,
            cx,
        );
        let username_input = new_input_state(
            i18n::string("placeholders.host_editor.username"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.username.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let password_input = new_input_state(
            localized_secret_placeholder(
                selected_profile_data
                    .as_ref()
                    .is_some_and(|profile| profile.has_stored_password),
                "placeholders.host_editor.password",
            ),
            "",
            true,
            window,
            cx,
        );
        let private_key_input = new_input_state(
            i18n::string("placeholders.host_editor.private_key_path"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.private_key_path.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let agent_identity_input = new_input_state(
            i18n::string("placeholders.host_editor.agent_identity"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.agent_identity.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let certificate_input = new_input_state(
            i18n::string("placeholders.host_editor.certificate_path"),
            selected_profile_data
                .as_ref()
                .map(|profile| profile.certificate_path.clone())
                .unwrap_or_default(),
            false,
            window,
            cx,
        );
        let passphrase_input = new_input_state(
            localized_secret_placeholder(
                selected_profile_data
                    .as_ref()
                    .is_some_and(|profile| profile.has_stored_passphrase),
                "placeholders.host_editor.key_passphrase",
            ),
            "",
            true,
            window,
            cx,
        );
        let startup_command_value = selected_profile_data
            .as_ref()
            .map(|profile| profile.startup_command.clone())
            .unwrap_or_default();
        let startup_command_input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("bash")
                .indent_guides(false)
                .folding(false)
                .searchable(false)
                .rows(4)
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .placeholder("")
                .default_value(startup_command_value)
        });
        set_code_editor_input_placeholder(
            &startup_command_input,
            i18n::string("placeholders.host_editor.startup_command"),
            false,
            window,
            cx,
        );
        let proxy_jump_profile_ids = selected_profile_data
            .as_ref()
            .map(|profile| profile.proxy_jump_profile_ids.clone())
            .unwrap_or_default();
        let environment_variable_rows = selected_profile_data
            .as_ref()
            .map(|profile| {
                Self::host_editor_environment_variable_rows(
                    &profile.environment_variables,
                    window,
                    cx,
                )
            })
            .unwrap_or_else(|| Self::host_editor_environment_variable_rows(&[], window, cx));
        let group_select = cx.new(|cx| {
            let mut state = SelectState::new(
                SearchableVec::new(selection.available_groups),
                None,
                window,
                cx,
            );
            if let Some(existing_group) = selected_existing_group.as_ref() {
                state.set_selected_value(existing_group, window, cx);
            }
            state
        });
        let managed_key_select = cx.new(|cx| {
            let mut state = SelectState::new(
                ManagedKeySelectItem::options(managed_keys),
                None,
                window,
                cx,
            )
            .searchable(true);
            if let Some(managed_key_id) = selected_profile_data
                .as_ref()
                .map(|profile| profile.managed_key_id.trim().to_string())
                .filter(|managed_key_id| !managed_key_id.is_empty())
            {
                state.set_selected_value(&managed_key_id, window, cx);
            }
            state
        });
        let selected_charset = selected_profile_data
            .as_ref()
            .map(|profile| profile.charset.trim().to_string())
            .filter(|charset| !charset.is_empty())
            .unwrap_or_else(|| DEFAULT_SESSION_CHARSET.to_string());
        let charset_select = cx.new(|cx| {
            let mut state = SelectState::new(
                SearchableVec::new(Self::available_session_charsets()),
                None,
                window,
                cx,
            )
            .searchable(true);
            state.set_selected_value(&selected_charset, window, cx);
            state
        });
        let proxy_jump_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(Vec::<ProxyJumpCandidateSelectItem>::new()),
                None,
                window,
                cx,
            )
            .searchable(true)
        });
        let snippet_script_input = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("bash")
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .placeholder("")
        });
        set_code_editor_input_placeholder(
            &snippet_script_input,
            i18n::string("placeholders.snippets.script_body"),
            true,
            window,
            cx,
        );

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
                snippets_panel: WorkspaceSnippetsForms {
                    filter_input: new_input_state(
                        i18n::string("placeholders.workspace.snippet_filter"),
                        "",
                        false,
                        window,
                        cx,
                    ),
                    selected_package_filter: None,
                },
            },
            host_editor: HostEditorForms {
                name_input,
                group_input,
                group_select,
                managed_key_select,
                proxy_jump_select,
                charset_select,
                creating_new_group: !selected_group.is_empty() && selected_existing_group.is_none(),
                tags_input,
                host_input,
                port_input,
                username_input,
                password_input,
                private_key_input,
                agent_identity_input,
                certificate_input,
                passphrase_input,
                startup_command_input,
                proxy_jump_profile_ids,
                selected_proxy_jump_hop: None,
                environment_variable_rows,
                shell_type: selected_profile_data
                    .as_ref()
                    .map(|profile| profile.shell_type)
                    .unwrap_or_default(),
                editing_auth_method: selection.editing_auth_method,
                agent_forwarding_enabled: selected_profile_data
                    .as_ref()
                    .is_some_and(|profile| profile.agent_forwarding),
            },
            snippets: SnippetsForms {
                filter_input: new_input_state(
                    i18n::string("placeholders.snippets.filter"),
                    "",
                    false,
                    window,
                    cx,
                ),
                description_input: new_input_state(
                    i18n::string("placeholders.snippets.description_example"),
                    "",
                    false,
                    window,
                    cx,
                ),
                package_input: new_input_state(
                    i18n::string("placeholders.snippets.new_package_name"),
                    "",
                    false,
                    window,
                    cx,
                ),
                package_select: cx.new(|cx| {
                    SelectState::new(
                        SearchableVec::new(Self::collect_available_snippet_packages(snippets)),
                        None,
                        window,
                        cx,
                    )
                }),
                creating_new_package: snippets.is_empty(),
                script_input: snippet_script_input,
            },
            panel: SessionPanelForms {
                hosts: HostsForms {
                    filter_input: new_input_state(
                        i18n::string("placeholders.hosts.filter"),
                        "",
                        false,
                        window,
                        cx,
                    ),
                },
                trusted: TrustedHostsForms {
                    filter_input: new_input_state(
                        i18n::string("placeholders.trusted.filter"),
                        "",
                        false,
                        window,
                        cx,
                    ),
                },
                forwarding: PortForwardingForms {
                    filter_input: new_input_state(
                        i18n::string("placeholders.forward.filter"),
                        "",
                        false,
                        window,
                        cx,
                    ),
                    profile_select: cx.new(|cx| {
                        SelectState::new(Self::forward_profile_options(profiles), None, window, cx)
                            .searchable(true)
                    }),
                    label_input: new_input_state(
                        i18n::string("placeholders.forward.rule_label"),
                        "",
                        false,
                        window,
                        cx,
                    ),
                    listen_host_input: new_input_state(
                        i18n::string("placeholders.forward.listen_host"),
                        "",
                        false,
                        window,
                        cx,
                    ),
                    listen_port_input: new_input_state(
                        i18n::string("placeholders.forward.listen_port"),
                        "",
                        false,
                        window,
                        cx,
                    ),
                    target_host_input: new_input_state(
                        i18n::string("placeholders.forward.target_host"),
                        "",
                        false,
                        window,
                        cx,
                    ),
                    target_port_input: new_input_state(
                        i18n::string("placeholders.forward.target_port"),
                        "",
                        false,
                        window,
                        cx,
                    ),
                },
            },
        }
    }

    pub(in crate::ui::shell) fn refresh_localized_placeholders(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host_editor = self.host_editor_forms();
        let selected_profile = if self.editor_state().host_editor_is_new {
            None
        } else {
            self.selected_profile()
                .and_then(|index| self.profiles().get(index).cloned())
        };
        let has_saved_password = selected_profile
            .as_ref()
            .is_some_and(|profile| profile.has_stored_password);
        let has_saved_passphrase = selected_profile
            .as_ref()
            .is_some_and(|profile| profile.has_stored_passphrase);

        for (input, key) in [
            (
                &host_editor.name_input,
                "placeholders.host_editor.profile_name",
            ),
            (
                &host_editor.group_input,
                "placeholders.host_editor.new_group_name",
            ),
            (&host_editor.tags_input, "placeholders.host_editor.tags"),
            (&host_editor.host_input, "placeholders.host_editor.host"),
            (
                &host_editor.username_input,
                "placeholders.host_editor.username",
            ),
            (
                &host_editor.private_key_input,
                "placeholders.host_editor.private_key_path",
            ),
            (
                &host_editor.agent_identity_input,
                "placeholders.host_editor.agent_identity",
            ),
            (
                &host_editor.certificate_input,
                "placeholders.host_editor.certificate_path",
            ),
        ] {
            set_input_placeholder(input, i18n::string(key), window, cx);
        }
        set_input_placeholder(&host_editor.port_input, "22", window, cx);
        set_input_placeholder(
            &host_editor.password_input,
            localized_secret_placeholder(has_saved_password, "placeholders.host_editor.password"),
            window,
            cx,
        );
        set_input_placeholder(
            &host_editor.passphrase_input,
            localized_secret_placeholder(
                has_saved_passphrase,
                "placeholders.host_editor.key_passphrase",
            ),
            window,
            cx,
        );
        set_code_editor_input_placeholder(
            &host_editor.startup_command_input,
            i18n::string("placeholders.host_editor.startup_command"),
            false,
            window,
            cx,
        );
        for row in &host_editor.environment_variable_rows {
            set_input_placeholder(
                &row.name_input,
                i18n::string("placeholders.host_editor.environment_variable_name"),
                window,
                cx,
            );
            set_input_placeholder(
                &row.value_input,
                i18n::string("placeholders.host_editor.environment_variable_value"),
                window,
                cx,
            );
        }

        let panel = self.panel_forms();
        for (input, key) in [
            (
                &panel.forwarding.filter_input,
                "placeholders.forward.filter",
            ),
            (
                &panel.forwarding.label_input,
                "placeholders.forward.rule_label",
            ),
            (
                &panel.forwarding.listen_host_input,
                "placeholders.forward.listen_host",
            ),
            (
                &panel.forwarding.listen_port_input,
                "placeholders.forward.listen_port",
            ),
            (
                &panel.forwarding.target_host_input,
                "placeholders.forward.target_host",
            ),
            (
                &panel.forwarding.target_port_input,
                "placeholders.forward.target_port",
            ),
            (&panel.hosts.filter_input, "placeholders.hosts.filter"),
            (&panel.trusted.filter_input, "placeholders.trusted.filter"),
        ] {
            set_input_placeholder(input, i18n::string(key), window, cx);
        }

        let snippets = self.snippets_forms();
        for (input, key) in [
            (&snippets.filter_input, "placeholders.snippets.filter"),
            (
                &snippets.description_input,
                "placeholders.snippets.description_example",
            ),
            (
                &snippets.package_input,
                "placeholders.snippets.new_package_name",
            ),
        ] {
            set_input_placeholder(input, i18n::string(key), window, cx);
        }
        set_code_editor_input_placeholder(
            &snippets.script_input,
            i18n::string("placeholders.snippets.script_body"),
            true,
            window,
            cx,
        );
        set_input_placeholder(
            &self.terminal_search_input(),
            i18n::string("placeholders.workspace.search_scrollback"),
            window,
            cx,
        );
        set_input_placeholder(
            &self.workspace_snippets_filter_input(),
            i18n::string("placeholders.workspace.snippet_filter"),
            window,
            cx,
        );
    }

    pub(super) fn new(
        args: SessionControllerArgs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let forms = Self::build_forms(
            &args.profiles,
            args.selected_profile,
            &args.managed_keys,
            &args.snippets,
            window,
            cx,
        );
        macro_rules! subscribe_to_change {
            ($input:expr) => {
                cx.subscribe(&$input, |_controller, _input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                })
            };
        }
        let terminal_search_input = forms.workspace.search.input.clone();
        let group_input = forms.host_editor.group_input.clone();
        let group_select = forms.host_editor.group_select.clone();
        let managed_key_select = forms.host_editor.managed_key_select.clone();
        let proxy_jump_select = forms.host_editor.proxy_jump_select.clone();
        let session_snippets_filter_input = forms.workspace.snippets_panel.filter_input.clone();
        let snippets_filter_input = forms.snippets.filter_input.clone();
        let snippet_package_select = forms.snippets.package_select.clone();
        let hosts_filter_input = forms.panel.hosts.filter_input.clone();
        let trusted_filter_input = forms.panel.trusted.filter_input.clone();
        let forwarding_filter_input = forms.panel.forwarding.filter_input.clone();
        let forward_profile_select = forms.panel.forwarding.profile_select.clone();
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
        let mut subscriptions = vec![
            cx.subscribe(
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
            ),
            subscribe_to_change!(group_input),
            cx.subscribe(
                &group_select,
                |controller, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                    let SelectEvent::Confirm(selected_group) = event;
                    if selected_group.is_some() {
                        controller.host_editor_forms_mut().creating_new_group = false;
                    }
                    cx.notify();
                },
            ),
            cx.subscribe(
                &managed_key_select,
                |controller, _, event: &SelectEvent<SearchableVec<ManagedKeySelectItem>>, cx| {
                    let SelectEvent::Confirm(selected_managed_key_id) = event;
                    if selected_managed_key_id.is_some()
                        && controller.host_editor_forms().editing_auth_method
                            != AuthMethod::ManagedKey
                    {
                        controller.host_editor_forms_mut().editing_auth_method =
                            AuthMethod::ManagedKey;
                        cx.notify();
                    }
                },
            ),
            cx.subscribe(
                &proxy_jump_select,
                |controller,
                 _,
                 event: &SelectEvent<SearchableVec<ProxyJumpCandidateSelectItem>>,
                 cx| {
                    let SelectEvent::Confirm(selected_profile_id) = event;
                    if let Some(selected_profile_id) = selected_profile_id.as_deref() {
                        controller.add_proxy_jump_profile(selected_profile_id, cx);
                    }
                },
            ),
            subscribe_to_change!(hosts_filter_input),
            subscribe_to_change!(trusted_filter_input),
            subscribe_to_change!(forwarding_filter_input),
            cx.subscribe(
                &forward_profile_select,
                |controller,
                 _,
                 event: &SelectEvent<SearchableVec<ForwardProfileSelectItem>>,
                 cx| {
                    let SelectEvent::Confirm(selected_profile_id) = event;
                    controller.select_port_forward_editor_profile(selected_profile_id.clone(), cx);
                },
            ),
            subscribe_to_change!(snippets_filter_input),
            subscribe_to_change!(session_snippets_filter_input),
            cx.subscribe(
                &snippet_package_select,
                |controller, _, event: &SelectEvent<SearchableVec<String>>, cx| {
                    let SelectEvent::Confirm(selected_package) = event;
                    if selected_package.is_some() {
                        controller.set_snippets_creating_new_package(false);
                    }
                    cx.notify();
                },
            ),
        ];
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
                session_store: args.session_store,
                snippet_store: args.snippet_store,
                secrets: args.secrets,
                known_hosts: args.known_hosts,
                local_vault_status: args.local_vault_status,
                auto_collect_session_monitoring: Cell::new(args.auto_collect_session_monitoring),
            },
            args.profiles,
            args.selected_profile,
            Some(forms.workspace),
            Some(forms.host_editor),
            Some(forms.snippets),
            Some(forms.panel),
            args.snippets,
            args.selected_snippet,
            args.known_hosts_entries,
            subscriptions,
        );
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
        selected_profile: Option<usize>,
        forms: Option<SessionWorkspaceForms>,
        host_editor_forms: Option<HostEditorForms>,
        snippets_forms: Option<SnippetsForms>,
        panel_forms: Option<SessionPanelForms>,
        snippets: Vec<SnippetRecord>,
        selected_snippet: Option<usize>,
        known_hosts_entries: Vec<KnownHostEntry>,
        subscriptions: Vec<Subscription>,
    ) -> Self {
        let port_profiles = profiles.clone();
        Self {
            host_password_visible: false,
            services,
            profiles: RefCell::new(profiles),
            selected_profile: Cell::new(selected_profile),
            forms: forms.map(RefCell::new),
            host_editor_forms: host_editor_forms.map(RefCell::new),
            snippets_forms: snippets_forms.map(RefCell::new),
            kbi_inputs: RefCell::new(Vec::new()),
            snippets: RefCell::new(snippets),
            selected_snippet: Cell::new(selected_snippet),
            known_hosts_entries: RefCell::new(known_hosts_entries),
            panel_forms,
            catalog_view: RefCell::new(SessionCatalogViewState {
                hosts_view_mode: ProfileViewMode::Grid,
                forward_view_mode: ProfileViewMode::Grid,
                snippets_view_mode: ProfileViewMode::Grid,
                hosts_group_filter: None,
                trusted_host_filter: TrustedHostFilter::All,
                snippets_package_filter: None,
            }),
            editor_state: RefCell::new(SessionEditorState {
                host_editor_open: false,
                host_editor_is_new: false,
                snippets_editor_open: false,
                port_forward_editor_open: false,
                port_forward_editor_profile_id: None,
                port_forward_editor_rule_id: None,
                port_forward_kind: PortForwardKind::Local,
            }),
            search_target: Cell::new(None),
            tabs: RefCell::new(HashMap::new()),
            shared_profile_monitoring: RefCell::new(HashMap::new()),
            monitor_source_tabs: RefCell::new(HashMap::new()),
            monitor_scroll_handle: ScrollHandle::new(),
            reported_terminal_focus_tab_id: RefCell::new(None),
            panel: RefCell::new(SessionPanelState::default()),
            pending_dialogs: RefCell::new(SessionPendingDialogs::default()),
            ports: Rc::new(RefCell::new(SessionPortState {
                snapshot: SessionPortSnapshot::new(port_profiles, Vec::new(), None, None),
                ..SessionPortState::default()
            })),
            terminal_focus_subscriptions: RefCell::new(None),
            _subscriptions: subscriptions,
        }
    }

    pub(in crate::ui::shell) fn rebind_terminal_focus_events(
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
                session_store: None,
                snippet_store: None,
                secrets: SecretStore::new_locked_vault(),
                known_hosts: KnownHostsStore::with_path(
                    std::env::temp_dir().join("miaominal-session-controller-test-known-hosts"),
                ),
                local_vault_status: LocalVaultStatus::Locked,
                auto_collect_session_monitoring: Cell::new(false),
            },
            profiles,
            None,
            None,
            None,
            None,
            None,
            Vec::new(),
            None,
            Vec::new(),
            Vec::new(),
        )
    }

    fn forms(&self) -> &RefCell<SessionWorkspaceForms> {
        self.forms
            .as_ref()
            .expect("session workspace forms are available in the application")
    }

    pub(in crate::ui::shell) fn profiles(&self) -> Ref<'_, Vec<SessionProfile>> {
        self.profiles.borrow()
    }

    pub(in crate::ui::shell) fn profiles_mut(&self) -> RefMut<'_, Vec<SessionProfile>> {
        self.profiles.borrow_mut()
    }

    pub(in crate::ui::shell) fn replace_profiles(&self, profiles: Vec<SessionProfile>) {
        *self.profiles.borrow_mut() = profiles;
        self.sync_port_profiles();
    }

    pub(in crate::ui::shell) fn selected_profile(&self) -> Option<usize> {
        self.selected_profile.get()
    }

    pub(in crate::ui::shell) fn set_selected_profile(&self, selected: Option<usize>) {
        self.selected_profile.set(selected);
    }

    fn profile_service(&self) -> ProfileService {
        ProfileService::new(
            self.services.session_store.clone(),
            self.services.secrets.clone(),
        )
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

    pub(in crate::ui::shell) fn start_terminal_session(
        &self,
        profile: SessionProfile,
        columns: usize,
        lines: usize,
        monitoring_enabled: bool,
    ) -> SessionConnection {
        self.terminal_service().start_session(
            profile,
            self.profiles.borrow().clone(),
            columns,
            lines,
            monitoring_enabled,
        )
    }

    pub(in crate::ui::shell) fn known_hosts(&self) -> KnownHostsStore {
        self.services.known_hosts.clone()
    }

    pub(in crate::ui::shell) fn session_store_available(&self) -> bool {
        self.services.session_store.is_some()
    }

    pub(in crate::ui::shell) fn current_host_editor_profile_id(&self) -> Option<String> {
        let index = self.selected_profile.get()?;
        self.profiles
            .borrow()
            .get(index)
            .map(|profile| profile.id.clone())
    }

    pub(in crate::ui::shell) fn available_proxy_jump_candidates(
        &self,
        target_profile_id: &str,
    ) -> Vec<SessionProfile> {
        let forms = self.host_editor_forms();
        let chained_ids: HashSet<&str> = forms
            .proxy_jump_profile_ids
            .iter()
            .map(String::as_str)
            .collect();
        let mut candidates = self
            .profiles
            .borrow()
            .iter()
            .filter(|profile| {
                profile.id != target_profile_id && !chained_ids.contains(profile.id.as_str())
            })
            .cloned()
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        candidates
    }

    pub(in crate::ui::shell) fn collect_available_groups(
        profiles: &[SessionProfile],
    ) -> Vec<String> {
        let mut groups = profiles
            .iter()
            .filter_map(|profile| {
                let group = profile.group.trim();
                (!group.is_empty()).then(|| group.to_string())
            })
            .collect::<Vec<_>>();
        groups.sort_by_key(|group| group.to_ascii_lowercase());
        groups.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        groups
    }

    pub(in crate::ui::shell) fn available_groups(&self) -> Vec<String> {
        Self::collect_available_groups(&self.profiles.borrow())
    }

    pub(in crate::ui::shell) fn proxy_jump_chain_profiles(&self) -> Vec<SessionProfile> {
        let profile_ids = self.host_editor_forms().proxy_jump_profile_ids;
        let profiles = self.profiles.borrow();
        profile_ids
            .iter()
            .filter_map(|profile_id| {
                profiles
                    .iter()
                    .find(|profile| profile.id == *profile_id)
                    .cloned()
            })
            .collect()
    }

    pub(in crate::ui::shell) fn set_auth_method(
        &self,
        auth_method: AuthMethod,
        cx: &mut Context<Self>,
    ) {
        self.host_editor_forms_mut().editing_auth_method = match auth_method {
            AuthMethod::KeyFile => AuthMethod::ManagedKey,
            _ => auth_method,
        };
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_agent_forwarding_enabled(
        &self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        self.host_editor_forms_mut().agent_forwarding_enabled = enabled;
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_shell_type(
        &self,
        shell_type: ShellType,
        cx: &mut Context<Self>,
    ) {
        self.host_editor_forms_mut().shell_type = shell_type;
        cx.notify();
    }

    pub(in crate::ui::shell) fn begin_new_group(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let forms = self.host_editor_forms();
        self.host_editor_forms_mut().creating_new_group = true;
        forms.group_select.update(cx, |select, cx| {
            select.set_selected_index(None, window, cx);
        });
        set_input_value(&forms.group_input, "", window, cx);
        cx.notify();
    }

    fn new_host_editor_environment_variable_row<T: 'static>(
        name: impl Into<String>,
        value: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> HostEditorEnvironmentVariableRow {
        HostEditorEnvironmentVariableRow {
            name_input: new_input_state(
                i18n::string("placeholders.host_editor.environment_variable_name"),
                name.into(),
                false,
                window,
                cx,
            ),
            value_input: new_input_state(
                i18n::string("placeholders.host_editor.environment_variable_value"),
                value.into(),
                false,
                window,
                cx,
            ),
        }
    }

    pub(in crate::ui::shell) fn host_editor_environment_variable_rows<T: 'static>(
        variables: &[SessionEnvironmentVariable],
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Vec<HostEditorEnvironmentVariableRow> {
        if variables.is_empty() {
            return vec![Self::new_host_editor_environment_variable_row(
                "", "", window, cx,
            )];
        }

        variables
            .iter()
            .map(|variable| {
                Self::new_host_editor_environment_variable_row(
                    variable.name.clone(),
                    variable.value.clone(),
                    window,
                    cx,
                )
            })
            .collect()
    }

    pub(in crate::ui::shell) fn add_environment_variable_row(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let row = Self::new_host_editor_environment_variable_row("", "", window, cx);
        self.host_editor_forms_mut()
            .environment_variable_rows
            .push(row);
        cx.notify();
    }

    pub(in crate::ui::shell) fn remove_environment_variable_row(
        &self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let needs_replacement = {
            let mut forms = self.host_editor_forms_mut();
            if index < forms.environment_variable_rows.len() {
                forms.environment_variable_rows.remove(index);
            }
            forms.environment_variable_rows.is_empty()
        };
        if needs_replacement {
            let replacement = Self::new_host_editor_environment_variable_row("", "", window, cx);
            self.host_editor_forms_mut()
                .environment_variable_rows
                .push(replacement);
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn select_proxy_jump_step(
        &self,
        step_index: usize,
        cx: &mut Context<Self>,
    ) {
        let mut forms = self.host_editor_forms_mut();
        forms.selected_proxy_jump_hop = if step_index < forms.proxy_jump_profile_ids.len() {
            Some(step_index)
        } else {
            None
        };
        drop(forms);
        cx.notify();
    }

    pub(in crate::ui::shell) fn move_selected_proxy_jump_hop_up(&self, cx: &mut Context<Self>) {
        let mut forms = self.host_editor_forms_mut();
        let Some(selected_index) = forms.selected_proxy_jump_hop else {
            return;
        };
        if selected_index == 0 {
            return;
        }
        forms
            .proxy_jump_profile_ids
            .swap(selected_index - 1, selected_index);
        forms.selected_proxy_jump_hop = Some(selected_index - 1);
        drop(forms);
        cx.notify();
    }

    pub(in crate::ui::shell) fn move_selected_proxy_jump_hop_down(&self, cx: &mut Context<Self>) {
        let mut forms = self.host_editor_forms_mut();
        let Some(selected_index) = forms.selected_proxy_jump_hop else {
            return;
        };
        if selected_index + 1 >= forms.proxy_jump_profile_ids.len() {
            return;
        }
        forms
            .proxy_jump_profile_ids
            .swap(selected_index, selected_index + 1);
        forms.selected_proxy_jump_hop = Some(selected_index + 1);
        drop(forms);
        cx.notify();
    }

    pub(in crate::ui::shell) fn remove_selected_proxy_jump_hop(&self, cx: &mut Context<Self>) {
        let mut forms = self.host_editor_forms_mut();
        let Some(selected_index) = forms.selected_proxy_jump_hop else {
            return;
        };
        if selected_index >= forms.proxy_jump_profile_ids.len() {
            forms.selected_proxy_jump_hop = None;
            return;
        }
        forms.proxy_jump_profile_ids.remove(selected_index);
        forms.selected_proxy_jump_hop = if forms.proxy_jump_profile_ids.is_empty() {
            None
        } else {
            Some(selected_index.min(forms.proxy_jump_profile_ids.len() - 1))
        };
        drop(forms);
        self.sync_proxy_jump_candidate_select_in_active_window(cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn close_host_editor(&self, cx: &mut Context<Self>) {
        if !self.editor_state().host_editor_open {
            return;
        }
        self.set_host_editor_state(false, false);
        cx.notify();
    }

    pub(in crate::ui::shell) fn request_selected_profile_delete(&self, cx: &mut Context<Self>) {
        let Some(index) = self.selected_profile() else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "profile.messages.select_profile_to_delete",
            )));
            return;
        };
        let Some(profile) = self.profiles.borrow().get(index).cloned() else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "profile.messages.select_profile_to_delete",
            )));
            return;
        };
        self.set_pending_profile_delete(Some(PendingProfileDeleteState {
            profile_id: profile.id,
            profile_name: profile.name,
            reload_inputs_after_delete: true,
        }));
        cx.notify();
    }

    pub(in crate::ui::shell) fn request_new_profile_editor(&self, cx: &mut Context<Self>) {
        cx.emit(AppCommand::OpenTab(TabOpenRequest::NewProfileEditor));
    }

    pub(in crate::ui::shell) fn request_profile_editor_at_index(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(profile_id) = self
            .profiles
            .borrow()
            .get(index)
            .map(|profile| profile.id.clone())
        else {
            return;
        };
        self.request_profile_editor(profile_id, false, cx);
    }

    pub(in crate::ui::shell) fn request_profile_editor(
        &self,
        profile_id: String,
        open_hosts_tab: bool,
        cx: &mut Context<Self>,
    ) {
        if !self
            .profiles
            .borrow()
            .iter()
            .any(|profile| profile.id == profile_id)
        {
            cx.emit(AppCommand::Feedback(i18n::string(
                "trusted.messages.profile_not_found",
            )));
            return;
        }
        cx.emit(AppCommand::OpenTab(TabOpenRequest::ProfileEditor {
            profile_id,
            open_hosts_tab,
        }));
    }

    pub(in crate::ui::shell) fn request_profile_connection_test(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.read_profile_from_inputs(
            self.current_profile_id(),
            ProfileInputPurpose::ConnectionTest,
            cx,
        ) {
            Ok(profile) => {
                cx.emit(AppCommand::OpenTab(TabOpenRequest::ProfileConnectionTest {
                    profile: Box::new(profile),
                }));
            }
            Err(error) => {
                if let Some(validation) = error.downcast_ref::<ValidationFailure>() {
                    let message = validation.message.clone();
                    cx.emit(AppCommand::Feedback(message.clone()));
                    window.push_notification(validation_notification(validation.kind, message), cx);
                } else {
                    let message = error.to_string();
                    cx.emit(AppCommand::Feedback(i18n::string_args(
                        "profile.messages.test_connection_failed",
                        &[("message", &message)],
                    )));
                    window.push_notification(
                        error_notification(
                            i18n::string("profile.messages.test_connection_failed_title"),
                            message,
                        ),
                        cx,
                    );
                }
                cx.notify();
            }
        }
    }

    pub(in crate::ui::shell) fn request_profile_save(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.read_profile_save_draft(cx) {
            Ok(profile) => {
                cx.emit(AppCommand::SaveProfileRequested(Box::new(profile)));
            }
            Err(error) => self.report_profile_save_error(error, window, cx),
        }
    }

    pub(in crate::ui::shell) fn commit_profile_save_request(
        &mut self,
        profile: SessionProfile,
        managed_key_options: Vec<ManagedKeySelectItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.save_prepared_profile(profile) {
            Ok(profile) => {
                self.load_selected_profile_into_inputs(managed_key_options, window, cx);
                self.set_host_editor_state(false, false);
                let message = if self.session_store_available() {
                    i18n::string_args("profile.messages.saved", &[("name", &profile.name)])
                } else {
                    i18n::string_args(
                        "profile.messages.saved_memory_only",
                        &[("name", &profile.name)],
                    )
                };
                cx.emit(AppCommand::Feedback(message));
                cx.notify();
            }
            Err(error) => self.report_profile_save_error(error, window, cx),
        }
    }

    pub(in crate::ui::shell) fn request_sftp_at_profile_index(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(profile_id) = self
            .profiles
            .borrow()
            .get(index)
            .map(|profile| profile.id.clone())
        else {
            return;
        };
        cx.emit(AppCommand::OpenTab(TabOpenRequest::Sftp {
            profile_id,
            owner: None,
        }));
    }

    pub(in crate::ui::shell) fn request_session_at_profile_index(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(profile_id) = self
            .profiles
            .borrow()
            .get(index)
            .map(|profile| profile.id.clone())
        else {
            return;
        };
        cx.emit(AppCommand::OpenTab(TabOpenRequest::Session { profile_id }));
    }

    pub(in crate::ui::shell) fn host_password_reveal_icon(&self) -> AppIcon {
        if self.host_password_visible() {
            AppIcon::EyeOff
        } else {
            AppIcon::Eye
        }
    }

    pub(in crate::ui::shell) fn host_editor_auth_method(auth_method: AuthMethod) -> AuthMethod {
        match auth_method {
            AuthMethod::KeyFile => AuthMethod::ManagedKey,
            _ => auth_method,
        }
    }

    fn saved_secret_placeholder(has_saved: bool, fallback_key: &'static str) -> String {
        if has_saved {
            i18n::string("placeholders.saved.keep_existing")
        } else {
            i18n::string(fallback_key)
        }
    }

    fn sync_group_controls(&self, group: &str, window: &mut Window, cx: &mut Context<Self>) {
        let group = group.trim();
        let available_groups = self.available_groups();
        let selected_existing_group = available_groups
            .iter()
            .find(|candidate| candidate.eq_ignore_ascii_case(group))
            .cloned();
        let forms = self.host_editor_forms();

        forms.group_select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(available_groups), window, cx);
            if let Some(existing_group) = selected_existing_group.as_ref() {
                select.set_selected_value(existing_group, window, cx);
            } else {
                select.set_selected_index(None, window, cx);
            }
        });

        let creating_new_group = !group.is_empty() && selected_existing_group.is_none();
        self.host_editor_forms_mut().creating_new_group = creating_new_group;
        set_input_value(
            &forms.group_input,
            if creating_new_group {
                group.to_string()
            } else {
                String::new()
            },
            window,
            cx,
        );
    }

    pub(in crate::ui::shell) fn sync_managed_key_select(
        &self,
        options: Vec<ManagedKeySelectItem>,
        selected_key_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let forms = self.host_editor_forms();
        let selected_key_id = selected_key_id
            .map(str::to_string)
            .or_else(|| forms.managed_key_select.read(cx).selected_value().cloned());
        let has_selected_key = selected_key_id.as_ref().is_some_and(|selected_key_id| {
            options.iter().any(|item| item.value() == selected_key_id)
        });

        forms.managed_key_select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(options), window, cx);
            if has_selected_key {
                if let Some(selected_key_id) = selected_key_id.as_ref() {
                    select.set_selected_value(selected_key_id, window, cx);
                }
            } else {
                select.set_selected_index(None, window, cx);
            }
        });
    }

    pub(in crate::ui::shell) fn sync_managed_key_select_in_active_window(
        &self,
        options: Vec<ManagedKeySelectItem>,
        selected_key_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let forms = self.host_editor_forms();
        let selected_key_id =
            selected_key_id.or_else(|| forms.managed_key_select.read(cx).selected_value().cloned());
        let has_selected_key = selected_key_id.as_ref().is_some_and(|selected_key_id| {
            options.iter().any(|item| item.value() == selected_key_id)
        });
        let managed_key_select = forms.managed_key_select;

        if let Some(window_handle) = cx.active_window()
            && let Err(error) = window_handle.update(cx, move |_, window, cx| {
                managed_key_select.update(cx, |select, cx| {
                    select.set_items(SearchableVec::new(options), window, cx);
                    if has_selected_key {
                        if let Some(selected_key_id) = selected_key_id.as_ref() {
                            select.set_selected_value(selected_key_id, window, cx);
                        }
                    } else {
                        select.set_selected_index(None, window, cx);
                    }
                });
            })
        {
            log::debug!("failed to refresh managed key options: {error:?}");
        }
    }

    fn sync_proxy_jump_candidate_select(
        &self,
        selected_profile_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target_profile_id = self.current_host_editor_profile_id().unwrap_or_default();
        let options = SearchableVec::new(
            self.available_proxy_jump_candidates(&target_profile_id)
                .iter()
                .map(ProxyJumpCandidateSelectItem::new)
                .collect::<Vec<_>>(),
        );
        let selected_profile_id = selected_profile_id.map(str::to_string);
        let proxy_jump_select = self.host_editor_forms().proxy_jump_select;

        proxy_jump_select.update(cx, |select, cx| {
            select.set_items(options, window, cx);
            if let Some(selected_profile_id) = selected_profile_id.as_ref() {
                select.set_selected_value(selected_profile_id, window, cx);
            } else {
                select.set_selected_index(None, window, cx);
            }
        });
    }

    pub(in crate::ui::shell) fn set_host_password_visibility(
        &mut self,
        visible: bool,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.host_password_visible = visible;
        let password_input = self.host_editor_forms().password_input;
        set_input_masked(&password_input, !visible, focus, window, cx);
    }

    pub(in crate::ui::shell) fn load_selected_profile_into_inputs(
        &mut self,
        managed_key_options: Vec<ManagedKeySelectItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile = self
            .selected_profile()
            .and_then(|index| self.profiles.borrow().get(index).cloned());
        if let Some(profile) = profile {
            self.populate_profile_inputs(&profile, managed_key_options, window, cx);
        } else {
            self.clear_profile_inputs(managed_key_options, window, cx);
        }
    }

    fn populate_profile_inputs(
        &mut self,
        profile: &SessionProfile,
        managed_key_options: Vec<ManagedKeySelectItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_host_password_visibility(false, false, window, cx);
        let forms = self.host_editor_forms();
        set_input_value(&forms.name_input, profile.name.clone(), window, cx);
        self.sync_group_controls(&profile.group, window, cx);
        set_input_value(&forms.tags_input, profile.tags.join(", "), window, cx);
        set_input_value(&forms.host_input, profile.host.clone(), window, cx);
        set_input_value(&forms.port_input, profile.port.to_string(), window, cx);
        set_input_value(&forms.username_input, profile.username.clone(), window, cx);
        set_input_value(&forms.password_input, "", window, cx);
        set_input_placeholder(
            &forms.password_input,
            Self::saved_secret_placeholder(
                profile.has_stored_password,
                "placeholders.host_editor.password",
            ),
            window,
            cx,
        );
        set_input_value(
            &forms.private_key_input,
            profile.private_key_path.clone(),
            window,
            cx,
        );
        self.sync_managed_key_select(
            managed_key_options,
            Some(&profile.managed_key_id),
            window,
            cx,
        );
        set_input_value(
            &forms.agent_identity_input,
            profile.agent_identity.clone(),
            window,
            cx,
        );
        set_input_value(
            &forms.certificate_input,
            profile.certificate_path.clone(),
            window,
            cx,
        );
        set_input_value(&forms.passphrase_input, "", window, cx);
        set_input_placeholder(
            &forms.passphrase_input,
            Self::saved_secret_placeholder(
                profile.has_stored_passphrase,
                "placeholders.host_editor.key_passphrase",
            ),
            window,
            cx,
        );
        set_input_value(
            &forms.startup_command_input,
            profile.startup_command.clone(),
            window,
            cx,
        );
        let selected_charset = if profile.charset.trim().is_empty() {
            DEFAULT_SESSION_CHARSET.to_string()
        } else {
            profile.charset.clone()
        };
        forms.charset_select.update(cx, |select, cx| {
            select.set_selected_value(&selected_charset, window, cx);
        });
        {
            let mut forms = self.host_editor_forms_mut();
            forms.proxy_jump_profile_ids = profile.proxy_jump_profile_ids.clone();
            forms.selected_proxy_jump_hop = None;
        }
        self.sync_proxy_jump_candidate_select(None, window, cx);
        let environment_variable_rows =
            Self::host_editor_environment_variable_rows(&profile.environment_variables, window, cx);
        let mut forms = self.host_editor_forms_mut();
        forms.environment_variable_rows = environment_variable_rows;
        forms.shell_type = profile.shell_type;
        forms.editing_auth_method = Self::host_editor_auth_method(profile.effective_auth_method());
        forms.agent_forwarding_enabled = profile.agent_forwarding;
    }

    fn clear_profile_inputs(
        &mut self,
        managed_key_options: Vec<ManagedKeySelectItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_host_password_visibility(false, false, window, cx);
        let forms = self.host_editor_forms();
        set_input_value(&forms.name_input, "", window, cx);
        self.sync_group_controls("", window, cx);
        set_input_value(&forms.tags_input, "", window, cx);
        set_input_value(&forms.host_input, "", window, cx);
        set_input_value(&forms.port_input, "22", window, cx);
        set_input_value(&forms.username_input, "", window, cx);
        set_input_value(&forms.password_input, "", window, cx);
        set_input_placeholder(
            &forms.password_input,
            i18n::string("placeholders.host_editor.password"),
            window,
            cx,
        );
        set_input_value(&forms.private_key_input, "", window, cx);
        self.sync_managed_key_select(managed_key_options, Some(""), window, cx);
        set_input_value(&forms.agent_identity_input, "", window, cx);
        set_input_value(&forms.certificate_input, "", window, cx);
        set_input_value(&forms.passphrase_input, "", window, cx);
        set_input_placeholder(
            &forms.passphrase_input,
            i18n::string("placeholders.host_editor.key_passphrase"),
            window,
            cx,
        );
        set_input_value(&forms.startup_command_input, "", window, cx);
        forms.charset_select.update(cx, |select, cx| {
            select.set_selected_value(&DEFAULT_SESSION_CHARSET.to_string(), window, cx);
        });
        {
            let mut forms = self.host_editor_forms_mut();
            forms.proxy_jump_profile_ids.clear();
            forms.selected_proxy_jump_hop = None;
        }
        self.sync_proxy_jump_candidate_select(None, window, cx);
        let environment_variable_rows =
            Self::host_editor_environment_variable_rows(&[], window, cx);
        let mut forms = self.host_editor_forms_mut();
        forms.environment_variable_rows = environment_variable_rows;
        forms.shell_type = ShellType::Posix;
        forms.editing_auth_method = AuthMethod::Password;
        forms.agent_forwarding_enabled = false;
    }

    pub(in crate::ui::shell) fn add_profile(
        &mut self,
        managed_key_options: Vec<ManagedKeySelectItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile =
            SessionProfile::blank(self.next_profile_id(), self.profiles.borrow().len() + 1);
        self.set_selected_profile(None);
        self.set_host_editor_state(true, true);
        self.populate_profile_inputs(&profile, managed_key_options, window, cx);
        cx.emit(AppCommand::Feedback(i18n::string(
            "profile.messages.new_profile_created",
        )));
        cx.notify();
    }

    pub(in crate::ui::shell) fn open_profile_editor(
        &mut self,
        index: usize,
        managed_key_options: Vec<ManagedKeySelectItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = self.profiles.borrow().get(index).cloned() else {
            return;
        };

        self.set_selected_profile(Some(index));
        self.populate_profile_inputs(&profile, managed_key_options, window, cx);
        self.set_host_editor_state(true, false);
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "navigation.messages.editing_profile",
            &[("name", &profile.name)],
        )));
        cx.notify();
    }

    pub(in crate::ui::shell) fn delete_profile_by_id(
        &mut self,
        profile_id: &str,
        profile_name: &str,
        reload_inputs_after_delete: bool,
        managed_key_options: Vec<ManagedKeySelectItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .profiles
            .borrow()
            .iter()
            .position(|profile| profile.id == profile_id)
        else {
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "profile.messages.already_removed",
                &[("name", profile_name)],
            )));
            cx.notify();
            return;
        };

        let deleted_selected_profile = self.selected_profile() == Some(index);
        let service = self.profile_service();
        let mut selected_profile = self.selected_profile();
        let outcome = {
            let mut profiles = self.profiles.borrow_mut();
            service.delete_profile(&mut profiles, &mut selected_profile, index)
        };
        let Some(outcome) = outcome else {
            return;
        };
        self.set_selected_profile(selected_profile);

        if deleted_selected_profile {
            self.set_host_editor_state(false, false);
        }

        let message = if let Err(error) = self.persist_profiles() {
            i18n::string_args(
                "profile.messages.deleted_local_save_failed",
                &[
                    ("name", &outcome.removed.name),
                    ("error", &error.to_string()),
                ],
            )
        } else {
            i18n::string_args(
                "profile.messages.deleted",
                &[("name", &outcome.removed.name)],
            )
        };

        if reload_inputs_after_delete {
            self.load_selected_profile_into_inputs(managed_key_options, window, cx);
        }

        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    pub(in crate::ui::shell) fn load_selected_profile_password_input(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let Some(index) = self.selected_profile() else {
            return Ok(());
        };
        let Some(profile) = self.profiles.borrow().get(index).cloned() else {
            return Ok(());
        };

        if !profile.has_stored_password {
            return Ok(());
        }

        let password = self
            .services
            .secrets
            .get(&profile.id, SecretKind::Password)?
            .unwrap_or_default();
        let password_input = self.host_editor_forms().password_input;
        set_input_value(&password_input, password, window, cx);
        Ok(())
    }

    pub(in crate::ui::shell) fn prepare_host_password_for_lock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.selected_profile() else {
            return;
        };
        let Some(profile) = self.profiles.borrow().get(index).cloned() else {
            return;
        };
        if !profile.has_stored_password {
            return;
        }

        let password_input = self.host_editor_forms().password_input;
        let current_password = password_input.read(cx).value().to_string();
        if current_password.is_empty() {
            self.set_host_password_visibility(false, false, window, cx);
            return;
        }

        match self.services.secrets.get(&profile.id, SecretKind::Password) {
            Ok(Some(stored_password)) if stored_password == current_password => {
                set_input_value(&password_input, "", window, cx);
            }
            Ok(_) => {}
            Err(error) => {
                log::warn!(
                    "failed to compare stored host password before locking local vault: {error:?}"
                );
            }
        }

        self.set_host_password_visibility(false, false, window, cx);
    }

    pub(in crate::ui::shell) fn reveal_host_password_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.load_selected_profile_password_input(window, cx) {
            let message = error.to_string();
            cx.emit(AppCommand::Feedback(message.clone()));
            window.push_notification(
                error_notification(
                    i18n::string("settings.sync.vault.notifications.failed_title"),
                    message,
                ),
                cx,
            );
            cx.notify();
            return;
        }

        self.set_host_password_visibility(true, true, window, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn toggle_host_password_visibility(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.host_password_visible() {
            self.set_host_password_visibility(false, true, window, cx);
            cx.notify();
            return;
        }

        let forms = self.host_editor_forms();
        let has_text = !forms.password_input.read(cx).value().is_empty();
        let has_stored_password = self
            .selected_profile()
            .and_then(|index| self.profiles.borrow().get(index).cloned())
            .is_some_and(|profile| profile.has_stored_password);

        if has_text || !has_stored_password {
            self.set_host_password_visibility(true, true, window, cx);
            cx.notify();
            return;
        }

        if self.services.local_vault_status == LocalVaultStatus::Locked {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Settings(
                super::SettingsDeferredCommand::RevealSecret(
                    crate::ui::shell::SecretRevealTarget::HostPassword,
                ),
            )));
            return;
        }

        self.reveal_host_password_input(window, cx);
    }

    pub(in crate::ui::shell) fn import_profiles(
        &self,
        batch: ImportedBatch,
    ) -> anyhow::Result<ImportedProfilesResult> {
        let service = self.profile_service();
        service.import_profiles(&mut self.profiles.borrow_mut(), batch)
    }

    fn group_value(&self, cx: &App) -> String {
        let forms = self.host_editor_forms();
        if forms.creating_new_group {
            forms.group_input.read(cx).value().trim().to_string()
        } else {
            forms
                .group_select
                .read(cx)
                .selected_value()
                .cloned()
                .unwrap_or_default()
                .trim()
                .to_string()
        }
    }

    fn session_charset_value(&self, cx: &App) -> String {
        self.host_editor_forms()
            .charset_select
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_else(|| DEFAULT_SESSION_CHARSET.to_string())
            .trim()
            .to_string()
    }

    fn read_environment_variables(
        &self,
        cx: &App,
    ) -> anyhow::Result<Vec<SessionEnvironmentVariable>> {
        let mut variables = Vec::new();
        let forms = self.host_editor_forms();

        for (index, row) in forms.environment_variable_rows.iter().enumerate() {
            let raw_name = row.name_input.read(cx).value().to_string();
            let raw_value = row.value_input.read(cx).value().to_string();
            let name = raw_name.trim().to_string();

            if name.is_empty() {
                if raw_value.trim().is_empty() {
                    continue;
                }

                let index = (index + 1).to_string();
                anyhow::bail!(i18n::string_args(
                    "errors.host_editor.environment_variables.missing_name",
                    &[("index", &index)],
                ));
            }

            if !is_valid_environment_variable_name(&name) {
                anyhow::bail!(i18n::string_args(
                    "errors.host_editor.environment_variables.invalid_name",
                    &[("name", &name)],
                ));
            }

            variables.push(SessionEnvironmentVariable {
                name,
                value: raw_value,
            });
        }

        Ok(variables)
    }

    fn read_proxy_jump_profile_ids(&self, target_profile_id: &str) -> anyhow::Result<Vec<String>> {
        let mut seen = HashSet::new();
        let mut resolved = Vec::new();
        let forms = self.host_editor_forms();
        let profiles = self.profiles.borrow();

        for profile_id in &forms.proxy_jump_profile_ids {
            let profile_id = profile_id.trim();
            if profile_id.is_empty() {
                continue;
            }
            if profile_id == target_profile_id {
                anyhow::bail!("host chaining cannot reference the host being edited");
            }
            if !seen.insert(profile_id.to_string()) {
                anyhow::bail!("host chaining cannot include the same saved host more than once");
            }

            let profile = profiles
                .iter()
                .find(|profile| profile.id == profile_id)
                .ok_or_else(|| anyhow::anyhow!("a selected jump host is no longer available"))?;
            resolved.push(profile.id.clone());
        }

        Ok(resolved)
    }

    pub(in crate::ui::shell) fn current_profile_id(&self) -> String {
        self.selected_profile()
            .and_then(|index| {
                self.profiles
                    .borrow()
                    .get(index)
                    .map(|profile| profile.id.clone())
            })
            .unwrap_or_else(|| self.next_profile_id())
    }

    pub(in crate::ui::shell) fn next_profile_id(&self) -> String {
        self.profile_service()
            .next_profile_id(&self.profiles.borrow())
    }

    fn read_profile_from_inputs(
        &self,
        profile_id: String,
        purpose: ProfileInputPurpose,
        cx: &App,
    ) -> anyhow::Result<SessionProfile> {
        let forms = self.host_editor_forms();
        let name = forms.name_input.read(cx).value().to_string();
        let group = self.group_value(cx);
        let tags_text = forms.tags_input.read(cx).value().to_string();
        let host = forms.host_input.read(cx).value().to_string();
        let port_text = forms.port_input.read(cx).value().to_string();
        let username = forms.username_input.read(cx).value().to_string();
        let password = forms.password_input.read(cx).value().to_string();
        let private_key_path = forms.private_key_input.read(cx).value().to_string();
        let managed_key_id = forms
            .managed_key_select
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_default();
        let agent_identity = forms.agent_identity_input.read(cx).value().to_string();
        let certificate_path = forms.certificate_input.read(cx).value().to_string();
        let passphrase = forms.passphrase_input.read(cx).value().to_string();
        let startup_command = forms.startup_command_input.read(cx).value().to_string();
        let charset = self.session_charset_value(cx);

        let name = name.trim().to_string();
        let group = group.trim().to_string();
        let tags = ProfileService::parse_tags(&tags_text);
        let host = host.trim().to_string();
        let username = username.trim().to_string();
        let private_key_path = private_key_path.trim().to_string();
        let managed_key_id = managed_key_id.trim().to_string();
        let agent_identity = agent_identity.trim().to_string();
        let certificate_path = certificate_path.trim().to_string();
        let startup_command = startup_command.trim().to_string();
        let charset = if charset.is_empty() {
            DEFAULT_SESSION_CHARSET.to_string()
        } else {
            charset
        };
        let environment_variables = self.read_environment_variables(cx)?;
        let proxy_jump_profile_ids = self.read_proxy_jump_profile_ids(&profile_id)?;

        if purpose.requires_name() && name.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.profile.validation.profile_name_required",
            ))
            .into());
        }
        if host.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.profile.validation.host_required",
            ))
            .into());
        }
        if username.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.profile.validation.username_required",
            ))
            .into());
        }

        let name = if name.is_empty() {
            format!("{}@{}", username, host)
        } else {
            name
        };
        let port: u16 = port_text.trim().parse().map_err(|_| {
            anyhow::anyhow!(ValidationFailure::invalid(i18n::string_args(
                "errors.profile.validation.invalid_port",
                &[("port", &port_text)],
            )))
        })?;

        let profiles = self.profiles.borrow();
        let existing = profiles.iter().find(|profile| profile.id == profile_id);
        let prior_password = existing.is_some_and(|profile| profile.has_stored_password);
        let prior_passphrase = existing.is_some_and(|profile| profile.has_stored_passphrase);
        let has_password = !password.trim().is_empty();
        let has_passphrase = !passphrase.trim().is_empty();
        let auth_method = Self::host_editor_auth_method(forms.editing_auth_method);

        match auth_method {
            AuthMethod::Password => {
                if !has_password && !prior_password {
                    return Err(ValidationFailure::required(i18n::string(
                        "errors.profile.validation.password_requires_password",
                    ))
                    .into());
                }
            }
            AuthMethod::KeyFile | AuthMethod::ManagedKey => {
                if managed_key_id.is_empty() {
                    return Err(ValidationFailure::required(i18n::string(
                        "errors.profile.validation.managed_key_requires_id",
                    ))
                    .into());
                }
            }
            AuthMethod::Agent => {
                if agent_identity.is_empty() {
                    return Err(ValidationFailure::required(i18n::string(
                        "errors.profile.validation.ssh_agent_requires_identity",
                    ))
                    .into());
                }
            }
            AuthMethod::KeyboardInteractive => {}
        }

        let private_key_path = if matches!(auth_method, AuthMethod::KeyFile) {
            private_key_path
        } else {
            String::new()
        };
        let passphrase = if matches!(auth_method, AuthMethod::KeyFile) {
            passphrase
        } else {
            String::new()
        };
        let has_stored_passphrase = if matches!(auth_method, AuthMethod::KeyFile) {
            has_passphrase || prior_passphrase
        } else {
            false
        };

        if !certificate_path.is_empty() && matches!(auth_method, AuthMethod::Password) {
            return Err(ValidationFailure::invalid(i18n::string(
                "errors.profile.validation.certificate_requires_key_based_identity",
            ))
            .into());
        }

        Ok(SessionProfile {
            id: profile_id,
            name,
            group,
            tags,
            kind: existing.map(|profile| profile.kind).unwrap_or_default(),
            host,
            port,
            username,
            password,
            auth_method: Some(auth_method),
            private_key_path,
            passphrase,
            managed_key_id,
            agent_identity: agent_identity.clone(),
            agent_identity_label: existing
                .map(|profile| profile.agent_identity_label.clone())
                .filter(|label| !label.trim().is_empty())
                .unwrap_or(agent_identity),
            certificate_path,
            agent_forwarding: forms.agent_forwarding_enabled,
            startup_command,
            charset,
            environment_variables,
            shell_type: forms.shell_type,
            proxy_jump_profile_ids,
            has_stored_password: has_password || prior_password,
            has_stored_passphrase,
            port_forwarding_rules: existing
                .map(|profile| profile.port_forwarding_rules.clone())
                .unwrap_or_default(),
            is_favorite: existing.map(|profile| profile.is_favorite).unwrap_or(false),
            last_connected_at: existing.and_then(|profile| profile.last_connected_at),
        })
    }

    pub(in crate::ui::shell) fn read_profile_save_draft(
        &self,
        cx: &App,
    ) -> anyhow::Result<SessionProfile> {
        self.read_profile_from_inputs(self.current_profile_id(), ProfileInputPurpose::Save, cx)
    }

    pub(in crate::ui::shell) fn save_prepared_profile(
        &self,
        profile: SessionProfile,
    ) -> anyhow::Result<SessionProfile> {
        let service = self.profile_service();
        service.commit_profile_secrets(&profile)?;
        let mut selected_profile = self.selected_profile();
        {
            let mut profiles = self.profiles.borrow_mut();
            service.upsert_profile(&mut profiles, &mut selected_profile, profile.clone());
        }
        self.selected_profile.set(selected_profile);
        self.persist_profiles()?;
        Ok(profile)
    }

    fn report_profile_save_error(
        &self,
        error: anyhow::Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(validation) = error.downcast_ref::<ValidationFailure>() {
            let message = validation.message.clone();
            cx.emit(AppCommand::Feedback(message.clone()));
            window.push_notification(validation_notification(validation.kind, message), cx);
        } else {
            let message = error.to_string();
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "profile.messages.save_failed",
                &[("message", &message)],
            )));
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn continue_save_profile_after_unlock(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile = match self.read_profile_save_draft(cx) {
            Ok(profile) => profile,
            Err(error) => {
                self.report_profile_save_error(error, window, cx);
                return;
            }
        };

        let service = self.profile_service();
        let mut profiles = self.profiles.borrow().clone();
        let mut selected_profile = self.selected_profile();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawn_result = std::thread::Builder::new()
            .name("post-unlock-profile-save".to_string())
            .spawn(move || {
                let result = (|| -> anyhow::Result<SaveProfileAfterUnlockResult> {
                    service.commit_profile_secrets(&profile)?;
                    service.upsert_profile(&mut profiles, &mut selected_profile, profile.clone());
                    service.persist_sessions(&profiles)?;
                    Ok(SaveProfileAfterUnlockResult {
                        profile,
                        profiles,
                        selected_profile,
                    })
                })();
                tx.send(result).ok();
            });

        if let Err(error) = spawn_result {
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "profile.messages.save_failed",
                &[("message", &error.to_string())],
            )));
            cx.notify();
            return;
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv().unwrap_or_else(|_| {
                        Err(anyhow::anyhow!("post-unlock profile save task cancelled"))
                    })
                })
                .await;

            let _ = this.update(cx, move |controller, cx| match result {
                Ok(result) => {
                    controller.replace_profiles(result.profiles);
                    controller.set_selected_profile(result.selected_profile);
                    controller.set_host_editor_state(false, false);
                    let message = if controller.session_store_available() {
                        i18n::string_args(
                            "profile.messages.saved",
                            &[("name", &result.profile.name)],
                        )
                    } else {
                        i18n::string_args(
                            "profile.messages.saved_memory_only",
                            &[("name", &result.profile.name)],
                        )
                    };
                    cx.emit(AppCommand::Feedback(message));
                    cx.notify();
                }
                Err(error) => {
                    cx.emit(AppCommand::Feedback(i18n::string_args(
                        "profile.messages.save_failed",
                        &[("message", &error.to_string())],
                    )));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn sync_proxy_jump_candidate_select_in_active_window(&self, cx: &mut Context<Self>) {
        let target_profile_id = self.current_host_editor_profile_id().unwrap_or_default();
        let options = SearchableVec::new(
            self.available_proxy_jump_candidates(&target_profile_id)
                .iter()
                .map(ProxyJumpCandidateSelectItem::new)
                .collect::<Vec<_>>(),
        );
        let select = self.host_editor_forms().proxy_jump_select;
        if let Some(window_handle) = cx.active_window()
            && let Err(error) = window_handle.update(cx, move |_, window, cx| {
                select.update(cx, |select, cx| {
                    select.set_items(options, window, cx);
                    select.set_selected_index(None, window, cx);
                });
            })
        {
            log::debug!("failed to refresh proxy jump candidates: {error:?}");
        }
    }

    fn add_proxy_jump_profile(&self, profile_id: &str, cx: &mut Context<Self>) {
        let forms = self.host_editor_forms();
        if forms
            .proxy_jump_profile_ids
            .iter()
            .any(|existing| existing == profile_id)
            || self
                .current_host_editor_profile_id()
                .is_some_and(|current_id| current_id == profile_id)
        {
            return;
        }

        {
            let mut forms = self.host_editor_forms_mut();
            forms.proxy_jump_profile_ids.push(profile_id.to_string());
            forms.selected_proxy_jump_hop = Some(forms.proxy_jump_profile_ids.len() - 1);
        }
        self.sync_proxy_jump_candidate_select_in_active_window(cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn localized_port_forward_kind_label(kind: PortForwardKind) -> String {
        localized_port_forward_kind_label(kind)
    }

    pub(in crate::ui::shell) fn rule_summary_label(rule: &PortForwardRule) -> String {
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

    pub(in crate::ui::shell) fn forward_profile_options(
        profiles: &[SessionProfile],
    ) -> SearchableVec<ForwardProfileSelectItem> {
        SearchableVec::new(
            profiles
                .iter()
                .map(ForwardProfileSelectItem::new)
                .collect::<Vec<_>>(),
        )
    }

    pub(in crate::ui::shell) fn sync_port_forward_profile_select(
        &self,
        selected_profile_id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let options = Self::forward_profile_options(&self.profiles.borrow());
        let selected_profile_id = selected_profile_id.map(str::to_string);
        let profile_select = self.panel_forms().forwarding.profile_select;

        profile_select.update(cx, |select, cx| {
            select.set_items(options, window, cx);
            if let Some(selected_profile_id) = selected_profile_id.as_ref() {
                select.set_selected_value(selected_profile_id, window, cx);
            } else {
                select.set_selected_index(None, window, cx);
            }
        });
    }

    pub(in crate::ui::shell) fn port_forward_rule_indices(
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

    pub(in crate::ui::shell) fn update_port_forward_rule_enabled_state(
        &self,
        profile_id: &str,
        rule_id: &str,
        enabled: bool,
    ) -> Option<(String, String)> {
        let (profile_index, rule_index) = self.port_forward_rule_indices(profile_id, rule_id)?;
        let mut profiles = self.profiles.borrow_mut();
        let profile = profiles.get_mut(profile_index)?;
        let rule = profile.port_forwarding_rules.get_mut(rule_index)?;
        rule.enabled = enabled;
        Some((profile.name.clone(), Self::rule_summary_label(rule)))
    }

    pub(in crate::ui::shell) fn request_port_forward_rule_removal(
        &self,
        profile_id: &str,
        rule_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some((profile_index, rule_index)) = self.port_forward_rule_indices(profile_id, rule_id)
        else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.rule_not_found",
            )));
            return;
        };

        let rule_details = {
            let profiles = self.profiles.borrow();
            let profile = &profiles[profile_index];
            profile
                .port_forwarding_rules
                .get(rule_index)
                .map(|rule| (profile.connection_label(), Self::rule_summary_label(rule)))
        };
        let Some((profile_label, rule_label)) = rule_details else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.rule_not_found",
            )));
            return;
        };

        self.set_pending_port_forward_rule_delete(Some(PendingPortForwardRuleDeleteState {
            profile_id: profile_id.to_string(),
            rule_id: rule_id.to_string(),
            profile_label,
            rule_label,
        }));
        cx.notify();
    }

    pub(in crate::ui::shell) fn cancel_port_forward_rule_removal(&self, cx: &mut Context<Self>) {
        if let Some(pending) = self.take_pending_port_forward_rule_delete() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::PortForwardRuleDelete(pending),
            ));
        }
    }

    pub(in crate::ui::shell) fn next_port_forward_rule_id(profile: &SessionProfile) -> String {
        let mut next = profile.port_forwarding_rules.len() + 1;
        loop {
            let candidate = format!("pf-{next}");
            if profile
                .port_forwarding_rules
                .iter()
                .all(|rule| rule.id != candidate)
            {
                return candidate;
            }
            next += 1;
        }
    }

    pub(in crate::ui::shell) fn sync_current_port_forward_rules_for_profile(
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

    pub(in crate::ui::shell) fn synced_sessions_suffix(count: usize) -> String {
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

    pub(in crate::ui::shell) fn forwarding_closed_connection_suffix(closed: bool) -> String {
        if closed {
            i18n::string("forwarding.messages.closed_connection_suffix")
        } else {
            String::new()
        }
    }

    pub(in crate::ui::shell) fn forwarding_stopped_tunnel_suffix(stopped: bool) -> String {
        if stopped {
            i18n::string("forwarding.messages.stopped_tunnel_suffix")
        } else {
            String::new()
        }
    }

    pub(in crate::ui::shell) fn persist_profiles(&self) -> anyhow::Result<()> {
        self.sync_port_profiles();
        self.profile_service()
            .persist_sessions(&self.profiles.borrow())
    }

    pub(in crate::ui::shell) fn duplicate_port_forward_rule(
        &self,
        profile_id: &str,
        rule_id: &str,
        cx: &mut Context<Self>,
    ) {
        let Some((profile_index, rule_index)) = self.port_forward_rule_indices(profile_id, rule_id)
        else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.rule_not_found",
            )));
            return;
        };

        let duplicate = {
            let profiles = self.profiles.borrow();
            let profile = &profiles[profile_index];
            profile
                .port_forwarding_rules
                .get(rule_index)
                .map(|source_rule| {
                    let mut duplicated_rule = source_rule.clone();
                    duplicated_rule.id = Self::next_port_forward_rule_id(profile);
                    let source_label = Self::rule_summary_label(source_rule);
                    duplicated_rule.label = i18n::string_args(
                        "forwarding.messages.duplicate_label",
                        &[("label", &source_label)],
                    );
                    (profile.name.clone(), duplicated_rule)
                })
        };
        let Some((profile_name, duplicated_rule)) = duplicate else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.rule_not_found",
            )));
            return;
        };

        self.profiles.borrow_mut()[profile_index]
            .port_forwarding_rules
            .insert(rule_index + 1, duplicated_rule.clone());
        let synced_sessions = self.sync_current_port_forward_rules_for_profile(profile_id);
        let synced_suffix = Self::synced_sessions_suffix(synced_sessions);
        let message = match self.persist_profiles() {
            Ok(()) => i18n::string_args(
                "forwarding.messages.duplicated",
                &[
                    ("rule", &duplicated_rule.label),
                    ("profile", &profile_name),
                    ("synced_suffix", &synced_suffix),
                ],
            ),
            Err(error) => {
                let error = error.to_string();
                i18n::string_args(
                    "forwarding.messages.duplicated_memory_only",
                    &[
                        ("rule", &duplicated_rule.label),
                        ("synced_suffix", &synced_suffix),
                        ("error", &error),
                    ],
                )
            }
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    pub(in crate::ui::shell) fn request_profile_delete_at_index(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = self.profiles.borrow().get(index).cloned() else {
            return;
        };

        self.set_pending_profile_delete(Some(PendingProfileDeleteState {
            profile_id: profile.id,
            profile_name: profile.name,
            reload_inputs_after_delete: false,
        }));
        cx.notify();
    }

    pub(in crate::ui::shell) fn duplicate_profile_at_index(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if index >= self.profiles.borrow().len() {
            return;
        }

        let original_name = self.profiles.borrow()[index].name.clone();
        let duplicate_name = i18n::string_args(
            "profile.messages.duplicate_name",
            &[("name", &original_name)],
        );
        let service = self.profile_service();
        let duplicated = {
            let mut profiles = self.profiles.borrow_mut();
            service.duplicate_profile(&mut profiles, index, duplicate_name)
        };
        let Some(duplicated) = duplicated else {
            return;
        };

        let message = if let Err(error) = self.persist_profiles() {
            let error = error.to_string();
            i18n::string_args(
                "profile.messages.duplicated_local_save_failed",
                &[("name", &duplicated.name), ("error", &error)],
            )
        } else {
            i18n::string_args(
                "profile.messages.duplicated_as",
                &[("name", &duplicated.name)],
            )
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    pub(in crate::ui::shell) fn toggle_profile_favorite(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if index >= self.profiles.borrow().len() {
            return;
        }

        let (name, is_favorite) = {
            let mut profiles = self.profiles.borrow_mut();
            let profile = &mut profiles[index];
            profile.is_favorite = !profile.is_favorite;
            (profile.name.clone(), profile.is_favorite)
        };

        let message = if let Err(error) = self.persist_profiles() {
            let error = error.to_string();
            if is_favorite {
                i18n::string_args(
                    "profile.messages.starred_local_save_failed",
                    &[("name", &name), ("error", &error)],
                )
            } else {
                i18n::string_args(
                    "profile.messages.unstarred_local_save_failed",
                    &[("name", &name), ("error", &error)],
                )
            }
        } else if is_favorite {
            i18n::string_args("profile.messages.added_to_favorites", &[("name", &name)])
        } else {
            i18n::string_args(
                "profile.messages.removed_from_favorites",
                &[("name", &name)],
            )
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    fn read_port_forward_rule_input_values(
        &self,
        cx: &App,
    ) -> anyhow::Result<PortForwardRuleInputValues> {
        let editor_state = self.editor_state();
        let kind = editor_state.port_forward_kind;
        let Some(profile_id) = editor_state.port_forward_editor_profile_id else {
            return Err(ValidationFailure::required(i18n::string(
                "errors.forwarding.validation.host_profile_required",
            ))
            .into());
        };
        let Some(profile_index) = self
            .profiles
            .borrow()
            .iter()
            .position(|profile| profile.id == profile_id)
        else {
            return Err(ValidationFailure::invalid(i18n::string(
                "errors.forwarding.validation.selected_host_profile_missing",
            ))
            .into());
        };

        let forms = self.panel_forms().forwarding;
        let label = forms.label_input.read(cx).value().to_string();
        let listen_host = forms.listen_host_input.read(cx).value().trim().to_string();
        let listen_port_text = forms.listen_port_input.read(cx).value().trim().to_string();
        let target_host = forms.target_host_input.read(cx).value().trim().to_string();
        let target_port_text = forms.target_port_input.read(cx).value().trim().to_string();

        if listen_host.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.forwarding.validation.listen_host_required",
            ))
            .into());
        }
        if listen_port_text.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.forwarding.validation.listen_port_required",
            ))
            .into());
        }
        if target_host.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.forwarding.validation.target_host_required",
            ))
            .into());
        }
        if target_port_text.is_empty() {
            return Err(ValidationFailure::required(i18n::string(
                "errors.forwarding.validation.target_port_required",
            ))
            .into());
        }

        let listen_port: u16 = listen_port_text.trim().parse().map_err(|_| {
            anyhow::anyhow!(ValidationFailure::invalid(i18n::string_args(
                "errors.forwarding.validation.invalid_listen_port",
                &[("port", &listen_port_text)],
            )))
        })?;
        let target_port: u16 = target_port_text.trim().parse().map_err(|_| {
            anyhow::anyhow!(ValidationFailure::invalid(i18n::string_args(
                "errors.forwarding.validation.invalid_target_port",
                &[("port", &target_port_text)],
            )))
        })?;

        let resolved_label = {
            let label = label.trim();
            if label.is_empty() {
                format!(
                    "{} {}:{} -> {}:{}",
                    Self::localized_port_forward_kind_label(kind),
                    listen_host,
                    listen_port,
                    target_host,
                    target_port
                )
            } else {
                label.to_string()
            }
        };

        Ok(PortForwardRuleInputValues {
            profile_id,
            profile_index,
            kind,
            resolved_label,
            listen_host,
            listen_port,
            target_host,
            target_port,
        })
    }

    fn report_port_forward_rule_save_error(
        &self,
        error: anyhow::Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(validation) = error.downcast_ref::<ValidationFailure>() {
            let message = validation.message.clone();
            cx.emit(AppCommand::Feedback(message.clone()));
            window.push_notification(validation_notification(validation.kind, message), cx);
        } else {
            let message = error.to_string();
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "forwarding.messages.save_failed",
                &[("message", &message)],
            )));
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn save_port_forward_rule(
        &self,
        sync_requires_local_vault_unlock: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let PortForwardRuleInputValues {
            profile_id,
            profile_index,
            kind,
            resolved_label,
            listen_host,
            listen_port,
            target_host,
            target_port,
        } = match self.read_port_forward_rule_input_values(cx) {
            Ok(values) => values,
            Err(error) => {
                self.report_port_forward_rule_save_error(error, window, cx);
                return;
            }
        };

        if sync_requires_local_vault_unlock {
            cx.emit(AppCommand::vault_unlock(DeferredAppCommand::Session(
                SessionDeferredCommand::SavePortForwardRule,
            )));
            return;
        }

        let editor_rule_id = self.editor_state().port_forward_editor_rule_id;
        if let Some(rule_id) = editor_rule_id {
            let rule_index = self.profiles.borrow()[profile_index]
                .port_forwarding_rules
                .iter()
                .position(|rule| rule.id == rule_id);
            let Some(rule_index) = rule_index else {
                self.set_port_forward_editor_rule_id(None);
                cx.emit(AppCommand::Feedback(i18n::string(
                    "forwarding.messages.rule_no_longer_exists",
                )));
                cx.notify();
                return;
            };

            let existing_enabled =
                self.profiles.borrow()[profile_index].port_forwarding_rules[rule_index].enabled;
            let updated_rule = PortForwardRule {
                id: rule_id,
                label: resolved_label,
                kind,
                listen_host,
                listen_port,
                target_host,
                target_port,
                enabled: existing_enabled,
            };

            let profile_name = {
                let mut profiles = self.profiles.borrow_mut();
                let profile = &mut profiles[profile_index];
                profile.port_forwarding_rules[rule_index] = updated_rule.clone();
                profile.name.clone()
            };
            let synced_sessions = self.sync_current_port_forward_rules_for_profile(&profile_id);
            let synced_suffix = Self::synced_sessions_suffix(synced_sessions);

            self.clear_port_forward_editor();
            let message = match self.persist_profiles() {
                Ok(()) => i18n::string_args(
                    "forwarding.messages.saved",
                    &[
                        ("rule", &updated_rule.label),
                        ("profile", &profile_name),
                        ("synced_suffix", &synced_suffix),
                    ],
                ),
                Err(error) => {
                    let error = error.to_string();
                    i18n::string_args(
                        "forwarding.messages.saved_memory_only",
                        &[
                            ("rule", &updated_rule.label),
                            ("synced_suffix", &synced_suffix),
                            ("error", &error),
                        ],
                    )
                }
            };
            cx.emit(AppCommand::Feedback(message));
            cx.notify();
            return;
        }

        let rule = {
            let profiles = self.profiles.borrow();
            let profile = &profiles[profile_index];
            PortForwardRule {
                id: Self::next_port_forward_rule_id(profile),
                label: resolved_label,
                kind,
                listen_host,
                listen_port,
                target_host,
                target_port,
                enabled: true,
            }
        };

        let profile_name = {
            let mut profiles = self.profiles.borrow_mut();
            let profile = &mut profiles[profile_index];
            profile.port_forwarding_rules.push(rule.clone());
            profile.name.clone()
        };
        let synced_sessions = self.sync_current_port_forward_rules_for_profile(&profile_id);
        let synced_suffix = Self::synced_sessions_suffix(synced_sessions);

        self.clear_port_forward_editor();
        let message = match self.persist_profiles() {
            Ok(()) => i18n::string_args(
                "forwarding.messages.added",
                &[
                    ("rule", &rule.label),
                    ("profile", &profile_name),
                    ("synced_suffix", &synced_suffix),
                ],
            ),
            Err(error) => {
                let error = error.to_string();
                i18n::string_args(
                    "forwarding.messages.added_memory_only",
                    &[
                        ("rule", &rule.label),
                        ("synced_suffix", &synced_suffix),
                        ("error", &error),
                    ],
                )
            }
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    pub(in crate::ui::shell) fn continue_save_port_forward_rule_after_unlock(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let PortForwardRuleInputValues {
            profile_id,
            profile_index,
            kind,
            resolved_label,
            listen_host,
            listen_port,
            target_host,
            target_port,
        } = match self.read_port_forward_rule_input_values(cx) {
            Ok(values) => values,
            Err(error) => {
                self.report_port_forward_rule_save_error(error, window, cx);
                return;
            }
        };

        let mut profiles = self.profiles.borrow().clone();
        let editor_rule_id = self.editor_state().port_forward_editor_rule_id;
        let (rule, profile_name, is_edit) = if let Some(rule_id) = editor_rule_id {
            let Some(rule_index) = profiles[profile_index]
                .port_forwarding_rules
                .iter()
                .position(|rule| rule.id == rule_id)
            else {
                self.set_port_forward_editor_rule_id(None);
                cx.emit(AppCommand::Feedback(i18n::string(
                    "forwarding.messages.rule_no_longer_exists",
                )));
                cx.notify();
                return;
            };

            let existing_enabled =
                profiles[profile_index].port_forwarding_rules[rule_index].enabled;
            let updated_rule = PortForwardRule {
                id: rule_id,
                label: resolved_label,
                kind,
                listen_host,
                listen_port,
                target_host,
                target_port,
                enabled: existing_enabled,
            };
            let profile_name = profiles[profile_index].name.clone();
            profiles[profile_index].port_forwarding_rules[rule_index] = updated_rule.clone();
            (updated_rule, profile_name, true)
        } else {
            let rule = {
                let profile = &profiles[profile_index];
                PortForwardRule {
                    id: Self::next_port_forward_rule_id(profile),
                    label: resolved_label,
                    kind,
                    listen_host,
                    listen_port,
                    target_host,
                    target_port,
                    enabled: true,
                }
            };
            let profile_name = profiles[profile_index].name.clone();
            profiles[profile_index]
                .port_forwarding_rules
                .push(rule.clone());
            (rule, profile_name, false)
        };

        let service = self.profile_service();
        let (tx, rx) = std::sync::mpsc::sync_channel::<
            anyhow::Result<SavePortForwardRuleAfterUnlockResult>,
        >(1);
        let spawn_result = std::thread::Builder::new()
            .name("post-unlock-port-forward-save".to_string())
            .spawn(move || {
                let persist_error = service
                    .persist_sessions(&profiles)
                    .err()
                    .map(|error| error.to_string());
                tx.send(Ok(SavePortForwardRuleAfterUnlockResult {
                    profiles,
                    profile_id,
                    profile_name,
                    rule,
                    is_edit,
                    persist_error,
                }))
                .ok();
            });

        if let Err(error) = spawn_result {
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "forwarding.messages.save_failed",
                &[("message", &error.to_string())],
            )));
            cx.notify();
            return;
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv().unwrap_or_else(|_| {
                        Err(anyhow::anyhow!(
                            "post-unlock port-forward save task cancelled"
                        ))
                    })
                })
                .await;

            let _ = this.update(cx, move |controller, cx| match result {
                Ok(result) => {
                    controller.replace_profiles(result.profiles);
                    let synced_sessions =
                        controller.sync_current_port_forward_rules_for_profile(&result.profile_id);
                    let synced_suffix = Self::synced_sessions_suffix(synced_sessions);
                    controller.clear_port_forward_editor();

                    let message = if let Some(error) = result.persist_error {
                        if result.is_edit {
                            i18n::string_args(
                                "forwarding.messages.saved_memory_only",
                                &[
                                    ("rule", &result.rule.label),
                                    ("synced_suffix", &synced_suffix),
                                    ("error", error.as_str()),
                                ],
                            )
                        } else {
                            i18n::string_args(
                                "forwarding.messages.added_memory_only",
                                &[
                                    ("rule", &result.rule.label),
                                    ("synced_suffix", &synced_suffix),
                                    ("error", error.as_str()),
                                ],
                            )
                        }
                    } else if result.is_edit {
                        i18n::string_args(
                            "forwarding.messages.saved",
                            &[
                                ("rule", &result.rule.label),
                                ("profile", &result.profile_name),
                                ("synced_suffix", &synced_suffix),
                            ],
                        )
                    } else {
                        i18n::string_args(
                            "forwarding.messages.added",
                            &[
                                ("rule", &result.rule.label),
                                ("profile", &result.profile_name),
                                ("synced_suffix", &synced_suffix),
                            ],
                        )
                    };
                    cx.emit(AppCommand::Feedback(message));
                    cx.notify();
                }
                Err(error) => {
                    cx.emit(AppCommand::Feedback(i18n::string_args(
                        "forwarding.messages.save_failed",
                        &[("message", &error.to_string())],
                    )));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(in crate::ui::shell) fn open_port_forward_panel(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_port_forward_editor_state(true, None, None, PortForwardKind::Local);
        let forms = self.panel_forms().forwarding;
        set_input_value(&forms.label_input, "", window, cx);
        set_input_value(&forms.listen_host_input, "", window, cx);
        set_input_value(&forms.listen_port_input, "", window, cx);
        set_input_value(&forms.target_host_input, "", window, cx);
        set_input_value(&forms.target_port_input, "", window, cx);
        self.sync_port_forward_profile_select(None, window, cx);
        let message = if self.profiles.borrow().is_empty() {
            i18n::string("forwarding.messages.create_host_profile_before_adding")
        } else {
            i18n::string("forwarding.messages.choose_host_profile_for_new_rule")
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    pub(in crate::ui::shell) fn edit_port_forward_rule(
        &self,
        profile_id: String,
        rule_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((profile_index, rule_index)) =
            self.port_forward_rule_indices(&profile_id, &rule_id)
        else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.rule_not_found",
            )));
            return;
        };

        let profile = self.profiles.borrow()[profile_index].clone();
        let Some(rule) = profile.port_forwarding_rules.get(rule_index).cloned() else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.rule_not_found",
            )));
            return;
        };

        self.selected_profile.set(Some(profile_index));
        self.set_port_forward_editor_state(
            true,
            Some(profile.id.clone()),
            Some(rule.id.clone()),
            rule.kind,
        );
        let forms = self.panel_forms().forwarding;
        set_input_value(&forms.label_input, rule.label.clone(), window, cx);
        set_input_value(
            &forms.listen_host_input,
            rule.listen_host.clone(),
            window,
            cx,
        );
        set_input_value(
            &forms.listen_port_input,
            rule.listen_port.to_string(),
            window,
            cx,
        );
        set_input_value(
            &forms.target_host_input,
            rule.target_host.clone(),
            window,
            cx,
        );
        set_input_value(
            &forms.target_port_input,
            rule.target_port.to_string(),
            window,
            cx,
        );
        self.sync_port_forward_profile_select(Some(profile.id.as_str()), window, cx);
        let rule_label = Self::rule_summary_label(&rule);
        cx.emit(AppCommand::Feedback(i18n::string_args(
            "forwarding.messages.editing_rule",
            &[("rule", &rule_label)],
        )));
        cx.notify();
    }

    pub(in crate::ui::shell) fn close_port_forward_rule_editor(&self, cx: &mut Context<Self>) {
        let editor_state = self.editor_state();
        if !editor_state.port_forward_editor_open {
            return;
        }

        let message = if editor_state.port_forward_editor_rule_id.is_some() {
            i18n::string("forwarding.messages.canceled_editing")
        } else {
            i18n::string("forwarding.messages.canceled_new")
        };
        self.clear_port_forward_editor();
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    fn select_port_forward_editor_profile(
        &self,
        profile_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self
            .editor_state
            .borrow()
            .port_forward_editor_rule_id
            .is_some()
        {
            return;
        }

        let Some(profile_id) = profile_id else {
            self.set_port_forward_editor_profile_id(None);
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.choose_profile_before_adding",
            )));
            cx.notify();
            return;
        };

        let profile = self
            .profiles
            .borrow()
            .iter()
            .enumerate()
            .find(|(_, profile)| profile.id == profile_id)
            .map(|(index, profile)| (index, profile.id.clone(), profile.name.clone()));
        let Some((profile_index, profile_id, profile_name)) = profile else {
            self.set_port_forward_editor_profile_id(None);
            cx.emit(AppCommand::Feedback(i18n::string(
                "forwarding.messages.profile_not_found",
            )));
            cx.notify();
            return;
        };

        self.selected_profile.set(Some(profile_index));
        self.set_port_forward_editor_profile_id(Some(profile_id));
        if self
            .editor_state
            .borrow()
            .port_forward_editor_rule_id
            .is_none()
        {
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "forwarding.messages.creating_rule_for_profile",
                &[("profile", &profile_name)],
            )));
        }
        cx.notify();
    }

    fn host_editor_forms_state(&self) -> &RefCell<HostEditorForms> {
        self.host_editor_forms
            .as_ref()
            .expect("host editor forms are available in the application")
    }

    pub(in crate::ui::shell) fn host_editor_forms(&self) -> HostEditorForms {
        self.host_editor_forms_state().borrow().clone()
    }

    pub(in crate::ui::shell) fn host_editor_forms_mut(&self) -> RefMut<'_, HostEditorForms> {
        self.host_editor_forms_state().borrow_mut()
    }

    fn snippets_forms_state(&self) -> &RefCell<SnippetsForms> {
        self.snippets_forms
            .as_ref()
            .expect("snippet forms are available in the application")
    }

    pub(in crate::ui::shell) fn snippets_forms(&self) -> SnippetsForms {
        self.snippets_forms_state().borrow().clone()
    }

    pub(in crate::ui::shell) fn collect_available_snippet_packages(
        snippets: &[SnippetRecord],
    ) -> Vec<String> {
        let mut packages = snippets
            .iter()
            .filter_map(|snippet| {
                let package = snippet.package.trim();
                (!package.is_empty()).then(|| package.to_string())
            })
            .collect::<Vec<_>>();
        packages.sort_by_key(|package| package.to_ascii_lowercase());
        packages.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        packages
    }

    pub(in crate::ui::shell) fn begin_new_snippet_package(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let forms = self.snippets_forms();
        self.set_snippets_creating_new_package(true);
        forms.package_select.update(cx, |select, cx| {
            select.set_selected_index(None, window, cx);
        });
        set_input_value(&forms.package_input, "", window, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn close_snippets_editor(&self, cx: &mut Context<Self>) {
        if !self.editor_state().snippets_editor_open {
            return;
        }
        self.set_snippets_editor_open(false);
        self.set_selected_snippet(None);
        cx.emit(AppCommand::Feedback(i18n::string(
            "snippets.messages.closed_sidebar",
        )));
        cx.notify();
    }

    pub(in crate::ui::shell) fn request_selected_snippet_delete(&self, cx: &mut Context<Self>) {
        let Some(index) = self.selected_snippet() else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "snippets.messages.select_to_delete",
            )));
            return;
        };
        let Some(snippet) = self.snippets().get(index).cloned() else {
            cx.emit(AppCommand::Feedback(i18n::string(
                "snippets.messages.select_to_delete",
            )));
            return;
        };
        self.set_pending_snippet_delete(Some(PendingSnippetDeleteState {
            snippet_id: snippet.id,
            snippet_description: snippet.description,
        }));
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_snippets_creating_new_package(&self, creating: bool) {
        self.snippets_forms_state()
            .borrow_mut()
            .creating_new_package = creating;
    }

    pub(in crate::ui::shell) fn snippets(&self) -> Ref<'_, Vec<SnippetRecord>> {
        self.snippets.borrow()
    }

    pub(in crate::ui::shell) fn replace_snippets(&self, snippets: Vec<SnippetRecord>) {
        *self.snippets.borrow_mut() = snippets;
    }

    pub(in crate::ui::shell) fn selected_snippet(&self) -> Option<usize> {
        self.selected_snippet.get()
    }

    pub(in crate::ui::shell) fn set_selected_snippet(&self, selected: Option<usize>) {
        self.selected_snippet.set(selected);
    }

    pub(in crate::ui::shell) fn known_hosts_entries(&self) -> Ref<'_, Vec<KnownHostEntry>> {
        self.known_hosts_entries.borrow()
    }

    pub(in crate::ui::shell) fn replace_known_hosts_entries(&self, entries: Vec<KnownHostEntry>) {
        *self.known_hosts_entries.borrow_mut() = entries;
    }

    pub(in crate::ui::shell) fn refresh_known_hosts(&self, cx: &mut Context<Self>) {
        match self.services.known_hosts.list() {
            Ok(entries) => self.replace_known_hosts_entries(entries),
            Err(error) => log::warn!("failed to refresh known_hosts list: {error:?}"),
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn refresh_known_hosts_with_feedback(&self, cx: &mut Context<Self>) {
        self.refresh_known_hosts(cx);
        cx.emit(AppCommand::Feedback(i18n::string(
            "trusted.messages.refreshed",
        )));
    }

    pub(in crate::ui::shell) fn select_trusted_known_host(
        &self,
        host: String,
        port: u16,
        fingerprint: String,
        cx: &mut Context<Self>,
    ) {
        self.set_selected_known_host(Some((host, port, fingerprint)));
        cx.notify();
    }

    pub(in crate::ui::shell) fn close_trusted_known_host_sidebar(&self, cx: &mut Context<Self>) {
        if self.take_selected_known_host().is_some() {
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn copy_known_host_fingerprint(
        &self,
        fingerprint: String,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(fingerprint));
        cx.emit(AppCommand::Feedback(i18n::string(
            "trusted.messages.copied_fingerprint",
        )));
    }

    pub(in crate::ui::shell) fn copy_known_host_address(
        &self,
        host: String,
        port: u16,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(format!("{host}:{port}")));
        cx.emit(AppCommand::Feedback(i18n::string(
            "trusted.messages.copied_address",
        )));
    }

    pub(in crate::ui::shell) fn open_linked_profile_from_known_host(
        &self,
        profile_id: String,
        cx: &mut Context<Self>,
    ) {
        self.request_profile_editor(profile_id, false, cx);
    }

    pub(in crate::ui::shell) fn request_trusted_known_host_removal(
        &self,
        host: String,
        port: u16,
        cx: &mut Context<Self>,
    ) {
        if !self
            .known_hosts_entries
            .borrow()
            .iter()
            .any(|entry| entry.host == host && entry.port == port)
        {
            let port_text = port.to_string();
            cx.emit(AppCommand::Feedback(i18n::string_args(
                "session.messages.no_host_key_entry",
                &[("host", &host), ("port", &port_text)],
            )));
            return;
        }

        self.set_pending_known_host_delete(Some(PendingKnownHostDeleteState { host, port }));
        cx.notify();
    }

    pub(in crate::ui::shell) fn confirm_trusted_known_host_removal(&self, cx: &mut Context<Self>) {
        let Some(pending) = self.take_pending_known_host_delete() else {
            return;
        };

        cx.emit(AppCommand::OverlayDismissed(
            DialogOverlaySnapshot::KnownHostDelete(pending.clone()),
        ));
        if self
            .selected_known_host()
            .as_ref()
            .is_some_and(|(host, port, _)| host == &pending.host && *port == pending.port)
        {
            self.set_selected_known_host(None);
        }

        let message = match self
            .services
            .known_hosts
            .remove(&pending.host, pending.port)
        {
            Ok(true) => {
                self.refresh_known_hosts(cx);
                let port = pending.port.to_string();
                i18n::string_args(
                    "session.messages.removed_host_key",
                    &[("host", &pending.host), ("port", &port)],
                )
            }
            Ok(false) => {
                let port = pending.port.to_string();
                i18n::string_args(
                    "session.messages.no_host_key_entry",
                    &[("host", &pending.host), ("port", &port)],
                )
            }
            Err(error) => i18n::string_args(
                "session.messages.remove_host_key_failed",
                &[("error", &error.to_string())],
            ),
        };
        cx.emit(AppCommand::Feedback(message));
        cx.notify();
    }

    pub(in crate::ui::shell) fn cancel_trusted_known_host_removal(&self, cx: &mut Context<Self>) {
        if let Some(pending) = self.take_pending_known_host_delete() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::KnownHostDelete(pending),
            ));
        }
    }

    pub(in crate::ui::shell) fn panel_forms(&self) -> SessionPanelForms {
        self.panel_forms
            .as_ref()
            .expect("session panel forms are available in the application")
            .clone()
    }

    pub(in crate::ui::shell) fn catalog_view(&self) -> SessionCatalogViewState {
        self.catalog_view.borrow().clone()
    }

    pub(in crate::ui::shell) fn set_hosts_view_mode(
        &mut self,
        mode: ProfileViewMode,
        cx: &mut Context<Self>,
    ) {
        self.catalog_view.get_mut().hosts_view_mode = mode;
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_forward_view_mode(
        &mut self,
        mode: ProfileViewMode,
        cx: &mut Context<Self>,
    ) {
        self.catalog_view.get_mut().forward_view_mode = mode;
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_snippets_view_mode(
        &mut self,
        mode: ProfileViewMode,
        cx: &mut Context<Self>,
    ) {
        self.catalog_view.get_mut().snippets_view_mode = mode;
        cx.notify();
    }

    pub(in crate::ui::shell) fn set_trusted_host_filter(
        &mut self,
        filter: TrustedHostFilter,
        cx: &mut Context<Self>,
    ) {
        self.catalog_view.get_mut().trusted_host_filter = filter;
        cx.notify();
    }

    pub(in crate::ui::shell) fn toggle_hosts_group_filter(
        &mut self,
        group: String,
        cx: &mut Context<Self>,
    ) {
        let status =
            if self.catalog_view.get_mut().hosts_group_filter.as_deref() == Some(group.as_str()) {
                self.catalog_view.get_mut().hosts_group_filter = None;
                i18n::string("hosts.messages.cleared_group_filter")
            } else {
                self.catalog_view.get_mut().hosts_group_filter = Some(group.clone());
                i18n::string_args("hosts.messages.filtering_by_group", &[("group", &group)])
            };
        cx.emit(AppCommand::Feedback(status));
        cx.notify();
    }

    pub(in crate::ui::shell) fn toggle_snippets_package_filter(
        &mut self,
        package: String,
        cx: &mut Context<Self>,
    ) {
        let selected_package = if self
            .catalog_view
            .get_mut()
            .snippets_package_filter
            .as_deref()
            == Some(package.as_str())
        {
            self.catalog_view.get_mut().snippets_package_filter = None;
            None
        } else {
            self.catalog_view.get_mut().snippets_package_filter = Some(package.clone());
            Some(package)
        };
        let status = if let Some(selected_package) = selected_package.as_ref() {
            i18n::string_args(
                "snippets.messages.filtering_by_package",
                &[("package", selected_package)],
            )
        } else {
            i18n::string("snippets.messages.viewing_all")
        };
        cx.emit(AppCommand::Feedback(status));
        cx.notify();
    }

    pub(in crate::ui::shell) fn clear_snippets_package_filter(&self) {
        self.catalog_view.borrow_mut().snippets_package_filter = None;
    }

    pub(in crate::ui::shell) fn editor_state(&self) -> SessionEditorState {
        self.editor_state.borrow().clone()
    }

    pub(in crate::ui::shell) fn set_host_editor_state(&self, open: bool, is_new: bool) {
        let mut state = self.editor_state.borrow_mut();
        state.host_editor_open = open;
        state.host_editor_is_new = is_new;
    }

    pub(in crate::ui::shell) fn set_snippets_editor_open(&self, open: bool) {
        self.editor_state.borrow_mut().snippets_editor_open = open;
    }

    pub(in crate::ui::shell) fn set_port_forward_editor_state(
        &self,
        open: bool,
        profile_id: Option<String>,
        rule_id: Option<String>,
        kind: PortForwardKind,
    ) {
        let mut state = self.editor_state.borrow_mut();
        state.port_forward_editor_open = open;
        state.port_forward_editor_profile_id = profile_id;
        state.port_forward_editor_rule_id = rule_id;
        state.port_forward_kind = kind;
    }

    pub(in crate::ui::shell) fn clear_port_forward_editor(&self) {
        let mut state = self.editor_state.borrow_mut();
        state.port_forward_editor_open = false;
        state.port_forward_editor_profile_id = None;
        state.port_forward_editor_rule_id = None;
    }

    pub(in crate::ui::shell) fn set_port_forward_editor_profile_id(
        &self,
        profile_id: Option<String>,
    ) {
        self.editor_state
            .borrow_mut()
            .port_forward_editor_profile_id = profile_id;
    }

    pub(in crate::ui::shell) fn set_port_forward_editor_rule_id(&self, rule_id: Option<String>) {
        self.editor_state.borrow_mut().port_forward_editor_rule_id = rule_id;
    }

    pub(in crate::ui::shell) fn set_port_forward_kind(
        &self,
        kind: PortForwardKind,
        cx: &mut Context<Self>,
    ) {
        self.editor_state.borrow_mut().port_forward_kind = kind;
        cx.notify();
    }

    pub(in crate::ui::shell) fn terminal_search(&self) -> Ref<'_, TerminalSearchForms> {
        Ref::map(self.forms().borrow(), |forms| &forms.search)
    }

    pub(in crate::ui::shell) fn terminal_search_mut(&self) -> RefMut<'_, TerminalSearchForms> {
        RefMut::map(self.forms().borrow_mut(), |forms| &mut forms.search)
    }

    pub(in crate::ui::shell) fn terminal_search_input(&self) -> Entity<InputState> {
        self.forms().borrow().search.input.clone()
    }

    pub(in crate::ui::shell) fn terminal_search_open(&self) -> bool {
        self.forms().borrow().search.open
    }

    pub(in crate::ui::shell) fn workspace_snippets_filter_input(&self) -> Entity<InputState> {
        self.forms().borrow().snippets_panel.filter_input.clone()
    }

    pub(in crate::ui::shell) fn workspace_snippets_selected_package_filter(
        &self,
    ) -> Option<String> {
        self.forms()
            .borrow()
            .snippets_panel
            .selected_package_filter
            .clone()
    }

    pub(in crate::ui::shell) fn toggle_workspace_snippets_package_filter(
        &self,
        package: &str,
    ) -> Option<String> {
        let mut forms = self.forms().borrow_mut();
        let selected = &mut forms.snippets_panel.selected_package_filter;
        if selected.as_deref() == Some(package) {
            *selected = None;
        } else {
            *selected = Some(package.to_string());
        }
        selected.clone()
    }

    pub(in crate::ui::shell) fn build_pending_tab(
        id: TabId,
        profile: SessionProfile,
        terminal: TerminalState,
        auto_collect_monitoring: bool,
    ) -> (TabState, SessionTabState) {
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
            pending_keyboard_interactive: None,
            reconnect_task: None,
            reconnect_attempt: 0,
            has_activity: false,
            monitoring: SessionMonitoringState::new(auto_collect_monitoring),
            purpose: SessionPurpose::Terminal,
        };
        (
            TabState::new(
                id,
                title,
                status,
                TabKindTag::Session,
                crate::ui::shell::workspace::TabPlacement::TopLevel,
            ),
            session,
        )
    }

    pub(in crate::ui::shell) fn build_port_forwarding_tab(
        id: TabId,
        profile: &SessionProfile,
        rule: &PortForwardRule,
        commands: SessionCommandSender,
    ) -> (TabState, SessionTabState) {
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
            commands: Some(commands),
            bytes_in: 0,
            bytes_out: 0,
            pending_host_key: None,
            pending_keyboard_interactive: None,
            reconnect_task: None,
            reconnect_attempt: 0,
            has_activity: false,
            monitoring: SessionMonitoringState::new(false),
            purpose: SessionPurpose::PortForwarding,
        };
        (
            TabState::new(
                id,
                title,
                i18n::string_args(
                    "tabs.initial.port_forward_connecting_to",
                    &[("profile", &profile_summary)],
                ),
                TabKindTag::Session,
                crate::ui::shell::workspace::TabPlacement::Background,
            ),
            session,
        )
    }

    pub(in crate::ui::shell) fn build_connection_test_tab(
        id: TabId,
        profile: &SessionProfile,
        commands: SessionCommandSender,
    ) -> (TabState, SessionTabState) {
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
            pending_keyboard_interactive: None,
            reconnect_task: None,
            reconnect_attempt: 0,
            has_activity: false,
            monitoring: SessionMonitoringState::new(false),
            purpose: SessionPurpose::ConnectionTest,
        };
        (
            TabState::new(
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
                crate::ui::shell::workspace::TabPlacement::Background,
            ),
            session,
        )
    }

    pub(in crate::ui::shell) fn tab(&self, tab_id: TabId) -> Option<Ref<'_, SessionTabState>> {
        Ref::filter_map(self.tabs.borrow(), |tabs| tabs.get(&tab_id)).ok()
    }

    pub(in crate::ui::shell) fn tab_mut(
        &self,
        tab_id: TabId,
    ) -> Option<RefMut<'_, SessionTabState>> {
        RefMut::filter_map(self.tabs.borrow_mut(), |tabs| tabs.get_mut(&tab_id)).ok()
    }

    pub(in crate::ui::shell) fn insert_tab(&self, tab_id: TabId, tab: SessionTabState) {
        assert!(
            self.tabs.borrow_mut().insert(tab_id, tab).is_none(),
            "duplicate session tab payload for {tab_id}"
        );
    }

    pub(in crate::ui::shell) fn remove_tab(&self, tab_id: TabId) -> Option<SessionTabState> {
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

    pub(in crate::ui::shell) fn monitor_scroll_handle(&self) -> ScrollHandle {
        self.monitor_scroll_handle.clone()
    }

    pub(in crate::ui::shell) fn reported_terminal_focus_tab_id(&self) -> Option<TabId> {
        *self.reported_terminal_focus_tab_id.borrow()
    }

    pub(in crate::ui::shell) fn take_reported_terminal_focus_tab_id(&self) -> Option<TabId> {
        self.reported_terminal_focus_tab_id.borrow_mut().take()
    }

    pub(in crate::ui::shell) fn set_reported_terminal_focus_tab_id(&self, tab_id: Option<TabId>) {
        *self.reported_terminal_focus_tab_id.borrow_mut() = tab_id;
    }

    pub(in crate::ui::shell) fn side_panel_open(&self) -> bool {
        self.panel.borrow().open
    }

    pub(in crate::ui::shell) fn set_side_panel_open(&self, open: bool) {
        self.panel.borrow_mut().open = open;
    }

    pub(in crate::ui::shell) fn toggle_side_panel(&self) {
        let mut panel = self.panel.borrow_mut();
        panel.open = !panel.open;
    }

    pub(in crate::ui::shell) fn side_panel_view(&self) -> SessionSidePanelView {
        self.panel.borrow().view
    }

    pub(in crate::ui::shell) fn set_side_panel_view(&self, view: SessionSidePanelView) {
        self.panel.borrow_mut().view = view;
    }

    pub(in crate::ui::shell) fn side_panel_transition_state(
        &self,
    ) -> (bool, Option<WorkspaceSidePanelTransition>) {
        let panel = self.panel.borrow();
        (panel.visible, panel.transition)
    }

    pub(in crate::ui::shell) fn set_side_panel_transition_state(
        &self,
        visible: bool,
        transition: Option<WorkspaceSidePanelTransition>,
    ) {
        let mut panel = self.panel.borrow_mut();
        panel.visible = visible;
        panel.transition = transition;
    }

    pub(in crate::ui::shell) fn selected_known_host(&self) -> Option<(String, u16, String)> {
        self.panel.borrow().selected_known_host.clone()
    }

    pub(in crate::ui::shell) fn set_selected_known_host(
        &self,
        selected: Option<(String, u16, String)>,
    ) {
        self.panel.borrow_mut().selected_known_host = selected;
    }

    pub(in crate::ui::shell) fn take_selected_known_host(&self) -> Option<(String, u16, String)> {
        self.panel.borrow_mut().selected_known_host.take()
    }

    pub(in crate::ui::shell) fn pending_host_key_prompt(
        &self,
        active_tab_id: Option<TabId>,
        ordered_tab_ids: &[TabId],
    ) -> Option<(TabId, HostKeyPrompt)> {
        if let Some(tab_id) = active_tab_id
            && let Some(prompt) = self
                .tab(tab_id)
                .and_then(|session| session.pending_host_key.clone())
        {
            return Some((tab_id, prompt));
        }

        ordered_tab_ids.iter().find_map(|tab_id| {
            self.tab(*tab_id)
                .and_then(|session| session.pending_host_key.clone())
                .map(|prompt| (*tab_id, prompt))
        })
    }

    pub(in crate::ui::shell) fn pending_keyboard_interactive_prompt(
        &self,
        active_tab_id: Option<TabId>,
        ordered_tab_ids: &[TabId],
    ) -> Option<(TabId, KbiChallenge)> {
        if let Some(tab_id) = active_tab_id
            && let Some(challenge) = self
                .tab(tab_id)
                .and_then(|session| session.pending_keyboard_interactive.clone())
        {
            return Some((tab_id, challenge));
        }

        ordered_tab_ids.iter().find_map(|tab_id| {
            self.tab(*tab_id)
                .and_then(|session| session.pending_keyboard_interactive.clone())
                .map(|challenge| (*tab_id, challenge))
        })
    }

    pub(in crate::ui::shell) fn sync_keyboard_interactive_inputs(
        &self,
        challenge: Option<KbiChallenge>,
        has_exiting_prompt: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut inputs = self.kbi_inputs.borrow_mut();
        if let Some(challenge) = challenge {
            if inputs.len() == challenge.prompts.len() {
                return;
            }
            *inputs = challenge
                .prompts
                .iter()
                .map(|prompt| new_input_state("", "", !prompt.echo, window, cx))
                .collect();
            drop(inputs);
            cx.notify();
        } else if !has_exiting_prompt && !inputs.is_empty() {
            inputs.clear();
            drop(inputs);
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn keyboard_interactive_inputs(&self) -> Vec<Entity<InputState>> {
        self.kbi_inputs.borrow().clone()
    }

    pub(in crate::ui::shell) fn resolve_host_key_prompt(
        &self,
        tab_id: TabId,
        decision: HostKeyDecision,
        cx: &mut Context<Self>,
    ) {
        let Some((prompt, commands)) = self.tab_mut(tab_id).and_then(|mut session| {
            let prompt = session.pending_host_key.take()?;
            Some((prompt, session.commands.clone()))
        }) else {
            return;
        };

        cx.emit(AppCommand::OverlayDismissed(
            DialogOverlaySnapshot::HostKey {
                tab_id,
                prompt: prompt.clone(),
            },
        ));
        let Some(commands) = commands else {
            return;
        };
        if let Err(error) = commands.respond_host_key(decision) {
            log::warn!("failed to deliver host key decision: {error:?}");
        }

        let message = match decision {
            HostKeyDecision::AcceptOnce => i18n::string_args(
                "session.messages.accepted_host_key_session_only",
                &[("host", &prompt.host)],
            ),
            HostKeyDecision::AcceptAndSave => {
                self.refresh_known_hosts(cx);
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

    pub(in crate::ui::shell) fn submit_keyboard_interactive(
        &self,
        tab_id: TabId,
        responses: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let Some((challenge, commands)) = self.tab_mut(tab_id).and_then(|mut session| {
            let challenge = session.pending_keyboard_interactive.take()?;
            Some((challenge, session.commands.clone()))
        }) else {
            return;
        };

        if let Some(commands) = commands {
            let _ = commands.respond_keyboard_interactive(responses);
        }
        cx.emit(AppCommand::OverlayDismissed(
            DialogOverlaySnapshot::KeyboardInteractive { tab_id, challenge },
        ));
        cx.notify();
    }

    pub(in crate::ui::shell) fn cancel_keyboard_interactive(
        &self,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) {
        let Some((challenge, commands)) = self.tab_mut(tab_id).and_then(|mut session| {
            let challenge = session.pending_keyboard_interactive.take()?;
            Some((challenge, session.commands.clone()))
        }) else {
            return;
        };

        if let Some(commands) = commands {
            let _ = commands.close();
        }
        cx.emit(AppCommand::OverlayDismissed(
            DialogOverlaySnapshot::KeyboardInteractive { tab_id, challenge },
        ));
        cx.notify();
    }

    pub(in crate::ui::shell) fn pending_profile_delete(&self) -> Option<PendingProfileDeleteState> {
        self.pending_dialogs.borrow().profile_delete.clone()
    }

    pub(in crate::ui::shell) fn set_pending_profile_delete(
        &self,
        pending: Option<PendingProfileDeleteState>,
    ) {
        self.pending_dialogs.borrow_mut().profile_delete = pending;
    }

    pub(in crate::ui::shell) fn take_pending_profile_delete(
        &self,
    ) -> Option<PendingProfileDeleteState> {
        self.pending_dialogs.borrow_mut().profile_delete.take()
    }

    pub(in crate::ui::shell) fn cancel_profile_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.take_pending_profile_delete() {
            cx.emit(AppCommand::OverlayDismissed(
                DialogOverlaySnapshot::ProfileDelete(pending),
            ));
        }
    }

    pub(in crate::ui::shell) fn pending_known_host_delete(
        &self,
    ) -> Option<PendingKnownHostDeleteState> {
        self.pending_dialogs.borrow().known_host_delete.clone()
    }

    pub(in crate::ui::shell) fn set_pending_known_host_delete(
        &self,
        pending: Option<PendingKnownHostDeleteState>,
    ) {
        self.pending_dialogs.borrow_mut().known_host_delete = pending;
    }

    pub(in crate::ui::shell) fn take_pending_known_host_delete(
        &self,
    ) -> Option<PendingKnownHostDeleteState> {
        self.pending_dialogs.borrow_mut().known_host_delete.take()
    }

    pub(in crate::ui::shell) fn pending_snippet_delete(&self) -> Option<PendingSnippetDeleteState> {
        self.pending_dialogs.borrow().snippet_delete.clone()
    }

    pub(in crate::ui::shell) fn set_pending_snippet_delete(
        &self,
        pending: Option<PendingSnippetDeleteState>,
    ) {
        self.pending_dialogs.borrow_mut().snippet_delete = pending;
    }

    pub(in crate::ui::shell) fn take_pending_snippet_delete(
        &self,
    ) -> Option<PendingSnippetDeleteState> {
        self.pending_dialogs.borrow_mut().snippet_delete.take()
    }

    pub(in crate::ui::shell) fn pending_port_forward_rule_delete(
        &self,
    ) -> Option<PendingPortForwardRuleDeleteState> {
        self.pending_dialogs
            .borrow()
            .port_forward_rule_delete
            .clone()
    }

    pub(in crate::ui::shell) fn set_pending_port_forward_rule_delete(
        &self,
        pending: Option<PendingPortForwardRuleDeleteState>,
    ) {
        self.pending_dialogs.borrow_mut().port_forward_rule_delete = pending;
    }

    pub(in crate::ui::shell) fn take_pending_port_forward_rule_delete(
        &self,
    ) -> Option<PendingPortForwardRuleDeleteState> {
        self.pending_dialogs
            .borrow_mut()
            .port_forward_rule_delete
            .take()
    }

    pub(in crate::ui::shell) fn online_session_count_for_profile(&self, profile_id: &str) -> usize {
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

    pub(in crate::ui::shell) fn has_port_forward_rule_session(
        &self,
        profile_id: &str,
        rule_id: &str,
    ) -> bool {
        self.port_forward_rule_session_id(profile_id, rule_id)
            .is_some()
    }

    pub(in crate::ui::shell) fn has_port_forward_rule_connection(
        &self,
        profile_id: &str,
        rule_id: &str,
    ) -> bool {
        let Some(tab_id) = self.port_forward_rule_session_id(profile_id, rule_id) else {
            return false;
        };
        self.tabs.borrow().get(&tab_id).is_some_and(|session| {
            matches!(session.connection_state, SessionConnectionState::Ready)
        })
    }

    pub(in crate::ui::shell) fn is_port_forward_rule_connecting(
        &self,
        profile_id: &str,
        rule_id: &str,
    ) -> bool {
        let Some(tab_id) = self.port_forward_rule_session_id(profile_id, rule_id) else {
            return false;
        };
        self.tabs.borrow().get(&tab_id).is_some_and(|session| {
            matches!(
                session.connection_state,
                SessionConnectionState::Connecting | SessionConnectionState::Reconnecting { .. }
            )
        })
    }

    pub(in crate::ui::shell) fn terminal_profile_ids(&self) -> Vec<String> {
        self.tabs
            .borrow()
            .values()
            .filter(|&session| session.purpose == SessionPurpose::Terminal)
            .map(|session| session.profile_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect()
    }

    pub(in crate::ui::shell) fn sync_port_forward_rules_for_profile(
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

    pub(in crate::ui::shell) fn monitoring_state_for_profile(
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

    pub(in crate::ui::shell) fn shared_monitoring_enabled(&self, profile_id: &str) -> Option<bool> {
        self.shared_profile_monitoring
            .borrow()
            .get(profile_id)
            .map(|state| state.auto_collect_enabled)
    }

    pub(in crate::ui::shell) fn monitoring_enabled_for_profile(
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

    pub(in crate::ui::shell) fn claim_profile_monitor_source(
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

    pub(in crate::ui::shell) fn set_profile_monitoring_enabled(
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

    pub(in crate::ui::shell) fn enable_profile_monitoring(
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

    pub(in crate::ui::shell) fn apply_auto_collect_monitoring_preference(
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

    pub(in crate::ui::shell) fn apply_profile_monitor_snapshot(
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

    pub(in crate::ui::shell) fn apply_profile_monitor_error(
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

    pub(in crate::ui::shell) fn refresh_profile_monitoring(
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

    pub(in crate::ui::shell) fn emit(&mut self, command: AppCommand, cx: &mut Context<Self>) {
        cx.emit(command);
    }

    pub(super) fn credentials_changed(
        &mut self,
        secrets: SecretStore,
        local_vault_status: LocalVaultStatus,
        cx: &mut Context<Self>,
    ) {
        self.services.secrets = secrets;
        self.services.local_vault_status = local_vault_status;
        cx.notify();
    }

    pub(in crate::ui::shell) fn host_password_visible(&self) -> bool {
        self.host_password_visible
    }

    pub(in crate::ui::shell) fn set_host_password_visible(
        &mut self,
        visible: bool,
        cx: &mut Context<Self>,
    ) {
        self.host_password_visible = visible;
        cx.notify();
    }

    pub(in crate::ui::shell) fn sync_port_snapshot(&self, snapshot: SessionPortSnapshot) {
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

    pub(in crate::ui::shell) fn query_port(&self) -> SessionQueryPort {
        SessionQueryPort {
            state: self.ports.clone(),
        }
    }

    pub(in crate::ui::shell) fn terminal_port(&self) -> SessionTerminalPort {
        SessionTerminalPort {
            state: self.ports.clone(),
        }
    }
}

impl EventEmitter<AppCommand> for SessionController {}

#[derive(Clone, Default)]
pub(in crate::ui::shell) struct SessionPortSnapshot {
    profiles: Vec<SessionProfile>,
    sessions: HashMap<TabId, SessionPortSession>,
    terminal_order: Vec<TabId>,
    active_profile_id: Option<String>,
    active_terminal_tab_id: Option<TabId>,
}

impl SessionPortSnapshot {
    pub(in crate::ui::shell) fn new(
        profiles: Vec<SessionProfile>,
        sessions: Vec<SessionPortSession>,
        active_profile_id: Option<String>,
        active_terminal_tab_id: Option<TabId>,
    ) -> Self {
        let terminal_order = sessions
            .iter()
            .filter(|session| session.purpose == SessionPurpose::Terminal)
            .map(|session| session.tab_id)
            .collect();
        let sessions = sessions
            .into_iter()
            .map(|session| (session.tab_id, session))
            .collect();
        Self {
            profiles,
            sessions,
            terminal_order,
            active_profile_id,
            active_terminal_tab_id,
        }
    }
}

#[derive(Clone)]
pub(in crate::ui::shell) struct SessionPortSession {
    tab_id: TabId,
    title: String,
    profile_id: String,
    pending_profile: Option<SessionProfile>,
    purpose: SessionPurpose,
    commands: Option<SessionCommandSender>,
}

impl SessionPortSession {
    pub(in crate::ui::shell) fn new(
        tab_id: TabId,
        title: String,
        profile_id: String,
        pending_profile: Option<SessionProfile>,
        purpose: SessionPurpose,
        commands: Option<SessionCommandSender>,
    ) -> Self {
        Self {
            tab_id,
            title,
            profile_id,
            pending_profile,
            purpose,
            commands,
        }
    }
}

#[derive(Clone)]
pub(in crate::ui::shell) struct SessionQueryPort {
    state: Rc<RefCell<SessionPortState>>,
}

impl SessionQueryPort {
    pub(in crate::ui::shell) fn profiles(&self) -> Vec<SessionProfile> {
        self.state.borrow().snapshot.profiles.clone()
    }

    pub(in crate::ui::shell) fn profile(&self, profile_id: &str) -> Option<SessionProfile> {
        self.state
            .borrow()
            .snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
    }

    pub(in crate::ui::shell) fn active_profile(&self) -> Option<SessionProfile> {
        let state = self.state.borrow();
        let profile_id = state.snapshot.active_profile_id.as_deref()?;
        state
            .snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
    }

    pub(in crate::ui::shell) fn has_active_terminal_session(&self) -> bool {
        self.state
            .borrow()
            .snapshot
            .active_terminal_tab_id
            .is_some()
    }

    pub(in crate::ui::shell) fn resolved_profile_for_session(
        &self,
        tab_id: TabId,
    ) -> Option<SessionProfile> {
        let state = self.state.borrow();
        let session = state.snapshot.sessions.get(&tab_id)?;
        session.pending_profile.clone().or_else(|| {
            state
                .snapshot
                .profiles
                .iter()
                .find(|profile| profile.id == session.profile_id)
                .cloned()
        })
    }
}

#[derive(Clone)]
pub(in crate::ui::shell) struct SessionTerminalPort {
    state: Rc<RefCell<SessionPortState>>,
}

impl SessionTerminalPort {
    pub(in crate::ui::shell) fn targets(&self) -> Vec<SessionTerminalTarget> {
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

    pub(in crate::ui::shell) fn target(&self, tab_id: TabId) -> Option<SessionTerminalTarget> {
        let state = self.state.borrow();
        let session = state.snapshot.sessions.get(&tab_id)?;
        (session.purpose == SessionPurpose::Terminal)
            .then(|| terminal_target(&state.snapshot, session))
    }

    pub(in crate::ui::shell) fn active_target(&self) -> Option<SessionTerminalTarget> {
        let tab_id = self.state.borrow().snapshot.active_terminal_tab_id?;
        self.target(tab_id)
    }

    pub(in crate::ui::shell) fn acquire(
        &self,
        tab_id: TabId,
    ) -> Result<TerminalLeaseGrant, TerminalLeaseError> {
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
        state.terminal_leases.insert(
            tab_id,
            TerminalLeaseEntry {
                tap: tap.clone(),
                retire_requested: false,
            },
        );
        Ok((
            TerminalLease {
                tab_id,
                tap,
                state: self.state.clone(),
            },
            output,
        ))
    }

    pub(in crate::ui::shell) fn forward_output(&self, tab_id: TabId, bytes: Vec<u8>) -> bool {
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

    pub(in crate::ui::shell) fn close_session(&self, tab_id: TabId) {
        SessionPortState::close_terminal_lease(&mut self.state.borrow_mut(), tab_id);
    }
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct SessionTerminalTarget {
    pub(in crate::ui::shell) tab_id: TabId,
    pub(in crate::ui::shell) title: String,
    pub(in crate::ui::shell) profile_id: String,
    pub(in crate::ui::shell) profile: Option<SessionProfile>,
    pub(in crate::ui::shell) command_available: bool,
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

pub(in crate::ui::shell) struct TerminalLeaseGrant {
    pub(in crate::ui::shell) commands: SessionCommandSender,
    pub(in crate::ui::shell) output: TerminalOutputReceiver,
    pub(in crate::ui::shell) lease: TerminalLease,
}

#[derive(Clone)]
pub(in crate::ui::shell) struct TerminalLease {
    tab_id: TabId,
    tap: TerminalOutputTap,
    state: Rc<RefCell<SessionPortState>>,
}

impl TerminalLease {
    pub(in crate::ui::shell) fn retire(&self) {
        let should_close = {
            let mut state = self.state.borrow_mut();
            let Some(entry) = state.terminal_leases.get_mut(&self.tab_id) else {
                self.tap.retire_lease();
                return;
            };
            if !entry.tap.same_channel(&self.tap) {
                self.tap.retire_lease();
                return;
            }
            entry.retire_requested = true;
            entry.tap.retire_lease();
            entry.tap.can_release_lease()
        };
        if should_close {
            SessionPortState::close_terminal_lease(&mut self.state.borrow_mut(), self.tab_id);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum TerminalLeaseError {
    Missing,
    Disconnected,
    Busy,
}

#[derive(Default)]
struct SessionPortState {
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
    retire_requested: bool,
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
            entry.tap.same_channel(tap) && entry.retire_requested && entry.tap.can_release_lease()
        });
    if should_close {
        SessionPortState::close_terminal_lease(&mut state.borrow_mut(), tab_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::shell::{SessionConnectionState, SessionMonitoringState, TerminalState};
    use miaominal_ssh::SessionEvent;

    fn session_payload(profile_id: &str) -> SessionTabState {
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
            pending_keyboard_interactive: None,
            reconnect_task: None,
            reconnect_attempt: 0,
            has_activity: false,
            monitoring: SessionMonitoringState::new(false),
            purpose: SessionPurpose::Terminal,
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
    fn query_port_starts_with_profiles_and_tracks_replacements() {
        let controller =
            SessionController::new_for_test_with_profiles(vec![profile("profile-a", "A")]);
        let query = controller.query_port();

        assert_eq!(query.profiles()[0].id, "profile-a");

        controller.replace_profiles(vec![profile("profile-b", "B")]);

        assert_eq!(query.profiles()[0].id, "profile-b");
        assert!(query.profile("profile-a").is_none());
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

    #[test]
    fn snapshot_exposes_active_profile_and_ordered_terminal_targets() {
        let (query, terminal) = ports();

        assert_eq!(
            query.active_profile().map(|profile| profile.id),
            Some("profile-b".to_string())
        );
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
        let (lease, _output) = terminal
            .acquire_output_lease(TabId::new(7))
            .expect("lease should be acquired");

        terminal.close_session(TabId::new(7));

        assert!(lease.tap.try_send(vec![1]).is_err());
        assert!(terminal.acquire_output_lease(TabId::new(7)).is_ok());
    }

    #[test]
    fn output_is_forwarded_only_to_the_current_terminal_lease() {
        let (_, terminal) = ports();
        let (old, _old_output) = terminal
            .acquire_output_lease(TabId::new(7))
            .expect("old lease should be acquired");
        old.retire();
        let (new, _new_output) = terminal
            .acquire_output_lease(TabId::new(7))
            .expect("new lease should be acquired");

        assert!(terminal.forward_output(TabId::new(7), vec![1]));
        assert!(old.tap.try_send(vec![2]).is_err());
        assert!(new.tap.try_send(vec![3]).is_ok());
        assert!(!terminal.forward_output(TabId::new(99), vec![4]));
    }
}
