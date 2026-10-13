use crate::ui::contracts::{PaneId, SplitAxis, TabId};
use crate::ui::features::session::{
    TerminalHoveredLink, TerminalLinkQuery, TerminalMouseGesture, TerminalScrollbarDrag,
};
use gpui_kit::{Bounds, FocusHandle, Pixels, Point};
use miaominal_terminal::{terminal_cell_width_default, terminal_line_height_default};
use std::collections::HashSet;
use std::time::{Duration, Instant};
#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct PaneSplitDragMarker {
    pub path: Vec<usize>,
    pub child_index: usize,
    pub axis: SplitAxis,
}

impl gpui_kit::Render for PaneSplitDragMarker {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        _cx: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::Styled;
        // Invisible 1x1 ghost so the cursor styling drives the visual feedback.
        gpui_kit::div().size(gpui_kit::px(1.0))
    }
}

#[derive(Clone)]
pub(crate) struct PaneSplitDragState {
    pub path: Vec<usize>,
    pub child_index: usize,
    pub axis: SplitAxis,
    pub initial_pointer: f32,
    pub initial_flex_a: f32,
    pub initial_flex_b: f32,
    pub container_size: f32,
}

#[derive(Clone, Debug)]
pub(crate) enum PaneSplitAnimationKind {
    Opening,
    Closing,
}

#[derive(Clone, Debug)]
pub(crate) struct PaneCloseAnimation {
    pub removed_pane_id: PaneId,
    pub hidden_tab_id: Option<TabId>,
}

#[derive(Clone, Debug)]
pub(crate) struct PaneSplitAnimation {
    pub kind: PaneSplitAnimationKind,
    pub path: Vec<usize>,
    pub child_index: usize,
    pub new_child_index: usize,
    pub axis: SplitAxis,
    pub from_flex_a: f32,
    pub from_flex_b: f32,
    pub to_flex_a: f32,
    pub to_flex_b: f32,
    pub started_at: Instant,
    pub duration: Duration,
    pub pending_close: Option<PaneCloseAnimation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaneTabDropZone {
    Center,
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PaneTabDropTarget {
    pub pane_id: PaneId,
    pub zone: PaneTabDropZone,
}

pub(crate) struct ParkedPane {
    pub active_tab: Option<TabId>,
    pub terminal_focus: FocusHandle,
    pub terminal_bounds: Option<Bounds<Pixels>>,
    pub terminal_cell_width: f32,
    pub terminal_line_height: f32,
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

impl ParkedPane {
    #[allow(dead_code)]
    pub fn empty(focus: FocusHandle) -> Self {
        Self {
            active_tab: None,
            terminal_focus: focus,
            terminal_bounds: None,
            terminal_cell_width: terminal_cell_width_default(),
            terminal_line_height: terminal_line_height_default(),
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
