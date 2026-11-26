//! Physical detached-window operations and feature-port wiring. Workspace owns
//! resource extraction, installation, rollback and source completion.
use super::app_view::{FloatingTabDragGhost, FloatingTabDragGhostBackend, HoveredExternalTabDrop};
use super::panes::{PaneTabDropTarget, PaneTabDropZone};
use super::*;
use crate::ui::bridge_security_platform::{
    DragGhostVisualSpec, cursor_client_position, topmost_window_under_cursor, window_hwnd,
};
use crate::ui::contracts::snippets::TransferredSnippetsUi;
use crate::ui::windowing::{
    AppWindowRegistration, DetachedWindowPlacement, compute_detached_window_placement,
    live_app_windows,
};
use crate::ui::workspace::transaction::{TransferPhase, TransferTransaction};
use crate::ui::workspace::transfer::{WorkspaceTransferExt, WorkspaceTransferHost};
use gpui_kit::{
    AnyWindowHandle, CursorStyle, DisplayId, DragMoveEvent, ExternalDragPayload, ExternalPaths,
    FileDragPaths, Global, Size, WeakEntity, WindowBackgroundAppearance, WindowBounds, WindowId,
    WindowKind, WindowOptions, point,
};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

pub(in crate::ui::shell) const TAB_DRAG_DETACH_MARGIN: f32 = 24.0;
const RIGHT_WINDOW_CONTROLS_BASE_WIDTH: f32 = 128.0;

static WAYLAND_TAB_DRAG_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_wayland_tab_drag_token_path() -> PathBuf {
    let seq = WAYLAND_TAB_DRAG_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(".miaominal-tab-drag-{}-{seq}", std::process::id()))
}

#[derive(Clone)]
struct WaylandCrossWindowTabDrag {
    token_path: PathBuf,
    source_window: AnyWindowHandle,
    source_view: WeakEntity<AppView>,
    tab_id: TabId,
    payload: DraggedTab,
    can_detach: bool,
    allow_pane_drop: bool,
    handled: bool,
}

#[derive(Default)]
struct ActiveWaylandCrossWindowTabDrag(Option<WaylandCrossWindowTabDrag>);

impl Global for ActiveWaylandCrossWindowTabDrag {}

pub(crate) fn is_internal_tab_drag_paths(paths: &ExternalPaths, cx: &App) -> bool {
    let Some(drag) = cx
        .try_global::<ActiveWaylandCrossWindowTabDrag>()
        .and_then(|active| active.0.as_ref())
    else {
        return false;
    };
    paths.paths().len() == 1 && paths.paths()[0] == drag.token_path
}

pub(in crate::ui::shell) fn drag_ghost_crosses_window_edge(
    position: Point<Pixels>,
    grab_offset: Point<Pixels>,
    ghost_size: Size<Pixels>,
    viewport_size: Size<Pixels>,
) -> bool {
    let vw = f32::from(viewport_size.width);
    let vh = f32::from(viewport_size.height);
    if vw <= 0.0 || vh <= 0.0 {
        return false;
    }
    let left = f32::from(position.x) - f32::from(grab_offset.x);
    let top = f32::from(position.y) - f32::from(grab_offset.y);
    let right = left + f32::from(ghost_size.width);
    let bottom = top + f32::from(ghost_size.height);
    left < 0.0 || top < 0.0 || right > vw || bottom > vh
}

pub(in crate::ui::shell) fn is_outside_window_for_tab_detach(
    position: Point<Pixels>,
    viewport_size: Size<Pixels>,
) -> bool {
    let x = f32::from(position.x);
    let y = f32::from(position.y);
    let width = f32::from(viewport_size.width);
    let height = f32::from(viewport_size.height);
    if width <= 0.0 || height <= 0.0 {
        return false;
    }
    x < -TAB_DRAG_DETACH_MARGIN
        || x > width + TAB_DRAG_DETACH_MARGIN
        || y < -TAB_DRAG_DETACH_MARGIN
        || y > height + TAB_DRAG_DETACH_MARGIN
}

fn next_tab_drag_detach_state(
    tab_id: TabId,
    can_detach: bool,
    position: Point<Pixels>,
    viewport_size: Size<Pixels>,
) -> Option<TabDragDetachState> {
    can_detach.then(|| TabDragDetachState {
        tab_id,
        outside_window: is_outside_window_for_tab_detach(position, viewport_size),
    })
}

fn resolve_tab_drag_ghost_feedback(
    has_external_target: bool,
    can_detach: bool,
    outside_window: bool,
) -> (TabDragGhostCue, CursorStyle) {
    if has_external_target {
        (TabDragGhostCue::DockIntoWindow, CursorStyle::ClosedHand)
    } else if can_detach && outside_window {
        (TabDragGhostCue::DetachToNewWindow, CursorStyle::DragCopy)
    } else {
        (TabDragGhostCue::None, CursorStyle::ClosedHand)
    }
}

pub(in crate::ui::shell) fn hit_test_topbar_drop(
    local_pos: Point<Pixels>,
    geometry: &TopbarDropGeometry,
    exclude_right_controls_width: f32,
) -> Option<usize> {
    let bar = geometry.bar_bounds?;
    let bar_left = f32::from(bar.origin.x);
    let bar_top = f32::from(bar.origin.y);
    let right_limit =
        (bar_left + f32::from(bar.size.width) - exclude_right_controls_width).max(bar_left + 16.0);
    let x = f32::from(local_pos.x);
    let y = f32::from(local_pos.y);
    if x < bar_left
        || x > right_limit
        || y < bar_top
        || y > bar_top + f32::from(bar.size.height) + 12.0
    {
        return None;
    }

    let mut tabs: Vec<Bounds<Pixels>> = geometry
        .tab_bounds
        .iter()
        .map(|(_, bounds)| *bounds)
        .collect();
    tabs.sort_by(|a, b| f32::from(a.origin.x).total_cmp(&f32::from(b.origin.x)));

    if tabs.is_empty() {
        Some(0)
    } else if let Some((index, _)) = tabs.iter().enumerate().find(|(_, rect)| {
        let mid_x = f32::from(rect.origin.x) + f32::from(rect.size.width) * 0.5;
        x < mid_x
    }) {
        Some(index)
    } else {
        Some(tabs.len())
    }
}

pub(in crate::ui::shell) fn hit_test_pane_drop_zone(
    local_pos: Point<Pixels>,
    bounds: Bounds<Pixels>,
    allow_center: bool,
) -> Option<PaneTabDropZone> {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    if width <= 0.0 || height <= 0.0 || !bounds.contains(&local_pos) {
        return None;
    }
    let rel_x = (f32::from(local_pos.x) - f32::from(bounds.origin.x)) / width;
    let rel_y = (f32::from(local_pos.y) - f32::from(bounds.origin.y)) / height;
    if rel_y < 0.24 {
        Some(PaneTabDropZone::Up)
    } else if rel_y > 0.76 {
        Some(PaneTabDropZone::Down)
    } else if rel_x < 0.24 {
        Some(PaneTabDropZone::Left)
    } else if rel_x > 0.76 {
        Some(PaneTabDropZone::Right)
    } else if allow_center {
        Some(PaneTabDropZone::Center)
    } else {
        [
            (rel_y, PaneTabDropZone::Up),
            (1.0 - rel_y, PaneTabDropZone::Down),
            (rel_x, PaneTabDropZone::Left),
            (1.0 - rel_x, PaneTabDropZone::Right),
        ]
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, zone)| zone)
    }
}

fn pick_fallback_target_window(
    mut candidates: Vec<AppWindowRegistration>,
    source_window_id: WindowId,
    ghost_window_id: Option<WindowId>,
    cursor_screen: Point<Pixels>,
    source_contains_cursor: bool,
    cx: &mut App,
) -> Option<AppWindowRegistration> {
    if let Some(stack) = cx.window_stack() {
        for handle in stack {
            let id = handle.window_id();
            if ghost_window_id == Some(id) {
                continue;
            }
            let hovered = handle
                .update(cx, |_, window, _| window.bounds().contains(&cursor_screen))
                .unwrap_or(false);
            if !hovered {
                continue;
            }
            if id == source_window_id {
                return None;
            }
            return candidates
                .iter()
                .position(|candidate| candidate.window.window_id() == id)
                .map(|index| candidates.remove(index));
        }
        return None;
    }

    if source_contains_cursor {
        return None;
    }

    for candidate in candidates.into_iter().rev() {
        let hovered = candidate
            .window
            .update(cx, |_, window, _| window.bounds().contains(&cursor_screen))
            .unwrap_or(false);
        if hovered {
            return Some(candidate);
        }
    }
    None
}

fn should_detach_tab_on_release(
    pending: TabDragDetachState,
    has_active_drag: bool,
    can_detach: bool,
    position: Point<Pixels>,
    viewport_size: Size<Pixels>,
) -> bool {
    has_active_drag
        && can_detach
        && pending.outside_window
        && is_outside_window_for_tab_detach(position, viewport_size)
}

fn should_use_drop_window_placement() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("WAYLAND_DISPLAY").is_none_or(|value| value.is_empty())
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

fn drag_ghost_visual_spec(payload: &DraggedTab, cue: TabDragGhostCue) -> DragGhostVisualSpec {
    let roles = miaominal_settings::current_theme().material.roles;
    let highlighted = cue != TabDragGhostCue::None;
    let (background_rgb, text_rgb) = if payload.is_active {
        (roles.secondary_container, roles.on_secondary_container)
    } else {
        (roles.surface_container_high, roles.on_surface_variant)
    };
    DragGhostVisualSpec {
        title: payload.title.clone(),
        logical_width: TOPBAR_TAB_WIDTH,
        logical_height: TAB_DRAG_GHOST_HEIGHT,
        corner_radius: TAB_DRAG_GHOST_RADIUS,
        background_rgb,
        background_alpha: if highlighted { 0.96 } else { 0.92 },
        text_rgb,
        border_rgb: highlighted.then_some(roles.primary),
        status_rgb: payload.status_color,
        detach_plus_rgb: (cue == TabDragGhostCue::DetachToNewWindow).then_some(roles.primary),
    }
}

impl AppView {
    pub(in crate::ui::shell) fn open_tab_in_new_window(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_tab_in_new_window_with_placement(tab_id, None, window, cx);
    }

    pub(in crate::ui::shell) fn open_tab_in_new_window_with_placement(
        &mut self,
        tab_id: TabId,
        placement: Option<DetachedWindowPlacement>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_open_tab_in_new_window(tab_id, cx) {
            return;
        }
        let target = match crate::ui::windowing::open_detached_window(placement, cx) {
            Ok(target) => target,
            Err(error) => {
                self.report_transfer_failure(error.to_string(), cx);
                return;
            }
        };
        let Some(bundle) = self.extract_detached_tab_bundle(tab_id, window, cx) else {
            let (target_window, _) = target.into_parts();
            let _ = target_window.update(cx, |_, window, _| window.remove_window());
            return;
        };
        let source = bundle.source();
        let transaction = Rc::new(RefCell::new(TransferTransaction::prepare(Box::new(bundle))));
        let target_transaction = transaction.clone();
        let (target_window, target_view) = target.into_parts();
        // WindowHandle::update rejects a closed/type-changed window before
        // invoking this closure. Destination preflight also returns the complete
        // bundle, so every failed install still has one source-owned payload.
        let result = target_window.update(cx, move |_, target_window, cx| {
            target_view.update(cx, |target, cx| {
                target_transaction.borrow_mut().install(|bundle| {
                    target
                        .install_detached_tab_bundle(*bundle, target_window, cx)
                        .map_err(|bundle| {
                            (
                                "destination window contains live resources".to_string(),
                                bundle,
                            )
                        })
                })
            })?;
            target_window.activate_window();
            Ok::<(), String>(())
        });
        let error = match result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(error) => Some(error.to_string()),
        };
        if let Some(error) = error {
            if let Some(bundle) = transaction.borrow_mut().rollback() {
                self.restore_detached_tab_bundle(*bundle, window, cx);
            }
            self.report_transfer_failure(error, cx);
            let _ = target_window.update(cx, |_, target_window, _| target_window.remove_window());
            return;
        }
        transaction.borrow_mut().commit();
        debug_assert_eq!(transaction.borrow().phase(), TransferPhase::Committed);
        self.finish_detached_tab_source(source, window, cx);
    }

    pub(in crate::ui::shell) fn can_drag_detach_tab(&self, tab_id: TabId, cx: &App) -> bool {
        if !self.can_open_tab_in_new_window(tab_id, cx) {
            return false;
        }
        if self.window_role == AppWindowRole::Detached
            && self
                .workspace
                .tabs
                .iter()
                .filter(|tab| tab.is_top_level())
                .count()
                <= 1
        {
            return false;
        }
        true
    }

    pub(in crate::ui::shell) fn close_tab_drag_ghost(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(ghost) = self.shell.tab_drag_ghost.take() else {
            return false;
        };
        ghost.native.invalidate();
        ghost.payload.set_in_window_ghost_hidden(false);
        if let FloatingTabDragGhostBackend::Popup { window, .. } = ghost.backend {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        }
        true
    }

    fn open_tab_drag_ghost(
        &mut self,
        payload: &DraggedTab,
        cursor_position: Point<Pixels>,
        cue: TabDragGhostCue,
        source_window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ghost_size = tab_drag_ghost_size();
        let grab_offset = payload.grab_offset();
        let local_origin = point(
            cursor_position.x - grab_offset.x,
            cursor_position.y - grab_offset.y,
        );
        if crate::ui::bridge_security_platform::uses_native_surface_drag_ghost(source_window) {
            payload.set_in_window_ghost_hidden(true);
            let visual = drag_ghost_visual_spec(payload, cue);
            let native = match crate::ui::bridge_security_platform::open_native_surface_drag_ghost(
                source_window,
                local_origin,
                &visual,
            ) {
                Ok(native) => native,
                Err(error) => {
                    payload.set_in_window_ghost_hidden(false);
                    log::debug!("failed to open native surface tab drag ghost: {error:?}");
                    return;
                }
            };
            source_window.refresh();
            self.shell.tab_drag_ghost = Some(FloatingTabDragGhost {
                backend: FloatingTabDragGhostBackend::NativeSurface,
                payload: payload.clone(),
                native,
                cue,
            });
            return;
        }
        let source_origin = source_window.bounds().origin;
        let cursor_screen = point(
            source_origin.x + cursor_position.x,
            source_origin.y + cursor_position.y,
        );
        let raw_origin = point(
            cursor_screen.x - grab_offset.x,
            cursor_screen.y - grab_offset.y,
        );
        let target_display = cx
            .displays()
            .into_iter()
            .find(|display| display.bounds().contains(&cursor_screen))
            .or_else(|| source_window.display(cx))
            .or_else(|| cx.primary_display());
        let display_id = target_display.as_ref().map(|display| display.id());
        let initial_origin = if let Some(display) = target_display.as_ref() {
            let db = display.bounds();
            let left = f32::from(db.origin.x);
            let top = f32::from(db.origin.y);
            let max_x = (left + f32::from(db.size.width) - f32::from(ghost_size.width)).max(left);
            let max_y = (top + f32::from(db.size.height) - f32::from(ghost_size.height)).max(top);
            point(
                px(f32::from(raw_origin.x).clamp(left, max_x)),
                px(f32::from(raw_origin.y).clamp(top, max_y)),
            )
        } else {
            raw_origin
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                initial_origin,
                ghost_size,
            ))),
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id,
            window_background: WindowBackgroundAppearance::Transparent,
            window_min_size: Some(ghost_size),
            ..Default::default()
        };
        // Hide the in-window drag pill before opening the popup so GPUI's
        // eager initial draw of the popup does not double-render active_drag.
        payload.set_in_window_ghost_hidden(true);
        let ghost_payload = payload.clone();
        let handle = match cx.open_window(options, move |_, cx| {
            cx.new(|_| TabDragGhostView::new(ghost_payload, cue))
        }) {
            Ok(handle) => handle,
            Err(error) => {
                payload.set_in_window_ghost_hidden(false);
                log::debug!("failed to open floating tab drag ghost window: {error:?}");
                return;
            }
        };
        let Ok(view) = handle.entity(cx) else {
            payload.set_in_window_ghost_hidden(false);
            let _ = handle.update(cx, |_, window, _| window.remove_window());
            return;
        };
        let native = match handle
            .update(cx, |_, ghost_window, _| {
                crate::ui::bridge_security_platform::configure_drag_ghost_window(
                    ghost_window,
                    grab_offset,
                    TAB_DRAG_GHOST_HEIGHT,
                    TAB_DRAG_GHOST_RADIUS,
                )
            })
            .and_then(|result| result)
        {
            Ok(native) => native,
            Err(error) => {
                payload.set_in_window_ghost_hidden(false);
                log::debug!("failed to configure floating tab drag ghost window: {error:?}");
                let _ = handle.update(cx, |_, window, _| window.remove_window());
                return;
            }
        };
        crate::ui::bridge_security_platform::schedule_drag_ghost_move(
            &native,
            raw_origin,
            local_origin,
            source_window.scale_factor(),
            grab_offset,
            TAB_DRAG_GHOST_HEIGHT,
            TAB_DRAG_GHOST_RADIUS,
            cx,
        );
        source_window.refresh();
        self.shell.tab_drag_ghost = Some(FloatingTabDragGhost {
            backend: FloatingTabDragGhostBackend::Popup {
                window: handle.into(),
                view,
            },
            payload: payload.clone(),
            native,
            cue,
        });
    }

    fn sync_tab_drag_ghost(
        &mut self,
        payload: &DraggedTab,
        position: Point<Pixels>,
        cue: TabDragGhostCue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !crate::ui::bridge_security_platform::supports_floating_drag_ghost(window) {
            return;
        }
        if self
            .shell
            .tab_drag_ghost
            .as_ref()
            .is_some_and(|ghost| ghost.payload.source_tab_id != payload.source_tab_id)
        {
            self.close_tab_drag_ghost(cx);
        }
        if self.shell.tab_drag_ghost.is_none() {
            if drag_ghost_crosses_window_edge(
                position,
                payload.grab_offset(),
                tab_drag_ghost_size(),
                window.viewport_size(),
            ) {
                self.open_tab_drag_ghost(payload, position, cue, window, cx);
            }
            return;
        }
        if let Some(ghost) = self.shell.tab_drag_ghost.as_mut() {
            let grab_offset = ghost.payload.grab_offset();
            let source_origin = window.bounds().origin;
            let local_origin = point(position.x - grab_offset.x, position.y - grab_offset.y);
            let target_origin = point(
                source_origin.x + position.x - grab_offset.x,
                source_origin.y + position.y - grab_offset.y,
            );
            crate::ui::bridge_security_platform::schedule_drag_ghost_move(
                &ghost.native,
                target_origin,
                local_origin,
                window.scale_factor(),
                grab_offset,
                TAB_DRAG_GHOST_HEIGHT,
                TAB_DRAG_GHOST_RADIUS,
                cx,
            );
            if ghost.cue != cue {
                ghost.cue = cue;
                match &ghost.backend {
                    FloatingTabDragGhostBackend::Popup { view, .. } => {
                        let view = view.clone();
                        view.update(cx, |view, cx| {
                            view.set_cue(cue);
                            cx.notify();
                        });
                    }
                    FloatingTabDragGhostBackend::NativeSurface => {
                        let visual = drag_ghost_visual_spec(payload, cue);
                        crate::ui::bridge_security_platform::update_native_drag_ghost_visual(
                            &ghost.native,
                            &visual,
                        );
                    }
                }
            }
        }
    }

    pub(in crate::ui::shell) fn hit_test_external_tab_drop(
        &self,
        local_pos: Point<Pixels>,
        allow_pane_drop: bool,
        window: &Window,
        cx: &App,
    ) -> Option<ExternalTabDropTarget> {
        if self.controllers.settings.read(cx).show_onboarding() {
            return None;
        }
        let scale = (f32::from(window.rem_size()) / 14.0).max(0.5);
        let exclude_right_controls_width = if cfg!(target_os = "macos") {
            0.0
        } else {
            RIGHT_WINDOW_CONTROLS_BASE_WIDTH * scale
        };
        if let Some(insertion_index) = hit_test_topbar_drop(
            local_pos,
            &self.shell.topbar_drop_geometry.borrow(),
            exclude_right_controls_width,
        ) {
            return Some(ExternalTabDropTarget::TabBar { insertion_index });
        }

        if !allow_pane_drop || !self.has_active_session() {
            return None;
        }
        let is_terminal_workspace = self
            .workspace
            .active_topbar_tab
            .and_then(|id| self.session_tab(id, cx))
            .is_some_and(|session| session.purpose() == SessionPurpose::Terminal);
        if !is_terminal_workspace {
            return None;
        }

        let mut pane_ids = Vec::new();
        if let Some(zoomed) = self.workspace.workspace.zoomed_pane {
            pane_ids.push(zoomed);
        } else {
            self.workspace
                .workspace
                .pane_layout
                .collect_pane_ids(&mut pane_ids);
        }
        for pane_id in pane_ids {
            let bounds = if pane_id == self.workspace.workspace.active_pane_id {
                self.workspace.workspace.active_pane.terminal_bounds
            } else {
                self.parked_pane(pane_id).and_then(|p| p.terminal_bounds)
            };
            let Some(bounds) = bounds else {
                continue;
            };
            let allow_center = self
                .pane_tab_index(pane_id)
                .and_then(|index| self.workspace.tabs.at(index))
                .is_some_and(|tab| tab.is_session());
            if let Some(zone) = hit_test_pane_drop_zone(local_pos, bounds, allow_center) {
                return Some(ExternalTabDropTarget::TerminalPane { pane_id, zone });
            }
        }
        None
    }

    pub(in crate::ui::shell) fn set_external_tab_drop_preview(
        &mut self,
        target: Option<ExternalTabDropTarget>,
        cx: &mut Context<Self>,
    ) {
        let next_pane_target = match target {
            Some(ExternalTabDropTarget::TerminalPane { pane_id, zone }) => {
                Some(PaneTabDropTarget { pane_id, zone })
            }
            _ => None,
        };
        let slot_target = match target {
            Some(ExternalTabDropTarget::TabBar { insertion_index }) => Some(insertion_index),
            _ => None,
        };
        let had_slots = !self.shell.topbar_drop_slots.is_empty();
        sync_topbar_drop_slots(
            &mut self.shell.topbar_drop_slots,
            slot_target,
            Instant::now(),
            support::CONTAINER_TRANSITION_DURATION,
        );
        let has_slots = !self.shell.topbar_drop_slots.is_empty();
        let target_changed = self.shell.external_tab_drop != target
            || self.workspace.workspace.pane_tab_drop_target != next_pane_target;
        self.shell.external_tab_drop = target;
        self.workspace.workspace.pane_tab_drop_target = next_pane_target;
        if target_changed || had_slots || has_slots {
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn consume_external_tab_drop_for_merge(
        &mut self,
        insertion_index: Option<usize>,
        cx: &mut Context<Self>,
    ) -> f32 {
        let now = Instant::now();
        self.shell.external_tab_drop = None;
        self.workspace.workspace.pane_tab_drop_target = None;
        let handoff = if let Some(index) = insertion_index {
            consume_topbar_drop_slot(
                &mut self.shell.topbar_drop_slots,
                index,
                now,
                support::CONTAINER_TRANSITION_DURATION,
            )
        } else {
            sync_topbar_drop_slots(
                &mut self.shell.topbar_drop_slots,
                None,
                now,
                support::CONTAINER_TRANSITION_DURATION,
            );
            0.0
        };
        cx.notify();
        handoff
    }

    fn clear_stale_hovered_external_target(
        &mut self,
        keep_window_id: Option<WindowId>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(hovered) = self.shell.hovered_external_target.take() else {
            return false;
        };
        if keep_window_id != Some(hovered.window.window_id()) {
            let _ = hovered.view.update(cx, |view, cx| {
                view.set_external_tab_drop_preview(None, cx);
            });
        }
        true
    }

    fn clear_hovered_external_target(&mut self, cx: &mut Context<Self>) -> bool {
        self.clear_stale_hovered_external_target(None, cx)
    }

    fn can_drag_merge_tab_into_pane(&self, tab_id: TabId, cx: &App) -> bool {
        let Some(index) = self.workspace.tabs.index_of(tab_id) else {
            return false;
        };
        let Some(tab) = self.workspace.tabs.at(index) else {
            return false;
        };
        tab.is_top_level()
            && tab.is_session()
            && self.transfer_session_purpose(tab_id, cx) == Some(SessionPurpose::Terminal)
            && self.owned_tab_indices_for_topbar(index).as_slice() == [index]
    }

    fn resolve_external_tab_drop_target(
        &self,
        tab_id: TabId,
        source_position: Point<Pixels>,
        source_window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<HoveredExternalTabDrop> {
        if !self.can_transfer_tab_to_existing_window(tab_id, cx) {
            return None;
        }
        let allow_pane_drop = self.can_drag_merge_tab_into_pane(tab_id, cx);
        let source_window_id = source_window.window_handle().window_id();
        let source_entity_id = cx.entity_id();
        let ghost_window_id = self
            .shell
            .tab_drag_ghost
            .as_ref()
            .and_then(FloatingTabDragGhost::window_id);
        let candidates: Vec<AppWindowRegistration> = live_app_windows(cx)
            .into_iter()
            .filter(|entry| {
                entry.window.window_id() != source_window_id
                    && Some(entry.window.window_id()) != ghost_window_id
                    && entry.view.entity_id() != source_entity_id
            })
            .collect();
        if candidates.is_empty() {
            return None;
        }

        if let Some(source_hwnd) = window_hwnd(source_window) {
            let mut hwnd_candidates: Vec<(isize, AppWindowRegistration)> = Vec::new();
            for reg in candidates {
                if let Ok(Some(hwnd)) = reg
                    .window
                    .update(cx, |_, target_win, _| window_hwnd(target_win))
                    && hwnd != source_hwnd
                {
                    hwnd_candidates.push((hwnd, reg));
                }
            }
            if hwnd_candidates.is_empty() {
                return None;
            }
            let candidate_hwnds: Vec<isize> =
                hwnd_candidates.iter().map(|(hwnd, _)| *hwnd).collect();
            let ignored_hwnds: Vec<isize> = self
                .shell
                .tab_drag_ghost
                .as_ref()
                .and_then(|ghost| ghost.native.raw_hwnd())
                .into_iter()
                .collect();
            let target_hwnd = topmost_window_under_cursor(&candidate_hwnds, &ignored_hwnds)?;
            let (_, reg) = hwnd_candidates
                .into_iter()
                .find(|(hwnd, _)| *hwnd == target_hwnd)?;
            let target_view = reg.view.upgrade()?;
            let view_for_hit = target_view.clone();
            let target = reg
                .window
                .update(cx, |_, target_win, cx| {
                    let local_pos = cursor_client_position(target_win)?;
                    view_for_hit.read(cx).hit_test_external_tab_drop(
                        local_pos,
                        allow_pane_drop,
                        target_win,
                        cx,
                    )
                })
                .ok()
                .flatten()?;
            return Some(HoveredExternalTabDrop {
                window: reg.window,
                view: target_view,
                target,
            });
        }

        if !crate::ui::bridge_security_platform::window_has_global_screen_bounds(source_window) {
            return None;
        }

        let cursor_screen = point(
            source_window.bounds().origin.x + source_position.x,
            source_window.bounds().origin.y + source_position.y,
        );
        let source_contains_cursor =
            Bounds::new(Point::default(), source_window.viewport_size()).contains(&source_position);
        let reg = pick_fallback_target_window(
            candidates,
            source_window_id,
            ghost_window_id,
            cursor_screen,
            source_contains_cursor,
            cx,
        )?;
        let target_view = reg.view.upgrade()?;
        let view_for_hit = target_view.clone();
        let target = reg
            .window
            .update(cx, |_, target_win, cx| {
                let origin = target_win.bounds().origin;
                let local_pos = point(cursor_screen.x - origin.x, cursor_screen.y - origin.y);
                view_for_hit.read(cx).hit_test_external_tab_drop(
                    local_pos,
                    allow_pane_drop,
                    target_win,
                    cx,
                )
            })
            .ok()
            .flatten()?;
        Some(HoveredExternalTabDrop {
            window: reg.window,
            view: target_view,
            target,
        })
    }

    pub(in crate::ui::shell) fn clear_tab_drag_detach(&mut self, cx: &mut Context<Self>) -> bool {
        let cleared_hover = self.clear_hovered_external_target(cx);
        let closed_ghost = self.close_tab_drag_ghost(cx);
        let cleared_state = self.shell.shell_state.tab_drag_detach.take().is_some();
        let cleared_dragging = self.shell.shell_state.dragging_tab_id.take().is_some();
        if cleared_hover || closed_ghost || cleared_state || cleared_dragging {
            cx.notify();
            true
        } else {
            false
        }
    }

    pub(in crate::ui::shell) fn handle_tab_drag_move(
        &mut self,
        event: &DragMoveEvent<DraggedTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let payload = event.drag(cx).clone();
        let tab_id = payload.source_tab_id;
        self.shell.shell_state.dragging_tab_id = Some(tab_id);

        let external_target =
            self.resolve_external_tab_drop_target(tab_id, event.event.position, window, cx);
        if let Some(next_hover) = external_target.as_ref() {
            let same_window = self
                .shell
                .hovered_external_target
                .as_ref()
                .is_some_and(|old| old.window.window_id() == next_hover.window.window_id());
            if !same_window {
                self.clear_hovered_external_target(cx);
            }
            let same_target = self
                .shell
                .hovered_external_target
                .as_ref()
                .is_some_and(|old| {
                    old.window.window_id() == next_hover.window.window_id()
                        && old.target == next_hover.target
                });
            if !same_target {
                let target_spec = next_hover.target;
                let _ = next_hover.view.update(cx, |view, cx| {
                    view.set_external_tab_drop_preview(Some(target_spec), cx);
                });
            }
            self.shell.hovered_external_target = Some(next_hover.clone());
        } else {
            self.clear_hovered_external_target(cx);
        }

        let can_detach = self.can_drag_detach_tab(tab_id, cx);
        let next = next_tab_drag_detach_state(
            tab_id,
            can_detach,
            event.event.position,
            window.viewport_size(),
        );
        let outside_window = next.is_some_and(|state| state.outside_window);
        let (cue, cursor_style) =
            resolve_tab_drag_ghost_feedback(external_target.is_some(), can_detach, outside_window);
        if self.shell.shell_state.tab_drag_detach != next
            || self.shell.tab_drag_ghost.as_ref().map(|ghost| ghost.cue) != Some(cue)
        {
            self.shell.shell_state.tab_drag_detach = next;
            if next.is_some() || external_target.is_some() {
                cx.set_active_drag_cursor_style(cursor_style, window);
            }
            cx.notify();
        } else if external_target.is_some() {
            cx.set_active_drag_cursor_style(cursor_style, window);
        }
        self.sync_tab_drag_ghost(&payload, event.event.position, cue, window, cx);
    }

    fn detached_window_placement_for_drop(
        &self,
        drop_position: Point<Pixels>,
        window: &Window,
        cx: &App,
    ) -> Option<DetachedWindowPlacement> {
        if !should_use_drop_window_placement() {
            return None;
        }
        let mut displays: Vec<(DisplayId, Bounds<Pixels>)> = cx
            .displays()
            .into_iter()
            .map(|display| (display.id(), display.bounds()))
            .collect();
        if displays.is_empty()
            && let Some(display) = window.display(cx)
        {
            displays.push((display.id(), display.bounds()));
        }
        let source_bounds = window.bounds();
        let preferred_size = window.window_bounds().get_bounds().size;
        compute_detached_window_placement(source_bounds, preferred_size, drop_position, &displays)
    }

    fn move_tab_to_existing_window(
        &mut self,
        tab_id: TabId,
        target: HoveredExternalTabDrop,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bundle) = self.extract_detached_tab_bundle(tab_id, window, cx) else {
            return;
        };
        let root_id = bundle.root_id();
        let source = bundle.source();
        let transaction = Rc::new(RefCell::new(TransferTransaction::prepare(Box::new(bundle))));
        let target_transaction = transaction.clone();
        let target_view = target.view;
        let target_spec = target.target;
        let result = target.window.update(cx, move |_, target_window, cx| {
            target_view.update(cx, |view, cx| {
                let tab_bar_insertion = match target_spec {
                    ExternalTabDropTarget::TabBar { insertion_index } => Some(insertion_index),
                    ExternalTabDropTarget::TerminalPane { .. } => None,
                };
                let handoff_visibility =
                    view.consume_external_tab_drop_for_merge(tab_bar_insertion, cx);
                let install_result = target_transaction.borrow_mut().install(|bundle| {
                    let merge_result = match target_spec {
                        ExternalTabDropTarget::TabBar { insertion_index } => view
                            .merge_transferred_tab_into_topbar(
                                *bundle,
                                insertion_index,
                                target_window,
                                cx,
                            ),
                        ExternalTabDropTarget::TerminalPane { pane_id, zone } => view
                            .merge_transferred_tab_into_pane(
                                *bundle,
                                pane_id,
                                zone,
                                target_window,
                                cx,
                            ),
                    };
                    merge_result.map_err(|bundle| {
                        (
                            "destination window rejected transferred tab".to_string(),
                            bundle,
                        )
                    })
                });
                if install_result.is_ok()
                    && matches!(target_spec, ExternalTabDropTarget::TabBar { .. })
                {
                    view.seed_topbar_tab_enter_visibility(root_id, handoff_visibility);
                }
                install_result
            })?;
            target_window.activate_window();
            Ok::<(), String>(())
        });
        let error = match result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(error) => Some(error.to_string()),
        };
        if let Some(error) = error {
            if let Some(bundle) = transaction.borrow_mut().rollback() {
                self.restore_detached_tab_bundle(*bundle, window, cx);
            }
            self.report_transfer_failure(error, cx);
            return;
        }
        transaction.borrow_mut().commit();
        debug_assert_eq!(transaction.borrow().phase(), TransferPhase::Committed);
        self.finish_detached_tab_source(source, window, cx);
    }

    pub(in crate::ui::shell) fn finish_external_tab_drop_on_capture_mouse_up(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab_id) = self.shell.shell_state.dragging_tab_id else {
            return false;
        };
        if !cx.has_active_drag() {
            self.clear_tab_drag_detach(cx);
            return false;
        }
        let Some(target) = self.resolve_external_tab_drop_target(tab_id, position, window, cx)
        else {
            if Bounds::new(Point::default(), window.viewport_size()).contains(&position) {
                self.clear_tab_drag_detach(cx);
                if cx.has_global::<ActiveWaylandCrossWindowTabDrag>() {
                    let window_handle = window.window_handle();
                    if let Some(drag) = cx
                        .global_mut::<ActiveWaylandCrossWindowTabDrag>()
                        .0
                        .as_mut()
                        && drag.source_window == window_handle
                    {
                        drag.handled = true;
                    }
                }
            }
            return false;
        };
        self.clear_stale_hovered_external_target(Some(target.window.window_id()), cx);
        self.shell.shell_state.tab_drag_detach = None;
        self.shell.shell_state.dragging_tab_id = None;
        cx.stop_active_drag(window);
        self.close_tab_drag_ghost(cx);
        self.move_tab_to_existing_window(tab_id, target, window, cx);
        true
    }

    pub(in crate::ui::shell) fn promote_wayland_tab_drag_to_platform(
        &mut self,
        payload: &DraggedTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<ExternalDragPayload> {
        if !crate::ui::bridge_security_platform::is_wayland_window(window) {
            return None;
        }
        let tab_id = payload.source_tab_id;
        if !self.can_transfer_tab_to_existing_window(tab_id, cx) {
            return None;
        }
        let source_window = window.window_handle();
        let source_window_id = source_window.window_id();
        let source_entity_id = cx.entity_id();
        let ghost_window_id = self
            .shell
            .tab_drag_ghost
            .as_ref()
            .and_then(FloatingTabDragGhost::window_id);
        let has_other_window = live_app_windows(cx).into_iter().any(|entry| {
            entry.window.window_id() != source_window_id
                && Some(entry.window.window_id()) != ghost_window_id
                && entry.view.entity_id() != source_entity_id
        });
        if !has_other_window {
            return None;
        }

        // Tear down the source window's edge subsurface before Wayland starts
        // the native data_device drag session.
        self.close_tab_drag_ghost(cx);

        let token_path = next_wayland_tab_drag_token_path();
        let can_detach = self.can_drag_detach_tab(tab_id, cx);
        let allow_pane_drop = self.can_drag_merge_tab_into_pane(tab_id, cx);
        cx.set_global(ActiveWaylandCrossWindowTabDrag(Some(
            WaylandCrossWindowTabDrag {
                token_path: token_path.clone(),
                source_window,
                source_view: cx.entity().downgrade(),
                tab_id,
                payload: payload.clone(),
                can_detach,
                allow_pane_drop,
                handled: false,
            },
        )));
        Some(ExternalDragPayload::Files(FileDragPaths::new([(
            token_path, false,
        )])))
    }

    fn sync_wayland_incoming_drag_ghost(
        &mut self,
        payload: &DraggedTab,
        position: Point<Pixels>,
        cue: TabDragGhostCue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !crate::ui::bridge_security_platform::uses_native_surface_drag_ghost(window) {
            return;
        }
        if self
            .shell
            .tab_drag_ghost
            .as_ref()
            .is_some_and(|ghost| ghost.payload.source_tab_id != payload.source_tab_id)
        {
            self.close_tab_drag_ghost(cx);
        }
        if self.shell.tab_drag_ghost.is_none() {
            self.open_tab_drag_ghost(payload, position, cue, window, cx);
            return;
        }
        if let Some(ghost) = self.shell.tab_drag_ghost.as_mut() {
            let grab_offset = ghost.payload.grab_offset();
            let local_origin = point(position.x - grab_offset.x, position.y - grab_offset.y);
            let target_origin = point(
                window.bounds().origin.x + local_origin.x,
                window.bounds().origin.y + local_origin.y,
            );
            crate::ui::bridge_security_platform::schedule_drag_ghost_move(
                &ghost.native,
                target_origin,
                local_origin,
                window.scale_factor(),
                grab_offset,
                TAB_DRAG_GHOST_HEIGHT,
                TAB_DRAG_GHOST_RADIUS,
                cx,
            );
            if ghost.cue != cue {
                ghost.cue = cue;
                match &ghost.backend {
                    FloatingTabDragGhostBackend::Popup { .. } => {}
                    FloatingTabDragGhostBackend::NativeSurface => {
                        let visual = drag_ghost_visual_spec(payload, cue);
                        crate::ui::bridge_security_platform::update_native_drag_ghost_visual(
                            &ghost.native,
                            &visual,
                        );
                    }
                }
            }
        }
    }

    pub(in crate::ui::shell) fn handle_wayland_incoming_tab_drag_move(
        &mut self,
        event: &DragMoveEvent<ExternalPaths>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !is_internal_tab_drag_paths(event.drag(cx), cx) {
            return;
        }
        let Some(drag) = cx
            .try_global::<ActiveWaylandCrossWindowTabDrag>()
            .and_then(|active| active.0.clone())
        else {
            return;
        };
        if drag.handled || drag.source_window == window.window_handle() {
            return;
        }
        self.shell.wayland_incoming_tab_drag = true;
        let target =
            self.hit_test_external_tab_drop(event.event.position, drag.allow_pane_drop, window, cx);
        self.set_external_tab_drop_preview(target, cx);
        let cue = if target.is_some() {
            TabDragGhostCue::DockIntoWindow
        } else {
            TabDragGhostCue::None
        };
        cx.set_active_drag_cursor_style(CursorStyle::ClosedHand, window);
        self.sync_wayland_incoming_drag_ghost(&drag.payload, event.event.position, cue, window, cx);
    }

    pub(in crate::ui::shell) fn finish_wayland_incoming_tab_drop_on_capture_mouse_up(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.shell.wayland_incoming_tab_drag {
            return false;
        }
        self.shell.wayland_incoming_tab_drag = false;
        self.close_tab_drag_ghost(cx);

        let window_handle = window.window_handle();
        let pending = if cx.has_global::<ActiveWaylandCrossWindowTabDrag>() {
            let active = cx.global_mut::<ActiveWaylandCrossWindowTabDrag>();
            match active.0.as_mut() {
                Some(drag) if !drag.handled && drag.source_window != window_handle => {
                    drag.handled = true;
                    Some((
                        drag.source_window,
                        drag.source_view.clone(),
                        drag.tab_id,
                        drag.allow_pane_drop,
                    ))
                }
                _ => None,
            }
        } else {
            None
        };
        let Some((source_window, source_view, tab_id, allow_pane_drop)) = pending else {
            self.set_external_tab_drop_preview(None, cx);
            return false;
        };

        cx.stop_active_drag(window);
        let target = self
            .hit_test_external_tab_drop(position, allow_pane_drop, window, cx)
            .or(self.shell.external_tab_drop);
        if let Some(target) = target {
            let target_view = cx.entity();
            let hovered = HoveredExternalTabDrop {
                window: window_handle,
                view: target_view.clone(),
                target,
            };
            cx.defer(move |cx| {
                let result = source_window.update(cx, |_, source_win, cx| {
                    source_view.update(cx, |source, cx| {
                        source.clear_tab_drag_detach(cx);
                        source.move_tab_to_existing_window(tab_id, hovered, source_win, cx);
                    })
                });
                if !matches!(result, Ok(Ok(()))) {
                    let _ = target_view.update(cx, |view, cx| {
                        view.set_external_tab_drop_preview(None, cx);
                    });
                }
            });
        } else {
            self.set_external_tab_drop_preview(None, cx);
        }
        true
    }

    pub(in crate::ui::shell) fn handle_wayland_tab_drag_file_drop_exit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shell.wayland_incoming_tab_drag {
            self.shell.wayland_incoming_tab_drag = false;
            self.set_external_tab_drop_preview(None, cx);
            self.close_tab_drag_ghost(cx);
        } else if cx
            .try_global::<ActiveWaylandCrossWindowTabDrag>()
            .and_then(|active| active.0.as_ref())
            .is_some_and(|drag| drag.source_window == window.window_handle())
        {
            self.close_tab_drag_ghost(cx);
        }
    }

    pub(in crate::ui::shell) fn handle_wayland_tab_drag_file_drop_end(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let window_handle = window.window_handle();
        let is_source = cx
            .try_global::<ActiveWaylandCrossWindowTabDrag>()
            .and_then(|active| active.0.as_ref())
            .is_some_and(|drag| drag.source_window == window_handle);
        if !is_source {
            return;
        }
        let Some(drag) = cx.global_mut::<ActiveWaylandCrossWindowTabDrag>().0.take() else {
            return;
        };

        let source_window_id = window_handle.window_id();
        for reg in live_app_windows(cx) {
            if reg.window.window_id() == source_window_id {
                continue;
            }
            let _ = reg.view.update(cx, |view, cx| {
                if view.shell.wayland_incoming_tab_drag {
                    view.shell.wayland_incoming_tab_drag = false;
                    view.set_external_tab_drop_preview(None, cx);
                    view.close_tab_drag_ghost(cx);
                }
            });
        }

        self.clear_tab_drag_detach(cx);
        if !drag.handled && drag.can_detach {
            self.open_tab_in_new_window_with_placement(drag.tab_id, None, window, cx);
        }
    }

    pub(in crate::ui::shell) fn finish_tab_drag_detach(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dragging_tab_id = self.shell.shell_state.dragging_tab_id.take();
        let pending = self.shell.shell_state.tab_drag_detach.take();
        let active_tab_id = dragging_tab_id.or_else(|| pending.map(|state| state.tab_id));
        if cx.has_active_drag()
            && let Some(tab_id) = active_tab_id
            && let Some(target) =
                self.resolve_external_tab_drop_target(tab_id, position, window, cx)
        {
            self.clear_stale_hovered_external_target(Some(target.window.window_id()), cx);
            cx.stop_active_drag(window);
            self.close_tab_drag_ghost(cx);
            self.move_tab_to_existing_window(tab_id, target, window, cx);
            return;
        }
        self.clear_hovered_external_target(cx);

        let should_detach = pending.is_some_and(|pending| {
            let can_detach = self.can_drag_detach_tab(pending.tab_id, cx);
            should_detach_tab_on_release(
                pending,
                cx.has_active_drag(),
                can_detach,
                position,
                window.viewport_size(),
            )
        });
        if let (true, Some(pending)) = (should_detach, pending) {
            let placement = self.detached_window_placement_for_drop(position, window, cx);
            cx.stop_active_drag(window);
            self.close_tab_drag_ghost(cx);
            self.open_tab_in_new_window_with_placement(pending.tab_id, placement, window, cx);
        } else {
            self.close_tab_drag_ghost(cx);
            if pending.is_some_and(|pending| pending.outside_window) {
                cx.notify();
            }
        }
    }

    fn report_transfer_failure(&mut self, error: String, cx: &mut Context<Self>) {
        log::error!("failed to install detached tab window: {error}");
        self.shell.status_message = i18n::string_args(
            "chrome.messages.open_in_new_window_failed",
            &[("error", &error)],
        );
        cx.notify();
    }
}

impl WorkspaceTransferHost for AppView {
    fn transfer_state(&self) -> &WorkspaceModel {
        &self.workspace
    }
    fn transfer_state_mut(&mut self) -> &mut WorkspaceModel {
        &mut self.workspace
    }
    fn transfer_session_purpose(&self, id: TabId, cx: &App) -> Option<SessionPurpose> {
        self.controllers.session.read(cx).tab_purpose(id)
    }
    fn transfer_has_session(&self, id: TabId, cx: &App) -> bool {
        self.controllers.session.read(cx).contains_tab(id)
    }
    fn transfer_has_sftp(&self, id: TabId, cx: &App) -> bool {
        self.controllers.sftp.read(cx).contains_tab(id)
    }
    fn transfer_terminal_monitor_profile(&self, id: TabId, cx: &App) -> Option<String> {
        self.controllers
            .session
            .read(cx)
            .terminal_monitor_profile(id)
    }
    fn transfer_visible_sftp_sidecar(&self, root: TabId, cx: &App) -> Option<TabId> {
        let session = self.controllers.session.read(cx);
        (session.side_panel_open() && session.side_panel_view() == SessionSidePanelView::Sftp)
            .then(|| self.session_side_panel_sftp_tab_id(cx))
            .flatten()
            .filter(|id| {
                self.workspace
                    .tabs
                    .get(*id)
                    .is_some_and(|tab| tab.owner() == Some(root))
            })
    }
    fn transfer_take_session(&mut self, id: TabId, cx: &App) -> Option<TransferredSessionTab> {
        self.controllers.session.read(cx).take_tab_for_transfer(id)
    }
    fn transfer_take_sftp(&mut self, id: TabId, cx: &App) -> Option<TransferredSftpTab> {
        self.controllers.sftp.read(cx).take_tab_for_transfer(id)
    }
    fn transfer_install_session(
        &mut self,
        id: TabId,
        payload: TransferredSessionTab,
        cx: &mut Context<Self>,
    ) {
        let installed = self.controllers.session.update(cx, |controller, cx| {
            controller.insert_transferred_tab(payload, cx)
        });
        debug_assert_eq!(installed, id);
    }
    fn transfer_install_sftp(
        &mut self,
        id: TabId,
        payload: TransferredSftpTab,
        cx: &mut Context<Self>,
    ) {
        let installed = self.controllers.sftp.update(cx, |controller, cx| {
            controller.insert_transferred_tab(payload, cx)
        });
        debug_assert_eq!(installed, id);
    }
    fn transfer_take_session_ui(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TransferredSessionWorkspaceUi {
        self.controllers.session.update(cx, |controller, cx| {
            controller.take_workspace_ui_for_transfer(window, cx)
        })
    }
    fn transfer_take_snippets_ui(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TransferredSnippetsUi {
        self.controllers.snippets.update(cx, |owner, cx| {
            owner.take_workspace_ui_for_transfer(window, cx)
        })
    }
    fn transfer_restore_snippets_ui(
        &mut self,
        ui: TransferredSnippetsUi,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.controllers.snippets.update(cx, |owner, cx| {
            owner.restore_workspace_ui_after_transfer(ui, window, cx)
        });
    }
    fn transfer_take_sftp_ui(
        &mut self,
        id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TransferredSftpWindowUi> {
        self.controllers.sftp.update(cx, |controller, cx| {
            controller.take_window_ui_for_transfer(id, window, cx)
        })
    }
    fn transfer_restore_session_ui(
        &mut self,
        ui: TransferredSessionWorkspaceUi,
        active: Option<TabId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.controllers.session.update(cx, |controller, cx| {
            controller.restore_workspace_ui_after_transfer(ui, active, window, cx)
        });
    }
    fn transfer_restore_sftp_ui(
        &mut self,
        ui: TransferredSftpWindowUi,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.controllers.sftp.update(cx, |controller, cx| {
            controller.restore_window_ui_after_transfer(ui, window, cx)
        });
    }
    fn transfer_retire_resources(&mut self, id: TabId, kind: TabKindTag, cx: &mut Context<Self>) {
        match kind {
            TabKindTag::Session | TabKindTag::LocalTerminal | TabKindTag::Telnet => {
                self.controllers.session.read(cx).retire_tab_resources(id);
            }
            TabKindTag::Sftp => self.controllers.sftp.read(cx).retire_tab_resources(id),
            TabKindTag::Hosts => {}
        }
    }
    fn transfer_prepare_workspace_for_window(
        &mut self,
        state: &mut TabWorkspaceState,
        cx: &mut Context<Self>,
    ) {
        Self::prepare_transferred_workspace_for_window(state, cx);
    }
    fn transfer_window_is_detached(&self) -> bool {
        self.window_role == AppWindowRole::Detached
    }
    fn transfer_remove_window(&mut self, window: &mut Window) {
        window.remove_window();
    }
    fn transfer_set_status(&mut self, status: String) {
        self.shell.status_message = status;
    }
    fn transfer_refresh_monitoring(&mut self, profile: &str, cx: &mut Context<Self>) {
        self.refresh_profile_monitoring(profile, None, cx);
    }
    fn transfer_owned_tab_indices_for_topbar(&self, root: usize) -> Vec<usize> {
        self.owned_tab_indices_for_topbar(root)
    }
    fn transfer_park_loaded_workspace(&mut self, cx: &mut Context<Self>) -> TabWorkspaceState {
        self.park_loaded_workspace(cx)
    }
    fn transfer_restore_loaded_workspace(&mut self, state: TabWorkspaceState) {
        self.restore_loaded_workspace(state);
    }
    fn transfer_reset_loaded_workspace(&mut self, cx: &mut Context<Self>) {
        self.reset_loaded_workspace(cx);
    }
    fn transfer_load_topbar_workspace(&mut self, index: usize, cx: &mut Context<Self>) {
        self.load_topbar_workspace(index, cx);
    }
    fn transfer_prune_closed_tab_references(&mut self) {
        self.prune_closed_tab_references();
    }
    fn transfer_nearest_visible_tab(&self, index: usize) -> Option<usize> {
        self.nearest_visible_tab(index)
    }
    fn transfer_open_hosts_tab(&mut self, cx: &mut Context<Self>) {
        self.open_hosts_tab(cx);
    }
    fn transfer_restore_active_topbar_sidebar_section(&mut self, cx: &mut Context<Self>) {
        self.restore_active_topbar_sidebar_section(cx);
    }
    fn transfer_sync_transferred_sftp_browser(
        &mut self,
        root: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let browser = self
            .workspace
            .tabs
            .get(root)
            .filter(|tab| tab.is_sftp())
            .map(|tab| tab.id)
            .or_else(|| self.transfer_visible_sftp_sidecar(root, cx));
        let Some(id) = browser else {
            return;
        };
        let prompt = self.should_prompt_sftp_download_destination(id, cx);
        self.controllers.sftp.update(cx, |controller, cx| {
            controller.set_download_destination_prompt_tab(id, prompt);
            controller.sync_path_inputs_for_tab(id, window, cx);
            controller.sync_tables_for_tab(id, cx);
        });
    }
    fn transfer_rebind_terminal_focus_reporting(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rebind_terminal_focus_reporting(window, cx);
    }
    fn transfer_sync_terminal_focus_reporting(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_terminal_focus_reporting(window, cx);
        self.sync_window_overlays(window, cx);
    }
    fn transfer_sync_session_port_snapshot(&self, cx: &App) {
        self.sync_session_port_snapshot(cx);
    }
    fn transfer_remove_tab_payload_and_metadata(&mut self, id: TabId, cx: &mut Context<Self>) {
        self.remove_tab_payload_and_metadata(id, cx);
    }
    fn transfer_retire_window_resources(&mut self, cx: &mut Context<Self>) {
        if cx
            .try_global::<ActiveWaylandCrossWindowTabDrag>()
            .and_then(|active| active.0.as_ref())
            .is_some_and(|drag| drag.source_view.entity_id() == cx.entity_id())
        {
            cx.global_mut::<ActiveWaylandCrossWindowTabDrag>().0 = None;
        }
        self.close_tab_drag_ghost(cx);
        self.controllers.session.read(cx).retire_rdp_sessions();
    }
    fn transfer_unload_active_topbar_workspace(&mut self, cx: &mut Context<Self>) {
        self.unload_active_topbar_workspace(cx);
    }
    fn transfer_handle_pane_tab_drop(
        &mut self,
        source_tab_id: TabId,
        target_pane_id: PaneId,
        zone: PaneTabDropZone,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_pane_tab_drop(source_tab_id, target_pane_id, zone, window, cx);
    }
}
use crate::ui::i18n;

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Empty, TestAppContext, TestSupportExt as _, size};

    #[test]
    fn drag_ghost_edge_detection_triggers_when_pill_reaches_viewport_boundary() {
        let viewport = size(px(1200.0), px(800.0));
        let ghost_size = tab_drag_ghost_size();
        let grab_offset = point(px(80.0), px(18.0));

        assert!(!drag_ghost_crosses_window_edge(
            point(px(300.0), px(100.0)),
            grab_offset,
            ghost_size,
            viewport,
        ));
        assert!(drag_ghost_crosses_window_edge(
            point(px(300.0), px(10.0)),
            grab_offset,
            ghost_size,
            viewport,
        ));
        assert!(drag_ghost_crosses_window_edge(
            point(px(40.0), px(100.0)),
            grab_offset,
            ghost_size,
            viewport,
        ));
        assert!(drag_ghost_crosses_window_edge(
            point(px(1100.0), px(100.0)),
            grab_offset,
            ghost_size,
            viewport,
        ));
        assert!(drag_ghost_crosses_window_edge(
            point(px(300.0), px(790.0)),
            grab_offset,
            ghost_size,
            viewport,
        ));
        assert!(!drag_ghost_crosses_window_edge(
            point(px(-10.0), px(-10.0)),
            grab_offset,
            ghost_size,
            size(px(0.0), px(800.0)),
        ));
    }

    #[test]
    fn dragged_tab_grab_offset_clamps_within_pill_bounds_and_toggles_visibility() {
        let shared = DraggedTab::new_ghost_shared();
        let tab = DraggedTab {
            source_tab_id: TabId::new(1),
            source_index: 0,
            source_pane_id: PaneId(1),
            is_active: true,
            title: "ssh".to_string(),
            status_color: None,
            ghost_shared: shared,
        };
        let cloned = tab.clone();

        tab.set_grab_offset(point(px(-20.0), px(999.0)));
        assert_eq!(
            cloned.grab_offset(),
            point(px(0.0), px(TAB_DRAG_GHOST_HEIGHT))
        );

        tab.set_grab_offset(point(px(999.0), px(-5.0)));
        assert_eq!(cloned.grab_offset(), point(px(TOPBAR_TAB_WIDTH), px(0.0)));

        assert!(!cloned.is_in_window_ghost_hidden());
        tab.set_in_window_ghost_hidden(true);
        assert!(cloned.is_in_window_ghost_hidden());
        tab.set_in_window_ghost_hidden(false);
        assert!(!cloned.is_in_window_ghost_hidden());
    }

    #[test]
    fn outside_window_detection_respects_detach_margin() {
        let viewport = size(px(1200.0), px(800.0));

        assert!(!is_outside_window_for_tab_detach(
            point(px(600.0), px(400.0)),
            viewport
        ));
        assert!(!is_outside_window_for_tab_detach(
            point(px(0.0), px(0.0)),
            viewport
        ));
        assert!(!is_outside_window_for_tab_detach(
            point(px(600.0), px(-10.0)),
            viewport
        ));
        assert!(!is_outside_window_for_tab_detach(
            point(px(-24.0), px(200.0)),
            viewport
        ));
        assert!(!is_outside_window_for_tab_detach(
            point(px(1224.0), px(200.0)),
            viewport
        ));
        assert!(!is_outside_window_for_tab_detach(
            point(px(600.0), px(824.0)),
            viewport
        ));

        assert!(is_outside_window_for_tab_detach(
            point(px(-25.0), px(200.0)),
            viewport
        ));
        assert!(is_outside_window_for_tab_detach(
            point(px(1225.0), px(200.0)),
            viewport
        ));
        assert!(is_outside_window_for_tab_detach(
            point(px(600.0), px(-25.0)),
            viewport
        ));
        assert!(is_outside_window_for_tab_detach(
            point(px(600.0), px(825.0)),
            viewport
        ));

        assert!(!is_outside_window_for_tab_detach(
            point(px(-40.0), px(-40.0)),
            size(px(0.0), px(800.0))
        ));
    }

    #[test]
    fn non_detachable_tab_never_arms_detach_state_outside_window() {
        let viewport = size(px(1200.0), px(800.0));
        let tab_id = TabId::new(7);

        assert_eq!(
            next_tab_drag_detach_state(tab_id, false, point(px(-80.0), px(120.0)), viewport),
            None
        );
        assert_eq!(
            next_tab_drag_detach_state(tab_id, true, point(px(-80.0), px(120.0)), viewport),
            Some(TabDragDetachState {
                tab_id,
                outside_window: true,
            })
        );
    }

    #[test]
    fn hit_test_topbar_drop_computes_insertion_index_and_respects_bounds() {
        let empty = TopbarDropGeometry {
            bar_bounds: Some(Bounds::new(
                point(px(0.0), px(0.0)),
                size(px(800.0), px(48.0)),
            )),
            tab_bounds: Vec::new(),
        };
        assert_eq!(
            hit_test_topbar_drop(point(px(120.0), px(24.0)), &empty, 128.0),
            Some(0)
        );
        // Right window controls exclusion zone.
        assert_eq!(
            hit_test_topbar_drop(point(px(720.0), px(24.0)), &empty, 128.0),
            None
        );
        // Vertical margin below topbar.
        assert_eq!(
            hit_test_topbar_drop(point(px(120.0), px(65.0)), &empty, 128.0),
            None
        );

        let geometry = TopbarDropGeometry {
            bar_bounds: Some(Bounds::new(
                point(px(0.0), px(0.0)),
                size(px(800.0), px(48.0)),
            )),
            tab_bounds: vec![
                (
                    TabId::new(2),
                    Bounds::new(point(px(200.0), px(8.0)), size(px(160.0), px(40.0))),
                ),
                (
                    TabId::new(1),
                    Bounds::new(point(px(28.0), px(8.0)), size(px(160.0), px(40.0))),
                ),
            ],
        };
        // Left of tab 1 midpoint (108) -> index 0.
        assert_eq!(
            hit_test_topbar_drop(point(px(60.0), px(24.0)), &geometry, 128.0),
            Some(0)
        );
        // Right of tab 1 midpoint, left of tab 2 midpoint (280) -> index 1.
        assert_eq!(
            hit_test_topbar_drop(point(px(220.0), px(24.0)), &geometry, 128.0),
            Some(1)
        );
        // Right of tab 2 midpoint -> index 2 after tab 2.
        assert_eq!(
            hit_test_topbar_drop(point(px(340.0), px(24.0)), &geometry, 128.0),
            Some(2)
        );
    }

    #[test]
    fn hit_test_pane_drop_zone_maps_edges_center_and_fallback() {
        let bounds = Bounds::new(point(px(100.0), px(50.0)), size(px(400.0), px(200.0)));

        assert_eq!(
            hit_test_pane_drop_zone(point(px(50.0), px(100.0)), bounds, true),
            None
        );
        assert_eq!(
            hit_test_pane_drop_zone(point(px(300.0), px(70.0)), bounds, true),
            Some(PaneTabDropZone::Up)
        );
        assert_eq!(
            hit_test_pane_drop_zone(point(px(300.0), px(230.0)), bounds, true),
            Some(PaneTabDropZone::Down)
        );
        assert_eq!(
            hit_test_pane_drop_zone(point(px(130.0), px(150.0)), bounds, true),
            Some(PaneTabDropZone::Left)
        );
        assert_eq!(
            hit_test_pane_drop_zone(point(px(470.0), px(150.0)), bounds, true),
            Some(PaneTabDropZone::Right)
        );
        assert_eq!(
            hit_test_pane_drop_zone(point(px(300.0), px(150.0)), bounds, true),
            Some(PaneTabDropZone::Center)
        );
        // When center is disallowed, fall back to nearest edge (150 is 0.50 vertically -> Top/Up or Left/Right, 110 is closer to Up).
        assert_eq!(
            hit_test_pane_drop_zone(point(px(300.0), px(110.0)), bounds, false),
            Some(PaneTabDropZone::Up)
        );
    }

    #[test]
    fn tab_drag_ghost_feedback_prioritizes_dock_then_detach() {
        assert_eq!(
            resolve_tab_drag_ghost_feedback(true, false, false),
            (TabDragGhostCue::DockIntoWindow, CursorStyle::ClosedHand)
        );
        assert_eq!(
            resolve_tab_drag_ghost_feedback(true, true, true),
            (TabDragGhostCue::DockIntoWindow, CursorStyle::ClosedHand)
        );
        assert_eq!(
            resolve_tab_drag_ghost_feedback(false, true, true),
            (TabDragGhostCue::DetachToNewWindow, CursorStyle::DragCopy)
        );
        assert_eq!(
            resolve_tab_drag_ghost_feedback(false, false, true),
            (TabDragGhostCue::None, CursorStyle::ClosedHand)
        );
        assert_eq!(
            resolve_tab_drag_ghost_feedback(false, true, false),
            (TabDragGhostCue::None, CursorStyle::ClosedHand)
        );
    }

    struct TabDragDetachTestHost {
        drag_detach: Option<TabDragDetachState>,
        detached_tabs: Rc<RefCell<Vec<TabId>>>,
        reordered_drops: Rc<RefCell<Vec<(TabId, TabId)>>>,
        detachable_tabs: Vec<TabId>,
    }

    impl TabDragDetachTestHost {
        fn is_detachable(&self, tab_id: TabId) -> bool {
            self.detachable_tabs.contains(&tab_id)
        }

        fn clear_detach(&mut self, cx: &mut Context<Self>) {
            if self.drag_detach.take().is_some() {
                cx.notify();
            }
        }

        fn handle_drag_move(
            &mut self,
            event: &DragMoveEvent<DraggedTab>,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            let tab_id = event.drag(cx).source_tab_id;
            let next = next_tab_drag_detach_state(
                tab_id,
                self.is_detachable(tab_id),
                event.event.position,
                window.viewport_size(),
            );
            if self.drag_detach != next {
                self.drag_detach = next;
                if let Some(state) = next {
                    cx.set_active_drag_cursor_style(
                        resolve_tab_drag_ghost_feedback(false, true, state.outside_window).1,
                        window,
                    );
                }
                cx.notify();
            }
        }

        fn finish_detach(
            &mut self,
            position: Point<Pixels>,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            let Some(pending) = self.drag_detach.take() else {
                return;
            };
            let can_detach = self.is_detachable(pending.tab_id);
            if should_detach_tab_on_release(
                pending,
                cx.has_active_drag(),
                can_detach,
                position,
                window.viewport_size(),
            ) {
                cx.stop_active_drag(window);
                self.detached_tabs.borrow_mut().push(pending.tab_id);
            }
            cx.notify();
        }

        fn render_tab(
            &self,
            element_id: &'static str,
            tab_id: TabId,
            index: usize,
            _cx: &mut Context<Self>,
        ) -> impl IntoElement {
            let payload = DraggedTab {
                source_tab_id: tab_id,
                source_index: index,
                source_pane_id: PaneId(1),
                is_active: index == 0,
                title: element_id.to_string(),
                status_color: None,
                ghost_shared: DraggedTab::new_ghost_shared(),
            };
            let reordered_drops = self.reordered_drops.clone();
            div()
                .id(element_id)
                .test_support()
                .w(px(120.0))
                .h(px(36.0))
                .on_drag(payload, |_, _, _, cx| cx.new(|_| Empty))
                .on_drop::<DraggedTab>(move |payload, _, _| {
                    reordered_drops
                        .borrow_mut()
                        .push((payload.source_tab_id, tab_id));
                })
        }
    }

    impl Render for TabDragDetachTestHost {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .on_drag_move::<DraggedTab>(cx.listener(|this, event, window, cx| {
                    this.handle_drag_move(event, window, cx);
                }))
                .capture_any_mouse_up(cx.listener(|this, event: &MouseUpEvent, _window, cx| {
                    if event.button == MouseButton::Left {
                        this.clear_detach(cx);
                    }
                }))
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseUpEvent, window, cx| {
                        this.finish_detach(event.position, window, cx);
                    }),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(self.render_tab("tab-1", TabId::new(10), 0, cx))
                        .child(self.render_tab("tab-2", TabId::new(20), 1, cx))
                        .child(self.render_tab("tab-3", TabId::new(30), 2, cx)),
                )
        }
    }

    #[gpui_kit::test]
    fn tab_drag_detach_integration_covers_outside_inside_margin_and_non_detachable(
        cx: &mut TestAppContext,
    ) {
        use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};

        cx.update(gpui_kit::init);
        let detached_tabs = Rc::new(RefCell::new(Vec::new()));
        let reordered_drops = Rc::new(RefCell::new(Vec::new()));
        let host_slot: Rc<RefCell<Option<Entity<TabDragDetachTestHost>>>> =
            Rc::new(RefCell::new(None));

        let detached_for_view = detached_tabs.clone();
        let reordered_for_view = reordered_drops.clone();
        let slot_for_view = host_slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|_| TabDragDetachTestHost {
                drag_detach: None,
                detached_tabs: detached_for_view,
                reordered_drops: reordered_for_view,
                detachable_tabs: vec![TabId::new(10), TabId::new(20)],
            });
            *slot_for_view.borrow_mut() = Some(view.clone());
            Root::new(view, window, cx)
        });
        let host = host_slot.borrow().clone().expect("host view initialized");

        cx.update(|window, cx| {
            window.render_frame(cx);
            let tab1 = window.find("tab-1").bounds();
            let tab2 = window.find("tab-2").bounds();
            let tab3 = window.find("tab-3").bounds();

            // Case 1: drag detachable tab outside past margin and release -> detach fires
            window.pointer_down(tab1.center(), MouseButton::Left, cx);
            window.pointer_move(
                tab1.center() + point(px(10.0), px(0.0)),
                Some(MouseButton::Left),
                cx,
            );
            window.pointer_move(point(px(-40.0), px(20.0)), Some(MouseButton::Left), cx);
            assert_eq!(
                host.read(cx).drag_detach,
                Some(TabDragDetachState {
                    tab_id: TabId::new(10),
                    outside_window: true,
                })
            );
            assert_eq!(cx.active_drag_cursor_style(), Some(CursorStyle::DragCopy));

            window.pointer_up(point(px(-40.0), px(20.0)), MouseButton::Left, cx);
            assert_eq!(*detached_tabs.borrow(), vec![TabId::new(10)]);
            assert!(reordered_drops.borrow().is_empty());
            assert!(!cx.has_active_drag());
            assert_eq!(host.read(cx).drag_detach, None);

            // Case 2: drag outside past margin, drag back onto sibling tab, and drop -> reorder only
            detached_tabs.borrow_mut().clear();
            window.pointer_down(tab1.center(), MouseButton::Left, cx);
            window.pointer_move(
                tab1.center() + point(px(10.0), px(0.0)),
                Some(MouseButton::Left),
                cx,
            );
            window.pointer_move(point(px(-40.0), px(20.0)), Some(MouseButton::Left), cx);
            assert_eq!(
                host.read(cx).drag_detach,
                Some(TabDragDetachState {
                    tab_id: TabId::new(10),
                    outside_window: true,
                })
            );
            window.pointer_move(tab2.center(), Some(MouseButton::Left), cx);
            assert_eq!(
                host.read(cx).drag_detach,
                Some(TabDragDetachState {
                    tab_id: TabId::new(10),
                    outside_window: false,
                })
            );
            assert_eq!(cx.active_drag_cursor_style(), Some(CursorStyle::ClosedHand));
            window.pointer_up(tab2.center(), MouseButton::Left, cx);
            assert!(detached_tabs.borrow().is_empty());
            assert_eq!(
                *reordered_drops.borrow(),
                vec![(TabId::new(10), TabId::new(20))]
            );
            assert!(!cx.has_active_drag());
            assert_eq!(host.read(cx).drag_detach, None);

            // Case 3: drag slightly past top edge within 24px margin and release -> no detach
            reordered_drops.borrow_mut().clear();
            window.pointer_down(tab1.center(), MouseButton::Left, cx);
            window.pointer_move(
                tab1.center() + point(px(10.0), px(0.0)),
                Some(MouseButton::Left),
                cx,
            );
            window.pointer_move(point(px(60.0), px(-12.0)), Some(MouseButton::Left), cx);
            assert_eq!(
                host.read(cx).drag_detach,
                Some(TabDragDetachState {
                    tab_id: TabId::new(10),
                    outside_window: false,
                })
            );
            assert_eq!(cx.active_drag_cursor_style(), Some(CursorStyle::ClosedHand));
            window.pointer_up(point(px(60.0), px(-12.0)), MouseButton::Left, cx);
            assert!(detached_tabs.borrow().is_empty());
            assert!(reordered_drops.borrow().is_empty());
            assert!(!cx.has_active_drag());
            assert_eq!(host.read(cx).drag_detach, None);

            // Case 4: non-detachable tab dragged outside and released -> no detach
            window.pointer_down(tab3.center(), MouseButton::Left, cx);
            window.pointer_move(
                tab3.center() + point(px(10.0), px(0.0)),
                Some(MouseButton::Left),
                cx,
            );
            window.pointer_move(point(px(-40.0), px(20.0)), Some(MouseButton::Left), cx);
            assert_eq!(host.read(cx).drag_detach, None);
            window.pointer_up(point(px(-40.0), px(20.0)), MouseButton::Left, cx);
            assert!(detached_tabs.borrow().is_empty());
            assert!(!cx.has_active_drag());
            assert_eq!(host.read(cx).drag_detach, None);
        });
    }

    #[gpui_kit::test]
    fn internal_tab_drag_paths_matches_only_exact_active_sentinel(cx: &mut TestAppContext) {
        use gpui_kit::{ExternalPaths, WeakEntity};

        cx.update(gpui_kit::init);
        let token_a = next_wayland_tab_drag_token_path();
        let token_b = next_wayland_tab_drag_token_path();
        assert!(token_a.is_absolute());
        assert_ne!(token_a, token_b);

        let matching = ExternalPaths([token_a.clone()].into_iter().collect());
        let other = ExternalPaths([token_b.clone()].into_iter().collect());
        let empty = ExternalPaths::default();
        let multiple = ExternalPaths([token_a.clone(), token_b].into_iter().collect());

        let source_window: AnyWindowHandle = cx.add_window(|_, _| Empty).into();

        cx.update(|cx| {
            assert!(!is_internal_tab_drag_paths(&matching, cx));

            let shared = DraggedTab::new_ghost_shared();
            cx.set_global(ActiveWaylandCrossWindowTabDrag(Some(
                WaylandCrossWindowTabDrag {
                    token_path: token_a,
                    source_window,
                    source_view: WeakEntity::<AppView>::new_invalid(),
                    tab_id: TabId::new(1),
                    payload: DraggedTab {
                        source_tab_id: TabId::new(1),
                        source_index: 0,
                        source_pane_id: PaneId(1),
                        is_active: true,
                        title: "ssh".to_string(),
                        status_color: None,
                        ghost_shared: shared,
                    },
                    can_detach: true,
                    allow_pane_drop: true,
                    handled: false,
                },
            )));

            assert!(is_internal_tab_drag_paths(&matching, cx));
            assert!(!is_internal_tab_drag_paths(&other, cx));
            assert!(!is_internal_tab_drag_paths(&empty, cx));
            assert!(!is_internal_tab_drag_paths(&multiple, cx));

            cx.global_mut::<ActiveWaylandCrossWindowTabDrag>().0 = None;
            assert!(!is_internal_tab_drag_paths(&matching, cx));
        });
    }

    #[gpui_kit::test]
    fn fallback_target_window_ignores_candidates_when_source_contains_cursor(
        cx: &mut TestAppContext,
    ) {
        use gpui_kit::WeakEntity;

        cx.update(gpui_kit::init);
        let source_win: AnyWindowHandle = cx.add_window(|_, _| Empty).into();
        let other_win: AnyWindowHandle = cx.add_window(|_, _| Empty).into();

        cx.update(|cx| {
            let candidate = AppWindowRegistration {
                window: other_win,
                view: WeakEntity::<AppView>::new_invalid(),
            };
            let cursor = point(px(10.0), px(10.0));
            assert!(
                pick_fallback_target_window(
                    vec![candidate.clone()],
                    source_win.window_id(),
                    None,
                    cursor,
                    true,
                    cx,
                )
                .is_none()
            );

            let picked = pick_fallback_target_window(
                vec![candidate],
                source_win.window_id(),
                None,
                cursor,
                false,
                cx,
            );
            assert_eq!(
                picked.map(|reg| reg.window.window_id()),
                Some(other_win.window_id())
            );
        });
    }

    struct WaylandFileDropTestHost {
        incoming_hover: bool,
        sentinel_moves: usize,
        unrelated_moves: usize,
        detached_on_end: usize,
        handled_drops: usize,
    }

    impl Render for WaylandFileDropTestHost {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let entity = cx.entity();
            div()
                .size_full()
                .on_drag_move::<ExternalPaths>(cx.listener(
                    |this, event: &DragMoveEvent<ExternalPaths>, _window, cx| {
                        if is_internal_tab_drag_paths(event.drag(cx), cx) {
                            this.incoming_hover = true;
                            this.sentinel_moves += 1;
                        } else {
                            this.unrelated_moves += 1;
                        }
                    },
                ))
                .capture_any_mouse_up(cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    if event.button != MouseButton::Left || !this.incoming_hover {
                        return;
                    }
                    this.incoming_hover = false;
                    let window_handle = window.window_handle();
                    if cx.has_global::<ActiveWaylandCrossWindowTabDrag>()
                        && let Some(drag) =
                            cx.global_mut::<ActiveWaylandCrossWindowTabDrag>().0.as_mut()
                        && drag.source_window != window_handle
                        && !drag.handled
                    {
                        drag.handled = true;
                        this.handled_drops += 1;
                    }
                    cx.stop_active_drag(window);
                    cx.stop_propagation();
                }))
                .child(
                    gpui_kit::canvas(
                        |_, _, _| {},
                        move |_, _, window, _| {
                            let exit_entity = entity.clone();
                            let end_entity = entity;
                            window.on_mouse_event(
                                move |event: &gpui_kit::FileDropEvent, phase, window, cx| {
                                    if phase != gpui_kit::DispatchPhase::Bubble {
                                        return;
                                    }
                                    match event {
                                        gpui_kit::FileDropEvent::Exited => {
                                            exit_entity.update(cx, |this, _cx| {
                                                this.incoming_hover = false;
                                            });
                                        }
                                        gpui_kit::FileDropEvent::Ended => {
                                            end_entity.update(cx, |this, cx| {
                                                let window_handle = window.window_handle();
                                                let is_source = cx
                                                    .try_global::<ActiveWaylandCrossWindowTabDrag>()
                                                    .and_then(|active| active.0.as_ref())
                                                    .is_some_and(|drag| {
                                                        drag.source_window == window_handle
                                                    });
                                                if is_source
                                                    && let Some(drag) = cx
                                                        .global_mut::<ActiveWaylandCrossWindowTabDrag>()
                                                        .0
                                                        .take()
                                                    && !drag.handled
                                                    && drag.can_detach
                                                {
                                                    this.detached_on_end += 1;
                                                }
                                            });
                                        }
                                        _ => {}
                                    }
                                },
                            );
                        },
                    )
                    .absolute()
                    .size_0(),
                )
        }
    }

    #[gpui_kit::test]
    fn wayland_file_drop_flow_tracks_sentinel_exit_and_handled_flag(cx: &mut TestAppContext) {
        use gpui_kit::{ExternalPaths, FileDropEvent, PlatformInput, WeakEntity};

        cx.update(gpui_kit::init);
        let (window_a_handle, view_a) = {
            let mut view_slot = None;
            let handle: AnyWindowHandle = cx
                .add_window(|window, cx| {
                    let view = cx.new(|_| WaylandFileDropTestHost {
                        incoming_hover: false,
                        sentinel_moves: 0,
                        unrelated_moves: 0,
                        detached_on_end: 0,
                        handled_drops: 0,
                    });
                    view_slot = Some(view.clone());
                    Root::new(view, window, cx)
                })
                .into();
            (handle, view_slot.unwrap())
        };
        let (window_b_handle, view_b) = {
            let mut view_slot = None;
            let handle: AnyWindowHandle = cx
                .add_window(|window, cx| {
                    let view = cx.new(|_| WaylandFileDropTestHost {
                        incoming_hover: false,
                        sentinel_moves: 0,
                        unrelated_moves: 0,
                        detached_on_end: 0,
                        handled_drops: 0,
                    });
                    view_slot = Some(view.clone());
                    Root::new(view, window, cx)
                })
                .into();
            (handle, view_slot.unwrap())
        };

        let sentinel = next_wayland_tab_drag_token_path();
        let sentinel_paths = ExternalPaths([sentinel.clone()].into_iter().collect());
        let unrelated_paths = ExternalPaths(
            [std::env::temp_dir().join("other.txt")]
                .into_iter()
                .collect(),
        );

        let seed_drag = |cx: &mut App, token: &PathBuf| {
            cx.set_global(ActiveWaylandCrossWindowTabDrag(Some(
                WaylandCrossWindowTabDrag {
                    token_path: token.clone(),
                    source_window: window_a_handle,
                    source_view: WeakEntity::<AppView>::new_invalid(),
                    tab_id: TabId::new(11),
                    payload: DraggedTab {
                        source_tab_id: TabId::new(11),
                        source_index: 0,
                        source_pane_id: PaneId(1),
                        is_active: true,
                        title: "ssh".to_string(),
                        status_color: None,
                        ghost_shared: DraggedTab::new_ghost_shared(),
                    },
                    can_detach: true,
                    allow_pane_drop: true,
                    handled: false,
                },
            )));
        };

        cx.update_window(window_a_handle, |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .unwrap();
        cx.update_window(window_b_handle, |_, window, cx| {
            window.draw(cx).clear(cx);
            seed_drag(cx, &sentinel);
            window.dispatch_event(
                PlatformInput::FileDrop(FileDropEvent::Entered {
                    position: point(px(20.0), px(20.0)),
                    paths: unrelated_paths,
                }),
                cx,
            );
            assert!(!view_b.read(cx).incoming_hover);
            assert_eq!(view_b.read(cx).unrelated_moves, 1);
            assert_eq!(view_b.read(cx).sentinel_moves, 0);
            window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Exited), cx);

            window.dispatch_event(
                PlatformInput::FileDrop(FileDropEvent::Entered {
                    position: point(px(24.0), px(24.0)),
                    paths: sentinel_paths.clone(),
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::FileDrop(FileDropEvent::Pending {
                    position: point(px(30.0), px(24.0)),
                }),
                cx,
            );
            assert!(view_b.read(cx).incoming_hover);
            assert_eq!(view_b.read(cx).sentinel_moves, 2);

            window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Exited), cx);
            assert!(!view_b.read(cx).incoming_hover);

            // Re-enter and drop onto window B -> handled flag suppresses detach on Ended.
            window.dispatch_event(
                PlatformInput::FileDrop(FileDropEvent::Entered {
                    position: point(px(24.0), px(24.0)),
                    paths: sentinel_paths.clone(),
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::FileDrop(FileDropEvent::Submit {
                    position: point(px(24.0), px(24.0)),
                }),
                cx,
            );
            assert!(!view_b.read(cx).incoming_hover);
            assert_eq!(view_b.read(cx).handled_drops, 1);
        })
        .unwrap();

        cx.update_window(window_a_handle, |_, window, cx| {
            window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Ended), cx);
            assert_eq!(view_a.read(cx).detached_on_end, 0);
            assert!(cx.global::<ActiveWaylandCrossWindowTabDrag>().0.is_none());

            // When not handled by any target window, Ended triggers detach.
            seed_drag(cx, &sentinel);
            window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Ended), cx);
            assert_eq!(view_a.read(cx).detached_on_end, 1);
            assert!(cx.global::<ActiveWaylandCrossWindowTabDrag>().0.is_none());
        })
        .unwrap();
    }
}
