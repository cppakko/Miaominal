//! Retained Session presentation/input state. Workspace stores it by stable PaneId.
//! Same-window parking retains it; moving to another window resets pointer,
//! held-key, focus and measured geometry state while live terminal resources travel.
use super::{TerminalKeyboardState, TerminalMouseReportingState};
use crate::ui::contracts::{PaneId, TabId};
use gpui_kit::{Bounds, FocusHandle, Pixels, Point};
use miaominal_terminal::{
    TerminalFreeTypeTarget, terminal_cell_width_default, terminal_line_height_default,
};
use std::{sync::Arc, time::Instant};

#[derive(Clone, Copy)]
pub(crate) struct TerminalScrollbarDrag {
    pub thumb_grab_offset: f32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TerminalMouseGesture {
    TraditionalSelection,
    FreeTypePendingCursor {
        origin: Point<Pixels>,
        target: TerminalFreeTypeTarget,
    },
    FreeTypeSelection,
    FreeTypePendingDrop {
        origin: Point<Pixels>,
    },
    FreeTypeDrop {
        target: Option<TerminalFreeTypeTarget>,
    },
}

impl TerminalMouseGesture {
    pub(crate) fn is_free_type(self) -> bool {
        !matches!(self, Self::TraditionalSelection)
    }

    pub(crate) fn drop_target(self) -> Option<TerminalFreeTypeTarget> {
        match self {
            Self::FreeTypeDrop { target } => target,
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TerminalPaneFreeTypeDropTarget {
    pub pane_id: PaneId,
    pub tab_id: TabId,
    pub target: TerminalFreeTypeTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TerminalHoveredLink {
    pub tab_id: TabId,
    pub line: usize,
    pub start_column: usize,
    pub end_column: usize,
    pub uri: Arc<str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TerminalLinkQuery {
    pub tab_id: TabId,
    pub generation: u64,
    pub line: usize,
    pub column: usize,
}

pub(crate) struct PaneViewState {
    pub terminal_focus: FocusHandle,
    pub terminal_bounds: Option<Bounds<Pixels>>,
    pub terminal_cell_width: f32,
    pub terminal_line_height: f32,
    pub terminal_ime_anchor: Option<(TabId, Bounds<Pixels>)>,
    pub terminal_mouse_gesture: Option<TerminalMouseGesture>,
    pub terminal_keyboard: TerminalKeyboardState,
    pub terminal_mouse_reporting: TerminalMouseReportingState,
    pub terminal_pointer_target: Option<TabId>,
    pub terminal_pointer_position: Option<Point<Pixels>>,
    pub terminal_link_query: Option<TerminalLinkQuery>,
    pub terminal_hovered_link: Option<TerminalHoveredLink>,
    pub terminal_link_open_modifier: bool,
    pub terminal_scrollbar_drag: Option<TerminalScrollbarDrag>,
    pub terminal_scrollbar_last_interaction_at: Option<Instant>,
}

impl PaneViewState {
    #[allow(dead_code)]
    pub fn new(focus: FocusHandle) -> Self {
        Self {
            terminal_focus: focus,
            terminal_bounds: None,
            terminal_cell_width: terminal_cell_width_default(),
            terminal_line_height: terminal_line_height_default(),
            terminal_ime_anchor: None,
            terminal_mouse_gesture: None,
            terminal_keyboard: TerminalKeyboardState::default(),
            terminal_mouse_reporting: TerminalMouseReportingState::default(),
            terminal_pointer_target: None,
            terminal_pointer_position: None,
            terminal_link_query: None,
            terminal_hovered_link: None,
            terminal_link_open_modifier: false,
            terminal_scrollbar_drag: None,
            terminal_scrollbar_last_interaction_at: None,
        }
    }
}

impl PaneViewState {
    pub(crate) fn reset_for_window(&mut self, focus: FocusHandle) {
        self.terminal_focus = focus;
        self.terminal_bounds = None;
        self.terminal_ime_anchor = None;
        self.terminal_mouse_gesture = None;
        self.terminal_keyboard.reset();
        self.terminal_mouse_reporting.reset();
        self.terminal_pointer_target = None;
        self.terminal_pointer_position = None;
        self.terminal_link_query = None;
        self.terminal_hovered_link = None;
        self.terminal_link_open_modifier = false;
        self.terminal_scrollbar_drag = None;
        self.terminal_scrollbar_last_interaction_at = None;
    }
}
