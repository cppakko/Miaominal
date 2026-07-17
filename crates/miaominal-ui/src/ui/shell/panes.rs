use super::AppView;
use super::workspace::{
    ClosePlan, PaneLayout, SplitAxis, SplitDirection, TabId, TabPlacement, TabRegistry,
    TabWorkspaceState,
};
use crate::ui::i18n;
use gpui::{Bounds, Context, FocusHandle, Pixels, Point};
use miaominal_terminal::{terminal_cell_width_default, terminal_line_height_default};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
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
    pub hidden_tab_id: Option<TabId>,
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
    pub tab_id: TabId,
    pub line: usize,
    pub start_column: usize,
    pub end_column: usize,
    pub uri: Arc<str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) struct TerminalLinkQuery {
    pub tab_id: TabId,
    pub generation: u64,
    pub line: usize,
    pub column: usize,
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
            terminal_dragging: false,
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

pub(in crate::ui::shell) struct ParkedPane {
    pub active_tab: Option<TabId>,
    pub terminal_focus: FocusHandle,
    pub terminal_bounds: Option<Bounds<Pixels>>,
    pub terminal_cell_width: f32,
    pub terminal_line_height: f32,
    pub terminal_dragging: bool,
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
            terminal_dragging: false,
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

impl AppView {
    pub(in crate::ui::shell) fn active_pane_id(&self) -> PaneId {
        self.workspace.workspace.active_pane_id
    }

    fn park_loaded_workspace(&mut self, cx: &mut Context<Self>) -> TabWorkspaceState {
        TabWorkspaceState {
            active_tab: self.workspace.workspace.active_tab.take(),
            active_pane_id: self.workspace.workspace.active_pane_id,
            active_pane: PaneViewState {
                terminal_focus: std::mem::replace(
                    &mut self.workspace.workspace.active_pane.terminal_focus,
                    cx.focus_handle(),
                ),
                terminal_bounds: self.workspace.workspace.active_pane.terminal_bounds.take(),
                terminal_cell_width: self.workspace.workspace.active_pane.terminal_cell_width,
                terminal_line_height: self.workspace.workspace.active_pane.terminal_line_height,
                terminal_dragging: std::mem::take(
                    &mut self.workspace.workspace.active_pane.terminal_dragging,
                ),
                terminal_mouse_reporting_active: std::mem::take(
                    &mut self
                        .workspace
                        .workspace
                        .active_pane
                        .terminal_mouse_reporting_active,
                ),
                last_reported_mouse_cell: self
                    .workspace
                    .workspace
                    .active_pane
                    .last_reported_mouse_cell
                    .take(),
                terminal_pointer_position: self
                    .workspace
                    .workspace
                    .active_pane
                    .terminal_pointer_position
                    .take(),
                terminal_link_query: self
                    .workspace
                    .workspace
                    .active_pane
                    .terminal_link_query
                    .take(),
                terminal_hovered_link: self
                    .workspace
                    .workspace
                    .active_pane
                    .terminal_hovered_link
                    .take(),
                terminal_link_open_modifier: std::mem::take(
                    &mut self
                        .workspace
                        .workspace
                        .active_pane
                        .terminal_link_open_modifier,
                ),
                terminal_scrollbar_drag: self
                    .workspace
                    .workspace
                    .active_pane
                    .terminal_scrollbar_drag
                    .take(),
                terminal_scrollbar_last_interaction_at: self
                    .workspace
                    .workspace
                    .active_pane
                    .terminal_scrollbar_last_interaction_at
                    .take(),
            },
            parked_panes: std::mem::take(&mut self.workspace.workspace.parked_panes),
            pane_layout: std::mem::replace(
                &mut self.workspace.workspace.pane_layout,
                PaneLayout::Leaf(PaneId(1)),
            ),
            next_pane_id: std::mem::replace(&mut self.workspace.workspace.next_pane_id, 2),
            pane_split_drag: self.workspace.workspace.pane_split_drag.take(),
            pane_split_animation: self.workspace.workspace.pane_split_animation.take(),
            pane_tab_drop_target: self.workspace.workspace.pane_tab_drop_target.take(),
        }
    }

    fn restore_loaded_workspace(&mut self, workspace: TabWorkspaceState) {
        self.workspace.workspace.active_tab = workspace.active_tab;
        self.workspace.workspace.active_pane_id = workspace.active_pane_id;
        self.workspace.workspace.active_pane.terminal_focus = workspace.active_pane.terminal_focus;
        self.workspace.workspace.active_pane.terminal_bounds =
            workspace.active_pane.terminal_bounds;
        self.workspace.workspace.active_pane.terminal_cell_width =
            workspace.active_pane.terminal_cell_width;
        self.workspace.workspace.active_pane.terminal_line_height =
            workspace.active_pane.terminal_line_height;
        self.workspace.workspace.active_pane.terminal_dragging =
            workspace.active_pane.terminal_dragging;
        self.workspace
            .workspace
            .active_pane
            .terminal_mouse_reporting_active =
            workspace.active_pane.terminal_mouse_reporting_active;
        self.workspace
            .workspace
            .active_pane
            .last_reported_mouse_cell = workspace.active_pane.last_reported_mouse_cell;
        self.workspace
            .workspace
            .active_pane
            .terminal_pointer_position = workspace.active_pane.terminal_pointer_position;
        self.workspace.workspace.active_pane.terminal_link_query =
            workspace.active_pane.terminal_link_query;
        self.workspace.workspace.active_pane.terminal_hovered_link =
            workspace.active_pane.terminal_hovered_link;
        self.workspace
            .workspace
            .active_pane
            .terminal_link_open_modifier = workspace.active_pane.terminal_link_open_modifier;
        self.workspace.workspace.active_pane.terminal_scrollbar_drag =
            workspace.active_pane.terminal_scrollbar_drag;
        self.workspace
            .workspace
            .active_pane
            .terminal_scrollbar_last_interaction_at =
            workspace.active_pane.terminal_scrollbar_last_interaction_at;
        self.workspace.workspace.parked_panes = workspace.parked_panes;
        self.workspace.workspace.pane_layout = workspace.pane_layout;
        self.workspace.workspace.next_pane_id = workspace.next_pane_id;
        self.workspace.workspace.pane_split_drag = workspace.pane_split_drag;
        self.workspace.workspace.pane_split_animation = workspace.pane_split_animation;
        self.workspace.workspace.pane_tab_drop_target = workspace.pane_tab_drop_target;
    }

    pub(in crate::ui::shell) fn reset_loaded_workspace(&mut self, cx: &mut Context<Self>) {
        self.restore_loaded_workspace(TabWorkspaceState::new(None, cx.focus_handle()));
    }

    pub(in crate::ui::shell) fn unload_active_topbar_workspace(&mut self, cx: &mut Context<Self>) {
        if let Some(tab_id) = self.workspace.active_topbar_tab
            && self
                .workspace
                .tabs
                .get(tab_id)
                .is_some_and(|tab| tab.is_top_level() && tab.is_session())
        {
            let workspace = self.park_loaded_workspace(cx);
            self.workspace.parked_workspaces.insert(tab_id, workspace);
        } else {
            self.reset_loaded_workspace(cx);
        }
    }

    pub(in crate::ui::shell) fn load_topbar_workspace(
        &mut self,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let tab_id = self.workspace.tabs.id_at(index);
        let workspace = tab_id
            .and_then(|tab_id| self.workspace.parked_workspaces.remove(&tab_id))
            .unwrap_or_else(|| TabWorkspaceState::new(tab_id, cx.focus_handle()));
        self.restore_loaded_workspace(workspace);
    }

    fn workspace_tab_indices_from_parts(
        tabs: &TabRegistry,
        active_tab: Option<TabId>,
        parked_panes: &HashMap<PaneId, ParkedPane>,
    ) -> Vec<usize> {
        let mut indices = Vec::new();
        if let Some(index) = active_tab.and_then(|tab_id| tabs.index_of(tab_id)) {
            indices.push(index);
        }
        indices.extend(
            parked_panes
                .values()
                .filter_map(|parked| parked.active_tab)
                .filter_map(|tab_id| tabs.index_of(tab_id)),
        );
        indices.sort_unstable();
        indices.dedup();
        indices
    }

    fn current_workspace_tab_indices(&self) -> Vec<usize> {
        Self::workspace_tab_indices_from_parts(
            &self.workspace.tabs,
            self.workspace.workspace.active_tab,
            &self.workspace.workspace.parked_panes,
        )
    }

    fn is_valid_pane_drop_source_index(&self, index: usize) -> bool {
        let Some(tab) = self.workspace.tabs.at(index) else {
            return false;
        };

        tab.is_top_level()
            && tab.is_session()
            && self.workspace.active_topbar_tab != self.workspace.tabs.id_at(index)
            && self.owned_tab_indices_for_topbar(index).as_slice() == [index]
    }

    pub(in crate::ui::shell) fn pane_drop_source_index(
        &self,
        source_tab_id: TabId,
    ) -> Option<usize> {
        let index = self
            .workspace
            .tabs
            .iter()
            .position(|tab| tab.id == source_tab_id)?;
        self.is_valid_pane_drop_source_index(index).then_some(index)
    }

    pub(in crate::ui::shell) fn pane_drop_source_ids(&self) -> Vec<TabId> {
        self.workspace
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
        let tab_id = if pane_id == self.workspace.workspace.active_pane_id {
            self.workspace.workspace.active_tab
        } else {
            self.parked_pane(pane_id).and_then(|pane| pane.active_tab)
        }?;
        self.workspace.tabs.index_of(tab_id)
    }

    pub(in crate::ui::shell) fn set_pane_active_tab(
        &mut self,
        pane_id: PaneId,
        tab_index: Option<usize>,
    ) {
        let tab_id = tab_index.and_then(|index| self.workspace.tabs.id_at(index));
        if pane_id == self.workspace.workspace.active_pane_id {
            self.workspace.workspace.active_tab = tab_id;
        } else if let Some(parked) = self.workspace.workspace.parked_panes.get_mut(&pane_id) {
            parked.active_tab = tab_id;
        }
    }

    pub(in crate::ui::shell) fn owned_tab_indices_for_topbar(&self, index: usize) -> Vec<usize> {
        let Some(tab) = self.workspace.tabs.at(index) else {
            return Vec::new();
        };
        if !tab.is_top_level() || !tab.is_session() {
            return vec![index];
        }
        let mut owned = if self.workspace.active_topbar_tab == self.workspace.tabs.id_at(index) {
            self.current_workspace_tab_indices()
        } else {
            self.workspace
                .parked_workspaces
                .get(&tab.id)
                .map(|workspace| {
                    Self::workspace_tab_indices_from_parts(
                        &self.workspace.tabs,
                        workspace.active_tab,
                        &workspace.parked_panes,
                    )
                })
                .unwrap_or_default()
        };
        owned.push(index);
        let owner_tab_id = tab.id;
        owned.extend(self.workspace.tabs.iter().enumerate().filter_map(
            |(candidate_index, candidate)| {
                (candidate.owner() == Some(owner_tab_id)).then_some(candidate_index)
            },
        ));
        owned.sort_unstable();
        owned.dedup();
        owned
    }

    pub(in crate::ui::shell) fn close_plan_for_index(&self, index: usize) -> Option<ClosePlan> {
        let tab = self.workspace.tabs.at(index)?;
        let root = tab.id;
        let tabs = self
            .workspace
            .tabs
            .ids()
            .filter_map(|tab_id| self.workspace.tabs.state(tab_id))
            .collect::<Vec<_>>();
        ClosePlan::from_tabs(root, &tabs)
    }

    pub(in crate::ui::shell) fn nearest_visible_tab(&self, preferred: usize) -> Option<usize> {
        self.workspace
            .tabs
            .iter()
            .enumerate()
            .skip(preferred)
            .find(|(_, tab)| tab.is_top_level())
            .map(|(index, _)| index)
            .or_else(|| {
                self.workspace
                    .tabs
                    .iter()
                    .enumerate()
                    .take(preferred)
                    .rev()
                    .find(|(_, tab)| tab.is_top_level())
                    .map(|(index, _)| index)
            })
    }

    fn retain_registered_tab(slot: &mut Option<TabId>, registered: &HashSet<TabId>) {
        if slot.is_some_and(|tab_id| !registered.contains(&tab_id)) {
            *slot = None;
        }
    }

    fn prune_workspace_tab_references(
        workspace: &mut TabWorkspaceState,
        registered: &HashSet<TabId>,
    ) {
        Self::retain_registered_tab(&mut workspace.active_tab, registered);
        for parked in workspace.parked_panes.values_mut() {
            Self::retain_registered_tab(&mut parked.active_tab, registered);
        }
    }

    pub(in crate::ui::shell) fn prune_closed_tab_references(&mut self) {
        let registered = self.workspace.tabs.ids().collect::<HashSet<_>>();
        Self::retain_registered_tab(&mut self.workspace.active_topbar_tab, &registered);
        Self::retain_registered_tab(&mut self.workspace.workspace.active_tab, &registered);
        Self::retain_registered_tab(&mut self.workspace.renaming_tab, &registered);
        for parked in self.workspace.workspace.parked_panes.values_mut() {
            Self::retain_registered_tab(&mut parked.active_tab, &registered);
        }
        self.workspace
            .parked_workspaces
            .retain(|tab_id, _| registered.contains(tab_id));
        for workspace in self.workspace.parked_workspaces.values_mut() {
            Self::prune_workspace_tab_references(workspace, &registered);
        }
    }

    pub(in crate::ui::shell) fn allocate_pane_id(&mut self) -> PaneId {
        let id = PaneId(self.workspace.workspace.next_pane_id);
        self.workspace.workspace.next_pane_id =
            self.workspace.workspace.next_pane_id.saturating_add(1);
        id
    }

    pub(in crate::ui::shell) fn parked_pane(&self, id: PaneId) -> Option<&ParkedPane> {
        self.workspace.workspace.parked_panes.get(&id)
    }

    #[allow(dead_code)]
    pub(in crate::ui::shell) fn pane_of_tab(&self, tab_id: TabId) -> Option<PaneId> {
        if self.workspace.workspace.active_tab == Some(tab_id) {
            return Some(self.workspace.workspace.active_pane_id);
        }
        for (pane_id, parked) in &self.workspace.workspace.parked_panes {
            if parked.active_tab == Some(tab_id) {
                return Some(*pane_id);
            }
        }
        None
    }

    pub(in crate::ui::shell) fn handle_pane_tab_drop(
        &mut self,
        source_tab_id: TabId,
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
        self.workspace.workspace.pane_tab_drop_target = None;
    }

    fn move_topbar_tab_to_pane_edge(
        &mut self,
        source_tab_id: TabId,
        target_pane_id: PaneId,
        direction: SplitDirection,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(source_index) = self.pane_drop_source_index(source_tab_id) else {
            return;
        };
        if !self
            .workspace
            .workspace
            .pane_layout
            .contains(target_pane_id)
        {
            return;
        }

        let mut moved_tab = self.workspace.tabs.remove(source_index);
        let Some(owner) = self.workspace.active_topbar_tab else {
            self.workspace.tabs.push(moved_tab);
            return;
        };
        let new_pane_id = self.allocate_pane_id();
        moved_tab.placement = TabPlacement::WorkspacePane {
            owner,
            pane: new_pane_id,
        };
        self.workspace.parked_workspaces.remove(&source_tab_id);
        let moved_title = moved_tab.title.clone();
        if let Some(mut session) = self.session_tab_mut(source_tab_id, cx) {
            session.has_activity = false;
        }

        let moved_index = self.workspace.tabs.len();
        self.workspace.tabs.push(moved_tab);

        let split_animation =
            self.workspace
                .workspace
                .pane_layout
                .split(target_pane_id, direction, new_pane_id);
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
            self.workspace.workspace.pane_split_animation = Some(PaneSplitAnimation {
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
        self.workspace
            .workspace
            .parked_panes
            .insert(new_pane_id, new_pane);
        self.set_pane_active_tab(new_pane_id, Some(moved_index));
        self.set_active_pane(new_pane_id, window, cx);
        self.shell.status_message = i18n::string_args(
            "status.workspace.moved_into_split",
            &[("title", moved_title.as_str())],
        );
        cx.notify();
    }

    fn swap_topbar_tab_with_pane(
        &mut self,
        source_tab_id: TabId,
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
            .workspace
            .tabs
            .at(target_index)
            .is_none_or(|tab| !tab.is_session())
        {
            return;
        }

        let source_title = self
            .workspace
            .tabs
            .at(source_index)
            .expect("pane drop source remains registered")
            .title
            .clone();
        let Some(owner) = self.workspace.active_topbar_tab else {
            return;
        };

        let Some(target_tab_id) = self.workspace.tabs.id_at(target_index) else {
            return;
        };
        let Some(swap) = self.workspace.swap_top_level_tab_with_pane(
            source_tab_id,
            target_tab_id,
            owner,
            target_pane_id,
        ) else {
            return;
        };

        self.workspace.parked_workspaces.insert(
            swap.promoted_tab_id,
            TabWorkspaceState::new(Some(swap.promoted_tab_id), cx.focus_handle()),
        );
        self.workspace.parked_workspaces.remove(&swap.moved_tab_id);
        if let Some(mut session) = self.session_tab_mut(swap.moved_tab_id, cx) {
            session.has_activity = false;
        }

        self.set_pane_active_tab(target_pane_id, Some(swap.moved_order_index));

        self.set_active_pane(target_pane_id, window, cx);
        self.shell.status_message = i18n::string_args(
            "status.workspace.moved_into_pane",
            &[("title", source_title.as_str())],
        );
        cx.notify();
    }
}
