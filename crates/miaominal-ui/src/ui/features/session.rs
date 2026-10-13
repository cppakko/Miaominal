//! Session owns live terminal resources, presentation and input policy.
mod controller;
pub(in crate::ui) mod input;
mod progress_panel;
pub(in crate::ui) mod view;
pub(in crate::ui) use controller::{
    MonitorChartPoint, PortForwardSessionStart, SessionConnectionState, SessionController,
    SessionControllerArgs, SessionFailureStatus, SessionPortSession, SessionPortSnapshot,
    SessionPurpose, SessionQueryPort, SessionSidePanelView, SessionTabState, SessionTerminalPort,
    SessionTerminalTarget, TerminalLease, TerminalLeaseError, TerminalLeaseGrant,
    TransferredSessionTab, TransferredSessionWorkspaceUi,
};
#[cfg(test)]
pub(in crate::ui) use controller::{SESSION_MONITOR_HISTORY_LIMIT, SessionMonitoringState};
pub(in crate::ui) use input::{
    PaneViewState, TerminalHoveredLink, TerminalLinkQuery, TerminalMouseGesture,
    TerminalPaneFreeTypeDropTarget, TerminalScrollbarDrag,
};
pub(in crate::ui) use input::{terminal_cell_width, terminal_line_height};
pub(in crate::ui) use progress_panel::SessionSftpProgressLayoutState;
pub(in crate::ui) use view::canvas::{
    TerminalCanvasHost, TerminalCanvasPrepaint, TerminalScrollbarMetrics,
    render_terminal_canvas_for_pane, terminal_ime_cursor_bounds, terminal_scrollbar_metrics,
    terminal_scrollbar_offset_for_pointer,
};
