//! Immutable Session results and runtime ports exposed to desktop consumers.
use crate::ui::contracts::workspace::{TabId, TabKindTag, TabPlacement};
use gpui_kit::App;
use miaominal_agent::TerminalOutputReceiver;
use miaominal_core::profile::SessionProfile;
use miaominal_ssh::SessionCommandSender;
use std::{collections::HashMap, rc::Rc};
#[derive(Clone, Debug)]
pub(in crate::ui) enum SessionNotificationTone {
    Success,
    Error,
}

#[derive(Clone, Debug)]
pub(in crate::ui) struct SessionNotificationRequest {
    pub(in crate::ui) tone: SessionNotificationTone,
    pub(in crate::ui) title: String,
    pub(in crate::ui) message: String,
    pub(in crate::ui) id: String,
}

#[derive(Clone, Debug)]
pub(in crate::ui) enum SessionEventTabRemoval {
    ConnectionTest { status_message: String },
}

#[derive(Clone, Debug)]
pub(in crate::ui) struct SessionEventOutcome {
    pub(in crate::ui) connected_profile: Option<(String, u64)>,
    pub(in crate::ui) tab_status: Option<String>,
    pub(in crate::ui) clipboard_writes: Vec<String>,
    pub(in crate::ui) notification: Option<SessionNotificationRequest>,
    pub(in crate::ui) removal: Option<SessionEventTabRemoval>,
    pub(in crate::ui) schedule_reconnect_error: Option<String>,
    pub(in crate::ui) refresh_monitoring_profile: Option<String>,
    pub(in crate::ui) should_notify: bool,
}

pub(in crate::ui) struct ClosedSessionTabState {
    pub(in crate::ui) tab_id: TabId,
    pub(in crate::ui) profile: SessionProfile,
    pub(in crate::ui) hidden_from_topbar: bool,
}

/// Immutable Session description offered to Workspace when a live payload opens.
/// Workspace owns registration, placement changes and subsequent title/status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionTabDescriptor {
    id: TabId,
    title: String,
    status: String,
    kind: TabKindTag,
    placement: TabPlacement,
}

impl SessionTabDescriptor {
    pub(crate) fn new(
        id: TabId,
        title: String,
        status: String,
        kind: TabKindTag,
        placement: TabPlacement,
    ) -> Self {
        Self {
            id,
            title,
            status,
            kind,
            placement,
        }
    }
    pub(crate) fn id(&self) -> TabId {
        self.id
    }
    pub(crate) fn title(&self) -> &str {
        &self.title
    }
    pub(crate) fn status(&self) -> &str {
        &self.status
    }
    pub(crate) fn kind(&self) -> TabKindTag {
        self.kind
    }
    pub(crate) fn placement(&self) -> TabPlacement {
        self.placement
    }
    pub(crate) fn with_status(mut self, status: String) -> Self {
        self.status = status;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum SessionPurpose {
    Terminal,
    PortForwarding,
    ConnectionTest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(in crate::ui) enum SessionSidePanelView {
    #[default]
    Monitor,
    Snippets,
    Sftp,
}

pub(in crate::ui) struct TransferredSessionWorkspaceUi {
    pub(in crate::ui) side_panel_open: bool,
    pub(in crate::ui) side_panel_view: SessionSidePanelView,
    pub(in crate::ui) terminal_search_query: String,
    pub(in crate::ui) terminal_search_open: bool,
    pub(in crate::ui) terminal_search_total: usize,
    pub(in crate::ui) terminal_search_current: Option<usize>,
    pub(in crate::ui) terminal_search_status: Option<String>,
    pub(in crate::ui) keyboard_interactive_values: Vec<String>,
    pub(in crate::ui) keyboard_interactive_target: Option<(TabId, u64)>,
}

#[derive(Clone, Default)]
pub(in crate::ui) struct SessionPortSnapshot {
    pub(in crate::ui) profiles: Vec<SessionProfile>,
    pub(in crate::ui) sessions: HashMap<TabId, SessionPortSession>,
    pub(in crate::ui) terminal_order: Vec<TabId>,
    pub(in crate::ui) active_profile_id: Option<String>,
    pub(in crate::ui) active_terminal_tab_id: Option<TabId>,
}

impl SessionPortSnapshot {
    pub(in crate::ui) fn new(
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
pub(in crate::ui) struct SessionPortSession {
    pub(in crate::ui) tab_id: TabId,
    pub(in crate::ui) title: String,
    pub(in crate::ui) profile_id: String,
    pub(in crate::ui) pending_profile: Option<SessionProfile>,
    pub(in crate::ui) purpose: SessionPurpose,
    pub(in crate::ui) commands: Option<SessionCommandSender>,
}

impl SessionPortSession {
    pub(in crate::ui) fn new(
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

#[derive(Clone, Debug)]
pub(in crate::ui) struct SessionTerminalTarget {
    pub(in crate::ui) tab_id: TabId,
    pub(in crate::ui) title: String,
    pub(in crate::ui) profile_id: String,
    pub(in crate::ui) profile: Option<SessionProfile>,
    pub(in crate::ui) command_available: bool,
}

pub(in crate::ui) struct TerminalLeaseGrant {
    pub(in crate::ui) commands: SessionCommandSender,
    pub(in crate::ui) output: TerminalOutputReceiver,
    pub(in crate::ui) lease: TerminalLease,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum TerminalLeaseError {
    Missing,
    Disconnected,
    Busy,
}

/// A private Session payload traveling through Workspace. Only its owner can
/// name the concrete payload and recover it; Workspace holds identity metadata.
pub(in crate::ui) struct TransferredSessionTab(Box<dyn std::any::Any>);
impl TransferredSessionTab {
    pub(in crate::ui) fn new<T: 'static>(payload: T) -> Self {
        Self(Box::new(payload))
    }
    pub(in crate::ui) fn into_payload<T: 'static>(self) -> T {
        *self
            .0
            .downcast::<T>()
            .expect("Session handoff must return to the Session owner")
    }
}

pub(in crate::ui) trait SessionQuerySource {
    fn active_profile_id(&self) -> Option<String>;
    fn resolved_profile_for_session(
        &self,
        tab_id: TabId,
        catalog: &super::CatalogQueryPort,
    ) -> Option<SessionProfile>;
}

#[derive(Clone)]
pub(in crate::ui) struct SessionQueryPort(Rc<dyn SessionQuerySource>);
impl SessionQueryPort {
    pub(in crate::ui) fn new(source: Rc<dyn SessionQuerySource>) -> Self {
        Self(source)
    }
    pub(in crate::ui) fn active_profile_id(&self) -> Option<String> {
        self.0.active_profile_id()
    }
    pub(in crate::ui) fn resolved_profile_for_session(
        &self,
        tab_id: TabId,
        catalog: &super::CatalogQueryPort,
    ) -> Option<SessionProfile> {
        self.0.resolved_profile_for_session(tab_id, catalog)
    }
}

pub(in crate::ui) trait SessionTerminalRuntime {
    fn paste_text(&self, tab_id: TabId, text: String, cx: &mut App) -> bool;
    fn targets(&self) -> Vec<SessionTerminalTarget>;
    fn target(&self, tab_id: TabId) -> Option<SessionTerminalTarget>;
    fn active_target(&self) -> Option<SessionTerminalTarget>;
    fn acquire(&self, tab_id: TabId) -> Result<TerminalLeaseGrant, TerminalLeaseError>;
    fn forward_output(&self, tab_id: TabId, bytes: Vec<u8>) -> bool;
    /// Recent plain text (`lines` lines) and selection of a terminal tab.
    fn text_snapshot(
        &self,
        tab_id: TabId,
        lines: usize,
        cx: &App,
    ) -> Option<(String, Option<String>)>;
    fn close_session(&self, tab_id: TabId);
    #[cfg(test)]
    fn acquire_output_lease_for_test(
        &self,
        tab_id: TabId,
    ) -> Result<(TerminalLease, TerminalOutputReceiver), TerminalLeaseError>;
}
#[derive(Clone)]
pub(in crate::ui) struct SessionTerminalPort(Rc<dyn SessionTerminalRuntime>);
impl SessionTerminalPort {
    pub(in crate::ui) fn new(runtime: Rc<dyn SessionTerminalRuntime>) -> Self {
        Self(runtime)
    }
    pub(in crate::ui) fn paste_text(&self, tab_id: TabId, text: String, cx: &mut App) -> bool {
        self.0.paste_text(tab_id, text, cx)
    }
    pub(in crate::ui) fn targets(&self) -> Vec<SessionTerminalTarget> {
        self.0.targets()
    }
    pub(in crate::ui) fn target(&self, tab_id: TabId) -> Option<SessionTerminalTarget> {
        self.0.target(tab_id)
    }
    pub(in crate::ui) fn active_target(&self) -> Option<SessionTerminalTarget> {
        self.0.active_target()
    }
    pub(in crate::ui) fn acquire(
        &self,
        tab_id: TabId,
    ) -> Result<TerminalLeaseGrant, TerminalLeaseError> {
        self.0.acquire(tab_id)
    }
    pub(in crate::ui) fn forward_output(&self, tab_id: TabId, bytes: Vec<u8>) -> bool {
        self.0.forward_output(tab_id, bytes)
    }
    pub(in crate::ui) fn text_snapshot(
        &self,
        tab_id: TabId,
        lines: usize,
        cx: &App,
    ) -> Option<(String, Option<String>)> {
        self.0.text_snapshot(tab_id, lines, cx)
    }
    pub(in crate::ui) fn close_session(&self, tab_id: TabId) {
        self.0.close_session(tab_id)
    }
    #[cfg(test)]
    pub(in crate::ui) fn acquire_output_lease(
        &self,
        tab_id: TabId,
    ) -> Result<(TerminalLease, TerminalOutputReceiver), TerminalLeaseError> {
        self.0.acquire_output_lease_for_test(tab_id)
    }
}

pub(in crate::ui) trait TerminalLeaseRetirement {
    fn retire(&self);
}
#[derive(Clone)]
pub(in crate::ui) struct TerminalLease(Rc<dyn TerminalLeaseRetirement>);
impl TerminalLease {
    pub(in crate::ui) fn new(lease: Rc<dyn TerminalLeaseRetirement>) -> Self {
        Self(lease)
    }
    pub(in crate::ui) fn retire(&self) {
        self.0.retire();
    }
}

/// Immutable chrome and navigation facts; terminal resources remain in Session.
#[derive(Clone)]
pub(in crate::ui) struct SessionTabSnapshot {
    profile_id: String,
    pending_profile: Option<SessionProfile>,
    purpose: SessionPurpose,
    connected: bool,
    connection_error: bool,
    needs_attention: bool,
    has_activity: bool,
    columns: usize,
    screen_lines: usize,
    bytes_in: u64,
    bytes_out: u64,
    sftp_progress_visible: bool,
}
impl SessionTabSnapshot {
    pub(in crate::ui) fn new(
        profile_id: String,
        pending_profile: Option<SessionProfile>,
        purpose: SessionPurpose,
    ) -> Self {
        Self {
            profile_id,
            pending_profile,
            purpose,
            connected: false,
            connection_error: false,
            needs_attention: false,
            has_activity: false,
            columns: 0,
            screen_lines: 0,
            bytes_in: 0,
            bytes_out: 0,
            sftp_progress_visible: false,
        }
    }
    pub(in crate::ui) fn with_connection(
        mut self,
        connected: bool,
        error: bool,
        attention: bool,
    ) -> Self {
        self.connected = connected;
        self.connection_error = error;
        self.needs_attention = attention;
        self
    }
    pub(in crate::ui) fn with_activity(mut self, activity: bool) -> Self {
        self.has_activity = activity;
        self
    }
    pub(in crate::ui) fn with_terminal(
        mut self,
        columns: usize,
        screen_lines: usize,
        bytes_in: u64,
        bytes_out: u64,
    ) -> Self {
        self.columns = columns;
        self.screen_lines = screen_lines;
        self.bytes_in = bytes_in;
        self.bytes_out = bytes_out;
        self
    }
    pub(in crate::ui) fn with_sftp_progress_visible(mut self, visible: bool) -> Self {
        self.sftp_progress_visible = visible;
        self
    }
    pub(in crate::ui) fn profile_id(&self) -> &str {
        &self.profile_id
    }
    pub(in crate::ui) fn pending_profile(&self) -> Option<&SessionProfile> {
        self.pending_profile.as_ref()
    }
    pub(in crate::ui) fn purpose(&self) -> SessionPurpose {
        self.purpose
    }
    pub(in crate::ui) fn connected(&self) -> bool {
        self.connected
    }
    pub(in crate::ui) fn connection_error(&self) -> bool {
        self.connection_error
    }
    pub(in crate::ui) fn needs_attention(&self) -> bool {
        self.needs_attention
    }
    pub(in crate::ui) fn has_activity(&self) -> bool {
        self.has_activity
    }
    pub(in crate::ui) fn columns(&self) -> usize {
        self.columns
    }
    pub(in crate::ui) fn screen_lines(&self) -> usize {
        self.screen_lines
    }
    pub(in crate::ui) fn bytes_in(&self) -> u64 {
        self.bytes_in
    }
    pub(in crate::ui) fn bytes_out(&self) -> u64 {
        self.bytes_out
    }
    pub(in crate::ui) fn sftp_progress_visible(&self) -> bool {
        self.sftp_progress_visible
    }
}

/// A newly prepared live Session payload. Only Session can inspect its contents.
pub(in crate::ui) struct PreparedSessionTab(Box<dyn std::any::Any>);
impl PreparedSessionTab {
    pub(in crate::ui) fn new<T: 'static>(payload: T) -> Self {
        Self(Box::new(payload))
    }
    pub(in crate::ui) fn into_payload<T: 'static>(self) -> T {
        *self
            .0
            .downcast::<T>()
            .expect("Prepared Session must install in the Session owner")
    }
}
