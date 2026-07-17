use super::panes::{
    PaneId, PaneSplitAnimation, PaneSplitDragState, PaneTabDropTarget, PaneViewState, ParkedPane,
};
use super::{
    ClosedSessionTabState, PrimaryViewKind, PrimaryViewTransition, SessionProfile,
    TopbarActiveTabTransition, TopbarTabEnterTransition, TopbarTabExitTransition,
    TopbarTabSnapshot,
};
#[cfg(test)]
use super::{SessionController, SessionTabState};
use crate::ui::i18n;
use gpui::{FocusHandle, ScrollHandle};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    ops::{Deref, DerefMut},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) struct TabDescriptor {
    pub(in crate::ui::shell) title: String,
    pub(in crate::ui::shell) status: String,
    pub(in crate::ui::shell) kind: TabKindTag,
    pub(in crate::ui::shell) placement: TabPlacement,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) struct TabState {
    pub(in crate::ui::shell) id: TabId,
    descriptor: TabDescriptor,
}

impl TabState {
    pub(in crate::ui::shell) fn new(
        id: TabId,
        title: String,
        status: String,
        kind: TabKindTag,
        placement: TabPlacement,
    ) -> Self {
        Self {
            id,
            descriptor: TabDescriptor {
                title,
                status,
                kind,
                placement,
            },
        }
    }

    fn into_parts(self) -> (TabId, TabDescriptor) {
        (self.id, self.descriptor)
    }
}

impl Deref for TabState {
    type Target = TabDescriptor;

    fn deref(&self) -> &Self::Target {
        &self.descriptor
    }
}

impl DerefMut for TabState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.descriptor
    }
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui::shell) struct RegisteredTab<'a> {
    pub(in crate::ui::shell) id: TabId,
    descriptor: &'a TabDescriptor,
}

impl Deref for RegisteredTab<'_> {
    type Target = TabDescriptor;

    fn deref(&self) -> &Self::Target {
        self.descriptor
    }
}

pub(in crate::ui::shell) struct RegisteredTabMut<'a> {
    descriptor: &'a mut TabDescriptor,
}

impl Deref for RegisteredTabMut<'_> {
    type Target = TabDescriptor;

    fn deref(&self) -> &Self::Target {
        self.descriptor
    }
}

impl DerefMut for RegisteredTabMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.descriptor
    }
}

impl TabState {
    pub(in crate::ui::shell) fn new_hosts(id: TabId) -> Self {
        let title = i18n::string("tabs.initial.hosts_tab_title");
        let status = i18n::string("navigation.section.hosts.title");

        Self::new(id, title, status, TabKindTag::Hosts, TabPlacement::TopLevel)
    }
}

impl TabDescriptor {
    pub(in crate::ui::shell) fn is_hosts(&self) -> bool {
        self.kind == TabKindTag::Hosts
    }

    pub(in crate::ui::shell) fn is_session(&self) -> bool {
        self.kind == TabKindTag::Session
    }

    pub(in crate::ui::shell) fn is_sftp(&self) -> bool {
        self.kind == TabKindTag::Sftp
    }

    pub(in crate::ui::shell) fn is_top_level(&self) -> bool {
        self.placement == TabPlacement::TopLevel
    }

    pub(in crate::ui::shell) fn owner(&self) -> Option<TabId> {
        self.placement.owner()
    }
}

pub(in crate::ui::shell) enum ClosedTabBundle {
    Hosts,
    Sftp {
        profile: SessionProfile,
    },
    SessionWorkspace {
        tabs: Vec<ClosedSessionTabState>,
        sftp_tabs: Vec<ClosedSftpTabState>,
        workspace: Option<TabWorkspaceState>,
    },
}

pub(in crate::ui::shell) struct ClosedSftpTabState {
    pub(in crate::ui::shell) tab_id: TabId,
    pub(in crate::ui::shell) owner: TabId,
    pub(in crate::ui::shell) profile: SessionProfile,
}

pub(in crate::ui::shell) struct WorkspaceModel {
    pub(in crate::ui::shell) tabs: TabRegistry,
    pub(in crate::ui::shell) active_topbar_tab: Option<TabId>,
    pub(in crate::ui::shell) topbar_tab_scroll_handle: ScrollHandle,
    pub(in crate::ui::shell) topbar_previous_visible_tabs: Vec<TopbarTabSnapshot>,
    pub(in crate::ui::shell) topbar_entering_tabs: Vec<TopbarTabEnterTransition>,
    pub(in crate::ui::shell) topbar_exiting_tabs: Vec<TopbarTabExitTransition>,
    pub(in crate::ui::shell) topbar_active_transition: Option<TopbarActiveTabTransition>,
    pub(in crate::ui::shell) topbar_visible_active_tab_id: Option<TabId>,
    pub(in crate::ui::shell) next_tab_id: TabId,
    pub(in crate::ui::shell) workspace: TabWorkspaceState,
    pub(in crate::ui::shell) parked_workspaces: HashMap<TabId, TabWorkspaceState>,
    pub(in crate::ui::shell) recently_closed_tabs: Vec<ClosedTabBundle>,
    pub(in crate::ui::shell) renaming_tab: Option<TabId>,
    pub(in crate::ui::shell) primary_view_transition: Option<PrimaryViewTransition>,
    pub(in crate::ui::shell) visible_primary_view: Option<PrimaryViewKind>,
    pub(in crate::ui::shell) terminal_originated_selection_drag: Option<PaneId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) struct WorkspacePaneSwap {
    pub(in crate::ui::shell) promoted_tab_id: TabId,
    pub(in crate::ui::shell) moved_tab_id: TabId,
    pub(in crate::ui::shell) moved_order_index: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(in crate::ui::shell) struct TabId(usize);

impl TabId {
    pub(in crate::ui::shell) const fn new(raw: usize) -> Self {
        Self(raw)
    }

    pub(in crate::ui::shell) const fn raw(self) -> usize {
        self.0
    }
}

impl fmt::Display for TabId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

fn take_next_tab_id(next_tab_id: &mut TabId) -> TabId {
    let tab_id = *next_tab_id;
    next_tab_id.0 = next_tab_id.0.saturating_add(1);
    tab_id
}

/// Stable tab storage. Display order is independent from identity, so moving a
/// tab never invalidates references held by panes or background tasks.
pub(in crate::ui::shell) struct TabRegistry {
    entries: HashMap<TabId, TabDescriptor>,
    order: Vec<TabId>,
}

impl TabRegistry {
    pub(in crate::ui::shell) fn from_tabs(tabs: impl IntoIterator<Item = TabState>) -> Self {
        let mut registry = Self {
            entries: HashMap::new(),
            order: Vec::new(),
        };
        for tab in tabs {
            registry.push(tab);
        }
        registry
    }

    pub(in crate::ui::shell) fn len(&self) -> usize {
        self.order.len()
    }

    pub(in crate::ui::shell) fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub(in crate::ui::shell) fn get(&self, tab_id: TabId) -> Option<RegisteredTab<'_>> {
        self.entries.get(&tab_id).map(|descriptor| RegisteredTab {
            id: tab_id,
            descriptor,
        })
    }

    pub(in crate::ui::shell) fn get_mut(&mut self, tab_id: TabId) -> Option<RegisteredTabMut<'_>> {
        self.entries
            .get_mut(&tab_id)
            .map(|descriptor| RegisteredTabMut { descriptor })
    }

    pub(in crate::ui::shell) fn at(&self, index: usize) -> Option<RegisteredTab<'_>> {
        let tab_id = self.id_at(index)?;
        self.get(tab_id)
    }

    pub(in crate::ui::shell) fn index_of(&self, tab_id: TabId) -> Option<usize> {
        self.order.iter().position(|candidate| *candidate == tab_id)
    }

    pub(in crate::ui::shell) fn id_at(&self, index: usize) -> Option<TabId> {
        self.order.get(index).copied()
    }

    pub(in crate::ui::shell) fn ids(
        &self,
    ) -> impl DoubleEndedIterator<Item = TabId> + ExactSizeIterator + '_ {
        self.order.iter().copied()
    }

    pub(in crate::ui::shell) fn iter(
        &self,
    ) -> impl DoubleEndedIterator<Item = RegisteredTab<'_>> + ExactSizeIterator {
        self.order.iter().map(|tab_id| {
            let descriptor = self
                .entries
                .get(tab_id)
                .expect("tab registry order and entries must stay in sync");
            RegisteredTab {
                id: *tab_id,
                descriptor,
            }
        })
    }

    pub(in crate::ui::shell) fn push(&mut self, tab: TabState) {
        let (tab_id, descriptor) = tab.into_parts();
        assert!(
            self.entries.insert(tab_id, descriptor).is_none(),
            "duplicate tab id {tab_id}"
        );
        self.order.push(tab_id);
    }

    pub(in crate::ui::shell) fn remove(&mut self, index: usize) -> TabState {
        let tab_id = self.order.remove(index);
        let descriptor = self
            .entries
            .remove(&tab_id)
            .expect("tab registry order and entries must stay in sync");
        TabState {
            id: tab_id,
            descriptor,
        }
    }

    pub(in crate::ui::shell) fn replace(&mut self, index: usize, tab: TabState) -> TabState {
        let previous_id = self.order[index];
        let (next_id, descriptor) = tab.into_parts();
        if previous_id != next_id {
            assert!(
                !self.entries.contains_key(&next_id),
                "duplicate tab id {next_id}"
            );
            self.order[index] = next_id;
        }
        let previous_descriptor = self
            .entries
            .remove(&previous_id)
            .expect("tab registry order and entries must stay in sync");
        self.entries.insert(next_id, descriptor);
        TabState {
            id: previous_id,
            descriptor: previous_descriptor,
        }
    }

    pub(in crate::ui::shell) fn remove_id(&mut self, tab_id: TabId) -> Option<TabState> {
        let index = self.index_of(tab_id)?;
        self.order.remove(index);
        self.entries.remove(&tab_id).map(|descriptor| TabState {
            id: tab_id,
            descriptor,
        })
    }

    pub(in crate::ui::shell) fn swap(&mut self, left: usize, right: usize) {
        self.order.swap(left, right);
    }

    pub(in crate::ui::shell) fn move_to(&mut self, from: usize, to: usize) {
        let tab_id = self.order.remove(from);
        self.order.insert(to, tab_id);
    }

    pub(in crate::ui::shell) fn state(&self, tab_id: TabId) -> Option<TabState> {
        self.entries
            .get(&tab_id)
            .cloned()
            .map(|descriptor| TabState {
                id: tab_id,
                descriptor,
            })
    }
}

impl WorkspaceModel {
    pub(in crate::ui::shell) fn new(
        initial_tab: TabState,
        next_tab_id: TabId,
        workspace: TabWorkspaceState,
    ) -> Self {
        debug_assert!(initial_tab.is_hosts());
        let active_topbar_tab = Some(initial_tab.id);
        Self {
            tabs: TabRegistry::from_tabs([initial_tab]),
            active_topbar_tab,
            topbar_tab_scroll_handle: gpui::ScrollHandle::new(),
            topbar_previous_visible_tabs: Vec::new(),
            topbar_entering_tabs: Vec::new(),
            topbar_exiting_tabs: Vec::new(),
            topbar_active_transition: None,
            topbar_visible_active_tab_id: None,
            next_tab_id,
            workspace,
            parked_workspaces: HashMap::new(),
            recently_closed_tabs: Vec::new(),
            renaming_tab: None,
            primary_view_transition: None,
            visible_primary_view: None,
            terminal_originated_selection_drag: None,
        }
    }

    pub(in crate::ui::shell) fn allocate_tab_id(&mut self) -> TabId {
        take_next_tab_id(&mut self.next_tab_id)
    }

    pub(in crate::ui::shell) fn park_workspace(
        &mut self,
        owner: TabId,
        workspace: TabWorkspaceState,
    ) {
        self.parked_workspaces.insert(owner, workspace);
    }

    pub(in crate::ui::shell) fn take_parked_workspace(
        &mut self,
        owner: TabId,
    ) -> Option<TabWorkspaceState> {
        self.parked_workspaces.remove(&owner)
    }

    pub(in crate::ui::shell) fn sync_workspace_placements(
        &mut self,
        owner: TabId,
        workspace: &TabWorkspaceState,
    ) {
        if let Some(tab_id) = workspace.active_tab.filter(|tab_id| *tab_id != owner)
            && let Some(mut tab) = self.tabs.get_mut(tab_id)
        {
            tab.placement = TabPlacement::WorkspacePane {
                owner,
                pane: workspace.active_pane_id,
            };
        }
        for (pane, parked) in &workspace.parked_panes {
            if let Some(tab_id) = parked.active_tab.filter(|tab_id| *tab_id != owner)
                && let Some(mut tab) = self.tabs.get_mut(tab_id)
            {
                tab.placement = TabPlacement::WorkspacePane { owner, pane: *pane };
            }
        }
    }

    pub(in crate::ui::shell) fn swap_top_level_tab_with_pane(
        &mut self,
        source_tab_id: TabId,
        target_tab_id: TabId,
        owner: TabId,
        pane: PaneId,
    ) -> Option<WorkspacePaneSwap> {
        let source_index = self.tabs.index_of(source_tab_id)?;
        let target_index = self.tabs.index_of(target_tab_id)?;
        if source_index == target_index
            || !self
                .tabs
                .get(source_tab_id)
                .is_some_and(|tab| tab.is_top_level() && tab.is_session())
            || !self
                .tabs
                .get(target_tab_id)
                .is_some_and(|tab| tab.is_session())
        {
            return None;
        }

        self.tabs.swap(source_index, target_index);
        let promoted_tab_id = self.tabs.at(source_index)?.id;
        self.tabs.get_mut(promoted_tab_id)?.placement = TabPlacement::TopLevel;
        let moved_tab_id = self.tabs.at(target_index)?.id;
        self.tabs.get_mut(moved_tab_id)?.placement = TabPlacement::WorkspacePane { owner, pane };

        Some(WorkspacePaneSwap {
            promoted_tab_id,
            moved_tab_id,
            moved_order_index: target_index,
        })
    }

    pub(in crate::ui::shell) fn push_hosts_tab(&mut self, tab: TabState) {
        debug_assert_eq!(tab.kind, TabKindTag::Hosts);
        self.tabs.push(tab);
    }

    pub(in crate::ui::shell) fn push_sftp_tab(&mut self, tab: TabState) {
        debug_assert_eq!(tab.kind, TabKindTag::Sftp);
        self.tabs.push(tab);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum TabKindTag {
    Hosts,
    Session,
    Sftp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum TabPlacement {
    TopLevel,
    WorkspacePane { owner: TabId, pane: PaneId },
    SessionSidecar { owner: TabId },
    Background,
}

impl TabPlacement {
    fn owner(self) -> Option<TabId> {
        match self {
            Self::WorkspacePane { owner, .. } | Self::SessionSidecar { owner } => Some(owner),
            Self::TopLevel | Self::Background => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) struct ClosePlan {
    root: TabId,
    tabs: Vec<TabId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum ClosePlanStep {
    Retire(TabId),
    Commit(TabId),
}

impl ClosePlan {
    pub(in crate::ui::shell) fn new(root: TabId, tabs: impl IntoIterator<Item = TabId>) -> Self {
        let mut seen = HashSet::new();
        let mut tabs = tabs
            .into_iter()
            .filter(|tab_id| seen.insert(*tab_id))
            .collect::<Vec<_>>();
        if seen.insert(root) {
            tabs.insert(0, root);
        }
        Self { root, tabs }
    }

    pub(in crate::ui::shell) fn from_tabs(root: TabId, tabs: &[TabState]) -> Option<Self> {
        tabs.iter().any(|tab| tab.id == root).then_some(())?;

        let mut closing = HashSet::from([root]);
        loop {
            let previous_len = closing.len();
            for tab in tabs {
                if tab
                    .placement
                    .owner()
                    .is_some_and(|owner| closing.contains(&owner))
                {
                    closing.insert(tab.id);
                }
            }
            if closing.len() == previous_len {
                break;
            }
        }

        Some(Self::new(
            root,
            tabs.iter()
                .filter_map(|tab| closing.contains(&tab.id).then_some(tab.id)),
        ))
    }

    pub(in crate::ui::shell) fn root(&self) -> TabId {
        self.root
    }

    #[cfg(test)]
    fn tabs(&self) -> &[TabId] {
        &self.tabs
    }

    pub(in crate::ui::shell) fn resource_first_steps(&self) -> Vec<ClosePlanStep> {
        self.tabs
            .iter()
            .rev()
            .copied()
            .map(ClosePlanStep::Retire)
            .chain(self.tabs.iter().rev().copied().map(ClosePlanStep::Commit))
            .collect()
    }
}

pub(in crate::ui::shell) fn reopened_tab_id(
    closed: Option<TabId>,
    reopened: &HashMap<TabId, TabId>,
) -> Option<TabId> {
    closed.and_then(|closed_id| reopened.get(&closed_id).copied())
}

#[derive(Clone, Debug)]
pub(in crate::ui::shell) struct PaneSplitAnimationTarget {
    pub path: Vec<usize>,
    pub child_index: usize,
    pub new_child_index: usize,
    pub axis: SplitAxis,
    pub from_flex_a: f32,
    pub from_flex_b: f32,
    pub to_flex_a: f32,
    pub to_flex_b: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum SplitDirection {
    Up,
    Down,
    Left,
    Right,
}

impl SplitDirection {
    pub fn axis(self) -> SplitAxis {
        match self {
            Self::Left | Self::Right => SplitAxis::Horizontal,
            Self::Up | Self::Down => SplitAxis::Vertical,
        }
    }

    pub fn places_new_before(self) -> bool {
        matches!(self, Self::Left | Self::Up)
    }
}

#[derive(Debug)]
pub(in crate::ui::shell) enum PaneLayout {
    Leaf(PaneId),
    Split {
        axis: SplitAxis,
        children: Vec<PaneLayout>,
        flexes: Vec<f32>,
    },
}

impl PaneLayout {
    #[allow(dead_code)]
    pub fn collect_pane_ids(&self, into: &mut Vec<PaneId>) {
        match self {
            PaneLayout::Leaf(id) => into.push(*id),
            PaneLayout::Split { children, .. } => {
                for c in children {
                    c.collect_pane_ids(into);
                }
            }
        }
    }

    pub fn contains(&self, target: PaneId) -> bool {
        match self {
            PaneLayout::Leaf(id) => *id == target,
            PaneLayout::Split { children, .. } => {
                children.iter().any(|child| child.contains(target))
            }
        }
    }

    pub fn first_leaf(&self) -> PaneId {
        match self {
            PaneLayout::Leaf(id) => *id,
            PaneLayout::Split { children, .. } => children
                .first()
                .map(|c| c.first_leaf())
                .unwrap_or(PaneId(0)),
        }
    }

    pub fn split(
        &mut self,
        target: PaneId,
        direction: SplitDirection,
        new: PaneId,
    ) -> Option<PaneSplitAnimationTarget> {
        let mut path = Vec::new();
        self.split_inner(target, direction, new, &mut path)
    }

    pub fn close_animation_target(&self, target: PaneId) -> Option<PaneSplitAnimationTarget> {
        let mut path = Vec::new();
        self.close_animation_target_inner(target, &mut path)
    }

    fn split_inner(
        &mut self,
        target: PaneId,
        direction: SplitDirection,
        new: PaneId,
        path: &mut Vec<usize>,
    ) -> Option<PaneSplitAnimationTarget> {
        let new_axis = direction.axis();
        let places_new_before = direction.places_new_before();
        match self {
            PaneLayout::Leaf(id) => {
                if *id == target {
                    let target_id = *id;
                    let new_leaf = PaneLayout::Leaf(new);
                    let target_leaf = PaneLayout::Leaf(target_id);
                    let (a, b) = if places_new_before {
                        (new_leaf, target_leaf)
                    } else {
                        (target_leaf, new_leaf)
                    };
                    *self = PaneLayout::Split {
                        axis: new_axis,
                        children: vec![a, b],
                        flexes: vec![0.5, 0.5],
                    };
                    let collapsed = collapsed_split_flex(1.0);
                    let (from_flex_a, from_flex_b) = if places_new_before {
                        (collapsed, 1.0 - collapsed)
                    } else {
                        (1.0 - collapsed, collapsed)
                    };

                    Some(PaneSplitAnimationTarget {
                        path: path.clone(),
                        child_index: 0,
                        new_child_index: if places_new_before { 0 } else { 1 },
                        axis: new_axis,
                        from_flex_a,
                        from_flex_b,
                        to_flex_a: 0.5,
                        to_flex_b: 0.5,
                    })
                } else {
                    None
                }
            }
            PaneLayout::Split {
                axis,
                children,
                flexes,
            } => {
                if *axis == new_axis
                    && let Some(idx) = children
                        .iter()
                        .position(|c| matches!(c, PaneLayout::Leaf(id) if *id == target))
                {
                    let insert_at = if places_new_before { idx } else { idx + 1 };
                    children.insert(insert_at, PaneLayout::Leaf(new));
                    let target_flex = flexes[idx];
                    flexes[idx] = target_flex / 2.0;
                    flexes.insert(insert_at, target_flex / 2.0);
                    let collapsed = collapsed_split_flex(target_flex);
                    let (from_flex_a, from_flex_b) = if places_new_before {
                        (collapsed, target_flex - collapsed)
                    } else {
                        (target_flex - collapsed, collapsed)
                    };

                    return Some(PaneSplitAnimationTarget {
                        path: path.clone(),
                        child_index: idx,
                        new_child_index: insert_at,
                        axis: *axis,
                        from_flex_a,
                        from_flex_b,
                        to_flex_a: target_flex / 2.0,
                        to_flex_b: target_flex / 2.0,
                    });
                }
                for (index, child) in children.iter_mut().enumerate() {
                    path.push(index);
                    if let Some(animation) = child.split_inner(target, direction, new, path) {
                        path.pop();
                        return Some(animation);
                    }
                    path.pop();
                }
                None
            }
        }
    }

    fn close_animation_target_inner(
        &self,
        target: PaneId,
        path: &mut Vec<usize>,
    ) -> Option<PaneSplitAnimationTarget> {
        match self {
            PaneLayout::Leaf(_) => None,
            PaneLayout::Split {
                axis,
                children,
                flexes,
            } => {
                if let Some(index) = children
                    .iter()
                    .position(|child| matches!(child, PaneLayout::Leaf(id) if *id == target))
                {
                    if children.len() < 2 {
                        return None;
                    }

                    let (
                        child_index,
                        new_child_index,
                        from_flex_a,
                        from_flex_b,
                        to_flex_a,
                        to_flex_b,
                    ) = if index + 1 < children.len() {
                        let total = flexes[index] + flexes[index + 1];
                        let collapsed = collapsed_split_flex(total);
                        (
                            index,
                            index,
                            flexes[index],
                            flexes[index + 1],
                            collapsed,
                            total - collapsed,
                        )
                    } else {
                        let total = flexes[index - 1] + flexes[index];
                        let collapsed = collapsed_split_flex(total);
                        (
                            index - 1,
                            index,
                            flexes[index - 1],
                            flexes[index],
                            total - collapsed,
                            collapsed,
                        )
                    };

                    return Some(PaneSplitAnimationTarget {
                        path: path.clone(),
                        child_index,
                        new_child_index,
                        axis: *axis,
                        from_flex_a,
                        from_flex_b,
                        to_flex_a,
                        to_flex_b,
                    });
                }

                for (index, child) in children.iter().enumerate() {
                    path.push(index);
                    if let Some(animation) = child.close_animation_target_inner(target, path) {
                        path.pop();
                        return Some(animation);
                    }
                    path.pop();
                }

                None
            }
        }
    }

    pub fn remove(&mut self, target: PaneId) -> bool {
        let removed = self.remove_inner(target);
        if removed {
            Self::collapse(self);
        }
        removed
    }

    fn remove_inner(&mut self, target: PaneId) -> bool {
        match self {
            PaneLayout::Leaf(_) => false,
            PaneLayout::Split {
                children, flexes, ..
            } => {
                if let Some(idx) = children
                    .iter()
                    .position(|c| matches!(c, PaneLayout::Leaf(id) if *id == target))
                {
                    children.remove(idx);
                    flexes.remove(idx);
                    let sum: f32 = flexes.iter().sum();
                    if sum > 0.0 {
                        for f in flexes.iter_mut() {
                            *f /= sum;
                        }
                    }
                    return true;
                }
                for c in children.iter_mut() {
                    if c.remove_inner(target) {
                        return true;
                    }
                }
                false
            }
        }
    }

    fn collapse(node: &mut PaneLayout) {
        if let PaneLayout::Split { children, .. } = node {
            for c in children.iter_mut() {
                Self::collapse(c);
            }
            if children.len() == 1 {
                let only = children.remove(0);
                *node = only;
            }
        }
    }

    #[allow(dead_code)]
    pub fn split_at_path_mut(&mut self, path: &[usize]) -> Option<&mut PaneLayout> {
        let mut node = self;
        for &i in path {
            match node {
                PaneLayout::Split { children, .. } => {
                    node = children.get_mut(i)?;
                }
                _ => return None,
            }
        }
        match node {
            PaneLayout::Split { .. } => Some(node),
            _ => None,
        }
    }
}

pub(in crate::ui::shell) fn collapsed_split_flex(total_flex: f32) -> f32 {
    (total_flex * 0.02).max(0.001).min(total_flex * 0.45)
}

pub(in crate::ui::shell) struct TabWorkspaceState {
    pub active_tab: Option<TabId>,
    pub active_pane_id: PaneId,
    pub active_pane: PaneViewState,
    pub parked_panes: HashMap<PaneId, ParkedPane>,
    pub pane_layout: PaneLayout,
    pub next_pane_id: usize,
    pub pane_split_drag: Option<PaneSplitDragState>,
    pub pane_split_animation: Option<PaneSplitAnimation>,
    pub pane_tab_drop_target: Option<PaneTabDropTarget>,
}

impl TabWorkspaceState {
    pub fn new(active_tab: Option<TabId>, focus: FocusHandle) -> Self {
        Self {
            active_tab,
            active_pane_id: PaneId(1),
            active_pane: PaneViewState::new(focus),
            parked_panes: HashMap::new(),
            pane_layout: PaneLayout::Leaf(PaneId(1)),
            next_pane_id: 2,
            pane_split_drag: None,
            pane_split_animation: None,
            pane_tab_drop_target: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::shell::{ParkedPane, SftpTabState};

    fn tab(id: usize, placement: TabPlacement) -> TabState {
        tab_with_kind(id, placement, TabKindTag::Session)
    }

    fn tab_with_kind(id: usize, placement: TabPlacement, kind: TabKindTag) -> TabState {
        TabState::new(
            TabId::new(id),
            format!("tab-{id}"),
            String::new(),
            kind,
            placement,
        )
    }

    #[test]
    fn registry_reorder_preserves_stable_identity() {
        let active = TabId::new(20);
        let mut registry = TabRegistry::from_tabs([
            TabState::new_hosts(TabId::new(10)),
            TabState::new_hosts(active),
            TabState::new_hosts(TabId::new(30)),
        ]);

        registry.move_to(2, 0);

        assert_eq!(
            registry.ids().collect::<Vec<_>>(),
            vec![TabId::new(30), TabId::new(10), active]
        );
        assert_eq!(registry.index_of(active), Some(2));
        assert_eq!(registry.get(active).map(|tab| tab.id), Some(active));
    }

    #[test]
    fn registry_separates_stable_lookup_from_display_order() {
        let first = TabId::new(41);
        let second = TabId::new(7);
        let mut registry =
            TabRegistry::from_tabs([TabState::new_hosts(first), TabState::new_hosts(second)]);

        registry.move_to(1, 0);

        assert_eq!(registry.at(0).map(|tab| tab.id), Some(second));
        assert_eq!(registry.get(first).map(|tab| tab.id), Some(first));
        assert_eq!(registry.index_of(first), Some(1));
    }

    #[test]
    fn parked_workspace_round_trip_preserves_stable_tab_ids() {
        let cx = gpui::TestAppContext::single();
        let focus = cx.read(|app| app.focus_handle());
        let owner = TabId::new(1);
        let child = TabId::new(2);
        let mut model = WorkspaceModel::new(
            TabState::new_hosts(owner),
            TabId::new(3),
            TabWorkspaceState::new(Some(owner), focus.clone()),
        );
        let parked = TabWorkspaceState::new(Some(child), focus);

        model.park_workspace(owner, parked);
        let restored = model
            .take_parked_workspace(owner)
            .expect("parked workspace exists");

        assert_eq!(restored.active_tab, Some(child));
        assert!(!model.parked_workspaces.contains_key(&owner));
        cx.quit();
    }

    #[test]
    fn workspace_placement_sync_tracks_active_and_parked_panes() {
        let cx = gpui::TestAppContext::single();
        let focus = cx.read(|app| app.focus_handle());
        let owner = TabId::new(1);
        let active_child = TabId::new(2);
        let parked_child = TabId::new(3);
        let mut model = WorkspaceModel::new(
            TabState::new_hosts(owner),
            TabId::new(4),
            TabWorkspaceState::new(Some(owner), focus.clone()),
        );
        model
            .tabs
            .push(tab(active_child.raw(), TabPlacement::TopLevel));
        model
            .tabs
            .push(tab(parked_child.raw(), TabPlacement::TopLevel));

        let mut workspace = TabWorkspaceState::new(Some(active_child), focus.clone());
        let parked_pane_id = PaneId(2);
        let mut parked_pane = ParkedPane::empty(focus);
        parked_pane.active_tab = Some(parked_child);
        workspace.parked_panes.insert(parked_pane_id, parked_pane);

        model.sync_workspace_placements(owner, &workspace);

        assert_eq!(
            model.tabs.get(active_child).map(|tab| tab.placement),
            Some(TabPlacement::WorkspacePane {
                owner,
                pane: workspace.active_pane_id,
            })
        );
        assert_eq!(
            model.tabs.get(parked_child).map(|tab| tab.placement),
            Some(TabPlacement::WorkspacePane {
                owner,
                pane: parked_pane_id,
            })
        );
        cx.quit();
    }

    #[test]
    fn center_swap_promotes_pane_tab_and_moves_top_level_tab_into_pane() {
        let cx = gpui::TestAppContext::single();
        let focus = cx.read(|app| app.focus_handle());
        let owner = TabId::new(1);
        let source = TabId::new(2);
        let target = TabId::new(3);
        let pane = PaneId(2);
        let mut model = WorkspaceModel::new(
            TabState::new_hosts(owner),
            TabId::new(4),
            TabWorkspaceState::new(Some(owner), focus),
        );
        model.tabs.push(tab(source.raw(), TabPlacement::TopLevel));
        model.tabs.push(tab(
            target.raw(),
            TabPlacement::WorkspacePane { owner, pane },
        ));

        let swap = model
            .swap_top_level_tab_with_pane(source, target, owner, pane)
            .expect("valid center swap");

        assert_eq!(swap.promoted_tab_id, target);
        assert_eq!(swap.moved_tab_id, source);
        assert_eq!(
            model.tabs.get(target).map(|tab| tab.placement),
            Some(TabPlacement::TopLevel)
        );
        assert_eq!(
            model.tabs.get(source).map(|tab| tab.placement),
            Some(TabPlacement::WorkspacePane { owner, pane })
        );
        assert_eq!(
            model.tabs.at(swap.moved_order_index).map(|tab| tab.id),
            Some(source)
        );
        cx.quit();
    }

    fn test_profile(id: &str) -> miaominal_core::profile::SessionProfile {
        let mut profile = miaominal_core::profile::SessionProfile::blank(id, 1);
        profile.name = id.to_string();
        profile.host = "example.com".into();
        profile.username = "root".into();
        profile
    }

    fn test_registry_with_payloads(
        profile_id: &str,
    ) -> (
        TabRegistry,
        HashMap<TabId, SessionTabState>,
        HashMap<TabId, SftpTabState>,
        TabId,
        TabId,
    ) {
        let profile = test_profile(profile_id);
        let session_id = TabId::new(1);
        let (session_tab, session) = SessionController::build_pending_tab(
            session_id,
            profile.clone(),
            miaominal_terminal::TerminalState::default(),
            false,
        );
        let sftp_id = TabId::new(2);
        let sftp_tab = TabState::new_sftp(sftp_id, &profile);
        let sftp = SftpTabState::new(&profile);
        (
            TabRegistry::from_tabs([TabState::new_hosts(TabId::new(0)), session_tab, sftp_tab]),
            HashMap::from([(session_id, session)]),
            HashMap::from([(sftp_id, sftp)]),
            session_id,
            sftp_id,
        )
    }

    #[test]
    fn registry_removal_does_not_mutate_domain_payload_stores() {
        let (mut tabs, mut session_tabs, mut sftp_tabs, session_id, sftp_id) =
            test_registry_with_payloads("session-profile");

        let removed = tabs.remove_id(session_id).expect("session exists");

        assert_eq!(removed.id, session_id);
        assert!(tabs.get(session_id).is_none());
        assert!(session_tabs.contains_key(&session_id));
        assert!(sftp_tabs.contains_key(&sftp_id));

        tabs.remove_id(sftp_id).expect("SFTP tab exists");
        assert!(tabs.get(sftp_id).is_none());
        assert!(sftp_tabs.contains_key(&sftp_id));

        session_tabs.remove(&session_id);
        sftp_tabs.remove(&sftp_id);
        assert!(!session_tabs.contains_key(&session_id));
        assert!(!sftp_tabs.contains_key(&sftp_id));
    }

    #[test]
    fn registry_reorder_keeps_payloads_bound_to_stable_ids() {
        let (mut tabs, session_tabs, sftp_tabs, session_id, sftp_id) =
            test_registry_with_payloads("stable-profile");

        let sftp_index = tabs.index_of(sftp_id).expect("SFTP tab exists");
        let moved = tabs.remove(sftp_index);
        tabs.push(moved);

        assert_eq!(
            session_tabs
                .get(&session_id)
                .map(|tab| tab.profile_id.as_str()),
            Some("stable-profile")
        );
        assert_eq!(
            sftp_tabs.get(&sftp_id).map(|tab| tab.profile_id.as_str()),
            Some("stable-profile")
        );
        assert_eq!(tabs.get(session_id).map(|tab| tab.id), Some(session_id));
        assert_eq!(tabs.get(sftp_id).map(|tab| tab.id), Some(sftp_id));
    }

    #[test]
    fn reopening_allocates_a_new_id_and_rejects_late_old_id_access() {
        let profile = test_profile("reopen-profile");
        let mut next_tab_id = TabId::new(1);
        let old_id = take_next_tab_id(&mut next_tab_id);
        let (old_tab, old_session) = SessionController::build_pending_tab(
            old_id,
            profile.clone(),
            miaominal_terminal::TerminalState::default(),
            false,
        );
        let mut tabs = TabRegistry::from_tabs([TabState::new_hosts(TabId::new(0)), old_tab]);
        let mut session_tabs = HashMap::from([(old_id, old_session)]);
        tabs.remove_id(old_id).expect("old tab exists");
        session_tabs.remove(&old_id).expect("old payload exists");

        let reopened_id = take_next_tab_id(&mut next_tab_id);
        let (reopened_tab, reopened_session) = SessionController::build_pending_tab(
            reopened_id,
            profile,
            miaominal_terminal::TerminalState::default(),
            false,
        );
        tabs.push(reopened_tab);
        session_tabs.insert(reopened_id, reopened_session);

        assert_ne!(old_id, reopened_id);
        assert!(
            tabs.get(old_id)
                .and_then(|tab| session_tabs.get(&tab.id))
                .is_none()
        );
        assert!(
            tabs.get(reopened_id)
                .and_then(|tab| session_tabs.get(&tab.id))
                .is_some()
        );
    }

    #[test]
    fn removing_a_preceding_tab_does_not_change_active_identity() {
        let active = TabId::new(20);
        let mut registry = TabRegistry::from_tabs([
            TabState::new_hosts(TabId::new(10)),
            TabState::new_hosts(active),
            TabState::new_hosts(TabId::new(30)),
        ]);

        let removed = registry.remove(0);

        assert_eq!(removed.id, TabId::new(10));
        assert_eq!(registry.index_of(active), Some(0));
        assert_eq!(registry.id_at(0), Some(active));
    }

    #[test]
    fn close_plan_cascades_workspace_and_sidecar_ownership_only() {
        let root = TabId::new(1);
        let pane = TabId::new(2);
        let sidecar = TabId::new(3);
        let nested_sidecar = TabId::new(4);
        let background = TabId::new(5);
        let other_root = TabId::new(6);
        let tabs = vec![
            tab(1, TabPlacement::TopLevel),
            tab(
                2,
                TabPlacement::WorkspacePane {
                    owner: root,
                    pane: PaneId(2),
                },
            ),
            tab(3, TabPlacement::SessionSidecar { owner: root }),
            tab(4, TabPlacement::SessionSidecar { owner: pane }),
            tab(5, TabPlacement::Background),
            tab(6, TabPlacement::TopLevel),
        ];

        let plan = ClosePlan::from_tabs(root, &tabs).expect("root exists");

        assert_eq!(plan.root(), root);
        assert_eq!(plan.tabs(), &[root, pane, sidecar, nested_sidecar]);
        assert!(!plan.tabs().contains(&background));
        assert!(!plan.tabs().contains(&other_root));
    }

    #[test]
    fn close_plan_retires_every_resource_before_committing_registry_changes() {
        let root = TabId::new(1);
        let pane = TabId::new(2);
        let sidecar = TabId::new(3);
        let plan = ClosePlan::new(root, [root, pane, sidecar]);

        assert_eq!(
            plan.resource_first_steps(),
            vec![
                ClosePlanStep::Retire(sidecar),
                ClosePlanStep::Retire(pane),
                ClosePlanStep::Retire(root),
                ClosePlanStep::Commit(sidecar),
                ClosePlanStep::Commit(pane),
                ClosePlanStep::Commit(root),
            ]
        );
    }

    #[test]
    fn session_owned_sftp_cascades_but_standalone_sftp_does_not() {
        let session = TabId::new(1);
        let sidecar = TabId::new(2);
        let standalone = TabId::new(3);
        let tabs = vec![
            tab(1, TabPlacement::TopLevel),
            tab_with_kind(
                2,
                TabPlacement::SessionSidecar { owner: session },
                TabKindTag::Sftp,
            ),
            tab_with_kind(3, TabPlacement::TopLevel, TabKindTag::Sftp),
        ];

        assert_eq!(
            ClosePlan::from_tabs(session, &tabs)
                .expect("session exists")
                .tabs(),
            &[session, sidecar]
        );
        assert_eq!(
            ClosePlan::from_tabs(standalone, &tabs)
                .expect("standalone SFTP exists")
                .tabs(),
            &[standalone]
        );
    }

    #[test]
    fn standalone_and_session_owned_sftp_reopen_with_fresh_ids() {
        let session = TabId::new(10);
        let owned = TabId::new(11);
        let standalone = TabId::new(12);
        let profile = test_profile("sftp-reopen");
        let mut tabs = TabRegistry::from_tabs([
            tab(session.raw(), TabPlacement::TopLevel),
            tab_with_kind(
                owned.raw(),
                TabPlacement::SessionSidecar { owner: session },
                TabKindTag::Sftp,
            ),
            tab_with_kind(standalone.raw(), TabPlacement::TopLevel, TabKindTag::Sftp),
        ]);
        let mut payloads = HashMap::from([
            (owned, SftpTabState::new(&profile)),
            (standalone, SftpTabState::new(&profile)),
        ]);

        for tab_id in [owned, standalone] {
            tabs.remove_id(tab_id).expect("old tab metadata exists");
            payloads.remove(&tab_id).expect("old SFTP payload exists");
        }

        let mut next_tab_id = TabId::new(20);
        let reopened_session = take_next_tab_id(&mut next_tab_id);
        let reopened_owned = take_next_tab_id(&mut next_tab_id);
        let reopened_standalone = take_next_tab_id(&mut next_tab_id);
        tabs.push(tab_with_kind(
            reopened_owned.raw(),
            TabPlacement::SessionSidecar {
                owner: reopened_session,
            },
            TabKindTag::Sftp,
        ));
        tabs.push(tab_with_kind(
            reopened_standalone.raw(),
            TabPlacement::TopLevel,
            TabKindTag::Sftp,
        ));
        payloads.insert(reopened_owned, SftpTabState::new(&profile));
        payloads.insert(reopened_standalone, SftpTabState::new(&profile));

        assert_ne!(owned, reopened_owned);
        assert_ne!(standalone, reopened_standalone);
        assert!(tabs.get(owned).is_none());
        assert!(tabs.get(standalone).is_none());
        assert!(payloads.get(&owned).is_none());
        assert!(payloads.get(&standalone).is_none());
        assert_eq!(
            tabs.get(reopened_owned).map(|tab| tab.placement),
            Some(TabPlacement::SessionSidecar {
                owner: reopened_session,
            })
        );
        assert!(payloads.contains_key(&reopened_owned));
        assert!(payloads.contains_key(&reopened_standalone));
    }

    #[test]
    fn closing_each_non_owned_category_is_independent() {
        let standalone = TabId::new(7);
        let background = TabId::new(8);
        let tabs = vec![
            tab_with_kind(7, TabPlacement::TopLevel, TabKindTag::Sftp),
            tab(8, TabPlacement::Background),
        ];

        assert_eq!(
            ClosePlan::from_tabs(standalone, &tabs)
                .expect("standalone exists")
                .tabs(),
            &[standalone]
        );
        assert_eq!(
            ClosePlan::from_tabs(background, &tabs)
                .expect("background exists")
                .tabs(),
            &[background]
        );
    }

    #[test]
    fn reopened_workspace_maps_old_ids_to_new_ids_and_drops_missing_tabs() {
        let old_root = TabId::new(10);
        let old_pane = TabId::new(11);
        let new_root = TabId::new(20);
        let new_pane = TabId::new(21);
        let reopened = HashMap::from([(old_root, new_root), (old_pane, new_pane)]);

        assert_eq!(reopened_tab_id(Some(old_root), &reopened), Some(new_root));
        assert_eq!(reopened_tab_id(Some(old_pane), &reopened), Some(new_pane));
        assert_eq!(reopened_tab_id(Some(TabId::new(12)), &reopened), None);
        assert_ne!(old_root, new_root);
        assert_ne!(old_pane, new_pane);
    }

    #[test]
    fn leaf_first_leaf_returns_self_id() {
        let layout = PaneLayout::Leaf(PaneId(7));
        assert_eq!(layout.first_leaf(), PaneId(7));
        assert!(layout.contains(PaneId(7)));
        assert!(!layout.contains(PaneId(8)));
    }

    #[test]
    fn split_leaf_creates_two_child_split() {
        let mut layout = PaneLayout::Leaf(PaneId(1));
        let animation = layout.split(PaneId(1), SplitDirection::Right, PaneId(2));
        assert!(animation.is_some());
        match &layout {
            PaneLayout::Split {
                axis,
                children,
                flexes,
            } => {
                assert_eq!(*axis, SplitAxis::Horizontal);
                assert_eq!(children.len(), 2);
                assert_eq!(flexes.as_slice(), &[0.5, 0.5]);
                assert!(matches!(children[0], PaneLayout::Leaf(PaneId(1))));
                assert!(matches!(children[1], PaneLayout::Leaf(PaneId(2))));
            }
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn split_left_places_new_before_target() {
        let mut layout = PaneLayout::Leaf(PaneId(1));
        layout.split(PaneId(1), SplitDirection::Left, PaneId(2));
        match &layout {
            PaneLayout::Split { children, .. } => {
                assert!(matches!(children[0], PaneLayout::Leaf(PaneId(2))));
                assert!(matches!(children[1], PaneLayout::Leaf(PaneId(1))));
            }
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn split_same_axis_inserts_sibling() {
        let mut layout = PaneLayout::Leaf(PaneId(1));
        layout.split(PaneId(1), SplitDirection::Right, PaneId(2));
        layout.split(PaneId(2), SplitDirection::Right, PaneId(3));
        match &layout {
            PaneLayout::Split {
                axis,
                children,
                flexes,
            } => {
                assert_eq!(*axis, SplitAxis::Horizontal);
                assert_eq!(children.len(), 3);
                assert_eq!(flexes.len(), 3);
                let total: f32 = flexes.iter().sum();
                assert!((total - 1.0).abs() < 1e-6);
                assert!(matches!(children[0], PaneLayout::Leaf(PaneId(1))));
                assert!(matches!(children[1], PaneLayout::Leaf(PaneId(2))));
                assert!(matches!(children[2], PaneLayout::Leaf(PaneId(3))));
            }
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn split_orthogonal_axis_nests() {
        let mut layout = PaneLayout::Leaf(PaneId(1));
        layout.split(PaneId(1), SplitDirection::Right, PaneId(2));
        layout.split(PaneId(2), SplitDirection::Down, PaneId(3));
        match &layout {
            PaneLayout::Split { axis, children, .. } => {
                assert_eq!(*axis, SplitAxis::Horizontal);
                assert_eq!(children.len(), 2);
                match &children[1] {
                    PaneLayout::Split {
                        axis: inner_axis,
                        children: inner_children,
                        ..
                    } => {
                        assert_eq!(*inner_axis, SplitAxis::Vertical);
                        assert_eq!(inner_children.len(), 2);
                    }
                    _ => panic!("expected nested split"),
                }
            }
            _ => panic!("expected split"),
        }
    }

    #[test]
    fn remove_collapses_single_child_split() {
        let mut layout = PaneLayout::Leaf(PaneId(1));
        layout.split(PaneId(1), SplitDirection::Right, PaneId(2));
        assert!(layout.remove(PaneId(2)));
        assert!(matches!(layout, PaneLayout::Leaf(PaneId(1))));
    }

    #[test]
    fn remove_renormalizes_flexes() {
        let mut layout = PaneLayout::Leaf(PaneId(1));
        layout.split(PaneId(1), SplitDirection::Right, PaneId(2));
        layout.split(PaneId(2), SplitDirection::Right, PaneId(3));
        assert!(layout.remove(PaneId(2)));
        match &layout {
            PaneLayout::Split {
                children, flexes, ..
            } => {
                assert_eq!(children.len(), 2);
                let total: f32 = flexes.iter().sum();
                assert!((total - 1.0).abs() < 1e-6);
            }
            _ => panic!("expected split with two remaining children"),
        }
    }

    #[test]
    fn remove_missing_pane_returns_false() {
        let mut layout = PaneLayout::Leaf(PaneId(1));
        assert!(!layout.remove(PaneId(99)));
    }

    #[test]
    fn collect_pane_ids_walks_full_tree() {
        let mut layout = PaneLayout::Leaf(PaneId(1));
        layout.split(PaneId(1), SplitDirection::Right, PaneId(2));
        layout.split(PaneId(2), SplitDirection::Down, PaneId(3));
        let mut ids = Vec::new();
        layout.collect_pane_ids(&mut ids);
        ids.sort_by_key(|id| id.0);
        assert_eq!(ids, vec![PaneId(1), PaneId(2), PaneId(3)]);
    }

    #[test]
    fn split_direction_axis_mapping() {
        assert_eq!(SplitDirection::Up.axis(), SplitAxis::Vertical);
        assert_eq!(SplitDirection::Down.axis(), SplitAxis::Vertical);
        assert_eq!(SplitDirection::Left.axis(), SplitAxis::Horizontal);
        assert_eq!(SplitDirection::Right.axis(), SplitAxis::Horizontal);
        assert!(SplitDirection::Up.places_new_before());
        assert!(SplitDirection::Left.places_new_before());
        assert!(!SplitDirection::Down.places_new_before());
        assert!(!SplitDirection::Right.places_new_before());
    }

    #[test]
    fn collapsed_split_flex_is_clamped() {
        let small = collapsed_split_flex(1.0);
        assert!(small > 0.0 && small < 0.5);
        let large = collapsed_split_flex(10.0);
        assert!(large <= 4.5);
    }

    #[test]
    fn close_animation_target_returns_none_for_unknown_pane() {
        let mut layout = PaneLayout::Leaf(PaneId(1));
        layout.split(PaneId(1), SplitDirection::Right, PaneId(2));
        assert!(layout.close_animation_target(PaneId(99)).is_none());
    }

    #[test]
    fn close_animation_target_returns_target_for_known_pane() {
        let mut layout = PaneLayout::Leaf(PaneId(1));
        layout.split(PaneId(1), SplitDirection::Right, PaneId(2));
        let target = layout
            .close_animation_target(PaneId(2))
            .expect("expected animation target for pane 2");
        assert_eq!(target.axis, SplitAxis::Horizontal);
    }
}
