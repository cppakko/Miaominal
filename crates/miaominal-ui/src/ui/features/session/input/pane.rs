use crate::ui::contracts::workspace::{PaneId, TabId};
use gpui_kit::{Bounds, FocusHandle, Pixels, Point};
use miaominal_terminal::{
    TerminalFreeTypeTarget, terminal_cell_width_default, terminal_line_height_default,
};
use std::{collections::HashSet, sync::Arc, time::Instant};
#[derive(Clone, Copy)]
pub(in crate::ui) struct TerminalScrollbarDrag {
    pub thumb_grab_offset: f32,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui) enum TerminalMouseGesture {
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
    pub(in crate::ui) fn is_free_type(self) -> bool {
        !matches!(self, Self::TraditionalSelection)
    }

    pub(in crate::ui) fn drop_target(self) -> Option<TerminalFreeTypeTarget> {
        match self {
            Self::FreeTypeDrop { target } => target,
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) struct TerminalPaneFreeTypeDropTarget {
    pub pane_id: PaneId,
    pub target: TerminalFreeTypeTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui) struct TerminalHoveredLink {
    pub tab_id: TabId,
    pub line: usize,
    pub start_column: usize,
    pub end_column: usize,
    pub uri: Arc<str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) struct TerminalLinkQuery {
    pub tab_id: TabId,
    pub generation: u64,
    pub line: usize,
    pub column: usize,
}

pub(in crate::ui) struct PaneViewState {
    pub terminal_focus: FocusHandle,
    pub terminal_bounds: Option<Bounds<Pixels>>,
    pub terminal_cell_width: f32,
    pub terminal_line_height: f32,
    pub terminal_ime_anchor: Option<(TabId, Bounds<Pixels>)>,
    pub terminal_mouse_gesture: Option<TerminalMouseGesture>,
    pub terminal_suppressed_key_releases: HashSet<String>,
    pub terminal_mouse_reporting_active: bool,
    pub last_reported_mouse_cell: Option<(usize, usize)>,
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
            terminal_suppressed_key_releases: HashSet::new(),
            terminal_mouse_reporting_active: false,
            last_reported_mouse_cell: None,
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
    pub(in crate::ui) fn reset_for_window(&mut self, focus: FocusHandle) {
        self.terminal_focus = focus;
        self.terminal_bounds = None;
        self.terminal_ime_anchor = None;
        self.terminal_mouse_gesture = None;
        self.terminal_suppressed_key_releases.clear();
        self.terminal_mouse_reporting_active = false;
        self.last_reported_mouse_cell = None;
        self.terminal_pointer_position = None;
        self.terminal_link_query = None;
        self.terminal_hovered_link = None;
        self.terminal_link_open_modifier = false;
        self.terminal_scrollbar_drag = None;
        self.terminal_scrollbar_last_interaction_at = None;
    }
}
