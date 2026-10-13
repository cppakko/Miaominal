//! Session owns live terminal resources, presentation and input policy.
mod controller;
mod input;
mod progress_panel;
mod view;
use controller::{
    MonitorChartPoint, SessionConnectionState, SessionFailureStatus, SessionTabState,
};
pub(in crate::ui) use controller::{
    PortForwardSessionStart, SessionController, SessionControllerArgs,
};
pub(in crate::ui) use input::{
    PaneViewState, TerminalDragState, TerminalHoveredLink, TerminalInputHost,
    TerminalKeyboardState, TerminalLinkQuery, TerminalMouseGesture, TerminalMouseReportingState,
    TerminalPanePresentation, TerminalScrollbarDrag, WorkspaceTerminalInputExt,
    bind_terminal_input_handlers,
};
pub(in crate::ui) use input::{terminal_cell_width, terminal_line_height};
use progress_panel::SessionSftpProgressLayoutState;
pub(in crate::ui) use view::canvas::{
    TerminalCanvasHost, TerminalCanvasPrepaint, render_terminal_canvas_for_pane,
    terminal_ime_cursor_bounds,
};

pub(in crate::ui) use crate::ui::contracts::session::{
    SessionPortSnapshot, SessionPurpose, SessionSidePanelView, TransferredSessionTab,
    TransferredSessionWorkspaceUi,
};
