use super::AppView;
use super::workspace::{PaneLayout, SplitAxis, SplitDirection, TabWorkspaceState};
use crate::terminal::{terminal_cell_width_default, terminal_line_height_default};
use crate::ui::i18n;
use gpui::{Bounds, Context, FocusHandle, Pixels, Point};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::ui::shell) struct PaneId(pub usize);

impl PaneId {
    pub fn raw(self) -> usize {
        self.0
    }
}

#[derive(Clone)]
#[allow(dead_code)]
pub(in crate::ui::shell) struct PaneSplitDragMarker {
    pub path: Vec<usize>,
    pub child_index: usize,
    pub axis: SplitAxis,
}

impl gpui::Render for PaneSplitDragMarker {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::Styled;
        // Invisible 1x1 ghost so the cursor styling drives the visual feedback.
        gpui::div().size(gpui::px(1.0))
    }
}

#[derive(Clone)]
pub(in crate::ui::shell) struct PaneSplitDragState {
    pub path: Vec<usize>,
    pub child_index: usize,
    pub axis: SplitAxis,
    pub initial_pointer: f32,
    pub initial_flex_a: f32,
    pub initial_flex_b: f32,
    pub container_size: f32,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) enum PaneSplitAnimationKind {
    Opening,
    Closing,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct PaneCloseAnimation {
    pub removed_pane_id: PaneId,
    pub hidden_tab_id: Option<usize>,
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct PaneSplitAnimation {
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

#[derive(Clone, Copy)]
pub(in crate::ui::shell) struct TerminalScrollbarDrag {
    pub thumb_grab_offset: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) struct TerminalHoveredLink {
    pub tab_id: usize,
    pub line: usize,
    pub column: usize,
    pub uri: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum PaneTabDropZone {
    Center,
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) struct PaneTabDropTarget {
    pub pane_id: PaneId,
    pub zone: PaneTabDropZone,
}

pub(in crate::ui::shell) struct PaneViewState {
    pub terminal_focus: FocusHandle,
    pub terminal_bounds: Option<Bounds<Pixels>>,
    pub terminal_cell_width: f32,
    pub terminal_line_height: f32,
    pub terminal_dragging: bool,
    pub terminal_mouse_reporting_active: bool,
    pub last_reported_mouse_cell: Option<(usize, usize)>,
    pub terminal_pointer_position: Option<Point<Pixels>>,
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
            terminal_dragging: false,
            terminal_mouse_reporting_active: false,
            last_reported_mouse_cell: None,
            terminal_pointer_position: None,
            terminal_hovered_link: None,
            terminal_link_open_modifier: false,
            terminal_scrollbar_drag: None,
            terminal_scrollbar_last_interaction_at: None,
        }
    }
}

pub(in crate::ui::shell) struct ParkedPane {
    pub active_tab: Option<usize>,
    pub terminal_focus: FocusHandle,
    pub terminal_bounds: Option<Bounds<Pixels>>,
    pub terminal_cell_width: f32,
    pub terminal_line_height: f32,
    pub terminal_dragging: bool,
    pub terminal_mouse_reporting_active: bool,
    pub last_reported_mouse_cell: Option<(usize, usize)>,
    pub terminal_pointer_position: Option<Point<Pixels>>,
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
            terminal_dragging: false,
            terminal_mouse_reporting_active: false,
            last_reported_mouse_cell: None,
            terminal_pointer_position: None,
            terminal_hovered_link: None,
            terminal_link_open_modifier: false,
            terminal_scrollbar_drag: None,
            terminal_scrollbar_last_interaction_at: None,
        }
    }
}

impl AppView {
    pub(in crate::ui::shell) fn active_pane_id(&self) -> PaneId {
        self.workspace_state.workspace.active_pane_id
    }

    fn park_loaded_workspace(&mut self, cx: &mut Context<Self>) -> TabWorkspaceState {
        TabWorkspaceState {
            active_tab: self.workspace_state.workspace.active_tab.take(),
            active_pane_id: self.workspace_state.workspace.active_pane_id,
            active_pane: PaneViewState {
                terminal_focus: std::mem::replace(
                    &mut self.workspace_state.workspace.active_pane.terminal_focus,
                    cx.focus_handle(),
                ),
                terminal_bounds: self
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_bounds
                    .take(),
                terminal_cell_width: self
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_cell_width,
                terminal_line_height: self
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_line_height,
                terminal_dragging: std::mem::take(
                    &mut self.workspace_state.workspace.active_pane.terminal_dragging,
                ),
                terminal_mouse_reporting_active: std::mem::take(
                    &mut self
                        .workspace_state
                        .workspace
                        .active_pane
                        .terminal_mouse_reporting_active,
                ),
                last_reported_mouse_cell: self
                    .workspace_state
                    .workspace
                    .active_pane
                    .last_reported_mouse_cell
                    .take(),
                terminal_pointer_position: self
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_pointer_position
                    .take(),
                terminal_hovered_link: self
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_hovered_link
                    .take(),
                terminal_link_open_modifier: std::mem::take(
                    &mut self
                        .workspace_state
                        .workspace
                        .active_pane
                        .terminal_link_open_modifier,
                ),
                terminal_scrollbar_drag: self
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_scrollbar_drag
                    .take(),
                terminal_scrollbar_last_interaction_at: self
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_scrollbar_last_interaction_at
                    .take(),
            },
            parked_panes: std::mem::take(&mut self.workspace_state.workspace.parked_panes),
            pane_layout: std::mem::replace(
                &mut self.workspace_state.workspace.pane_layout,
                PaneLayout::Leaf(PaneId(1)),
            ),
            next_pane_id: std::mem::replace(&mut self.workspace_state.workspace.next_pane_id, 2),
            pane_split_drag: self.workspace_state.workspace.pane_split_drag.take(),
            pane_split_animation: self.workspace_state.workspace.pane_split_animation.take(),
            pane_tab_drop_target: self.workspace_state.workspace.pane_tab_drop_target.take(),
        }
    }

    fn restore_loaded_workspace(&mut self, workspace: TabWorkspaceState) {
        self.workspace_state.workspace.active_tab = workspace.active_tab;
        self.workspace_state.workspace.active_pane_id = workspace.active_pane_id;
        self.workspace_state.workspace.active_pane.terminal_focus =
            workspace.active_pane.terminal_focus;
        self.workspace_state.workspace.active_pane.terminal_bounds =
            workspace.active_pane.terminal_bounds;
        self.workspace_state
            .workspace
            .active_pane
            .terminal_cell_width = workspace.active_pane.terminal_cell_width;
        self.workspace_state
            .workspace
            .active_pane
            .terminal_line_height = workspace.active_pane.terminal_line_height;
        self.workspace_state.workspace.active_pane.terminal_dragging =
            workspace.active_pane.terminal_dragging;
        self.workspace_state
            .workspace
            .active_pane
            .terminal_mouse_reporting_active =
            workspace.active_pane.terminal_mouse_reporting_active;
        self.workspace_state
            .workspace
            .active_pane
            .last_reported_mouse_cell = workspace.active_pane.last_reported_mouse_cell;
        self.workspace_state
            .workspace
            .active_pane
            .terminal_pointer_position = workspace.active_pane.terminal_pointer_position;
        self.workspace_state
            .workspace
            .active_pane
            .terminal_hovered_link = workspace.active_pane.terminal_hovered_link;
        self.workspace_state
            .workspace
            .active_pane
            .terminal_link_open_modifier = workspace.active_pane.terminal_link_open_modifier;
        self.workspace_state
            .workspace
            .active_pane
            .terminal_scrollbar_drag = workspace.active_pane.terminal_scrollbar_drag;
        self.workspace_state
            .workspace
            .active_pane
            .terminal_scrollbar_last_interaction_at =
            workspace.active_pane.terminal_scrollbar_last_interaction_at;
        self.workspace_state.workspace.parked_panes = workspace.parked_panes;
        self.workspace_state.workspace.pane_layout = workspace.pane_layout;
        self.workspace_state.workspace.next_pane_id = workspace.next_pane_id;
        self.workspace_state.workspace.pane_split_drag = workspace.pane_split_drag;
        self.workspace_state.workspace.pane_split_animation = workspace.pane_split_animation;
        self.workspace_state.workspace.pane_tab_drop_target = workspace.pane_tab_drop_target;
    }

    pub(in crate::ui::shell) fn reset_loaded_workspace(&mut self, cx: &mut Context<Self>) {
        self.restore_loaded_workspace(TabWorkspaceState::new(None, cx.focus_handle()));
    }

    pub(in crate::ui::shell) fn unload_active_topbar_workspace(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.workspace_state.active_topbar_tab
            && self
                .workspace_state
                .tabs
                .get(index)
                .is_some_and(|tab| !tab.hidden_from_topbar && tab.as_session().is_some())
        {
            let workspace = self.park_loaded_workspace(cx);
            if let Some(tab) = self.workspace_state.tabs.get_mut(index) {
                tab.workspace = Some(workspace);
            }
        } else {
            self.reset_loaded_workspace(cx);
        }
    }

    pub(in crate::ui::shell) fn load_topbar_workspace(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let workspace = self
            .workspace_state
            .tabs
            .get_mut(index)
            .and_then(|tab| tab.workspace.take())
            .unwrap_or_else(|| TabWorkspaceState::new(Some(index), cx.focus_handle()));
        self.restore_loaded_workspace(workspace);
    }

    fn workspace_tab_indices_from_parts(
        active_tab: Option<usize>,
        parked_panes: &HashMap<PaneId, ParkedPane>,
    ) -> Vec<usize> {
        let mut indices = Vec::new();
        if let Some(index) = active_tab {
            indices.push(index);
        }
        indices.extend(parked_panes.values().filter_map(|parked| parked.active_tab));
        indices.sort_unstable();
        indices.dedup();
        indices
    }

    fn current_workspace_tab_indices(&self) -> Vec<usize> {
        Self::workspace_tab_indices_from_parts(
            self.workspace_state.workspace.active_tab,
            &self.workspace_state.workspace.parked_panes,
        )
    }

    fn is_valid_pane_drop_source_index(&self, index: usize) -> bool {
        let Some(tab) = self.workspace_state.tabs.get(index) else {
            return false;
        };

        !tab.hidden_from_topbar
            && tab.as_session().is_some()
            && self.workspace_state.active_topbar_tab != Some(index)
            && self.owned_tab_indices_for_topbar(index).as_slice() == [index]
    }

    pub(in crate::ui::shell) fn pane_drop_source_index(
        &self,
        source_tab_id: usize,
    ) -> Option<usize> {
        let index = self
            .workspace_state
            .tabs
            .iter()
            .position(|tab| tab.id == source_tab_id)?;
        self.is_valid_pane_drop_source_index(index).then_some(index)
    }

    pub(in crate::ui::shell) fn pane_drop_source_ids(&self) -> Vec<usize> {
        self.workspace_state
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| {
                self.is_valid_pane_drop_source_index(index)
                    .then_some(tab.id)
            })
            .collect()
    }

    pub(in crate::ui::shell) fn pane_tab_index(&self, pane_id: PaneId) -> Option<usize> {
        if pane_id == self.workspace_state.workspace.active_pane_id {
            self.workspace_state.workspace.active_tab
        } else {
            self.parked_pane(pane_id).and_then(|pane| pane.active_tab)
        }
    }

    pub(in crate::ui::shell) fn set_pane_active_tab(
        &mut self,
        pane_id: PaneId,
        tab_index: Option<usize>,
    ) {
        if pane_id == self.workspace_state.workspace.active_pane_id {
            self.workspace_state.workspace.active_tab = tab_index;
        } else if let Some(parked) = self
            .workspace_state
            .workspace
            .parked_panes
            .get_mut(&pane_id)
        {
            parked.active_tab = tab_index;
        }
    }

    pub(in crate::ui::shell) fn owned_tab_indices_for_topbar(&self, index: usize) -> Vec<usize> {
        let Some(tab) = self.workspace_state.tabs.get(index) else {
            return Vec::new();
        };
        if tab.hidden_from_topbar || tab.as_session().is_none() {
            return vec![index];
        }
        let mut owned = if self.workspace_state.active_topbar_tab == Some(index) {
            self.current_workspace_tab_indices()
        } else {
            tab.workspace
                .as_ref()
                .map(|workspace| {
                    Self::workspace_tab_indices_from_parts(
                        workspace.active_tab,
                        &workspace.parked_panes,
                    )
                })
                .unwrap_or_default()
        };
        owned.push(index);
        owned.sort_unstable();
        owned.dedup();
        owned
    }

    pub(in crate::ui::shell) fn nearest_visible_tab(&self, preferred: usize) -> Option<usize> {
        self.workspace_state
            .tabs
            .iter()
            .enumerate()
            .skip(preferred)
            .find(|(_, tab)| !tab.hidden_from_topbar)
            .map(|(index, _)| index)
            .or_else(|| {
                self.workspace_state
                    .tabs
                    .iter()
                    .enumerate()
                    .take(preferred)
                    .rev()
                    .find(|(_, tab)| !tab.hidden_from_topbar)
                    .map(|(index, _)| index)
            })
    }

    fn remap_slot_after_removal(slot: &mut Option<usize>, removed: &[usize]) {
        let Some(index) = *slot else {
            return;
        };
        if removed.binary_search(&index).is_ok() {
            *slot = None;
            return;
        }
        let shift = removed
            .iter()
            .take_while(|&&removed_index| removed_index < index)
            .count();
        *slot = Some(index - shift);
    }

    fn remap_workspace_after_removal(workspace: &mut TabWorkspaceState, removed: &[usize]) {
        Self::remap_slot_after_removal(&mut workspace.active_tab, removed);
        for parked in workspace.parked_panes.values_mut() {
            Self::remap_slot_after_removal(&mut parked.active_tab, removed);
        }
    }

    pub(in crate::ui::shell) fn remap_all_tab_indices_after_removal(&mut self, removed: &[usize]) {
        Self::remap_slot_after_removal(&mut self.workspace_state.active_topbar_tab, removed);
        Self::remap_slot_after_removal(&mut self.workspace_state.workspace.active_tab, removed);
        Self::remap_slot_after_removal(&mut self.workspace_state.renaming_tab, removed);
        for parked in self.workspace_state.workspace.parked_panes.values_mut() {
            Self::remap_slot_after_removal(&mut parked.active_tab, removed);
        }
        for tab in &mut self.workspace_state.tabs {
            if let Some(workspace) = tab.workspace.as_mut() {
                Self::remap_workspace_after_removal(workspace, removed);
            }
        }
    }

    fn remap_index_after_move(index: usize, from: usize, dest: usize) -> usize {
        if index == from {
            dest
        } else {
            let mut shifted = index;
            if index > from {
                shifted -= 1;
            }
            if shifted >= dest {
                shifted += 1;
            }
            shifted
        }
    }

    fn remap_slot_after_move(slot: &mut Option<usize>, from: usize, dest: usize) {
        if let Some(index) = *slot {
            *slot = Some(Self::remap_index_after_move(index, from, dest));
        }
    }

    fn remap_workspace_after_move(workspace: &mut TabWorkspaceState, from: usize, dest: usize) {
        Self::remap_slot_after_move(&mut workspace.active_tab, from, dest);
        for parked in workspace.parked_panes.values_mut() {
            Self::remap_slot_after_move(&mut parked.active_tab, from, dest);
        }
    }

    pub(in crate::ui::shell) fn remap_all_tab_indices_after_move(
        &mut self,
        from: usize,
        dest: usize,
    ) {
        Self::remap_slot_after_move(&mut self.workspace_state.active_topbar_tab, from, dest);
        Self::remap_slot_after_move(&mut self.workspace_state.workspace.active_tab, from, dest);
        Self::remap_slot_after_move(&mut self.workspace_state.renaming_tab, from, dest);
        for parked in self.workspace_state.workspace.parked_panes.values_mut() {
            Self::remap_slot_after_move(&mut parked.active_tab, from, dest);
        }
        for tab in &mut self.workspace_state.tabs {
            if let Some(workspace) = tab.workspace.as_mut() {
                Self::remap_workspace_after_move(workspace, from, dest);
            }
        }
    }

    pub(in crate::ui::shell) fn allocate_pane_id(&mut self) -> PaneId {
        let id = PaneId(self.workspace_state.workspace.next_pane_id);
        self.workspace_state.workspace.next_pane_id = self
            .workspace_state
            .workspace
            .next_pane_id
            .saturating_add(1);
        id
    }

    pub(in crate::ui::shell) fn parked_pane(&self, id: PaneId) -> Option<&ParkedPane> {
        self.workspace_state.workspace.parked_panes.get(&id)
    }

    #[allow(dead_code)]
    pub(in crate::ui::shell) fn pane_of_tab(&self, tab_id: usize) -> Option<PaneId> {
        if self
            .workspace_state
            .workspace
            .active_tab
            .and_then(|i| self.workspace_state.tabs.get(i))
            .map(|t| t.id)
            == Some(tab_id)
        {
            return Some(self.workspace_state.workspace.active_pane_id);
        }
        for (pane_id, parked) in &self.workspace_state.workspace.parked_panes {
            if parked
                .active_tab
                .and_then(|i| self.workspace_state.tabs.get(i))
                .map(|t| t.id)
                == Some(tab_id)
            {
                return Some(*pane_id);
            }
        }
        None
    }

    pub(in crate::ui::shell) fn handle_pane_tab_drop(
        &mut self,
        source_tab_id: usize,
        target_pane_id: PaneId,
        zone: PaneTabDropZone,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        match zone {
            PaneTabDropZone::Center => {
                self.swap_topbar_tab_with_pane(source_tab_id, target_pane_id, window, cx)
            }
            PaneTabDropZone::Up => self.move_topbar_tab_to_pane_edge(
                source_tab_id,
                target_pane_id,
                SplitDirection::Up,
                window,
                cx,
            ),
            PaneTabDropZone::Down => self.move_topbar_tab_to_pane_edge(
                source_tab_id,
                target_pane_id,
                SplitDirection::Down,
                window,
                cx,
            ),
            PaneTabDropZone::Left => self.move_topbar_tab_to_pane_edge(
                source_tab_id,
                target_pane_id,
                SplitDirection::Left,
                window,
                cx,
            ),
            PaneTabDropZone::Right => self.move_topbar_tab_to_pane_edge(
                source_tab_id,
                target_pane_id,
                SplitDirection::Right,
                window,
                cx,
            ),
        }
        self.workspace_state.workspace.pane_tab_drop_target = None;
    }

    fn move_topbar_tab_to_pane_edge(
        &mut self,
        source_tab_id: usize,
        target_pane_id: PaneId,
        direction: SplitDirection,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(source_index) = self.pane_drop_source_index(source_tab_id) else {
            return;
        };
        if !self
            .workspace_state
            .workspace
            .pane_layout
            .contains(target_pane_id)
        {
            return;
        }

        let mut moved_tab = self.workspace_state.tabs.remove(source_index);
        self.remap_all_tab_indices_after_removal(&[source_index]);

        moved_tab.hidden_from_topbar = true;
        moved_tab.workspace = None;
        let moved_title = moved_tab.title.clone();
        if let Some(session) = moved_tab.as_session_mut() {
            session.has_activity = false;
        }

        let moved_index = self.workspace_state.tabs.len();
        self.workspace_state.tabs.push(moved_tab);

        let new_pane_id = self.allocate_pane_id();
        let split_animation = self.workspace_state.workspace.pane_layout.split(
            target_pane_id,
            direction,
            new_pane_id,
        );
        if split_animation.is_none() {
            return;
        }
        if let Some(animation) = split_animation {
            let _ = self.set_split_flex_pair(
                &animation.path,
                animation.child_index,
                animation.from_flex_a,
                animation.from_flex_b,
            );
            self.workspace_state.workspace.pane_split_animation = Some(PaneSplitAnimation {
                kind: PaneSplitAnimationKind::Opening,
                path: animation.path,
                child_index: animation.child_index,
                new_child_index: animation.new_child_index,
                axis: animation.axis,
                from_flex_a: animation.from_flex_a,
                from_flex_b: animation.from_flex_b,
                to_flex_a: animation.to_flex_a,
                to_flex_b: animation.to_flex_b,
                started_at: Instant::now(),
                duration: super::support::CONTAINER_TRANSITION_DURATION,
                pending_close: None,
            });
        }

        let new_pane = ParkedPane::empty(cx.focus_handle());
        self.workspace_state
            .workspace
            .parked_panes
            .insert(new_pane_id, new_pane);
        self.set_pane_active_tab(new_pane_id, Some(moved_index));
        self.set_active_pane(new_pane_id, window, cx);
        self.status_message = i18n::string_args(
            "status.workspace.moved_into_split",
            &[("title", moved_title.as_str())],
        );
        cx.notify();
    }

    fn swap_topbar_tab_with_pane(
        &mut self,
        source_tab_id: usize,
        target_pane_id: PaneId,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(source_index) = self.pane_drop_source_index(source_tab_id) else {
            return;
        };
        let Some(target_index) = self.pane_tab_index(target_pane_id) else {
            return;
        };
        if source_index == target_index {
            return;
        }
        if self
            .workspace_state
            .tabs
            .get(target_index)
            .and_then(crate::ui::shell::state::TabState::as_session)
            .is_none()
        {
            return;
        }

        let source_title = self.workspace_state.tabs[source_index].title.clone();
        let target_was_hidden = self.workspace_state.tabs[target_index].hidden_from_topbar;

        self.workspace_state.tabs.swap(source_index, target_index);

        self.workspace_state.tabs[source_index].hidden_from_topbar = false;
        self.workspace_state.tabs[source_index].workspace = Some(TabWorkspaceState::new(
            Some(source_index),
            cx.focus_handle(),
        ));

        self.workspace_state.tabs[target_index].hidden_from_topbar = target_was_hidden;
        self.workspace_state.tabs[target_index].workspace = None;
        if let Some(session) = self.workspace_state.tabs[target_index].as_session_mut() {
            session.has_activity = false;
        }

        self.set_active_pane(target_pane_id, window, cx);
        self.status_message = i18n::string_args(
            "status.workspace.moved_into_pane",
            &[("title", source_title.as_str())],
        );
        cx.notify();
    }
}
