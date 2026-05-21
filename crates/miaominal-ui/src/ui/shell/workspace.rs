use super::panes::{
    PaneId, PaneSplitAnimation, PaneSplitDragState, PaneTabDropTarget, PaneViewState, ParkedPane,
};
use gpui::FocusHandle;
use std::collections::HashMap;

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
    pub active_tab: Option<usize>,
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
    pub fn new(active_tab: Option<usize>, focus: FocusHandle) -> Self {
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
