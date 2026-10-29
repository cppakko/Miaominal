//! Interactive SSH sessions, connection state, and terminal frames.
//!
//! A session owns one `miaominal-terminal` state, one SSH session (started
//! through the shared connector), the typed connection phase, and the pending
//! authentication request. Kotlin never parses log strings: it reads the
//! snapshot, responds with request ids, and pulls render frames.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::{Notify, watch};

use miaominal_core::forwarding::HostKeyDecision;
use miaominal_ssh::{
    AuthenticationRequest, HostKeyRequest, SessionCommand, SessionConnection, SessionEvent,
    SessionPhase,
};
use miaominal_terminal::{
    SearchMatchKind, TerminalColor, TerminalEvent, TerminalInputModes, TerminalKeyPhase,
    TerminalKeyStroke, TerminalModifiers, TerminalScroll, TerminalSelectionKind, TerminalState,
    TerminalTheme, encode_terminal_input, sanitize_paste,
};

use crate::error::BackendError;
use crate::events::{BackendEvent, EventBus};
use crate::platform::PlatformHost;

/// Maximum time an explicit disconnect waits for the session task to finish.
const DISCONNECT_GRACE: Duration = Duration::from_secs(8);
/// Upper bound for frame rows (guards the grid size).
const MAX_FRAME_LINES: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SshSessionPhase {
    Connecting,
    WaitingForTrust,
    WaitingForAuthentication,
    OpeningShell,
    Ready,
    Closing,
    Closed,
    Failed,
}

impl SshSessionPhase {
    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Closed | Self::Failed)
    }
}

fn phase_from_event(phase: SessionPhase) -> SshSessionPhase {
    match phase {
        SessionPhase::Connecting => SshSessionPhase::Connecting,
        SessionPhase::WaitingForTrust => SshSessionPhase::WaitingForTrust,
        SessionPhase::WaitingForAuthentication => SshSessionPhase::WaitingForAuthentication,
        SessionPhase::OpeningShell => SshSessionPhase::OpeningShell,
        SessionPhase::Ready => SshSessionPhase::Ready,
        SessionPhase::Closing => SshSessionPhase::Closing,
        SessionPhase::Closed => SshSessionPhase::Closed,
        SessionPhase::Failed => SshSessionPhase::Failed,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SshRequestKind {
    HostKey,
    KeyboardInteractive,
}

/// Sampling state of the remote monitor; refreshed through `monitor_snapshot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MonitorStatus {
    /// Sampling is paused; no new samples are requested.
    Disabled,
    /// Enabled, waiting for the first sample.
    Waiting,
    /// The last sample succeeded.
    Ready,
    /// The last sample failed; `failure`/`failure_kind` describe why.
    Failed,
}

/// Stable classification of a monitoring failure (never a log string).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MonitorFailureKind {
    None,
    /// The remote OS has no supported sampling script.
    UnsupportedPlatform,
    /// The remote sampling command failed.
    CommandFailed,
    /// The SSH transport or channel failed.
    Transport,
    Other,
}

fn classify_monitor_failure(message: &str) -> MonitorFailureKind {
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("unsupported remote os") {
        return MonitorFailureKind::UnsupportedPlatform;
    }
    if lowered.contains("monitoring command failed") {
        return MonitorFailureKind::CommandFailed;
    }
    if lowered.contains("channel")
        || lowered.contains("connection")
        || lowered.contains("closed")
        || lowered.contains("eof")
        || lowered.contains("broken pipe")
    {
        return MonitorFailureKind::Transport;
    }
    MonitorFailureKind::Other
}

/// Platform-independent remote resource metrics.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct MonitorMetricsDto {
    pub platform: String,
    pub hostname: Option<String>,
    pub logical_cpu_count: Option<u32>,
    pub uptime_seconds: Option<u64>,
    pub cpu_percent: f64,
    pub memory_percent: f64,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub swap_percent: f64,
    pub swap_used_bytes: u64,
    pub swap_total_bytes: u64,
    pub disk_percent: f64,
    pub disk_used_bytes: Option<u64>,
    pub disk_total_bytes: Option<u64>,
    pub network_rx_kbps: f64,
    pub network_tx_kbps: f64,
    pub load: f64,
}

impl From<&miaominal_ssh::SessionMonitorSnapshot> for MonitorMetricsDto {
    fn from(snapshot: &miaominal_ssh::SessionMonitorSnapshot) -> Self {
        Self {
            platform: match snapshot.platform {
                miaominal_core::forwarding::SessionMonitorPlatform::Linux => "Linux",
                miaominal_core::forwarding::SessionMonitorPlatform::Macos => "Macos",
                miaominal_core::forwarding::SessionMonitorPlatform::Windows => "Windows",
            }
            .to_string(),
            hostname: snapshot.hostname.clone(),
            logical_cpu_count: snapshot.logical_cpu_count,
            uptime_seconds: snapshot.uptime_seconds,
            cpu_percent: snapshot.cpu_percent,
            memory_percent: snapshot.memory_percent,
            memory_used_bytes: snapshot.memory_used_bytes,
            memory_total_bytes: snapshot.memory_total_bytes,
            swap_percent: snapshot.swap_percent,
            swap_used_bytes: snapshot.swap_used_bytes,
            swap_total_bytes: snapshot.swap_total_bytes,
            disk_percent: snapshot.disk_percent,
            disk_used_bytes: snapshot.disk_used_bytes,
            disk_total_bytes: snapshot.disk_total_bytes,
            network_rx_kbps: snapshot.network_rx_kbps,
            network_tx_kbps: snapshot.network_tx_kbps,
            load: snapshot.load,
        }
    }
}

/// Pull-based monitor state: the latest sample replaces the previous one
/// (never a queue), so a page that is away does not accumulate samples.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct SshMonitorSnapshot {
    pub enabled: bool,
    pub status: MonitorStatus,
    pub failure_kind: MonitorFailureKind,
    pub failure: Option<String>,
    pub metrics: Option<MonitorMetricsDto>,
    /// Bumped once per accepted sample.
    pub sample_sequence: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SshPromptDto {
    pub prompt: String,
    pub echo: bool,
}

/// Pending authentication or trust request bound to its connection and hop.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SshSessionRequest {
    pub request_id: u64,
    pub connection_id: u64,
    pub kind: SshRequestKind,
    pub hop_label: String,
    pub hop_index: u32,
    pub hop_total: u32,
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
    pub previous_fingerprint: Option<String>,
    pub key_changed: bool,
    pub name: String,
    pub instructions: String,
    pub prompts: Vec<SshPromptDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SshSessionSnapshot {
    pub connection_id: u64,
    pub host_id: String,
    pub host_label: String,
    pub phase: SshSessionPhase,
    pub failure: Option<String>,
    pub hop_label: Option<String>,
    pub request: Option<SshSessionRequest>,
    pub columns: u32,
    pub lines: u32,
    /// Monotonic revision bumped on every state or terminal change; pass it to
    /// `await_update` to wait for the next change.
    pub revision: u64,
    pub exit_code: Option<u32>,
    pub exit_signal: Option<String>,
    pub started_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TerminalSelectionKindDto {
    Simple,
    Block,
    Semantic,
    Lines,
}

impl TerminalSelectionKindDto {
    fn into_terminal(self) -> TerminalSelectionKind {
        match self {
            Self::Simple => TerminalSelectionKind::Simple,
            Self::Block => TerminalSelectionKind::Block,
            Self::Semantic => TerminalSelectionKind::Semantic,
            Self::Lines => TerminalSelectionKind::Lines,
        }
    }
}

/// Cell attribute bits; kept in sync with the Kotlin renderer.
pub mod cell_flags {
    pub const BOLD: u32 = 1 << 0;
    pub const ITALIC: u32 = 1 << 1;
    pub const DIM: u32 = 1 << 2;
    pub const UNDERLINE: u32 = 1 << 3;
    pub const STRIKETHROUGH: u32 = 1 << 4;
    pub const WIDE: u32 = 1 << 5;
    pub const SPACER: u32 = 1 << 6;
    pub const CURSOR: u32 = 1 << 7;
    pub const SEARCH_MATCH: u32 = 1 << 8;
    pub const SEARCH_CURRENT: u32 = 1 << 9;
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TerminalFrameCell {
    /// Base character plus any zero-width combining characters.
    pub text: String,
    /// `0xAARRGGBB`.
    pub fg: u32,
    pub bg: u32,
    pub flags: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TerminalFrameRow {
    pub row: u32,
    pub cells: Vec<TerminalFrameCell>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct TerminalSelectionDto {
    pub start_line: i32,
    pub start_column: u32,
    pub end_line: i32,
    pub end_column: u32,
    pub block: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct TerminalFrameModes {
    pub bracketed_paste: bool,
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub focus_events: bool,
    pub mouse_protocol: u32,
    pub mouse_encoding: u32,
    pub alternate_screen: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TerminalRenderFrame {
    /// Version of this frame; pass it back to receive changed rows only.
    pub frame_version: u64,
    /// Terminal generation this frame was built from.
    pub revision: u64,
    /// True when every row is present (first frame, resize, or stale version).
    pub full: bool,
    pub columns: u32,
    pub lines: u32,
    pub cursor_visible: bool,
    pub cursor_line: i32,
    pub cursor_column: u32,
    pub focused: bool,
    pub display_offset: u32,
    pub history_size: u32,
    pub default_fg: u32,
    pub default_bg: u32,
    pub selection: Option<TerminalSelectionDto>,
    pub search_total: u32,
    pub search_current: Option<u32>,
    pub modes: TerminalFrameModes,
    pub rows: Vec<TerminalFrameRow>,
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TerminalUpdate {
    pub revision: u64,
    pub phase: SshSessionPhase,
    /// True when the timeout elapsed without a change.
    pub idle: bool,
    pub closed: bool,
}

fn color_to_argb(color: TerminalColor) -> u32 {
    let a = (color.a.clamp(0.0, 1.0) * 255.0).round() as u32;
    let r = (color.r.clamp(0.0, 1.0) * 255.0).round() as u32;
    let g = (color.g.clamp(0.0, 1.0) * 255.0).round() as u32;
    let b = (color.b.clamp(0.0, 1.0) * 255.0).round() as u32;
    (a << 24) | (r << 16) | (g << 8) | b
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn cell_flags_of(cell: &miaominal_terminal::TerminalCell) -> u32 {
    let mut flags = 0u32;
    if cell.bold {
        flags |= cell_flags::BOLD;
    }
    if cell.italic {
        flags |= cell_flags::ITALIC;
    }
    if cell.dim {
        flags |= cell_flags::DIM;
    }
    if cell.underline {
        flags |= cell_flags::UNDERLINE;
    }
    if cell.strikethrough {
        flags |= cell_flags::STRIKETHROUGH;
    }
    if cell.wide {
        flags |= cell_flags::WIDE;
    }
    if cell.spacer {
        flags |= cell_flags::SPACER;
    }
    if cell.is_cursor {
        flags |= cell_flags::CURSOR;
    }
    match cell.search_match {
        SearchMatchKind::None => {}
        SearchMatchKind::Match => flags |= cell_flags::SEARCH_MATCH,
        SearchMatchKind::Current => flags |= cell_flags::SEARCH_CURRENT,
    }
    flags
}

fn build_row(row: usize, cells: &[miaominal_terminal::TerminalCell]) -> TerminalFrameRow {
    TerminalFrameRow {
        row: row as u32,
        cells: cells
            .iter()
            .map(|cell| {
                let mut text = String::new();
                if cell.spacer || cell.character == '\0' {
                    text.push(' ');
                } else {
                    text.push(cell.character);
                    if !cell.zero_width.is_empty() {
                        text.extend(cell.zero_width.iter());
                    }
                }
                TerminalFrameCell {
                    text,
                    fg: color_to_argb(cell.fg),
                    bg: color_to_argb(cell.bg),
                    flags: cell_flags_of(cell),
                }
            })
            .collect(),
    }
}

fn selection_dto(terminal: &TerminalState) -> Option<TerminalSelectionDto> {
    terminal
        .selection_viewport_range()
        .map(|range| TerminalSelectionDto {
            start_line: range.start_line,
            start_column: range.start_column as u32,
            end_line: range.end_line,
            end_column: range.end_column as u32,
            block: range.block,
        })
}

fn modes_dto(terminal: &TerminalState) -> TerminalFrameModes {
    let modes: TerminalInputModes = terminal.input_modes();
    TerminalFrameModes {
        bracketed_paste: modes.bracketed_paste,
        app_cursor: modes.app_cursor,
        app_keypad: modes.app_keypad,
        focus_events: modes.focus_in_out,
        mouse_protocol: match terminal.mouse_protocol() {
            miaominal_terminal::MouseProtocol::Off => 0,
            miaominal_terminal::MouseProtocol::Normal => 1,
            miaominal_terminal::MouseProtocol::ButtonEvent => 2,
            miaominal_terminal::MouseProtocol::AnyEvent => 3,
        },
        mouse_encoding: match terminal.mouse_encoding() {
            miaominal_terminal::MouseEncoding::Default => 0,
            miaominal_terminal::MouseEncoding::Sgr => 1,
            miaominal_terminal::MouseEncoding::Utf8 => 2,
        },
        alternate_screen: terminal.alternate_scroll_active(),
    }
}

struct SessionState {
    host_id: String,
    phase: SshSessionPhase,
    failure: Option<String>,
    hop_label: Option<String>,
    request: Option<SshSessionRequest>,
    exit_code: Option<u32>,
    exit_signal: Option<String>,
    revision: u64,
    started_at_ms: u64,
    monitor: MonitorState,
}

/// Latest remote-monitor state; samples are replaced, never queued.
#[derive(Default)]
struct MonitorState {
    enabled: bool,
    status: MonitorStatus,
    failure_kind: MonitorFailureKind,
    failure: Option<String>,
    metrics: Option<MonitorMetricsDto>,
    sample_sequence: u64,
    updated_at_ms: u64,
}

impl Default for MonitorStatus {
    fn default() -> Self {
        Self::Disabled
    }
}

impl Default for MonitorFailureKind {
    fn default() -> Self {
        Self::None
    }
}

struct FrameCache {
    version: u64,
    generation: u64,
    columns: usize,
    lines: usize,
    focused: bool,
    rows: Arc<Vec<TerminalFrameRow>>,
    cursor_visible: bool,
    cursor_line: i32,
    cursor_column: u32,
    display_offset: u32,
    history_size: u32,
    default_fg: u32,
    default_bg: u32,
    selection: Option<TerminalSelectionDto>,
    search_total: u32,
    search_current: Option<u32>,
    modes: TerminalFrameModes,
}

/// Shared session state; the Kotlin handle and the event task both use it.
pub(crate) struct SshSessionInner {
    connection_id: u64,
    host: Arc<dyn PlatformHost>,
    /// Weak back-reference: the registry owns the sessions, so a strong link
    /// here would keep every session (and its terminal/parser thread) alive
    /// after a backend restart.
    registry: Weak<SessionRegistry>,
    terminal: TerminalState,
    state: Mutex<SessionState>,
    wake: Notify,
    commands: Mutex<Option<miaominal_ssh::SessionCommandSender>>,
    consumer: Mutex<Option<tokio::task::JoinHandle<()>>>,
    ssh_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    frame: Mutex<Option<FrameCache>>,
    close_requested: AtomicBool,
    focused: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl SshSessionInner {
    fn new(
        connection_id: u64,
        host: Arc<dyn PlatformHost>,
        registry: &Arc<SessionRegistry>,
        host_id: String,
        host_label: String,
        terminal: TerminalState,
    ) -> Arc<Self> {
        Arc::new(Self {
            connection_id,
            host,
            registry: Arc::downgrade(registry),
            terminal,
            state: Mutex::new(SessionState {
                host_id,
                phase: SshSessionPhase::Connecting,
                failure: None,
                hop_label: Some(host_label),
                request: None,
                exit_code: None,
                exit_signal: None,
                revision: 1,
                started_at_ms: now_ms(),
                monitor: MonitorState::default(),
            }),
            wake: Notify::new(),
            commands: Mutex::new(None),
            consumer: Mutex::new(None),
            ssh_thread: Mutex::new(None),
            frame: Mutex::new(None),
            close_requested: AtomicBool::new(false),
            focused: AtomicBool::new(true),
        })
    }

    fn attach(
        &self,
        commands: miaominal_ssh::SessionCommandSender,
        consumer: tokio::task::JoinHandle<()>,
        ssh_thread: Option<std::thread::JoinHandle<()>>,
    ) {
        let mut slot = lock(&self.commands);
        if self.close_requested.load(Ordering::SeqCst) {
            let _ = commands.close();
        }
        *slot = Some(commands);
        drop(slot);
        *lock(&self.consumer) = Some(consumer);
        *lock(&self.ssh_thread) = ssh_thread;
    }

    /// Publish a control event through the owning registry, if it still exists.
    fn publish(&self, event: BackendEvent) {
        if let Some(registry) = self.registry.upgrade() {
            registry.publish(event);
        }
    }

    fn bump_revision(&self) -> u64 {
        let mut state = lock(&self.state);
        state.revision = state.revision.wrapping_add(1);
        state.revision
    }

    fn set_phase(
        &self,
        phase: SshSessionPhase,
        hop_label: Option<String>,
        failure: Option<String>,
    ) {
        {
            let mut state = lock(&self.state);
            if state.phase.is_terminal() {
                return;
            }
            state.phase = phase;
            if hop_label.is_some() {
                state.hop_label = hop_label;
            }
            if phase == SshSessionPhase::Failed {
                state.failure = failure;
            }
            state.revision = state.revision.wrapping_add(1);
        }
        self.wake.notify_waiters();
        self.publish(BackendEvent::SessionPhaseChanged {
            connection_id: self.connection_id,
            phase,
        });
    }

    fn set_request(&self, request: Option<SshSessionRequest>) {
        let mut state = lock(&self.state);
        state.request = request;
        state.revision = state.revision.wrapping_add(1);
    }

    fn fail(&self, message: String) {
        let already_terminal = lock(&self.state).phase.is_terminal();
        if already_terminal {
            return;
        }
        self.set_phase(SshSessionPhase::Failed, None, Some(message));
    }

    fn mark_closed(&self) {
        {
            let mut state = lock(&self.state);
            state.request = None;
            if !state.phase.is_terminal() {
                state.phase = SshSessionPhase::Closed;
            }
            state.revision = state.revision.wrapping_add(1);
        }
        self.wake.notify_waiters();
        self.publish(BackendEvent::SessionPhaseChanged {
            connection_id: self.connection_id,
            phase: lock(&self.state).phase,
        });
    }

    fn publish_request(&self, request: SshSessionRequest) {
        let event = BackendEvent::SessionRequest {
            request: request.clone(),
        };
        self.set_request(Some(request));
        self.wake.notify_waiters();
        self.publish(event);
    }

    /// Apply one SSH session event. Runs on the session task.
    fn apply_event(&self, event: SessionEvent) {
        match event {
            SessionEvent::Phase(change) => {
                let hop = change.hop.map(|hop| hop.label);
                self.set_phase(phase_from_event(change.phase), hop, None);
            }
            SessionEvent::HostKeyPrompt(request) => {
                let dto = host_key_request_dto(self.connection_id, &request);
                self.set_phase(
                    SshSessionPhase::WaitingForTrust,
                    Some(request.hop.label.clone()),
                    None,
                );
                self.publish_request(dto);
            }
            SessionEvent::KeyboardInteractivePrompt(request) => {
                let dto = authentication_request_dto(self.connection_id, &request);
                self.set_phase(
                    SshSessionPhase::WaitingForAuthentication,
                    Some(request.hop.label.clone()),
                    None,
                );
                self.publish_request(dto);
            }
            SessionEvent::RequestExpired { request_id } => {
                {
                    let mut state = lock(&self.state);
                    if state
                        .request
                        .as_ref()
                        .is_some_and(|request| request.request_id == request_id)
                    {
                        state.request = None;
                    }
                    state.revision = state.revision.wrapping_add(1);
                }
                self.wake.notify_waiters();
                self.publish(BackendEvent::SessionRequestExpired {
                    connection_id: self.connection_id,
                    request_id,
                });
            }
            SessionEvent::Output(bytes) => {
                // Parsing is asynchronous; wait for it so terminal responses
                // (PtyWrite, OSC 52) are drained before the next remote output.
                self.terminal.push_bytes_blocking(&bytes);
                self.drain_terminal_events();
                self.bump_revision();
                self.wake.notify_waiters();
            }
            SessionEvent::Exited { exit_code, signal } => {
                let event = BackendEvent::SessionExited {
                    connection_id: self.connection_id,
                    exit_code,
                    signal: signal.clone(),
                };
                {
                    let mut state = lock(&self.state);
                    state.exit_code = Some(exit_code);
                    state.exit_signal = signal;
                    state.revision = state.revision.wrapping_add(1);
                }
                self.wake.notify_waiters();
                self.publish(event);
            }
            SessionEvent::Error(message) => {
                if self.phase() == SshSessionPhase::Closing {
                    // A user-requested close surfaces as a cancellation error.
                    self.mark_closed();
                } else {
                    self.fail(message);
                }
            }
            SessionEvent::Closed => {
                self.mark_closed();
            }
            SessionEvent::Connected(_)
            | SessionEvent::Status(_)
            | SessionEvent::PortForwardReconnecting { .. }
            | SessionEvent::PortForwardNotice(_)
            | SessionEvent::PortForwardActive(_)
            | SessionEvent::PortForwardFailed(_) => {}
            SessionEvent::MonitorUpdated(snapshot) => {
                let metrics = MonitorMetricsDto::from(&snapshot);
                let mut state = lock(&self.state);
                if !state.monitor.enabled {
                    return;
                }
                state.monitor.status = MonitorStatus::Ready;
                state.monitor.failure_kind = MonitorFailureKind::None;
                state.monitor.failure = None;
                state.monitor.metrics = Some(metrics);
                state.monitor.sample_sequence = state.monitor.sample_sequence.wrapping_add(1);
                state.monitor.updated_at_ms = now_ms();
            }
            SessionEvent::MonitorFailed(message) => {
                let mut state = lock(&self.state);
                if !state.monitor.enabled {
                    return;
                }
                state.monitor.status = MonitorStatus::Failed;
                state.monitor.failure_kind = classify_monitor_failure(&message);
                state.monitor.failure = Some(message);
            }
        }
    }

    fn drain_terminal_events(&self) {
        while let Some(event) = self.terminal.try_recv_event() {
            match event {
                TerminalEvent::PtyWrite(text) => {
                    if let Some(commands) = lock(&self.commands).clone() {
                        let _ = commands.send_bytes(text.into_bytes());
                    }
                }
                TerminalEvent::ClipboardStore(content) => {
                    self.host.store_clipboard(content);
                }
                TerminalEvent::Bell => {}
            }
        }
    }

    fn send_command(&self, command: SessionCommand) -> Result<(), BackendError> {
        let commands = lock(&self.commands).clone();
        let Some(commands) = commands else {
            return Err(BackendError::Closed);
        };
        let result = match command {
            SessionCommand::Send(bytes) => commands.send_bytes(bytes),
            SessionCommand::Resize { columns, lines } => commands.resize(columns, lines),
            SessionCommand::SetMonitoringEnabled(enabled) => {
                commands.set_monitoring_enabled(enabled)
            }
            SessionCommand::HostKeyDecision {
                request_id,
                decision,
            } => commands.respond_host_key(request_id, decision),
            SessionCommand::KeyboardInteractiveResponse {
                request_id,
                answers,
            } => commands.respond_keyboard_interactive(request_id, answers),
            SessionCommand::SyncPortForwardRules(rules) => commands.sync_port_forward_rules(rules),
            SessionCommand::Close => commands.close(),
        };
        result.map_err(|_| BackendError::Closed)
    }

    fn phase(&self) -> SshSessionPhase {
        lock(&self.state).phase
    }

    fn is_writable(&self) -> bool {
        matches!(
            self.phase(),
            SshSessionPhase::Ready | SshSessionPhase::Closing
        )
    }

    // ------------------------------------------------------------------
    // Remote monitoring
    // ------------------------------------------------------------------

    /// Enable or pause backend sampling. Pausing stops new samples; the last
    /// snapshot stays readable so a returning page can render immediately.
    fn set_monitoring(&self, enabled: bool) -> Result<(), BackendError> {
        if self.phase().is_terminal() {
            return Err(BackendError::Closed);
        }
        {
            let mut state = lock(&self.state);
            state.monitor.enabled = enabled;
            if enabled {
                if state.monitor.status != MonitorStatus::Ready {
                    state.monitor.status = MonitorStatus::Waiting;
                }
                state.monitor.failure = None;
                state.monitor.failure_kind = MonitorFailureKind::None;
            } else {
                state.monitor.status = MonitorStatus::Disabled;
            }
        }
        if lock(&self.commands).is_none() {
            return Ok(());
        }
        self.send_command(SessionCommand::SetMonitoringEnabled(enabled))
    }

    fn monitor_snapshot(&self) -> SshMonitorSnapshot {
        let state = lock(&self.state);
        SshMonitorSnapshot {
            enabled: state.monitor.enabled,
            status: state.monitor.status,
            failure_kind: state.monitor.failure_kind,
            failure: state.monitor.failure.clone(),
            metrics: state.monitor.metrics.clone(),
            sample_sequence: state.monitor.sample_sequence,
            updated_at_ms: state.monitor.updated_at_ms,
        }
    }

    // ------------------------------------------------------------------
    // Terminal actions
    // ------------------------------------------------------------------

    fn write_bytes(&self, bytes: Vec<u8>) -> Result<(), BackendError> {
        if !self.is_writable() {
            return Err(BackendError::SessionNotReady {
                session_detail: format!("session is {:?}", self.phase()),
            });
        }
        if bytes.is_empty() {
            return Ok(());
        }
        self.send_command(SessionCommand::Send(bytes))
    }

    fn handle_key(
        &self,
        key: String,
        key_char: Option<String>,
        control: bool,
        alt: bool,
        shift: bool,
        phase: u32,
    ) -> Result<bool, BackendError> {
        if self.phase().is_terminal() {
            return Err(BackendError::Closed);
        }
        let modes = self.terminal.input_modes();
        let keystroke = TerminalKeyStroke {
            modifiers: TerminalModifiers {
                control,
                alt,
                shift,
                platform: false,
                function: false,
            },
            key,
            key_char,
        };
        let phase = match phase {
            1 => TerminalKeyPhase::Repeat,
            2 => TerminalKeyPhase::Release,
            _ => TerminalKeyPhase::Press,
        };
        let Some(bytes) = encode_terminal_input(
            miaominal_terminal::TerminalKeyEvent::new(&keystroke, phase),
            modes,
        ) else {
            return Ok(false);
        };
        self.write_bytes(bytes)?;
        Ok(true)
    }

    fn paste(&self, text: String) -> Result<(), BackendError> {
        let bracketed = self.terminal.bracketed_paste_enabled();
        self.write_bytes(sanitize_paste(&text, bracketed))
    }

    fn resize(&self, columns: u32, lines: u32) -> Result<(), BackendError> {
        let columns = columns.max(2) as usize;
        let lines = lines.max(1) as usize;
        let mut terminal = self.terminal.clone();
        terminal.resize(columns, lines);
        self.bump_revision();
        self.wake.notify_waiters();
        if self.phase().is_terminal() || lock(&self.commands).is_none() {
            return Ok(());
        }
        let _ = self.send_command(SessionCommand::Resize { columns, lines });
        Ok(())
    }

    fn scroll(&self, lines: i32) {
        let mut terminal = self.terminal.clone();
        terminal.scroll(TerminalScroll::Lines(lines));
        self.bump_revision();
        self.wake.notify_waiters();
    }

    fn scroll_to(&self, display_offset: u32) {
        let mut terminal = self.terminal.clone();
        terminal.scroll_to_display_offset(display_offset as usize);
        self.bump_revision();
        self.wake.notify_waiters();
    }

    fn search(&self, pattern: Option<String>) -> Result<u32, BackendError> {
        let mut terminal = self.terminal.clone();
        let total = match pattern {
            Some(pattern) if !pattern.is_empty() => terminal
                .set_search(&pattern)
                .map_err(|detail| BackendError::InvalidArgument { detail })?,
            _ => {
                terminal.clear_search();
                0
            }
        };
        self.bump_revision();
        self.wake.notify_waiters();
        Ok(total as u32)
    }

    fn next_match(&self) {
        let mut terminal = self.terminal.clone();
        terminal.next_match();
        self.bump_revision();
        self.wake.notify_waiters();
    }

    fn prev_match(&self) {
        let mut terminal = self.terminal.clone();
        terminal.prev_match();
        self.bump_revision();
        self.wake.notify_waiters();
    }

    fn start_selection(&self, line: i32, column: u32, side: u32, kind: TerminalSelectionKindDto) {
        let mut terminal = self.terminal.clone();
        terminal.start_selection_with_kind(
            line,
            column as usize,
            side_from_u32(side),
            kind.into_terminal(),
        );
        self.bump_revision();
        self.wake.notify_waiters();
    }

    fn update_selection(&self, line: i32, column: u32, side: u32) {
        let mut terminal = self.terminal.clone();
        terminal.update_selection(line, column as usize, side_from_u32(side));
        self.bump_revision();
        self.wake.notify_waiters();
    }

    fn clear_selection(&self) {
        let mut terminal = self.terminal.clone();
        terminal.clear_selection();
        self.bump_revision();
        self.wake.notify_waiters();
    }

    fn selection_text(&self) -> Option<String> {
        self.terminal.selection_text()
    }

    fn set_focus(&self, focused: bool) {
        if self.focused.swap(focused, Ordering::SeqCst) != focused {
            self.bump_revision();
            self.wake.notify_waiters();
        }
    }

    /// Apply a new palette to the live terminal. Sessions capture the settings
    /// theme when they are created, so a settings change must push the fresh
    /// theme here instead of waiting for a reconnect.
    fn refresh_terminal_theme(&self, theme: TerminalTheme) {
        if self.terminal.set_theme(theme) {
            self.bump_revision();
            self.wake.notify_waiters();
        }
    }

    // ------------------------------------------------------------------
    // Frames
    // ------------------------------------------------------------------

    fn render_frame(&self, since_frame_version: u64) -> TerminalRenderFrame {
        let focused = self.focused.load(Ordering::SeqCst);
        let generation = self.terminal.generation();
        let closed = {
            let state = lock(&self.state);
            state.phase.is_terminal()
        };

        {
            let cache = lock(&self.frame);
            if let Some(cached) = cache.as_ref()
                && cached.generation == generation
                && cached.focused == focused
            {
                let rows = if since_frame_version == cached.version {
                    Vec::new()
                } else {
                    cached.rows.as_ref().clone()
                };
                return TerminalRenderFrame {
                    frame_version: cached.version,
                    revision: cached.generation,
                    full: since_frame_version != cached.version,
                    columns: cached.columns as u32,
                    lines: cached.lines as u32,
                    cursor_visible: cached.cursor_visible,
                    cursor_line: cached.cursor_line,
                    cursor_column: cached.cursor_column,
                    focused: cached.focused,
                    display_offset: cached.display_offset,
                    history_size: cached.history_size,
                    default_fg: cached.default_fg,
                    default_bg: cached.default_bg,
                    selection: cached.selection,
                    search_total: cached.search_total,
                    search_current: cached.search_current,
                    modes: cached.modes,
                    rows,
                    closed,
                };
            }
        }

        let snapshot = self.terminal.snapshot(focused);
        let lines = snapshot.cells.len().min(MAX_FRAME_LINES);
        let first_rows = (0..lines)
            .map(|row| build_row(row, &snapshot.cells[row]))
            .collect::<Vec<_>>();

        let next_cache;
        let (version, full, rows) = {
            let cache = lock(&self.frame);
            match cache.as_ref() {
                Some(previous)
                    if previous.version == since_frame_version
                        && previous.columns == snapshot.columns
                        && previous.lines == lines =>
                {
                    let changed = diff_rows(&previous.rows, &first_rows);
                    (previous.version + 1, false, changed)
                }
                Some(previous) => (previous.version + 1, true, first_rows.clone()),
                None => (1, true, first_rows.clone()),
            }
        };
        let (search_total, search_current) = self.terminal.search_status();
        next_cache = FrameCache {
            version,
            generation,
            columns: snapshot.columns,
            lines,
            focused,
            rows: Arc::new(first_rows),
            cursor_visible: snapshot.focused_cursor,
            cursor_line: snapshot.cursor.viewport_line,
            cursor_column: snapshot.cursor.column as u32,
            display_offset: snapshot.display_offset as u32,
            history_size: snapshot.history_size as u32,
            default_fg: color_to_argb(snapshot.default_fg),
            default_bg: color_to_argb(snapshot.default_bg),
            selection: selection_dto(&self.terminal),
            search_total: search_total as u32,
            search_current: search_current.map(|index| index as u32),
            modes: modes_dto(&self.terminal),
        };
        let result = TerminalRenderFrame {
            frame_version: next_cache.version,
            revision: next_cache.generation,
            full,
            columns: next_cache.columns as u32,
            lines: next_cache.lines as u32,
            cursor_visible: next_cache.cursor_visible,
            cursor_line: next_cache.cursor_line,
            cursor_column: next_cache.cursor_column,
            focused: next_cache.focused,
            display_offset: next_cache.display_offset,
            history_size: next_cache.history_size,
            default_fg: next_cache.default_fg,
            default_bg: next_cache.default_bg,
            selection: next_cache.selection,
            search_total: next_cache.search_total,
            search_current: next_cache.search_current,
            modes: next_cache.modes,
            rows,
            closed,
        };
        *lock(&self.frame) = Some(next_cache);
        result
    }

    // ------------------------------------------------------------------
    // Authentication responses and lifecycle
    // ------------------------------------------------------------------

    fn respond_host_key(
        &self,
        request_id: u64,
        decision: SshHostKeyDecision,
    ) -> Result<(), BackendError> {
        let is_current = {
            let state = lock(&self.state);
            state.request.as_ref().is_some_and(|request| {
                request.request_id == request_id && request.kind == SshRequestKind::HostKey
            })
        };
        if !is_current {
            return Err(BackendError::RequestExpired { request_id });
        }
        self.set_request(None);
        self.bump_revision();
        self.wake.notify_waiters();
        self.send_command(SessionCommand::HostKeyDecision {
            request_id,
            decision: match decision {
                SshHostKeyDecision::AcceptOnce => HostKeyDecision::AcceptOnce,
                SshHostKeyDecision::AcceptAndSave => HostKeyDecision::AcceptAndSave,
                SshHostKeyDecision::Reject => HostKeyDecision::Reject,
            },
        })
    }

    fn respond_keyboard_interactive(
        &self,
        request_id: u64,
        answers: Vec<String>,
    ) -> Result<(), BackendError> {
        let is_current = {
            let state = lock(&self.state);
            state.request.as_ref().is_some_and(|request| {
                request.request_id == request_id
                    && request.kind == SshRequestKind::KeyboardInteractive
            })
        };
        if !is_current {
            return Err(BackendError::RequestExpired { request_id });
        }
        self.set_request(None);
        self.bump_revision();
        self.wake.notify_waiters();
        self.send_command(SessionCommand::KeyboardInteractiveResponse {
            request_id,
            answers,
        })
    }

    pub(crate) fn request_close(&self) {
        self.close_requested.store(true, Ordering::SeqCst);
        if !self.phase().is_terminal() {
            self.set_phase(SshSessionPhase::Closing, None, None);
        }
        if let Some(commands) = lock(&self.commands).clone() {
            let _ = commands.close();
        }
        self.wake.notify_waiters();
    }

    /// Abort the event task without waiting; used when the registry is dropped.
    fn abort_consumer(&self) {
        if let Some(handle) = lock(&self.consumer).take() {
            handle.abort();
        }
    }

    pub(crate) async fn wait_for_task(&self) {
        let handle = lock(&self.consumer).take();
        if let Some(mut handle) = handle {
            if tokio::time::timeout(DISCONNECT_GRACE, &mut handle)
                .await
                .is_err()
            {
                handle.abort();
            }
        }
        // The event task ends when the SSH worker drops its sender; joining the
        // worker proves no connection task outlives `disconnect`.
        let thread = lock(&self.ssh_thread).take();
        if let Some(thread) = thread {
            let joined = crate::runtime::offload(move || {
                let _ = thread.join();
            });
            let _ = tokio::time::timeout(DISCONNECT_GRACE, joined).await;
        }
    }

    /// Terminal cleanup when the session task exits without an event.
    fn finish_task(&self) {
        if !self.phase().is_terminal() {
            self.mark_closed();
        }
        self.wake.notify_waiters();
    }

    fn snapshot(&self) -> SshSessionSnapshot {
        let state = lock(&self.state);
        SshSessionSnapshot {
            connection_id: self.connection_id,
            host_id: state.host_id.clone(),
            host_label: state.hop_label.clone().unwrap_or_default(),
            phase: state.phase,
            failure: state.failure.clone(),
            hop_label: state.hop_label.clone(),
            request: state.request.clone(),
            columns: self.terminal.columns() as u32,
            lines: self.terminal.screen_lines() as u32,
            revision: state.revision,
            exit_code: state.exit_code,
            exit_signal: state.exit_signal.clone(),
            started_at_ms: state.started_at_ms,
        }
    }

    async fn await_update(
        &self,
        since_revision: u64,
        timeout: Duration,
    ) -> Result<TerminalUpdate, BackendError> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            // notify_waiters does not retain a permit. Create this future
            // before reading the revision so an update before the await is
            // still observed, even if the future has not been polled yet.
            let notified = self.wake.notified();
            let (revision, phase) = {
                let state = lock(&self.state);
                (state.revision, state.phase)
            };
            if revision != since_revision {
                return Ok(TerminalUpdate {
                    revision,
                    phase,
                    idle: false,
                    closed: phase.is_terminal(),
                });
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(TerminalUpdate {
                    revision,
                    phase,
                    idle: true,
                    closed: phase.is_terminal(),
                });
            }
            tokio::select! {
                () = notified => {}
                () = tokio::time::sleep(remaining) => {}
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SshHostKeyDecision {
    AcceptOnce,
    AcceptAndSave,
    Reject,
}

fn side_from_u32(side: u32) -> alacritty_terminal::index::Side {
    if side == 1 {
        alacritty_terminal::index::Side::Right
    } else {
        alacritty_terminal::index::Side::Left
    }
}

fn host_key_request_dto(connection_id: u64, request: &HostKeyRequest) -> SshSessionRequest {
    SshSessionRequest {
        request_id: request.request_id,
        connection_id,
        kind: SshRequestKind::HostKey,
        hop_label: request.hop.label.clone(),
        hop_index: request.hop.index,
        hop_total: request.hop.total,
        host: request.prompt.host.clone(),
        port: request.prompt.port,
        algorithm: request.prompt.algorithm.clone(),
        fingerprint: request.prompt.fingerprint.clone(),
        previous_fingerprint: request.prompt.previous_fingerprint.clone(),
        key_changed: request.prompt.previous_fingerprint.is_some(),
        name: String::new(),
        instructions: String::new(),
        prompts: Vec::new(),
    }
}

fn authentication_request_dto(
    connection_id: u64,
    request: &AuthenticationRequest,
) -> SshSessionRequest {
    SshSessionRequest {
        request_id: request.request_id,
        connection_id,
        kind: SshRequestKind::KeyboardInteractive,
        hop_label: request.hop.label.clone(),
        hop_index: request.hop.index,
        hop_total: request.hop.total,
        host: request.hop.label.clone(),
        port: 0,
        algorithm: String::new(),
        fingerprint: String::new(),
        previous_fingerprint: None,
        key_changed: false,
        name: request.challenge.name.clone(),
        instructions: request.challenge.instructions.clone(),
        prompts: request
            .challenge
            .prompts
            .iter()
            .map(|prompt| SshPromptDto {
                prompt: prompt.prompt.clone(),
                echo: prompt.echo,
            })
            .collect(),
    }
}

fn diff_rows(previous: &[TerminalFrameRow], current: &[TerminalFrameRow]) -> Vec<TerminalFrameRow> {
    let mut changed = Vec::new();
    for (index, row) in current.iter().enumerate() {
        if previous.get(index) != Some(row) {
            changed.push(row.clone());
        }
    }
    changed
}

/// Registry of live sessions owned by one `Backend` handle.
pub(crate) struct SessionRegistry {
    lifecycle: Arc<Mutex<crate::backend::LifecycleState>>,
    events: Arc<EventBus>,
    sessions: Mutex<HashMap<u64, Arc<SshSessionInner>>>,
    next_connection_id: AtomicU64,
}

impl SessionRegistry {
    pub(crate) fn new(
        lifecycle: Arc<Mutex<crate::backend::LifecycleState>>,
        events: Arc<EventBus>,
    ) -> Arc<Self> {
        Arc::new(Self {
            lifecycle,
            events,
            sessions: Mutex::new(HashMap::new()),
            next_connection_id: AtomicU64::new(1),
        })
    }

    pub(crate) fn publish(&self, event: BackendEvent) {
        let state = lock(&self.lifecycle);
        if state.closed {
            return;
        }
        self.events.publish(event);
    }

    pub(crate) fn next_connection_id(&self) -> u64 {
        self.next_connection_id.fetch_add(1, Ordering::SeqCst)
    }

    pub(crate) fn insert(&self, session: Arc<SshSessionInner>) {
        lock(&self.sessions).insert(session.connection_id, session);
    }

    fn start(
        &self,
        connect: impl FnOnce() -> Arc<SshSessionInner>,
    ) -> Result<Arc<SshSessionInner>, BackendError> {
        // Worker creation and registration commit atomically with shutdown.
        let state = lock(&self.lifecycle);
        if state.closed {
            return Err(BackendError::Closed);
        }
        let inner = connect();
        self.insert(Arc::clone(&inner));
        Ok(inner)
    }

    pub(crate) fn get(&self, connection_id: u64) -> Option<Arc<SshSessionInner>> {
        lock(&self.sessions).get(&connection_id).cloned()
    }

    pub(crate) fn remove(&self, connection_id: u64) -> Option<Arc<SshSessionInner>> {
        lock(&self.sessions).remove(&connection_id)
    }

    pub(crate) fn snapshots(&self) -> Vec<SshSessionSnapshot> {
        let sessions = lock(&self.sessions).values().cloned().collect::<Vec<_>>();
        sessions.iter().map(|session| session.snapshot()).collect()
    }

    /// Re-resolve the settings palette once and apply it to every live session;
    /// called after the settings store changes so open terminals follow theme
    /// and seed edits without a reconnect.
    pub(crate) fn refresh_terminal_themes(&self) {
        let theme = TerminalTheme::from_settings();
        let sessions = lock(&self.sessions).values().cloned().collect::<Vec<_>>();
        for session in sessions {
            session.refresh_terminal_theme(theme);
        }
    }

    /// Ask every active session to stop; used by `Backend::shutdown` and by
    /// `restart` through the close watch signal.
    pub(crate) fn request_close_all(&self) {
        let sessions = lock(&self.sessions).values().cloned().collect::<Vec<_>>();
        for session in sessions {
            session.request_close();
        }
    }

    /// Drop every retained session. Aborting the consumer drops the event
    /// receiver, which unblocks the SSH worker's sends; the worker itself was
    /// already cancelled by [`SessionRegistry::request_close_all`].
    pub(crate) fn clear(&self) {
        let sessions = lock(&self.sessions)
            .drain()
            .map(|(_, session)| session)
            .collect::<Vec<_>>();
        for session in sessions {
            session.abort_consumer();
        }
    }
}

/// Running session task: applies SSH events until the session closes.
async fn run_session_task(
    inner: Arc<SshSessionInner>,
    mut events: miaominal_ssh::SessionEventReceiver,
    commands: miaominal_ssh::SessionCommandSender,
    mut close_rx: watch::Receiver<bool>,
) {
    let mut closing = false;
    loop {
        tokio::select! {
            event = events.recv() => {
                match event {
                    Some(event) => {
                        // Parsing and frame-materializing are CPU work; keep the
                        // async workers free while the parse completes.
                        let task_inner = Arc::clone(&inner);
                        let _ = crate::runtime::offload(move || task_inner.apply_event(event)).await;
                    }
                    None => break,
                }
            }
            _ = wait_for_close(&mut close_rx), if !closing => {
                closing = true;
                let _ = commands.close();
            }
        }
    }
    inner.finish_task();
}

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

/// Start one interactive session through the shared connector.
pub(crate) async fn start_session(
    host: Arc<dyn PlatformHost>,
    data: Arc<crate::data::MobileData>,
    registry: Arc<SessionRegistry>,
    close_rx: watch::Receiver<bool>,
    host_id: String,
    columns: u32,
    lines: u32,
) -> Result<Arc<SshSessionInner>, BackendError> {
    let context = {
        let host_id = host_id.clone();
        crate::runtime::offload(move || data.session_context(&host_id)).await?
    };

    if context.vault_locked {
        return Err(BackendError::VaultLocked);
    }

    registry.start(|| {
        let connection_id = registry.next_connection_id();
        let theme = miaominal_terminal::TerminalTheme::from_settings();
        let terminal =
            TerminalState::new_with_theme(columns.max(2) as usize, lines.max(1) as usize, theme);

        let connection = miaominal_ssh::start_session(
            &crate::runtime::handle(),
            context.profile,
            context.all_profiles,
            context.all_proxies,
            context.secrets,
            context.known_hosts,
            columns.max(2) as usize,
            lines.max(1) as usize,
            false,
        );
        let SessionConnection {
            commands,
            events,
            thread,
        } = connection;

        let inner = SshSessionInner::new(
            connection_id,
            host,
            &registry,
            host_id,
            context.host_label,
            terminal,
        );
        let task = crate::runtime::runtime().spawn(run_session_task(
            Arc::clone(&inner),
            events,
            commands.clone(),
            close_rx,
        ));
        inner.attach(commands, task, thread);
        inner
    })
}

/// Kotlin handle for one interactive SSH session.
#[derive(uniffi::Object)]
pub struct SshSession {
    inner: Arc<SshSessionInner>,
}

impl SshSession {
    pub(crate) fn new(inner: Arc<SshSessionInner>) -> Self {
        Self { inner }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl SshSession {
    pub fn connection_id(&self) -> u64 {
        self.inner.connection_id
    }

    /// Consistent view of the phase, pending request, and terminal size.
    pub fn snapshot(&self) -> SshSessionSnapshot {
        self.inner.snapshot()
    }

    /// Render frame: full on first use/resize/stale version, changed rows for a
    /// matching version, empty rows when nothing changed.
    pub fn render_frame(&self, since_frame_version: u64) -> TerminalRenderFrame {
        self.inner.render_frame(since_frame_version)
    }

    /// Raw bytes for the remote PTY (already encoded input, paste, answers).
    pub fn write_bytes(&self, bytes: Vec<u8>) -> Result<(), BackendError> {
        self.inner.write_bytes(bytes)
    }

    /// Encode one key press through the shared terminal input encoder and send
    /// it to the remote PTY. `phase` is 0 press, 1 repeat, 2 release.
    pub fn handle_key(
        &self,
        key: String,
        key_char: Option<String>,
        control: bool,
        alt: bool,
        shift: bool,
        phase: u32,
    ) -> Result<bool, BackendError> {
        self.inner
            .handle_key(key, key_char, control, alt, shift, phase)
    }

    pub fn paste(&self, text: String) -> Result<(), BackendError> {
        self.inner.paste(text)
    }

    pub fn resize(&self, columns: u32, lines: u32) -> Result<(), BackendError> {
        self.inner.resize(columns, lines)
    }

    pub fn scroll(&self, lines: i32) {
        self.inner.scroll(lines);
    }

    pub fn scroll_to(&self, display_offset: u32) {
        self.inner.scroll_to(display_offset);
    }

    /// Set (`Some`) or clear (`None`) the scrollback search pattern.
    pub fn search(&self, pattern: Option<String>) -> Result<u32, BackendError> {
        self.inner.search(pattern)
    }

    pub fn next_match(&self) {
        self.inner.next_match();
    }

    pub fn prev_match(&self) {
        self.inner.prev_match();
    }

    pub fn start_selection(
        &self,
        line: i32,
        column: u32,
        side: u32,
        kind: TerminalSelectionKindDto,
    ) {
        self.inner.start_selection(line, column, side, kind);
    }

    pub fn update_selection(&self, line: i32, column: u32, side: u32) {
        self.inner.update_selection(line, column, side);
    }

    pub fn clear_selection(&self) {
        self.inner.clear_selection();
    }

    pub fn selection_text(&self) -> Option<String> {
        self.inner.selection_text()
    }

    pub fn set_focus(&self, focused: bool) {
        self.inner.set_focus(focused);
    }

    /// Enable or pause remote-monitor sampling. The app pauses sampling when
    /// the monitor page leaves the screen or the app goes to the background.
    pub fn set_monitoring(&self, enabled: bool) -> Result<(), BackendError> {
        self.inner.set_monitoring(enabled)
    }

    /// Latest monitor state; samples replace each other instead of queueing.
    pub fn monitor_snapshot(&self) -> SshMonitorSnapshot {
        self.inner.monitor_snapshot()
    }

    pub fn respond_host_key(
        &self,
        request_id: u64,
        decision: SshHostKeyDecision,
    ) -> Result<(), BackendError> {
        self.inner.respond_host_key(request_id, decision)
    }

    pub fn respond_keyboard_interactive(
        &self,
        request_id: u64,
        answers: Vec<String>,
    ) -> Result<(), BackendError> {
        self.inner.respond_keyboard_interactive(request_id, answers)
    }

    /// Explicit cancellation: cancels pending trust/authentication waits and
    /// asks the SSH session to close. Idempotent.
    pub fn cancel(&self) {
        self.inner.request_close();
    }

    /// Orderly, idempotent close that waits for the session task to finish.
    pub async fn disconnect(&self) -> Result<(), BackendError> {
        self.inner.request_close();
        let inner = Arc::clone(&self.inner);
        crate::runtime::run_task(async move {
            inner.wait_for_task().await;
            Ok(())
        })
        .await?
    }

    pub fn is_closed(&self) -> bool {
        self.inner.phase().is_terminal()
    }

    /// Long-poll until the revision changes, the timeout elapses, or the
    /// session reaches a terminal phase.
    pub async fn await_update(
        &self,
        since_revision: u64,
        timeout_ms: u64,
    ) -> Result<TerminalUpdate, BackendError> {
        let inner = Arc::clone(&self.inner);
        let timeout = Duration::from_millis(timeout_ms.max(1));
        crate::runtime::run_task(async move { inner.await_update(since_revision, timeout).await })
            .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::LifecycleState;
    use crate::test_support::test_host;
    use miaominal_core::forwarding::{HostKeyPrompt, KbiChallenge, KbiPrompt};
    use miaominal_ssh::{SessionChannels, SessionPhaseChange};

    struct Harness {
        inner: Arc<SshSessionInner>,
        commands: tokio::sync::mpsc::UnboundedReceiver<SessionCommand>,
    }

    fn harness(columns: usize, lines: usize) -> Harness {
        let lifecycle = Arc::new(Mutex::new(LifecycleState::default()));
        let registry = SessionRegistry::new(lifecycle, EventBus::new());
        let SessionChannels {
            connection,
            commands,
            events: _events,
        } = miaominal_ssh::session_channels();
        let inner = SshSessionInner::new(
            1,
            test_host(),
            &registry,
            "host-1".to_string(),
            "Host One".to_string(),
            TerminalState::new_with_theme(
                columns,
                lines,
                miaominal_terminal::TerminalTheme::from_settings(),
            ),
        );
        let task = crate::runtime::runtime().spawn(async {});
        inner.attach(connection.commands.clone(), task, None);
        Harness { inner, commands }
    }

    fn hop(label: &str) -> miaominal_ssh::SessionHop {
        miaominal_ssh::SessionHop::new(label, 1, 1)
    }

    #[test]
    fn shutdown_rejects_late_session_creation() {
        let lifecycle = Arc::new(Mutex::new(LifecycleState::default()));
        let registry = SessionRegistry::new(Arc::clone(&lifecycle), EventBus::new());
        lock(&lifecycle).closed = true;
        assert!(matches!(
            registry.start(|| panic!("must not create a worker after shutdown")),
            Err(BackendError::Closed)
        ));
        assert!(registry.snapshots().is_empty());
    }

    #[test]
    fn close_before_attach_is_delivered_to_worker() {
        let harness = harness(80, 24);
        lock(&harness.inner.commands).take();
        harness.inner.request_close();
        let mut channels = miaominal_ssh::session_channels();
        harness.inner.attach(
            channels.connection.commands,
            crate::runtime::runtime().spawn(async {}),
            None,
        );
        assert!(matches!(
            channels.commands.try_recv(),
            Ok(SessionCommand::Close)
        ));
    }

    fn phase_event(phase: SessionPhase) -> SessionEvent {
        SessionEvent::Phase(SessionPhaseChange {
            phase,
            hop: Some(hop("Target")),
        })
    }

    fn host_key_request(request_id: u64) -> HostKeyRequest {
        HostKeyRequest {
            request_id,
            hop: hop("Jump"),
            prompt: HostKeyPrompt {
                host: "example.test".to_string(),
                port: 22,
                algorithm: "ssh-ed25519".to_string(),
                fingerprint: "SHA256:abc".to_string(),
                previous_fingerprint: Some("SHA256:old".to_string()),
            },
        }
    }

    fn authentication_request(request_id: u64) -> AuthenticationRequest {
        AuthenticationRequest {
            request_id,
            hop: hop("Jump"),
            challenge: KbiChallenge {
                name: "Two-factor".to_string(),
                instructions: "Enter the code".to_string(),
                prompts: vec![
                    KbiPrompt {
                        prompt: "Password:".to_string(),
                        echo: false,
                    },
                    KbiPrompt {
                        prompt: "OTP:".to_string(),
                        echo: true,
                    },
                ],
            },
        }
    }

    fn settle(inner: &SshSessionInner) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while inner.terminal.has_pending_input() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(
            !inner.terminal.has_pending_input(),
            "terminal parser timed out"
        );
    }

    #[test]
    fn phases_follow_the_state_machine_and_ready_gates_writes() {
        let harness = harness(80, 24);
        let inner = &harness.inner;

        inner.apply_event(phase_event(SessionPhase::WaitingForAuthentication));
        assert_eq!(inner.phase(), SshSessionPhase::WaitingForAuthentication);
        assert!(inner.write_bytes(b"x".to_vec()).is_err());

        inner.apply_event(phase_event(SessionPhase::OpeningShell));
        assert_eq!(inner.phase(), SshSessionPhase::OpeningShell);
        assert!(inner.write_bytes(b"x".to_vec()).is_err());

        inner.apply_event(phase_event(SessionPhase::Ready));
        assert_eq!(inner.phase(), SshSessionPhase::Ready);
        inner
            .write_bytes(b"x".to_vec())
            .expect("ready sessions accept input");

        inner.apply_event(phase_event(SessionPhase::Closing));
        assert_eq!(inner.phase(), SshSessionPhase::Closing);

        inner.apply_event(phase_event(SessionPhase::Closed));
        assert_eq!(inner.phase(), SshSessionPhase::Closed);
        assert!(inner.write_bytes(b"x".to_vec()).is_err());
        // Terminal phases never move backwards.
        inner.apply_event(phase_event(SessionPhase::Ready));
        assert_eq!(inner.phase(), SshSessionPhase::Closed);
    }

    #[test]
    fn theme_refresh_rethemes_live_sessions_without_reconnect() {
        let harness = harness(4, 2);
        let inner = &harness.inner;
        let first = inner.render_frame(0);
        let initial = TerminalTheme::from_settings();
        let next = TerminalTheme {
            default_fg: initial.default_fg ^ 0x00ff_ff,
            default_bg: initial.default_bg ^ 0x00ff_ff,
            ..initial
        };

        inner.refresh_terminal_theme(next);
        let refreshed = inner.render_frame(first.frame_version);
        // Frame colors carry an opaque alpha byte; theme colors are RGB hex.
        assert_eq!(
            refreshed.default_fg & 0x00ff_ffff,
            next.default_fg & 0x00ff_ffff
        );
        assert_eq!(
            refreshed.default_bg & 0x00ff_ffff,
            next.default_bg & 0x00ff_ffff
        );

        // An unchanged theme must not bump the revision or wake the frame pump.
        let revision = inner.snapshot().revision;
        inner.refresh_terminal_theme(next);
        assert_eq!(inner.snapshot().revision, revision);
    }

    #[tokio::test]
    async fn host_key_answers_are_bound_to_the_request_id() {
        let mut harness = harness(80, 24);
        let inner = &harness.inner;

        inner.apply_event(SessionEvent::HostKeyPrompt(host_key_request(7)));
        let snapshot = inner.snapshot();
        assert_eq!(snapshot.phase, SshSessionPhase::WaitingForTrust);
        let request = snapshot.request.expect("pending request");
        assert_eq!(request.request_id, 7);
        assert_eq!(request.kind, SshRequestKind::HostKey);
        assert_eq!(request.hop_label, "Jump");
        assert!(request.key_changed);

        let stale = inner.respond_host_key(6, SshHostKeyDecision::AcceptOnce);
        assert!(matches!(
            stale,
            Err(BackendError::RequestExpired { request_id: 6 })
        ));

        inner
            .respond_host_key(7, SshHostKeyDecision::AcceptAndSave)
            .expect("current request should be accepted");
        assert!(inner.snapshot().request.is_none());

        match harness.commands.recv().await {
            Some(SessionCommand::HostKeyDecision {
                request_id,
                decision,
            }) => {
                assert_eq!(request_id, 7);
                assert!(matches!(decision, HostKeyDecision::AcceptAndSave));
            }
            other => panic!("expected host key decision, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn keyboard_interactive_answers_are_bound_to_the_request_id() {
        let mut harness = harness(80, 24);
        let inner = &harness.inner;

        inner.apply_event(phase_event(SessionPhase::WaitingForAuthentication));
        inner.apply_event(SessionEvent::KeyboardInteractivePrompt(
            authentication_request(11),
        ));
        let request = inner.snapshot().request.expect("pending request");
        assert_eq!(request.kind, SshRequestKind::KeyboardInteractive);
        assert_eq!(request.prompts.len(), 2);
        assert_eq!(request.prompts[0].prompt, "Password:");
        assert!(!request.prompts[0].echo);

        let stale = inner.respond_keyboard_interactive(10, vec!["x".into()]);
        assert!(matches!(
            stale,
            Err(BackendError::RequestExpired { request_id: 10 })
        ));

        inner
            .respond_keyboard_interactive(11, vec!["secret".into(), "123456".into()])
            .expect("current request should be accepted");
        match harness.commands.recv().await {
            Some(SessionCommand::KeyboardInteractiveResponse {
                request_id,
                answers,
            }) => {
                assert_eq!(request_id, 11);
                assert_eq!(answers, vec!["secret".to_string(), "123456".to_string()]);
            }
            other => panic!("expected keyboard-interactive response, got {other:?}"),
        }

        // A late expiry event for another request must not clear newer state.
        inner.apply_event(SessionEvent::KeyboardInteractivePrompt(
            authentication_request(12),
        ));
        inner.apply_event(SessionEvent::RequestExpired { request_id: 11 });
        assert_eq!(inner.snapshot().request.expect("request").request_id, 12);
    }

    #[test]
    fn disconnect_is_idempotent_and_leaves_no_task() {
        let harness = harness(80, 24);
        let inner = &harness.inner;

        inner.apply_event(phase_event(SessionPhase::Ready));
        inner.request_close();
        inner.request_close();
        crate::runtime::runtime().block_on(inner.wait_for_task());
        // A second wait finds no task and returns immediately.
        crate::runtime::runtime().block_on(inner.wait_for_task());
        inner.finish_task();
        assert_eq!(inner.phase(), SshSessionPhase::Closed);
    }

    #[test]
    fn cancellation_marks_an_unfinished_session_closed() {
        let harness = harness(80, 24);
        harness.inner.request_close();
        crate::runtime::runtime().block_on(harness.inner.wait_for_task());
        harness.inner.finish_task();
        let snapshot = harness.inner.snapshot();
        assert_eq!(snapshot.phase, SshSessionPhase::Closed);
    }

    #[test]
    fn full_frames_and_changed_rows_reconstruct_the_same_grid() {
        let harness = harness(20, 4);
        let inner = &harness.inner;

        inner.apply_event(SessionEvent::Output(b"hello\r\nworld".to_vec()));
        settle(inner);
        let first = inner.render_frame(0);
        assert!(first.full);
        assert_eq!(first.lines, 4);
        assert_eq!(first.rows.len(), 4);

        inner.apply_event(SessionEvent::Output(b"!!\r\nsecond line".to_vec()));
        settle(inner);
        let delta = inner.render_frame(first.frame_version);
        assert!(!delta.full);
        assert!(!delta.rows.is_empty());
        assert!(delta.rows.len() < first.rows.len());

        let merged = merge_rows(&first.rows, &delta.rows);
        let fresh = inner.render_frame(0);
        assert!(fresh.full);
        assert_eq!(merged, fresh.rows);

        let idle = inner.render_frame(fresh.frame_version);
        assert!(!idle.full);
        assert!(idle.rows.is_empty());
        assert_eq!(idle.frame_version, fresh.frame_version);
    }

    fn merge_rows(base: &[TerminalFrameRow], delta: &[TerminalFrameRow]) -> Vec<TerminalFrameRow> {
        let mut merged = base.to_vec();
        for row in delta {
            merged[row.row as usize] = row.clone();
        }
        merged
    }

    #[test]
    fn resize_requests_a_full_frame_and_updates_geometry() {
        let harness = harness(20, 4);
        let inner = &harness.inner;

        let first = inner.render_frame(0);
        inner.resize(30, 6).expect("resize");
        let resized = inner.render_frame(first.frame_version);
        assert!(resized.full);
        assert_eq!(resized.columns, 30);
        assert_eq!(resized.lines, 6);
        let snapshot = inner.snapshot();
        assert_eq!(snapshot.columns, 30);
        assert_eq!(snapshot.lines, 6);
    }

    #[test]
    fn scroll_and_search_update_the_frame_metadata() {
        let harness = harness(20, 2);
        let inner = &harness.inner;

        inner.apply_event(SessionEvent::Output(
            b"one\r\ntwo\r\nthree\r\nfour".to_vec(),
        ));
        settle(inner);

        let total = inner.search(Some("t".to_string())).expect("search");
        assert!(total >= 2);
        inner.next_match();
        let frame = inner.render_frame(0);
        assert_eq!(frame.search_total, total);
        assert!(frame.search_current.is_some());

        inner.scroll(1);
        let frame = inner.render_frame(0);
        assert!(frame.display_offset > 0);

        inner.scroll_to(0);
        let frame = inner.render_frame(0);
        assert_eq!(frame.display_offset, 0);

        assert_eq!(inner.search(None).expect("clear search"), 0);
    }

    #[test]
    fn selection_round_trips_through_the_terminal() {
        let harness = harness(20, 4);
        let inner = &harness.inner;

        inner.apply_event(SessionEvent::Output(b"hello world".to_vec()));
        settle(inner);

        inner.start_selection(0, 0, 0, TerminalSelectionKindDto::Simple);
        inner.update_selection(0, 5, 1);
        // Side::Right includes the pointed column, so the trailing space is part
        // of the selection; this matches the desktop behavior.
        assert_eq!(inner.selection_text().as_deref(), Some("hello "));
        let frame = inner.render_frame(0);
        let selection = frame.selection.expect("selection in frame");
        assert_eq!(selection.start_line, 0);
        assert_eq!(selection.start_column, 0);
        assert_eq!(selection.end_column, 5);

        inner.clear_selection();
        assert!(inner.selection_text().is_none());
    }

    #[test]
    fn await_update_wakes_on_output_and_times_out_when_idle() {
        let harness = harness(20, 4);
        let inner = &harness.inner;
        let baseline = inner.snapshot().revision;

        let inner2 = Arc::clone(inner);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            inner2.apply_event(SessionEvent::Output(b"ping".to_vec()));
        });

        let update = crate::runtime::runtime()
            .block_on(inner.await_update(baseline, Duration::from_secs(2)))
            .expect("await_update");
        assert!(!update.idle);
        assert!(update.revision > baseline);

        let update = crate::runtime::runtime()
            .block_on(inner.await_update(update.revision, Duration::from_millis(30)))
            .expect("await_update");
        assert!(update.idle);
    }

    #[test]
    fn session_failure_is_terminal_and_snapshots_report_it() {
        let harness = harness(80, 24);
        let inner = &harness.inner;

        inner.apply_event(SessionEvent::Error("authentication failed".to_string()));
        let snapshot = inner.snapshot();
        assert_eq!(snapshot.phase, SshSessionPhase::Failed);
        assert_eq!(snapshot.failure.as_deref(), Some("authentication failed"));

        inner.apply_event(SessionEvent::Closed);
        assert_eq!(inner.snapshot().phase, SshSessionPhase::Failed);
    }

    #[test]
    fn registry_keeps_sessions_until_explicitly_removed() {
        let lifecycle = Arc::new(Mutex::new(LifecycleState::default()));
        let registry = SessionRegistry::new(lifecycle, EventBus::new());
        let inner = SshSessionInner::new(
            42,
            test_host(),
            &registry,
            "host-42".to_string(),
            "Host 42".to_string(),
            TerminalState::new_with_theme(
                40,
                10,
                miaominal_terminal::TerminalTheme::from_settings(),
            ),
        );
        registry.insert(Arc::clone(&inner));

        assert_eq!(registry.snapshots().len(), 1);
        assert!(registry.get(42).is_some());
        assert_eq!(registry.remove(42).map(|s| s.connection_id), Some(42));
        assert_eq!(registry.remove(42).map(|s| s.connection_id), None);
        assert!(registry.snapshots().is_empty());
    }

    #[test]
    fn dropping_the_registry_entry_releases_the_session_and_terminal() {
        let lifecycle = Arc::new(Mutex::new(LifecycleState::default()));
        let registry = SessionRegistry::new(lifecycle, EventBus::new());
        let inner = SshSessionInner::new(
            43,
            test_host(),
            &registry,
            "host-43".to_string(),
            "Host 43".to_string(),
            TerminalState::new_with_theme(
                40,
                10,
                miaominal_terminal::TerminalTheme::from_settings(),
            ),
        );
        let weak = Arc::downgrade(&inner);
        registry.insert(Arc::clone(&inner));
        drop(inner);

        // The registry owns one strong reference while the entry exists.
        assert!(weak.upgrade().is_some());
        registry.remove(43);
        assert!(
            weak.upgrade().is_none(),
            "the registry must not keep a session alive through a reference cycle"
        );
    }

    #[test]
    fn clear_drops_every_retained_session() {
        let lifecycle = Arc::new(Mutex::new(LifecycleState::default()));
        let registry = SessionRegistry::new(lifecycle, EventBus::new());
        let first = SshSessionInner::new(
            44,
            test_host(),
            &registry,
            "host-44".to_string(),
            "Host 44".to_string(),
            TerminalState::new_with_theme(
                40,
                10,
                miaominal_terminal::TerminalTheme::from_settings(),
            ),
        );
        let second = SshSessionInner::new(
            45,
            test_host(),
            &registry,
            "host-45".to_string(),
            "Host 45".to_string(),
            TerminalState::new_with_theme(
                40,
                10,
                miaominal_terminal::TerminalTheme::from_settings(),
            ),
        );
        let first_weak = Arc::downgrade(&first);
        let second_weak = Arc::downgrade(&second);
        registry.insert(Arc::clone(&first));
        registry.insert(Arc::clone(&second));
        drop(first);
        drop(second);

        registry.clear();

        assert!(registry.snapshots().is_empty());
        assert!(first_weak.upgrade().is_none());
        assert!(second_weak.upgrade().is_none());
    }

    #[test]
    fn closed_registry_does_not_publish_events() {
        let lifecycle = Arc::new(Mutex::new(LifecycleState::default()));
        let registry = SessionRegistry::new(Arc::clone(&lifecycle), EventBus::new());
        {
            let mut state = lock(&lifecycle);
            state.closed = true;
        }
        // Publishing after close must not panic and must leave the bus at zero.
        registry.publish(BackendEvent::Ready);
        let bus_sequence = {
            let state = lock(&lifecycle);
            state.closed
        };
        assert!(bus_sequence);
    }

    fn monitor_sample(cpu_percent: f64) -> miaominal_ssh::SessionMonitorSnapshot {
        miaominal_ssh::SessionMonitorSnapshot {
            platform: miaominal_core::forwarding::SessionMonitorPlatform::Linux,
            hostname: Some("demo".to_string()),
            logical_cpu_count: Some(4),
            uptime_seconds: Some(60),
            cpu_percent,
            memory_percent: 14.0,
            memory_used_bytes: 1024,
            memory_total_bytes: 4096,
            swap_percent: 0.0,
            swap_used_bytes: 0,
            swap_total_bytes: 0,
            disk_percent: 50.0,
            disk_used_bytes: Some(10),
            disk_total_bytes: Some(20),
            network_rx_kbps: 1.0,
            network_tx_kbps: 2.0,
            load: 0.5,
        }
    }

    #[test]
    fn monitor_samples_replace_each_other_and_only_while_enabled() {
        let harness = harness(20, 4);
        let inner = &harness.inner;

        assert_eq!(inner.monitor_snapshot().status, MonitorStatus::Disabled);
        inner.apply_event(SessionEvent::MonitorUpdated(monitor_sample(1.0)));
        assert!(
            inner.monitor_snapshot().metrics.is_none(),
            "a disabled monitor must ignore samples"
        );

        inner.set_monitoring(true).expect("enable monitoring");
        inner.apply_event(SessionEvent::MonitorUpdated(monitor_sample(11.0)));
        inner.apply_event(SessionEvent::MonitorUpdated(monitor_sample(22.0)));
        let snapshot = inner.monitor_snapshot();
        assert!(snapshot.enabled);
        assert_eq!(snapshot.status, MonitorStatus::Ready);
        assert_eq!(
            snapshot.sample_sequence, 2,
            "samples merge instead of queueing"
        );
        assert_eq!(
            snapshot.metrics.as_ref().map(|metrics| metrics.cpu_percent),
            Some(22.0)
        );
        assert_eq!(
            snapshot
                .metrics
                .as_ref()
                .map(|metrics| metrics.platform.as_str()),
            Some("Linux")
        );

        inner.set_monitoring(false).expect("disable monitoring");
        inner.apply_event(SessionEvent::MonitorUpdated(monitor_sample(33.0)));
        let snapshot = inner.monitor_snapshot();
        assert!(!snapshot.enabled);
        assert_eq!(snapshot.status, MonitorStatus::Disabled);
        assert_eq!(snapshot.sample_sequence, 2);
    }

    #[test]
    fn monitor_failures_are_typed_instead_of_strings() {
        let harness = harness(20, 4);
        let inner = &harness.inner;
        inner.set_monitoring(true).expect("enable monitoring");

        inner.apply_event(SessionEvent::MonitorFailed(
            "unsupported remote OS for monitoring: Plan9".to_string(),
        ));
        let snapshot = inner.monitor_snapshot();
        assert_eq!(snapshot.status, MonitorStatus::Failed);
        assert_eq!(
            snapshot.failure_kind,
            MonitorFailureKind::UnsupportedPlatform
        );
        assert!(snapshot.failure.is_some());

        inner.apply_event(SessionEvent::MonitorFailed(
            "remote monitoring command failed: missing counters".to_string(),
        ));
        assert_eq!(
            inner.monitor_snapshot().failure_kind,
            MonitorFailureKind::CommandFailed
        );

        // A later sample clears the failure.
        inner.apply_event(SessionEvent::MonitorUpdated(monitor_sample(5.0)));
        let snapshot = inner.monitor_snapshot();
        assert_eq!(snapshot.status, MonitorStatus::Ready);
        assert_eq!(snapshot.failure_kind, MonitorFailureKind::None);
        assert!(snapshot.failure.is_none());
    }

    #[test]
    fn a_terminal_session_rejects_monitor_commands() {
        let harness = harness(20, 4);
        let inner = &harness.inner;
        inner.apply_event(SessionEvent::Error("connection lost".to_string()));
        assert!(matches!(
            inner.set_monitoring(true),
            Err(BackendError::Closed)
        ));
    }
}
