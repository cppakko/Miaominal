use super::pane::{
    PaneViewState, TerminalHoveredLink, TerminalLinkQuery, TerminalMouseGesture,
    TerminalPaneFreeTypeDropTarget, TerminalScrollbarDrag,
};
use super::{TerminalKeyboardState, TerminalMouseMotion};
use crate::ui::contracts::commands::AppCommand;
use crate::ui::contracts::{PaneId, TabId, TerminalWorkspaceRequest};
use crate::ui::features::session::SessionController;
use crate::ui::features::session::view::canvas::{
    TerminalScrollbarMetrics, terminal_scrollbar_metrics, terminal_scrollbar_offset_for_pointer,
};
#[cfg(test)]
use gpui_kit::px;
use gpui_kit::{
    App, Bounds, Context, Entity, KeyDownEvent, KeyUpEvent, ModifiersChangedEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, ScrollWheelEvent, Window,
};
use miaominal_terminal::{MouseEncoding, MouseProtocol, TerminalSelectionKind};
use std::time::{Duration, Instant};
const TERMINAL_SCROLLBAR_IDLE_HIDE_DELAY: Duration = Duration::from_millis(1200);
use crate::ui::i18n;
use alacritty_terminal::index::Side;
use miaominal_settings::TerminalRightClickBehavior;
use miaominal_terminal::TerminalFreeTypeTarget;

/// Window adapter for Session-owned input behavior. It exposes no catalog, tab registry,
/// business dialogs, or mutable Session resources.
pub(crate) trait TerminalInputHost: Render + Sized {
    fn terminal_owner(&self) -> Entity<SessionController>;
    fn active_terminal_tab_id(&self) -> Option<TabId>;
    fn active_terminal_pane_id(&self) -> PaneId;
    fn active_terminal_pane(&self) -> &PaneViewState;
    fn active_terminal_pane_mut(&mut self) -> &mut PaneViewState;
    fn terminal_pane_ids(&self) -> Vec<PaneId>;
    fn terminal_pane_presentation(&self, pane_id: PaneId) -> Option<TerminalPanePresentation>;
    fn for_each_parked_terminal_input(
        &mut self,
        visit: impl FnMut(&mut TerminalKeyboardState, &mut Option<TerminalMouseGesture>),
    );
    fn terminal_drag(&self) -> &TerminalDragState;
    fn terminal_drag_mut(&mut self) -> &mut TerminalDragState;
    fn set_terminal_scrollbar_interaction(&mut self, pane_id: PaneId, at: Instant) -> bool;
    fn dispatch_terminal_workspace_request(
        &mut self,
        request: TerminalWorkspaceRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
}

#[derive(Clone, Copy)]
#[non_exhaustive]
pub(crate) struct TerminalPanePresentation {
    pub(crate) tab_id: Option<TabId>,
    pub(crate) bounds: Option<Bounds<Pixels>>,
    pub(crate) cell_width: f32,
    pub(crate) line_height: f32,
    pub(crate) pointer_position: Option<Point<Pixels>>,
    pub(crate) dragging_scrollbar: bool,
    pub(crate) scrollbar_interaction: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct TerminalDragState {
    pub(crate) source_tab: Option<TabId>,
    pub(crate) source_pane: Option<PaneId>,
    pub(crate) drop_target: Option<TerminalPaneFreeTypeDropTarget>,
}

const TERMINAL_FREE_TYPE_DRAG_THRESHOLD: f32 = 4.0;

fn terminal_scrollbar_is_visible(
    last_interaction_at: Option<Instant>,
    pointer_over_track: bool,
    dragging_scrollbar: bool,
    now: Instant,
) -> bool {
    dragging_scrollbar
        || pointer_over_track
        || last_interaction_at.is_some_and(|interaction_at| {
            now.duration_since(interaction_at) < TERMINAL_SCROLLBAR_IDLE_HIDE_DELAY
        })
}

fn clamp_terminal_pointer_position(
    position: gpui_kit::Point<gpui_kit::Pixels>,
    bounds: gpui_kit::Bounds<gpui_kit::Pixels>,
) -> gpui_kit::Point<gpui_kit::Pixels> {
    let min_x = f32::from(bounds.origin.x);
    let max_x = min_x + f32::from(bounds.size.width);
    let min_y = f32::from(bounds.origin.y);
    let max_y = min_y + f32::from(bounds.size.height);

    gpui_kit::Point::new(
        gpui_kit::Pixels::from(f32::from(position.x).clamp(min_x, max_x)),
        gpui_kit::Pixels::from(f32::from(position.y).clamp(min_y, max_y)),
    )
}

fn terminal_drag_scroll_delta(
    position: gpui_kit::Point<gpui_kit::Pixels>,
    bounds: gpui_kit::Bounds<gpui_kit::Pixels>,
    line_height: f32,
) -> Option<i32> {
    let top = f32::from(bounds.origin.y);
    let bottom = top + f32::from(bounds.size.height);
    let pointer_y = f32::from(position.y);

    let scroll_lines = if pointer_y < top {
        let scroll_delta = (top - pointer_y).powf(1.1);
        (scroll_delta / line_height.max(1.0)).ceil() as i32
    } else if pointer_y > bottom {
        let scroll_delta = -((pointer_y - bottom).powf(1.1));
        (scroll_delta / line_height.max(1.0)).floor() as i32
    } else {
        return None;
    };

    Some(scroll_lines.clamp(-3, 3))
}

fn terminal_selection_side(
    position: Point<Pixels>,
    clamped_position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    cell_width: f32,
) -> Side {
    let left = f32::from(bounds.origin.x);
    let right = left + f32::from(bounds.size.width);
    let top = f32::from(bounds.origin.y);
    let bottom = top + f32::from(bounds.size.height);
    let pointer_x = f32::from(position.x);
    let pointer_y = f32::from(position.y);

    if pointer_x < left || pointer_y < top {
        return Side::Left;
    }
    if pointer_x > right || pointer_y > bottom {
        return Side::Right;
    }

    let clamped_x = f32::from(clamped_position.x);
    if clamped_x >= right {
        return Side::Right;
    }

    let cell_width = cell_width.max(1.0);
    let cell_x = (clamped_x - left).max(0.0) % cell_width;
    if cell_x > cell_width / 2.0 {
        Side::Right
    } else {
        Side::Left
    }
}

fn copy_terminal_selection<V: TerminalInputHost>(this: &mut V, cx: &mut Context<V>) -> bool {
    let Some(tab_id) = this.active_terminal_tab_id() else {
        return false;
    };
    let controller = this.terminal_owner().clone();
    controller.update(cx, |controller, cx| {
        controller.copy_terminal_selection(tab_id, cx)
    })
}

fn paste_into_terminal<V: TerminalInputHost>(this: &mut V, cx: &mut Context<V>) {
    let Some(tab_id) = this.active_terminal_tab_id() else {
        return;
    };
    let controller = this.terminal_owner().clone();
    controller.update(cx, |controller, cx| {
        controller.paste_terminal_clipboard(tab_id, cx)
    });
}

fn active_terminal_has_selection<V: TerminalInputHost>(this: &V, cx: &App) -> bool {
    this.active_terminal_tab_id().is_some_and(|tab_id| {
        this.terminal_owner()
            .read(cx)
            .terminal_has_selection(tab_id)
    })
}

fn terminal_pointer_drag_threshold_exceeded(
    origin: Point<Pixels>,
    position: Point<Pixels>,
) -> bool {
    let dx = f32::from(position.x) - f32::from(origin.x);
    let dy = f32::from(position.y) - f32::from(origin.y);
    dx * dx + dy * dy >= TERMINAL_FREE_TYPE_DRAG_THRESHOLD.powi(2)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalLeftMouseRoute {
    Application,
    FreeType,
    TraditionalSelection,
}

fn terminal_left_mouse_route(
    free_type_enabled: bool,
    mouse_reporting_enabled: bool,
    modifiers: &gpui_kit::Modifiers,
) -> TerminalLeftMouseRoute {
    if modifiers.shift {
        TerminalLeftMouseRoute::TraditionalSelection
    } else if free_type_enabled && (!mouse_reporting_enabled || modifiers.alt) {
        TerminalLeftMouseRoute::FreeType
    } else if mouse_reporting_enabled {
        TerminalLeftMouseRoute::Application
    } else {
        TerminalLeftMouseRoute::TraditionalSelection
    }
}

fn terminal_invalid_free_type_target_is_consumed(
    route: TerminalLeftMouseRoute,
    mouse_reporting_enabled: bool,
    modifiers: &gpui_kit::Modifiers,
) -> bool {
    route == TerminalLeftMouseRoute::FreeType && mouse_reporting_enabled && modifiers.alt
}

fn terminal_traditional_selection_is_block(
    free_type_enabled: bool,
    modifiers: &gpui_kit::Modifiers,
) -> bool {
    modifiers.alt && (!free_type_enabled || modifiers.shift)
}

fn terminal_mouse_motion_reporting_allowed(
    modifiers: &gpui_kit::Modifiers,
    free_type_gesture_active: bool,
) -> bool {
    !modifiers.shift && !free_type_gesture_active
}

fn terminal_link_open_requested(modifiers: &gpui_kit::Modifiers) -> bool {
    modifiers.control || modifiers.platform
}

fn reset_stale_terminal_pointer<V: TerminalInputHost>(this: &mut V) -> bool {
    let target = this.active_terminal_tab_id();
    let pane = this.active_terminal_pane_mut();
    let manipulating = pane.terminal_mouse_gesture.is_some()
        || pane.terminal_scrollbar_drag.is_some()
        || pane.terminal_mouse_reporting.is_active();
    if !manipulating || pane.terminal_pointer_target == target {
        return false;
    }
    pane.terminal_mouse_gesture = None;
    pane.terminal_scrollbar_drag = None;
    pane.terminal_mouse_reporting.reset();
    pane.terminal_pointer_target = None;
    true
}

fn clear_terminal_keyboard(keyboard: &mut TerminalKeyboardState) -> bool {
    keyboard.reset()
}

#[derive(Clone, Copy)]
struct TerminalPaneMetrics {
    tab_id: TabId,
    bounds: Bounds<Pixels>,
    cell_width: f32,
    line_height: f32,
}

fn terminal_pane_metrics<V: TerminalInputHost>(
    this: &V,
    pane_id: PaneId,
) -> Option<TerminalPaneMetrics> {
    let pane = this.terminal_pane_presentation(pane_id)?;
    Some(TerminalPaneMetrics {
        tab_id: pane.tab_id?,
        bounds: pane.bounds?,
        cell_width: pane.cell_width,
        line_height: pane.line_height,
    })
}

fn terminal_pane_at_position<V: TerminalInputHost>(
    this: &V,
    position: Point<Pixels>,
) -> Option<PaneId> {
    this.terminal_pane_ids().into_iter().find(|id| {
        this.terminal_pane_presentation(*id)
            .and_then(|pane| pane.bounds)
            .is_some_and(|bounds| bounds.contains(&position))
    })
}

fn terminal_position_to_cell_and_side_for_pane<V: TerminalInputHost>(
    this: &V,
    pane_id: PaneId,
    position: Point<Pixels>,
    clamp_to_bounds: bool,
    cx: &App,
) -> Option<(i32, usize, Side)> {
    let metrics = terminal_pane_metrics(this, pane_id)?;
    let position = if clamp_to_bounds {
        clamp_terminal_pointer_position(position, metrics.bounds)
    } else {
        metrics.bounds.contains(&position).then_some(position)?
    };
    let viewport = this
        .terminal_owner()
        .read(cx)
        .terminal_viewport_state(metrics.tab_id)?;
    let rel_x = (f32::from(position.x) - f32::from(metrics.bounds.origin.x)).max(0.0);
    let rel_y = (f32::from(position.y) - f32::from(metrics.bounds.origin.y)).max(0.0);
    let cell_width = metrics.cell_width.max(1.0);
    let line_height = metrics.line_height.max(1.0);
    let side = terminal_selection_side(position, position, metrics.bounds, cell_width);
    let column = ((rel_x / cell_width).floor() as i32).max(0) as usize;
    let column = column.min(viewport.columns.saturating_sub(1));
    let row = (rel_y / line_height).floor() as i32;
    let row = row.min(viewport.screen_lines.saturating_sub(1) as i32);
    Some((row - viewport.display_offset as i32, column, side))
}

fn terminal_free_type_drop_target_at_position<V: TerminalInputHost>(
    this: &V,
    position: Point<Pixels>,
    cx: &App,
) -> Option<TerminalPaneFreeTypeDropTarget> {
    let pane_id = terminal_pane_at_position(this, position)?;
    let metrics = terminal_pane_metrics(this, pane_id)?;
    let (line, column, side) =
        terminal_position_to_cell_and_side_for_pane(this, pane_id, position, false, cx)?;
    let target = this.terminal_owner().read(cx).terminal_free_type_target(
        metrics.tab_id,
        line,
        column,
        side,
    )?;
    Some(TerminalPaneFreeTypeDropTarget {
        pane_id,
        tab_id: metrics.tab_id,
        target,
    })
}

fn terminal_free_type_drop_target_for_drag<V: TerminalInputHost>(
    this: &V,
    source_pane_id: PaneId,
    position: Point<Pixels>,
    cx: &App,
) -> Option<TerminalPaneFreeTypeDropTarget> {
    if terminal_pane_at_position(this, position).is_some() {
        return terminal_free_type_drop_target_at_position(this, position, cx);
    }

    let metrics = terminal_pane_metrics(this, source_pane_id)?;
    let (line, column, side) =
        terminal_position_to_cell_and_side_for_pane(this, source_pane_id, position, true, cx)?;
    let target = this.terminal_owner().read(cx).terminal_free_type_target(
        metrics.tab_id,
        line,
        column,
        side,
    )?;
    Some(TerminalPaneFreeTypeDropTarget {
        pane_id: source_pane_id,
        tab_id: metrics.tab_id,
        target,
    })
}

fn split_terminal_free_type_drop_target(
    source_pane_id: PaneId,
    drop_target: Option<TerminalPaneFreeTypeDropTarget>,
) -> (
    Option<TerminalFreeTypeTarget>,
    Option<TerminalPaneFreeTypeDropTarget>,
) {
    match drop_target {
        Some(drop) if drop.pane_id == source_pane_id => (Some(drop.target), None),
        Some(drop) => (None, Some(drop)),
        None => (None, None),
    }
}

fn compute_free_type_drop_target<V: TerminalInputHost>(
    this: &V,
    source_pane_id: PaneId,
    position: Point<Pixels>,
    cx: &App,
) -> (
    Option<TerminalFreeTypeTarget>,
    Option<TerminalPaneFreeTypeDropTarget>,
) {
    split_terminal_free_type_drop_target(
        source_pane_id,
        terminal_free_type_drop_target_for_drag(this, source_pane_id, position, cx),
    )
}

fn terminal_free_type_selection_kind(click_count: usize) -> Option<TerminalSelectionKind> {
    match click_count {
        1 => None,
        2 => Some(TerminalSelectionKind::Semantic),
        _ => Some(TerminalSelectionKind::Lines),
    }
}

fn terminal_copy_selection_kind(click_count: usize, block: bool) -> TerminalSelectionKind {
    if block {
        return TerminalSelectionKind::Block;
    }

    match click_count {
        2 => TerminalSelectionKind::Semantic,
        3.. => TerminalSelectionKind::Lines,
        _ => TerminalSelectionKind::Simple,
    }
}

fn clear_terminal_selection<V: TerminalInputHost>(this: &mut V, cx: &mut Context<V>) {
    let Some(tab_id) = this.active_terminal_tab_id() else {
        return;
    };
    if this
        .terminal_owner()
        .read(cx)
        .clear_terminal_selection(tab_id)
    {
        cx.notify();
    }
}

fn active_terminal_mouse_mode<V: TerminalInputHost>(
    this: &V,
    cx: &App,
) -> Option<(MouseProtocol, MouseEncoding)> {
    let tab_id = this.active_terminal_tab_id()?;
    this.terminal_owner().read(cx).terminal_mouse_mode(tab_id)
}

pub(crate) trait WorkspaceTerminalInputExt: Sized {
    fn clear_terminal_free_type_interactions(&mut self, cx: &mut Context<Self>);
    fn handle_terminal_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    );

    fn handle_terminal_key_up(
        &mut self,
        event: &KeyUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    );

    fn handle_terminal_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        line_height: f32,
        cx: &mut Context<Self>,
    );

    fn handle_terminal_mouse_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>);

    fn handle_terminal_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>);

    fn handle_terminal_mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>);

    fn handle_terminal_hover(&mut self, hovered: bool, cx: &mut Context<Self>);

    fn handle_terminal_modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        cx: &mut Context<Self>,
    );

    fn handle_terminal_middle_click(&mut self, cx: &mut Context<Self>);

    fn handle_terminal_secondary_click_action(&mut self, cx: &mut Context<Self>);

    fn terminal_originated_selection_drag_active(&self) -> bool;

    fn start_terminal_originated_selection_drag(&mut self, cx: &mut Context<Self>);

    fn clear_terminal_originated_selection_drag(&mut self, cx: &mut Context<Self>);

    fn defer_clear_terminal_originated_selection_drag(&mut self, cx: &mut Context<Self>);

    fn event_position_to_cell_and_side(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<(i32, usize, Side)>;

    fn event_position_to_cell_and_side_clamped(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<(i32, usize, Side)>;

    fn position_to_cell_and_side(
        &self,
        position: Point<Pixels>,
        clamp_to_bounds: bool,
        cx: &App,
    ) -> Option<(i32, usize, Side)>;

    fn event_position_to_viewport_cell(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<(usize, usize)>;

    fn event_position_to_viewport_cell_clamped(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<(usize, usize)>;

    fn position_to_viewport_cell(
        &self,
        position: Point<Pixels>,
        clamp_to_bounds: bool,
        cx: &App,
    ) -> Option<(usize, usize)>;

    fn terminal_link_at_position(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<TerminalHoveredLink>;

    fn terminal_link_at_cell(
        &self,
        line: usize,
        column: usize,
        cx: &App,
    ) -> Option<TerminalHoveredLink>;

    fn set_terminal_hover_state(
        &mut self,
        position: Option<Point<Pixels>>,
        open_modifier: bool,
        cx: &mut Context<Self>,
    );

    fn touch_terminal_scrollbar_visibility(&mut self, pane_id: PaneId, cx: &mut Context<Self>);

    fn terminal_scrollbar_visible(&self, pane_id: PaneId, cx: &App) -> bool;

    fn rebind_terminal_focus_reporting(&mut self, window: &mut Window, cx: &mut Context<Self>);

    fn sync_terminal_focus_reporting(&mut self, window: &mut Window, cx: &mut Context<Self>);

    fn clear_terminal_focus_reporting(&mut self, cx: &mut Context<Self>);

    fn release_reported_terminal_focus(&mut self, cx: &mut Context<Self>);

    fn current_terminal_focus_report_target(&self, window: &Window, cx: &App) -> Option<TabId>;

    fn terminal_scrollbar_metrics(&self, cx: &App) -> Option<TerminalScrollbarMetrics>;

    fn terminal_scrollbar_metrics_for_pane(
        &self,
        pane_id: PaneId,
        cx: &App,
    ) -> Option<TerminalScrollbarMetrics>;
}

impl<V: TerminalInputHost> WorkspaceTerminalInputExt for V {
    fn clear_terminal_free_type_interactions(&mut self, cx: &mut Context<Self>) {
        let active_was_free_type = self
            .active_terminal_pane()
            .terminal_mouse_gesture
            .is_some_and(TerminalMouseGesture::is_free_type);
        if active_was_free_type {
            self.active_terminal_pane_mut().terminal_mouse_gesture = None;
        }
        let mut changed = active_was_free_type;
        changed |= clear_terminal_keyboard(&mut self.active_terminal_pane_mut().terminal_keyboard);

        self.for_each_parked_terminal_input(|keyboard, gesture| {
            if gesture.is_some_and(TerminalMouseGesture::is_free_type) {
                *gesture = None;
                changed = true;
            }
            changed |= clear_terminal_keyboard(keyboard);
        });

        if active_was_free_type {
            self.clear_terminal_originated_selection_drag(cx);
        }
        if changed {
            cx.notify();
        }
    }

    fn handle_terminal_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let mut captured = self
            .active_terminal_pane()
            .terminal_keyboard
            .pressed_target(key);
        if event.is_held && captured.is_none() {
            self.for_each_parked_terminal_input(|keyboard, _| {
                captured = captured.or_else(|| keyboard.pressed_target(key));
            });
        }
        if !event.is_held {
            self.for_each_parked_terminal_input(|keyboard, _| {
                keyboard.take_release_target(key);
            });
        }
        let tab_id = if event.is_held && captured.is_some() {
            captured
        } else {
            self.active_terminal_tab_id().map(|id| {
                self.active_terminal_pane_mut()
                    .terminal_keyboard
                    .record_press(key, id, event.is_held)
            })
        };
        let dispatch = self.terminal_owner().update(cx, |controller, cx| {
            controller.route_terminal_key_down(tab_id, event, cx)
        });
        if dispatch.scrolled() {
            self.touch_terminal_scrollbar_visibility(self.active_terminal_pane_id(), cx);
        }
        if let Some(request) = dispatch.request() {
            self.dispatch_terminal_workspace_request(request, window, cx);
        }
        // A local command can activate another pane: suppress its release in the resulting pane.
        if dispatch.suppress_release() {
            self.active_terminal_pane_mut()
                .terminal_keyboard
                .take_release_target(event.keystroke.key.as_str());
            self.for_each_parked_terminal_input(|keyboard, _| {
                keyboard.take_release_target(event.keystroke.key.as_str());
            });
            self.active_terminal_pane_mut()
                .terminal_keyboard
                .suppress_release(event.keystroke.key.as_str());
        }
        if dispatch.preserve_focus() {
            window.focus(&self.active_terminal_pane().terminal_focus, cx);
            cx.stop_propagation();
        }
    }

    fn handle_terminal_key_up(
        &mut self,
        event: &KeyUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut tab_id = self
            .active_terminal_pane_mut()
            .terminal_keyboard
            .take_release_target(event.keystroke.key.as_str());
        self.for_each_parked_terminal_input(|keyboard, _| {
            let parked_target = keyboard.take_release_target(event.keystroke.key.as_str());
            tab_id = tab_id.or(parked_target);
        });
        let preserve_focus = self.terminal_owner().update(cx, |controller, cx| {
            controller.route_terminal_key_up(
                tab_id,
                event,
                &mut self.active_terminal_pane_mut().terminal_keyboard,
                cx,
            )
        });
        if preserve_focus {
            window.focus(&self.active_terminal_pane().terminal_focus, cx);
            cx.stop_propagation();
        }
    }

    fn handle_terminal_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        line_height: f32,
        cx: &mut Context<Self>,
    ) {
        let tab_id = self.active_terminal_tab_id();
        let viewport_cell = self.event_position_to_viewport_cell(event.position, cx);
        let scrolled = self.terminal_owner().update(cx, |controller, cx| {
            controller.route_terminal_wheel(tab_id, event, line_height, viewport_cell, cx)
        });
        if scrolled {
            self.touch_terminal_scrollbar_visibility(self.active_terminal_pane_id(), cx);
        }
    }

    fn handle_terminal_mouse_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        self.clear_terminal_originated_selection_drag(cx);
        let target = self.active_terminal_tab_id();
        self.active_terminal_pane_mut().terminal_pointer_target = target;
        if event.button == MouseButton::Left {
            gpui_kit::component::GlobalState::suppress_text_selection(cx);
        }

        if event.button == MouseButton::Left
            && let Some(metrics) = self.terminal_scrollbar_metrics(cx)
            && metrics.track_bounds.contains(&event.position)
        {
            let thumb_grab_offset = if metrics.thumb_bounds.contains(&event.position) {
                (f32::from(event.position.y) - f32::from(metrics.thumb_bounds.origin.y))
                    .clamp(0.0, f32::from(metrics.thumb_bounds.size.height))
            } else {
                f32::from(metrics.thumb_bounds.size.height) / 2.0
            };
            let target_offset = terminal_scrollbar_offset_for_pointer(
                &metrics,
                event.position.y,
                thumb_grab_offset,
            );

            let Some(tab_id) = self.active_terminal_tab_id() else {
                return;
            };
            if !self
                .terminal_owner()
                .read(cx)
                .scroll_terminal_to_display_offset(tab_id, target_offset)
            {
                return;
            }
            self.active_terminal_pane_mut().terminal_mouse_gesture = None;
            self.set_terminal_hover_state(None, false, cx);
            self.active_terminal_pane_mut().terminal_scrollbar_drag =
                Some(TerminalScrollbarDrag { thumb_grab_offset });
            self.touch_terminal_scrollbar_visibility(self.active_terminal_pane_id(), cx);
            cx.notify();
            return;
        }

        let free_type_enabled = miaominal_settings::current_settings().terminal_free_type_mode;
        let mouse_reporting_enabled =
            active_terminal_mouse_mode(self, cx).is_some_and(|(protocol, _)| protocol.is_enabled());
        let left_mouse_route = (event.button == MouseButton::Left).then(|| {
            terminal_left_mouse_route(free_type_enabled, mouse_reporting_enabled, &event.modifiers)
        });

        if event.button == MouseButton::Left
            && terminal_link_open_requested(&event.modifiers)
            && let Some(link) = self.terminal_link_at_position(event.position, cx)
        {
            let uri = link.uri.clone();
            let message = if let Err(error) = open::that(uri.as_ref()) {
                let error = error.to_string();
                i18n::string_args(
                    "session.terminal_messages.failed_to_open_link",
                    &[("uri", uri.as_ref()), ("error", &error)],
                )
            } else {
                i18n::string_args(
                    "session.terminal_messages.opened_link",
                    &[("uri", uri.as_ref())],
                )
            };
            self.terminal_owner().update(cx, |owner, cx| {
                owner.emit(AppCommand::Feedback(message), cx);
                cx.notify();
            });
            return;
        }

        if event.button == MouseButton::Left
            && event.click_count == 1
            && left_mouse_route == Some(TerminalLeftMouseRoute::FreeType)
            && let Some((line, column, _side)) =
                self.event_position_to_cell_and_side(event.position, cx)
            && let Some(tab_id) = self.active_terminal_tab_id()
            && self
                .terminal_owner()
                .read(cx)
                .terminal_selection_contains(tab_id, line, column)
        {
            self.active_terminal_pane_mut().terminal_mouse_gesture =
                Some(TerminalMouseGesture::FreeTypePendingDrop {
                    origin: event.position,
                });
            self.start_terminal_originated_selection_drag(cx);
            self.set_terminal_hover_state(None, false, cx);
            self.active_terminal_pane_mut().terminal_scrollbar_drag = None;
            cx.notify();
            return;
        }

        if !event.modifiers.shift
            && (event.button != MouseButton::Left
                || left_mouse_route == Some(TerminalLeftMouseRoute::Application))
        {
            let tab_id = self.active_terminal_tab_id();
            let cell = self.event_position_to_viewport_cell(event.position, cx);
            let reported = self.terminal_owner().update(cx, |controller, cx| {
                controller.report_terminal_mouse_press(
                    tab_id,
                    event,
                    cell,
                    &mut self.active_terminal_pane_mut().terminal_mouse_reporting,
                    cx,
                )
            });
            if reported {
                self.active_terminal_pane_mut().terminal_mouse_gesture = None;
                self.set_terminal_hover_state(None, false, cx);
                self.active_terminal_pane_mut().terminal_scrollbar_drag = None;
                return;
            }
        }

        match event.button {
            MouseButton::Left => {
                let Some((line, column, side)) =
                    self.event_position_to_cell_and_side(event.position, cx)
                else {
                    return;
                };
                let Some(tab_id) = self.active_terminal_tab_id() else {
                    return;
                };

                if left_mouse_route == Some(TerminalLeftMouseRoute::FreeType) {
                    if let Some(target) = self
                        .terminal_owner()
                        .read(cx)
                        .terminal_free_type_target(tab_id, line, column, side)
                    {
                        let selection_kind = terminal_free_type_selection_kind(event.click_count);
                        let gesture = if let Some(kind) = selection_kind {
                            if !self
                                .terminal_owner()
                                .read(cx)
                                .start_terminal_free_type_selection(tab_id, target, kind)
                            {
                                return;
                            }
                            TerminalMouseGesture::FreeTypeSelection
                        } else {
                            TerminalMouseGesture::FreeTypePendingCursor {
                                origin: event.position,
                                target,
                            }
                        };
                        self.active_terminal_pane_mut().terminal_mouse_gesture = Some(gesture);
                        self.start_terminal_originated_selection_drag(cx);
                        self.set_terminal_hover_state(None, false, cx);
                        self.active_terminal_pane_mut().terminal_scrollbar_drag = None;
                        cx.notify();
                        return;
                    }

                    if terminal_invalid_free_type_target_is_consumed(
                        TerminalLeftMouseRoute::FreeType,
                        mouse_reporting_enabled,
                        &event.modifiers,
                    ) {
                        self.active_terminal_pane_mut().terminal_mouse_gesture = None;
                        self.set_terminal_hover_state(None, false, cx);
                        self.active_terminal_pane_mut().terminal_scrollbar_drag = None;
                        return;
                    }
                }

                let block =
                    terminal_traditional_selection_is_block(free_type_enabled, &event.modifiers);
                if !self.terminal_owner().read(cx).start_terminal_selection(
                    tab_id,
                    line,
                    column,
                    side,
                    terminal_copy_selection_kind(event.click_count, block),
                ) {
                    return;
                }
                self.active_terminal_pane_mut().terminal_mouse_gesture =
                    Some(TerminalMouseGesture::TraditionalSelection);
                self.start_terminal_originated_selection_drag(cx);
                self.set_terminal_hover_state(None, false, cx);
                self.active_terminal_pane_mut().terminal_scrollbar_drag = None;
                cx.notify();
            }
            MouseButton::Middle => {
                self.handle_terminal_middle_click(cx);
            }
            _ => {}
        }
    }

    fn handle_terminal_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if reset_stale_terminal_pointer(self) {
            self.clear_terminal_originated_selection_drag(cx);
            self.set_terminal_hover_state(None, false, cx);
            cx.notify();
            return;
        }
        let open_modifier = event.modifiers.control || event.modifiers.platform;

        if let Some(drag) = self.active_terminal_pane().terminal_scrollbar_drag {
            self.set_terminal_hover_state(None, open_modifier, cx);
            let Some(metrics) = self.terminal_scrollbar_metrics(cx) else {
                self.active_terminal_pane_mut().terminal_scrollbar_drag = None;
                return;
            };
            let target_offset = terminal_scrollbar_offset_for_pointer(
                &metrics,
                event.position.y,
                drag.thumb_grab_offset,
            );

            let Some(tab_id) = self.active_terminal_tab_id() else {
                self.active_terminal_pane_mut().terminal_scrollbar_drag = None;
                return;
            };
            if !self
                .terminal_owner()
                .read(cx)
                .scroll_terminal_to_display_offset(tab_id, target_offset)
            {
                self.active_terminal_pane_mut().terminal_scrollbar_drag = None;
                return;
            }
            cx.notify();
            return;
        }

        if event.pressed_button == Some(MouseButton::Left)
            && self.active_terminal_pane().terminal_mouse_gesture.is_none()
            && !self
                .active_terminal_pane()
                .terminal_mouse_reporting
                .is_active()
        {
            self.set_terminal_hover_state(None, open_modifier, cx);
            return;
        }

        if self.active_terminal_pane().terminal_mouse_gesture.is_none()
            && !self
                .active_terminal_pane()
                .terminal_mouse_reporting
                .is_active()
        {
            self.set_terminal_hover_state(Some(event.position), open_modifier, cx);
        } else {
            self.set_terminal_hover_state(None, open_modifier, cx);
        }

        let free_type_gesture_active = self
            .active_terminal_pane()
            .terminal_mouse_gesture
            .is_some_and(TerminalMouseGesture::is_free_type);
        if terminal_mouse_motion_reporting_allowed(&event.modifiers, free_type_gesture_active) {
            let tab_id = self.active_terminal_tab_id();
            let cell = self.event_position_to_viewport_cell_clamped(event.position, cx);
            let motion = self.terminal_owner().update(cx, |controller, cx| {
                controller.report_terminal_mouse_motion(
                    tab_id,
                    event,
                    cell,
                    &mut self.active_terminal_pane_mut().terminal_mouse_reporting,
                    cx,
                )
            });
            match motion {
                TerminalMouseMotion::Reported => {
                    self.active_terminal_pane_mut().terminal_mouse_gesture = None;
                    self.clear_terminal_originated_selection_drag(cx);
                    return;
                }
                TerminalMouseMotion::Unchanged => return,
                TerminalMouseMotion::Ignored => {}
            }
        }

        let Some(gesture) = self.active_terminal_pane().terminal_mouse_gesture else {
            return;
        };

        let Some(tab_id) = self.active_terminal_tab_id() else {
            return;
        };

        match gesture {
            TerminalMouseGesture::TraditionalSelection => {
                let drag_scroll_delta =
                    self.active_terminal_pane()
                        .terminal_bounds
                        .and_then(|bounds| {
                            terminal_drag_scroll_delta(
                                event.position,
                                bounds,
                                self.active_terminal_pane().terminal_line_height,
                            )
                        });
                if let Some(delta) = drag_scroll_delta
                    && !self
                        .terminal_owner()
                        .read(cx)
                        .scroll_terminal_display_offset_by(tab_id, delta)
                {
                    return;
                }
                let Some((line, column, side)) =
                    self.event_position_to_cell_and_side_clamped(event.position, cx)
                else {
                    return;
                };
                self.terminal_owner()
                    .read(cx)
                    .update_terminal_selection(tab_id, line, column, side);
            }
            TerminalMouseGesture::FreeTypePendingCursor { origin, target } => {
                if !terminal_pointer_drag_threshold_exceeded(origin, event.position) {
                    return;
                }
                if !self
                    .terminal_owner()
                    .read(cx)
                    .start_terminal_free_type_selection(
                        tab_id,
                        target,
                        TerminalSelectionKind::Simple,
                    )
                {
                    self.active_terminal_pane_mut().terminal_mouse_gesture = None;
                    self.clear_terminal_originated_selection_drag(cx);
                    return;
                }
                self.active_terminal_pane_mut().terminal_mouse_gesture =
                    Some(TerminalMouseGesture::FreeTypeSelection);
                if let Some((line, column, side)) =
                    self.event_position_to_cell_and_side_clamped(event.position, cx)
                {
                    self.terminal_owner()
                        .read(cx)
                        .update_terminal_selection(tab_id, line, column, side);
                }
            }
            TerminalMouseGesture::FreeTypeSelection => {
                let Some((line, column, side)) =
                    self.event_position_to_cell_and_side_clamped(event.position, cx)
                else {
                    return;
                };
                self.terminal_owner()
                    .read(cx)
                    .update_terminal_selection(tab_id, line, column, side);
            }
            TerminalMouseGesture::FreeTypePendingDrop { origin, .. } => {
                if !terminal_pointer_drag_threshold_exceeded(origin, event.position) {
                    return;
                }
                let source_pane_id = self.active_terminal_pane_id();
                let (target, cross_pane_target) =
                    compute_free_type_drop_target(self, source_pane_id, event.position, cx);
                self.terminal_drag_mut().drop_target = cross_pane_target;
                self.active_terminal_pane_mut().terminal_mouse_gesture =
                    Some(TerminalMouseGesture::FreeTypeDrop { target });
            }
            TerminalMouseGesture::FreeTypeDrop { target: previous } => {
                let source_pane_id = self.active_terminal_pane_id();
                let (target, cross_pane_target) =
                    compute_free_type_drop_target(self, source_pane_id, event.position, cx);
                if target == previous && cross_pane_target == self.terminal_drag().drop_target {
                    return;
                }
                self.terminal_drag_mut().drop_target = cross_pane_target;
                self.active_terminal_pane_mut().terminal_mouse_gesture =
                    Some(TerminalMouseGesture::FreeTypeDrop { target });
            }
        }
        cx.notify();
    }

    fn handle_terminal_mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if reset_stale_terminal_pointer(self) {
            self.clear_terminal_originated_selection_drag(cx);
            self.set_terminal_hover_state(None, false, cx);
            cx.notify();
            return;
        }
        if self
            .active_terminal_pane_mut()
            .terminal_scrollbar_drag
            .take()
            .is_some()
        {
            self.touch_terminal_scrollbar_visibility(self.active_terminal_pane_id(), cx);
            cx.notify();
            return;
        }

        let tab_id = self.active_terminal_tab_id();
        let cell = self.event_position_to_viewport_cell_clamped(event.position, cx);
        let reported = self.terminal_owner().update(cx, |controller, cx| {
            controller.report_terminal_mouse_release(
                tab_id,
                event,
                cell,
                &mut self.active_terminal_pane_mut().terminal_mouse_reporting,
                cx,
            )
        });
        if reported {
            self.active_terminal_pane_mut().terminal_mouse_gesture = None;
            self.clear_terminal_originated_selection_drag(cx);
            return;
        }

        if event.button == MouseButton::Right {
            let settings = miaominal_settings::current_settings();
            let force_context_menu =
                settings.terminal_shift_right_click_context_menu && event.modifiers.shift;
            if !force_context_menu
                && settings.terminal_right_click_behavior
                    == TerminalRightClickBehavior::CopySelectionOrPaste
            {
                self.handle_terminal_secondary_click_action(cx);
            }
            self.set_terminal_hover_state(
                Some(event.position),
                event.modifiers.control || event.modifiers.platform,
                cx,
            );
            return;
        }

        if event.button != MouseButton::Left {
            self.set_terminal_hover_state(
                Some(event.position),
                event.modifiers.control || event.modifiers.platform,
                cx,
            );
            self.clear_terminal_originated_selection_drag(cx);
            return;
        }
        let Some(gesture) = self
            .active_terminal_pane_mut()
            .terminal_mouse_gesture
            .take()
        else {
            self.set_terminal_hover_state(
                Some(event.position),
                event.modifiers.control || event.modifiers.platform,
                cx,
            );
            self.clear_terminal_originated_selection_drag(cx);
            return;
        };
        self.defer_clear_terminal_originated_selection_drag(cx);

        let Some(tab_id) = self.active_terminal_tab_id() else {
            return;
        };
        let cross_pane_drop_target = self.terminal_drag_mut().drop_target.take();

        match gesture {
            TerminalMouseGesture::TraditionalSelection => {
                let has_selection = self
                    .terminal_owner()
                    .read(cx)
                    .terminal_has_selection(tab_id);
                if !has_selection {
                    clear_terminal_selection(self, cx);
                }
            }
            TerminalMouseGesture::FreeTypePendingCursor { target, .. } => {
                let controller = self.terminal_owner().clone();
                controller.update(cx, |controller, cx| {
                    controller.send_terminal_free_type_cursor(tab_id, target, cx);
                });
            }
            TerminalMouseGesture::FreeTypePendingDrop { .. } => {}
            TerminalMouseGesture::FreeTypeSelection => {
                if !self
                    .terminal_owner()
                    .read(cx)
                    .terminal_has_free_type_selection(tab_id)
                    && let Some((line, column, side)) =
                        self.event_position_to_cell_and_side_clamped(event.position, cx)
                    && let Some(target) = self
                        .terminal_owner()
                        .read(cx)
                        .terminal_free_type_target(tab_id, line, column, side)
                {
                    let controller = self.terminal_owner().clone();
                    controller.update(cx, |controller, cx| {
                        controller.send_terminal_free_type_cursor(tab_id, target, cx);
                    });
                }
            }
            TerminalMouseGesture::FreeTypeDrop {
                target: Some(target),
            } => {
                let duplicate = event.modifiers.control || event.modifiers.platform;
                let controller = self.terminal_owner().clone();
                controller.update(cx, |controller, cx| {
                    controller.send_terminal_selection_drop(tab_id, target, duplicate, cx);
                });
            }
            TerminalMouseGesture::FreeTypeDrop { target: None } => {
                if let Some(drop) = cross_pane_drop_target
                    && let Some(target_tab_id) = terminal_pane_metrics(self, drop.pane_id)
                        .filter(|metrics| metrics.tab_id == drop.tab_id)
                        .map(|metrics| metrics.tab_id)
                {
                    let duplicate = event.modifiers.control || event.modifiers.platform;
                    let controller = self.terminal_owner().clone();
                    controller.update(cx, |controller, cx| {
                        controller.send_terminal_selection_cross_pane_drop(
                            tab_id,
                            target_tab_id,
                            drop.target,
                            duplicate,
                            cx,
                        );
                    });
                }
            }
        }
        self.set_terminal_hover_state(
            Some(event.position),
            event.modifiers.control || event.modifiers.platform,
            cx,
        );
        cx.notify();
    }

    fn handle_terminal_hover(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            return;
        }

        self.set_terminal_hover_state(None, false, cx);
    }

    fn handle_terminal_modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        cx: &mut Context<Self>,
    ) {
        self.set_terminal_hover_state(
            self.active_terminal_pane().terminal_pointer_position,
            event.modifiers.control || event.modifiers.platform,
            cx,
        );
    }

    fn handle_terminal_middle_click(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.active_terminal_tab_id() else {
            return;
        };
        let controller = self.terminal_owner().clone();
        controller.update(cx, |controller, cx| {
            controller.paste_terminal_selection_or_clipboard(tab_id, cx)
        });
    }

    fn handle_terminal_secondary_click_action(&mut self, cx: &mut Context<Self>) {
        if active_terminal_has_selection(self, cx) {
            if copy_terminal_selection(self, cx) {
                clear_terminal_selection(self, cx);
            }
        } else {
            paste_into_terminal(self, cx);
        }
    }

    fn terminal_originated_selection_drag_active(&self) -> bool {
        self.terminal_drag().source_pane.is_some()
    }

    fn start_terminal_originated_selection_drag(&mut self, cx: &mut Context<Self>) {
        let pane_id = self.active_terminal_pane_id();
        let tab_id = self.active_terminal_tab_id();
        let target_changed = self.terminal_drag_mut().drop_target.take().is_some();
        if self.terminal_drag().source_pane != Some(pane_id)
            || self.terminal_drag().source_tab != tab_id
            || target_changed
        {
            self.terminal_drag_mut().source_pane = Some(pane_id);
            self.terminal_drag_mut().source_tab = tab_id;
            cx.notify();
        }
    }

    fn clear_terminal_originated_selection_drag(&mut self, cx: &mut Context<Self>) {
        let source_cleared = self.terminal_drag_mut().source_pane.take().is_some();
        self.terminal_drag_mut().source_tab = None;
        let target_cleared = self.terminal_drag_mut().drop_target.take().is_some();
        if source_cleared || target_cleared {
            cx.notify();
        }
    }

    fn defer_clear_terminal_originated_selection_drag(&mut self, cx: &mut Context<Self>) {
        let Some(pane_id) = self.terminal_drag().source_pane else {
            return;
        };

        let tab_id = self.terminal_drag().source_tab;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(0))
                .await;

            this.update(cx, |this, cx| {
                if this.terminal_drag().source_pane == Some(pane_id)
                    && this.terminal_drag().source_tab == tab_id
                    && this.active_terminal_pane().terminal_mouse_gesture.is_none()
                {
                    this.clear_terminal_originated_selection_drag(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn event_position_to_cell_and_side(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<(i32, usize, Side)> {
        self.position_to_cell_and_side(position, false, cx)
    }

    fn event_position_to_cell_and_side_clamped(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<(i32, usize, Side)> {
        self.position_to_cell_and_side(position, true, cx)
    }

    fn position_to_cell_and_side(
        &self,
        position: Point<Pixels>,
        clamp_to_bounds: bool,
        cx: &App,
    ) -> Option<(i32, usize, Side)> {
        terminal_position_to_cell_and_side_for_pane(
            self,
            self.active_terminal_pane_id(),
            position,
            clamp_to_bounds,
            cx,
        )
    }

    fn event_position_to_viewport_cell(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<(usize, usize)> {
        self.position_to_viewport_cell(position, false, cx)
    }

    fn event_position_to_viewport_cell_clamped(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<(usize, usize)> {
        self.position_to_viewport_cell(position, true, cx)
    }

    fn position_to_viewport_cell(
        &self,
        position: Point<Pixels>,
        clamp_to_bounds: bool,
        cx: &App,
    ) -> Option<(usize, usize)> {
        let bounds = self.active_terminal_pane().terminal_bounds?;
        let position = if clamp_to_bounds {
            clamp_terminal_pointer_position(position, bounds)
        } else {
            if !bounds.contains(&position) {
                return None;
            }
            position
        };

        let tab_id = self.active_terminal_tab_id()?;
        let viewport = self
            .terminal_owner()
            .read(cx)
            .terminal_viewport_state(tab_id)?;
        let columns = viewport.columns;
        let screen_lines = viewport.screen_lines;

        let rel_x = (f32::from(position.x) - f32::from(bounds.origin.x)).max(0.0);
        let rel_y = (f32::from(position.y) - f32::from(bounds.origin.y)).max(0.0);
        let cell_width = self.active_terminal_pane().terminal_cell_width.max(1.0);
        let line_height = self.active_terminal_pane().terminal_line_height.max(1.0);

        let column = (rel_x / cell_width).floor() as usize;
        let column = column.min(columns.saturating_sub(1));
        let line = (rel_y / line_height).floor() as usize;
        let line = line.min(screen_lines.saturating_sub(1));
        Some((line, column))
    }

    fn terminal_link_at_position(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<TerminalHoveredLink> {
        let (line, column) = self.event_position_to_viewport_cell(position, cx)?;
        self.terminal_link_at_cell(line, column, cx)
    }

    fn terminal_link_at_cell(
        &self,
        line: usize,
        column: usize,
        cx: &App,
    ) -> Option<TerminalHoveredLink> {
        let tab_id = self.active_terminal_tab_id()?;
        let link = self
            .terminal_owner()
            .read(cx)
            .terminal_link_at(tab_id, line, column)?;

        Some(TerminalHoveredLink {
            tab_id,
            line,
            start_column: link.start_column,
            end_column: link.end_column,
            uri: link.uri,
        })
    }

    fn set_terminal_hover_state(
        &mut self,
        position: Option<Point<Pixels>>,
        open_modifier: bool,
        cx: &mut Context<Self>,
    ) {
        let cell = position.and_then(|position| self.event_position_to_viewport_cell(position, cx));
        let previous_modifier = self.active_terminal_pane().terminal_link_open_modifier;
        let previous_link = self.active_terminal_pane().terminal_hovered_link.clone();
        let active_terminal = self.active_terminal_tab_id().and_then(|tab_id| {
            self.terminal_owner()
                .read(cx)
                .terminal_viewport_state(tab_id)
                .map(|viewport| (tab_id, viewport.generation))
        });
        let query = cell
            .zip(active_terminal)
            .map(|((line, column), (tab_id, generation))| TerminalLinkQuery {
                tab_id,
                generation,
                line,
                column,
            });
        let previous_query = self.active_terminal_pane().terminal_link_query;
        let should_query = open_modifier && query != previous_query;
        let hovered_link = if !open_modifier {
            None
        } else if should_query {
            cell.and_then(|(line, column)| self.terminal_link_at_cell(line, column, cx))
        } else {
            previous_link.clone()
        };
        let changed = previous_modifier != open_modifier || previous_link != hovered_link;

        self.active_terminal_pane_mut().terminal_pointer_position = position;
        self.active_terminal_pane_mut().terminal_link_query =
            open_modifier.then_some(query).flatten();
        self.active_terminal_pane_mut().terminal_link_open_modifier = open_modifier;
        self.active_terminal_pane_mut().terminal_hovered_link = hovered_link;

        if changed {
            cx.notify();
        }
    }

    fn touch_terminal_scrollbar_visibility(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        let interaction_at = Instant::now();
        if !self.set_terminal_scrollbar_interaction(pane_id, interaction_at) {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(TERMINAL_SCROLLBAR_IDLE_HIDE_DELAY)
                .await;
            this.update(cx, |this, cx| {
                if this
                    .terminal_pane_presentation(pane_id)
                    .is_some_and(|pane| pane.scrollbar_interaction == Some(interaction_at))
                {
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn terminal_scrollbar_visible(&self, pane_id: PaneId, cx: &App) -> bool {
        let Some(pane) = self.terminal_pane_presentation(pane_id) else {
            return false;
        };
        let pointer_over_track = pane.pointer_position.is_some_and(|position| {
            self.terminal_scrollbar_metrics_for_pane(pane_id, cx)
                .is_some_and(|metrics| metrics.track_bounds.contains(&position))
        });
        terminal_scrollbar_is_visible(
            pane.scrollbar_interaction,
            pointer_over_track,
            pane.dragging_scrollbar,
            Instant::now(),
        )
    }

    fn rebind_terminal_focus_reporting(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let terminal_focus = self.active_terminal_pane().terminal_focus.clone();
        self.terminal_owner().update(cx, |controller, cx| {
            controller.rebind_terminal_focus_events(terminal_focus, window, cx);
        });
    }

    fn sync_terminal_focus_reporting(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next_reported_tab_id = self.current_terminal_focus_report_target(window, cx);
        if next_reported_tab_id.is_none() {
            self.clear_terminal_focus_reporting(cx);
            return;
        }
        if next_reported_tab_id
            == self
                .terminal_owner()
                .read(cx)
                .reported_terminal_focus_tab_id()
        {
            return;
        }

        self.release_reported_terminal_focus(cx);

        if let Some(tab_id) = next_reported_tab_id
            && {
                let controller = self.terminal_owner().clone();
                controller.update(cx, |controller, cx| {
                    controller.send_terminal_focus_report(tab_id, true, cx)
                })
            }
        {
            self.terminal_owner()
                .read(cx)
                .set_reported_terminal_focus_tab_id(Some(tab_id));
        }
    }

    fn clear_terminal_focus_reporting(&mut self, cx: &mut Context<Self>) {
        self.clear_terminal_originated_selection_drag(cx);
        if clear_terminal_keyboard(&mut self.active_terminal_pane_mut().terminal_keyboard) {
            cx.notify();
        }
        self.release_reported_terminal_focus(cx);
    }

    fn release_reported_terminal_focus(&mut self, cx: &mut Context<Self>) {
        if let Some(previous_tab_id) = self
            .terminal_owner()
            .read(cx)
            .take_reported_terminal_focus_tab_id()
        {
            let controller = self.terminal_owner().clone();
            controller.update(cx, |controller, cx| {
                controller.send_terminal_focus_report(previous_tab_id, false, cx);
            });
        }
    }

    fn current_terminal_focus_report_target(&self, window: &Window, cx: &App) -> Option<TabId> {
        if !window.is_window_active()
            || !self
                .active_terminal_pane()
                .terminal_focus
                .is_focused(window)
        {
            return None;
        }

        let tab_id = self.active_terminal_tab_id()?;
        self.terminal_owner()
            .read(cx)
            .terminal_input_modes(tab_id)?
            .focus_in_out
            .then_some(tab_id)
    }

    fn terminal_scrollbar_metrics(&self, cx: &App) -> Option<TerminalScrollbarMetrics> {
        self.terminal_scrollbar_metrics_for_pane(self.active_terminal_pane_id(), cx)
    }

    fn terminal_scrollbar_metrics_for_pane(
        &self,
        pane_id: PaneId,
        cx: &App,
    ) -> Option<TerminalScrollbarMetrics> {
        let pane = self.terminal_pane_presentation(pane_id)?;
        let bounds = pane.bounds?;
        let tab_id = pane.tab_id?;
        let viewport = self
            .terminal_owner()
            .read(cx)
            .terminal_viewport_state(tab_id)?;

        terminal_scrollbar_metrics(
            bounds,
            viewport.screen_lines,
            viewport.history_size,
            viewport.display_offset,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_link_query_changes_with_terminal_generation() {
        let previous = TerminalLinkQuery {
            tab_id: TabId::new(7),
            generation: 10,
            line: 3,
            column: 4,
        };
        let current = TerminalLinkQuery {
            generation: 11,
            ..previous
        };

        assert_ne!(previous, current);
    }

    #[test]
    fn free_type_drag_starts_only_after_threshold() {
        let origin = gpui_kit::point(px(10.0), px(10.0));

        assert!(!terminal_pointer_drag_threshold_exceeded(
            origin,
            gpui_kit::point(px(13.0), px(10.0)),
        ));
        assert!(terminal_pointer_drag_threshold_exceeded(
            origin,
            gpui_kit::point(px(14.0), px(10.0)),
        ));
    }

    #[test]
    fn free_type_click_count_selects_semantic_then_lines() {
        assert_eq!(terminal_free_type_selection_kind(1), None);
        assert_eq!(
            terminal_free_type_selection_kind(2),
            Some(TerminalSelectionKind::Semantic)
        );
        assert_eq!(
            terminal_free_type_selection_kind(3),
            Some(TerminalSelectionKind::Lines)
        );
    }

    #[test]
    fn copy_click_count_selects_words_lines_and_blocks() {
        assert_eq!(
            terminal_copy_selection_kind(1, false),
            TerminalSelectionKind::Simple
        );
        assert_eq!(
            terminal_copy_selection_kind(2, false),
            TerminalSelectionKind::Semantic
        );
        assert_eq!(
            terminal_copy_selection_kind(3, false),
            TerminalSelectionKind::Lines
        );
        assert_eq!(
            terminal_copy_selection_kind(2, true),
            TerminalSelectionKind::Block
        );
    }

    #[test]
    fn left_mouse_route_uses_alt_to_override_application_mouse_reporting() {
        let plain = gpui_kit::Modifiers::default();
        let alt = gpui_kit::Modifiers {
            alt: true,
            ..Default::default()
        };
        let shift = gpui_kit::Modifiers {
            shift: true,
            ..Default::default()
        };
        let shift_alt = gpui_kit::Modifiers {
            shift: true,
            alt: true,
            ..Default::default()
        };

        assert_eq!(
            terminal_left_mouse_route(true, true, &plain),
            TerminalLeftMouseRoute::Application
        );
        assert_eq!(
            terminal_left_mouse_route(true, true, &alt),
            TerminalLeftMouseRoute::FreeType
        );
        assert_eq!(
            terminal_left_mouse_route(true, true, &shift),
            TerminalLeftMouseRoute::TraditionalSelection
        );
        assert_eq!(
            terminal_left_mouse_route(true, true, &shift_alt),
            TerminalLeftMouseRoute::TraditionalSelection
        );
        assert_eq!(
            terminal_left_mouse_route(true, false, &plain),
            TerminalLeftMouseRoute::FreeType
        );
        assert_eq!(
            terminal_left_mouse_route(true, false, &alt),
            TerminalLeftMouseRoute::FreeType
        );
        assert_eq!(
            terminal_left_mouse_route(false, true, &alt),
            TerminalLeftMouseRoute::Application
        );
        assert_eq!(
            terminal_left_mouse_route(false, false, &alt),
            TerminalLeftMouseRoute::TraditionalSelection
        );
    }

    #[test]
    fn alt_target_miss_is_consumed_only_for_mouse_reporting_override() {
        let alt = gpui_kit::Modifiers {
            alt: true,
            ..Default::default()
        };

        assert!(terminal_invalid_free_type_target_is_consumed(
            TerminalLeftMouseRoute::FreeType,
            true,
            &alt,
        ));
        assert!(!terminal_invalid_free_type_target_is_consumed(
            TerminalLeftMouseRoute::FreeType,
            false,
            &alt,
        ));
        assert!(!terminal_invalid_free_type_target_is_consumed(
            TerminalLeftMouseRoute::Application,
            true,
            &alt,
        ));
    }

    #[test]
    fn shift_alt_keeps_block_selection_while_free_type_is_enabled() {
        let alt = gpui_kit::Modifiers {
            alt: true,
            ..Default::default()
        };
        let shift_alt = gpui_kit::Modifiers {
            shift: true,
            alt: true,
            ..Default::default()
        };

        assert!(!terminal_traditional_selection_is_block(true, &alt));
        assert!(terminal_traditional_selection_is_block(true, &shift_alt));
        assert!(terminal_traditional_selection_is_block(false, &alt));
    }

    #[test]
    fn active_free_type_gesture_suppresses_application_mouse_motion() {
        let plain = gpui_kit::Modifiers::default();
        let shift = gpui_kit::Modifiers {
            shift: true,
            ..Default::default()
        };

        assert!(terminal_mouse_motion_reporting_allowed(&plain, false));
        assert!(!terminal_mouse_motion_reporting_allowed(&plain, true));
        assert!(!terminal_mouse_motion_reporting_allowed(&shift, false));
    }

    #[test]
    fn control_or_platform_modifier_requests_link_opening() {
        let control = gpui_kit::Modifiers {
            control: true,
            ..Default::default()
        };
        let platform = gpui_kit::Modifiers {
            platform: true,
            ..Default::default()
        };

        assert!(terminal_link_open_requested(&control));
        assert!(terminal_link_open_requested(&platform));
    }

    #[test]
    fn focus_loss_clears_suppressed_terminal_key_releases() {
        let mut releases = TerminalKeyboardState::default();
        releases.suppress_release("enter");

        assert!(clear_terminal_keyboard(&mut releases));
        assert!(!releases.consume_release("enter"));
        assert!(!clear_terminal_keyboard(&mut releases));
    }

    #[test]
    fn free_type_drop_target_splits_local_and_cross_pane_destinations() {
        let target = TerminalFreeTypeTarget::new(1, 2, Side::Right);
        let source_pane_id = PaneId(1);

        assert_eq!(
            split_terminal_free_type_drop_target(
                source_pane_id,
                Some(TerminalPaneFreeTypeDropTarget {
                    pane_id: source_pane_id,
                    tab_id: TabId::new(7),
                    target,
                }),
            ),
            (Some(target), None)
        );
        let cross_pane = TerminalPaneFreeTypeDropTarget {
            pane_id: PaneId(2),
            tab_id: TabId::new(7),
            target,
        };
        assert_eq!(
            split_terminal_free_type_drop_target(source_pane_id, Some(cross_pane)),
            (None, Some(cross_pane))
        );
        assert_eq!(
            split_terminal_free_type_drop_target(source_pane_id, None),
            (None, None)
        );
    }

    #[test]
    fn terminal_scrollbar_stays_visible_with_recent_scroll() {
        let now = Instant::now();
        let recent_scroll = now
            .checked_sub(Duration::from_millis(500))
            .expect("recent instant should be valid");

        assert!(terminal_scrollbar_is_visible(
            Some(recent_scroll),
            false,
            false,
            now,
        ));
    }

    #[test]
    fn terminal_scrollbar_hides_after_idle_timeout() {
        let now = Instant::now();
        let stale_scroll = now
            .checked_sub(TERMINAL_SCROLLBAR_IDLE_HIDE_DELAY + Duration::from_millis(1))
            .expect("stale instant should be valid");

        assert!(!terminal_scrollbar_is_visible(
            Some(stale_scroll),
            false,
            false,
            now,
        ));
    }

    #[test]
    fn terminal_scrollbar_stays_visible_while_hovered_or_dragged() {
        let now = Instant::now();

        assert!(terminal_scrollbar_is_visible(None, true, false, now));
        assert!(terminal_scrollbar_is_visible(None, false, true, now));
    }
}
