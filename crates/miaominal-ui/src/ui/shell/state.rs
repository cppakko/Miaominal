use super::*;
use gpui_kit::{Bounds, Empty, Size, point, size};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};
use std::time::{Duration, Instant};

pub(in crate::ui::shell) const TAB_DRAG_GHOST_HEIGHT: f32 = 36.0;
pub(in crate::ui::shell) const TAB_DRAG_GHOST_RADIUS: f32 = 14.0;

pub(in crate::ui::shell) fn tab_drag_ghost_size() -> Size<Pixels> {
    size(px(TOPBAR_TAB_WIDTH), px(TAB_DRAG_GHOST_HEIGHT))
}

#[derive(Debug)]
pub(in crate::ui::shell) struct DragGhostShared {
    grab_offset_x: AtomicU32,
    grab_offset_y: AtomicU32,
    in_window_ghost_hidden: AtomicBool,
}

impl DragGhostShared {
    fn new() -> Self {
        Self {
            grab_offset_x: AtomicU32::new((TOPBAR_TAB_WIDTH * 0.5).to_bits()),
            grab_offset_y: AtomicU32::new((TAB_DRAG_GHOST_HEIGHT * 0.5).to_bits()),
            in_window_ghost_hidden: AtomicBool::new(false),
        }
    }

    pub(in crate::ui::shell) fn set_grab_offset(&self, offset: Point<Pixels>) {
        let x = f32::from(offset.x).clamp(0.0, TOPBAR_TAB_WIDTH);
        let y = f32::from(offset.y).clamp(0.0, TAB_DRAG_GHOST_HEIGHT);
        self.grab_offset_x.store(x.to_bits(), Ordering::Relaxed);
        self.grab_offset_y.store(y.to_bits(), Ordering::Relaxed);
    }

    pub(in crate::ui::shell) fn grab_offset(&self) -> Point<Pixels> {
        point(
            px(f32::from_bits(self.grab_offset_x.load(Ordering::Relaxed))),
            px(f32::from_bits(self.grab_offset_y.load(Ordering::Relaxed))),
        )
    }

    pub(in crate::ui::shell) fn set_in_window_ghost_hidden(&self, hidden: bool) {
        self.in_window_ghost_hidden.store(hidden, Ordering::Relaxed);
    }

    pub(in crate::ui::shell) fn is_in_window_ghost_hidden(&self) -> bool {
        self.in_window_ghost_hidden.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(in crate::ui::shell) struct DraggedTab {
    pub(in crate::ui::shell) source_tab_id: TabId,
    pub(in crate::ui::shell) source_index: usize,
    pub(in crate::ui::shell) source_pane_id: PaneId,
    pub(in crate::ui::shell) is_active: bool,
    pub(in crate::ui::shell) title: String,
    pub(in crate::ui::shell) status_color: Option<u32>,
    pub(in crate::ui::shell) ghost_shared: Arc<DragGhostShared>,
}

impl DraggedTab {
    pub(in crate::ui::shell) fn new_ghost_shared() -> Arc<DragGhostShared> {
        Arc::new(DragGhostShared::new())
    }

    pub(in crate::ui::shell) fn set_grab_offset(&self, offset: Point<Pixels>) {
        self.ghost_shared.set_grab_offset(offset);
    }

    pub(in crate::ui::shell) fn grab_offset(&self) -> Point<Pixels> {
        self.ghost_shared.grab_offset()
    }

    pub(in crate::ui::shell) fn set_in_window_ghost_hidden(&self, hidden: bool) {
        self.ghost_shared.set_in_window_ghost_hidden(hidden);
    }

    pub(in crate::ui::shell) fn is_in_window_ghost_hidden(&self) -> bool {
        self.ghost_shared.is_in_window_ghost_hidden()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) enum TabDragGhostCue {
    None,
    DetachToNewWindow,
    DockIntoWindow,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::ui::shell) enum ExternalTabDropTarget {
    TabBar {
        insertion_index: usize,
    },
    TerminalPane {
        pane_id: PaneId,
        zone: super::panes::PaneTabDropZone,
    },
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui::shell) struct TopbarDropSlotTransition {
    pub(in crate::ui::shell) insertion_index: usize,
    pub(in crate::ui::shell) from_progress: f32,
    pub(in crate::ui::shell) target_progress: f32,
    pub(in crate::ui::shell) started_at: Instant,
    pub(in crate::ui::shell) duration: Duration,
}

impl TopbarDropSlotTransition {
    pub(in crate::ui::shell) fn raw_progress(&self, now: Instant) -> f32 {
        let duration_seconds = self.duration.as_secs_f32();
        if duration_seconds <= f32::EPSILON {
            return 1.0;
        }
        let elapsed = now.saturating_duration_since(self.started_at);
        (elapsed.as_secs_f32() / duration_seconds).clamp(0.0, 1.0)
    }

    pub(in crate::ui::shell) fn progress(&self, now: Instant) -> f32 {
        let raw = self.raw_progress(now);
        let eased = raw * raw * (3.0 - 2.0 * raw);
        (self.from_progress + (self.target_progress - self.from_progress) * eased).clamp(0.0, 1.0)
    }

    pub(in crate::ui::shell) fn is_animating(&self, now: Instant) -> bool {
        self.raw_progress(now) < 1.0 && (self.target_progress - self.from_progress).abs() > 0.001
    }
}

pub(in crate::ui::shell) fn prune_closed_topbar_drop_slots(
    slots: &mut Vec<TopbarDropSlotTransition>,
    now: Instant,
) {
    slots.retain(|slot| slot.target_progress > 0.001 || slot.progress(now) > 0.001);
}

pub(in crate::ui::shell) fn sync_topbar_drop_slots(
    slots: &mut Vec<TopbarDropSlotTransition>,
    target_index: Option<usize>,
    now: Instant,
    duration: Duration,
) {
    prune_closed_topbar_drop_slots(slots, now);
    for slot in slots.iter_mut() {
        if Some(slot.insertion_index) == target_index {
            if slot.target_progress < 1.0 {
                let current = slot.progress(now);
                slot.from_progress = current;
                slot.target_progress = 1.0;
                slot.started_at = now;
                slot.duration = duration.mul_f32((1.0 - current).clamp(0.0, 1.0));
            }
        } else if slot.target_progress > 0.0 {
            let current = slot.progress(now);
            slot.from_progress = current;
            slot.target_progress = 0.0;
            slot.started_at = now;
            slot.duration = duration.mul_f32(current.clamp(0.0, 1.0));
        }
    }
    if let Some(index) = target_index
        && !slots.iter().any(|slot| slot.insertion_index == index)
    {
        slots.push(TopbarDropSlotTransition {
            insertion_index: index,
            from_progress: 0.0,
            target_progress: 1.0,
            started_at: now,
            duration,
        });
    }
    prune_closed_topbar_drop_slots(slots, now);
}

pub(in crate::ui::shell) fn consume_topbar_drop_slot(
    slots: &mut Vec<TopbarDropSlotTransition>,
    insertion_index: usize,
    now: Instant,
    duration: Duration,
) -> f32 {
    let handoff = slots
        .iter()
        .position(|slot| slot.insertion_index == insertion_index)
        .map(|index| slots.remove(index).progress(now))
        .unwrap_or(0.0);
    for slot in slots.iter_mut() {
        if slot.insertion_index > insertion_index {
            slot.insertion_index += 1;
        }
        if slot.target_progress > 0.0 {
            let current = slot.progress(now);
            slot.from_progress = current;
            slot.target_progress = 0.0;
            slot.started_at = now;
            slot.duration = duration.mul_f32(current.clamp(0.0, 1.0));
        }
    }
    prune_closed_topbar_drop_slots(slots, now);
    handoff
}

#[derive(Clone, Debug, Default)]
pub(in crate::ui::shell) struct TopbarDropGeometry {
    pub(in crate::ui::shell) bar_bounds: Option<Bounds<Pixels>>,
    pub(in crate::ui::shell) tab_bounds: Vec<(TabId, Bounds<Pixels>)>,
}

pub(in crate::ui::shell) fn render_dragged_tab_pill(
    tab: &DraggedTab,
    cue: TabDragGhostCue,
) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;
    let highlighted = cue != TabDragGhostCue::None;
    let show_detach_plus = cue == TabDragGhostCue::DetachToNewWindow;

    h_flex()
        .w(px(TOPBAR_TAB_WIDTH))
        .h(px(TAB_DRAG_GHOST_HEIGHT))
        .px_3()
        .gap_2()
        .items_center()
        .rounded(px(TAB_DRAG_GHOST_RADIUS))
        .bg(rgb(if tab.is_active {
            roles.secondary_container
        } else {
            roles.surface_container_high
        }))
        .when(highlighted, |this| {
            this.border_1().border_color(rgb(roles.primary))
        })
        .opacity(if highlighted { 0.96 } else { 0.92 })
        .text_size(miaominal_settings::FontSize::Body.scaled())
        .text_color(rgb(if tab.is_active {
            roles.on_secondary_container
        } else {
            roles.on_surface_variant
        }))
        .child(
            h_flex()
                .flex_1()
                .min_w(px(0.0))
                .gap_2()
                .items_center()
                .when_some(tab.status_color, |this, color| {
                    this.child(div().size(px(7.0)).rounded(px(999.0)).bg(rgb(color)))
                })
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .child(tab.title.clone()),
                )
                .when(show_detach_plus, |this| {
                    this.child(
                        Icon::new(AppIcon::Plus)
                            .size(px(14.0))
                            .flex_shrink_0()
                            .text_color(rgb(roles.primary)),
                    )
                }),
        )
}

impl Render for DraggedTab {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if self.is_in_window_ghost_hidden() {
            return Empty.into_any_element();
        }
        render_dragged_tab_pill(self, TabDragGhostCue::None).into_any_element()
    }
}

pub(in crate::ui::shell) struct TabDragGhostView {
    tab: DraggedTab,
    cue: TabDragGhostCue,
}

impl TabDragGhostView {
    pub(in crate::ui::shell) fn new(tab: DraggedTab, cue: TabDragGhostCue) -> Self {
        Self { tab, cue }
    }

    pub(in crate::ui::shell) fn set_cue(&mut self, cue: TabDragGhostCue) {
        self.cue = cue;
    }
}

impl Render for TabDragGhostView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .font(miaominal_settings::interface_font())
            .child(render_dragged_tab_pill(&self.tab, self.cue))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::shell) struct TabDragDetachState {
    pub(in crate::ui::shell) tab_id: TabId,
    pub(in crate::ui::shell) outside_window: bool,
}

pub(in crate::ui::shell) struct ShellState {
    pub(in crate::ui::shell) sidebar_section: SidebarSection,
    pub(in crate::ui::shell) page_editor_sidebar_transition: Option<PageEditorSidebarTransition>,
    pub(in crate::ui::shell) visible_page_editor_sidebar: Option<PageEditorSidebarKind>,
    pub(in crate::ui::shell) tab_drag_detach: Option<TabDragDetachState>,
    pub(in crate::ui::shell) dragging_tab_id: Option<TabId>,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            sidebar_section: SidebarSection::Hosts,
            page_editor_sidebar_transition: None,
            visible_page_editor_sidebar: None,
            tab_drag_detach: None,
            dragging_tab_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_state_starts_on_ssh_profiles() {
        assert_eq!(ShellState::default().sidebar_section, SidebarSection::Hosts);
    }

    #[test]
    fn sync_topbar_drop_slots_opens_reverses_smoothly_and_prunes() {
        let mut slots = Vec::new();
        let t0 = Instant::now();
        let duration = Duration::from_millis(200);

        sync_topbar_drop_slots(&mut slots, Some(1), t0, duration);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].insertion_index, 1);
        assert!(slots[0].is_animating(t0));
        assert!(slots[0].progress(t0) <= 0.001);

        let t1 = t0 + duration;
        assert!((slots[0].progress(t1) - 1.0).abs() < 1e-5);
        assert!(!slots[0].is_animating(t1));

        // Slide from slot 1 to slot 2: combined width remains 1.0 across the transition.
        sync_topbar_drop_slots(&mut slots, Some(2), t1, duration);
        assert_eq!(slots.len(), 2);
        let t2 = t1 + Duration::from_millis(80);
        let p1_mid = slots[0].progress(t2);
        let p2_mid = slots[1].progress(t2);
        assert!((p1_mid + p2_mid - 1.0).abs() < 1e-5);
        assert!(p1_mid > 0.5 && p2_mid > 0.0 && p2_mid < 0.5);

        // Reverse mid-flight back to slot 1: scaled remaining durations stay symmetric.
        sync_topbar_drop_slots(&mut slots, Some(1), t2, duration);
        let t3 = t2 + Duration::from_millis(35);
        let p1_rev = slots[0].progress(t3);
        let p2_rev = slots[1].progress(t3);
        assert!((p1_rev + p2_rev - 1.0).abs() < 1e-5);
        assert!(p1_rev > p1_mid);
        assert!(p2_rev < p2_mid);

        // Once reversal completes, collapsing slot 2 is pruned.
        let t4 = t2 + duration;
        prune_closed_topbar_drop_slots(&mut slots, t4);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].insertion_index, 1);
        assert!((slots[0].progress(t4) - 1.0).abs() < 1e-5);

        // Leaving tab bar collapses slot 1 and prunes once finished.
        sync_topbar_drop_slots(&mut slots, None, t4, duration);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].target_progress, 0.0);
        let t5 = t4 + duration;
        prune_closed_topbar_drop_slots(&mut slots, t5);
        assert!(slots.is_empty());
    }

    #[test]
    fn consume_topbar_drop_slot_returns_handoff_and_shifts_closing_slots() {
        let mut slots = Vec::new();
        let t0 = Instant::now();
        let duration = Duration::from_millis(200);

        sync_topbar_drop_slots(&mut slots, Some(3), t0, duration);
        let t1 = t0 + duration;
        sync_topbar_drop_slots(&mut slots, Some(1), t1, duration);

        let t2 = t1 + Duration::from_millis(90);
        let expected_handoff = slots
            .iter()
            .find(|slot| slot.insertion_index == 1)
            .unwrap()
            .progress(t2);
        let handoff = consume_topbar_drop_slot(&mut slots, 1, t2, duration);

        assert!((handoff - expected_handoff).abs() < 1e-5);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].insertion_index, 4);
        assert_eq!(slots[0].target_progress, 0.0);
    }
}
