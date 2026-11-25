use super::*;
use gpui_kit::{Bounds, Empty, Size, point, size};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

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
        indicator_x: Pixels,
    },
    TerminalPane {
        pane_id: PaneId,
        zone: super::panes::PaneTabDropZone,
    },
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
}
