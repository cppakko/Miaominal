mod cursor_blink;
mod gestures;
mod keymap;
mod mouse_reporting;
mod pane;
mod region;
mod routing;
#[cfg(test)]
pub(in crate::ui) use cursor_blink::TERMINAL_CURSOR_BLINK_INTERVAL;
pub(in crate::ui) use cursor_blink::{TerminalCursorBlink, TerminalCursorBlinkTarget};
pub(in crate::ui) use gestures::{
    TerminalDragState, TerminalInputHost, TerminalPanePresentation, WorkspaceTerminalInputExt,
};
pub(in crate::ui) use keymap::{terminal_cell_width, terminal_line_height};
pub(in crate::ui) use mouse_reporting::{TerminalMouseMotion, TerminalMouseReportingState};
pub(in crate::ui) use pane::{
    PaneViewState, TerminalHoveredLink, TerminalLinkQuery, TerminalMouseGesture,
    TerminalScrollbarDrag,
};
pub(in crate::ui) use region::bind_terminal_input_handlers;
pub(in crate::ui) use routing::TerminalKeyboardState;
