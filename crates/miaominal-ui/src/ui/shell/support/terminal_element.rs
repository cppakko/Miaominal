use super::super::*;
use crate::ui::features::session::view::canvas::{TerminalCanvasHost, TerminalCanvasPrepaint};

impl TerminalCanvasHost for AppView {
    fn prepare_terminal_canvas_prepaint(
        &mut self,
        pane_id: PaneId,
        bounds: Bounds<Pixels>,
        cell_width: f32,
        line_height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TerminalCanvasPrepaint> {
        prepare_terminal_canvas_prepaint(self, pane_id, bounds, cell_width, line_height, window, cx)
    }

    fn handle_terminal_outside_mouse_move(
        &mut self,
        pane_id: PaneId,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        if self.active_pane_id() != pane_id {
            return;
        }
        let pane = &self.workspace.workspace.active_pane;
        if pane.terminal_mouse_gesture.is_some()
            || pane.terminal_mouse_reporting_active
            || pane.terminal_scrollbar_drag.is_some()
        {
            self.handle_terminal_mouse_move(event, cx);
        }
    }

    fn handle_terminal_outside_mouse_up(
        &mut self,
        pane_id: PaneId,
        event: &MouseUpEvent,
        cx: &mut Context<Self>,
    ) {
        if self.active_pane_id() != pane_id {
            return;
        }
        let pane = &self.workspace.workspace.active_pane;
        if pane.terminal_mouse_gesture.is_some()
            || pane.terminal_mouse_reporting_active
            || pane.terminal_scrollbar_drag.is_some()
        {
            self.handle_terminal_mouse_up(event, cx);
        }
    }
}

fn prepare_terminal_canvas_prepaint(
    this: &mut AppView,
    pane_id: PaneId,
    bounds: Bounds<Pixels>,
    cell_width: f32,
    line_height: f32,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<AppView>,
) -> Option<TerminalCanvasPrepaint> {
    let (active_tab_id, focus, local_drop_target, metrics_changed) = if pane_id
        == this.workspace.workspace.active_pane_id
    {
        let metrics_changed = this.workspace.workspace.active_pane.terminal_bounds != Some(bounds)
            || this.workspace.workspace.active_pane.terminal_cell_width != cell_width
            || this.workspace.workspace.active_pane.terminal_line_height != line_height;
        this.workspace.workspace.active_pane.terminal_bounds = Some(bounds);
        this.workspace.workspace.active_pane.terminal_cell_width = cell_width;
        this.workspace.workspace.active_pane.terminal_line_height = line_height;

        (
            this.workspace.workspace.active_tab,
            this.workspace.workspace.active_pane.terminal_focus.clone(),
            this.workspace
                .workspace
                .active_pane
                .terminal_mouse_gesture
                .and_then(|gesture| gesture.drop_target()),
            metrics_changed,
        )
    } else {
        let parked = this.workspace.workspace.parked_panes.get_mut(&pane_id)?;
        let metrics_changed = parked.terminal_bounds != Some(bounds)
            || parked.terminal_cell_width != cell_width
            || parked.terminal_line_height != line_height;
        parked.terminal_bounds = Some(bounds);
        parked.terminal_cell_width = cell_width;
        parked.terminal_line_height = line_height;

        (
            parked.active_tab,
            parked.terminal_focus.clone(),
            parked
                .terminal_mouse_gesture
                .and_then(|gesture| gesture.drop_target()),
            metrics_changed,
        )
    };

    let drop_target = this
        .workspace
        .terminal_free_type_drop_target
        .filter(|drop| drop.pane_id == pane_id)
        .map(|drop| drop.target)
        .or(local_drop_target);

    let resized = active_tab_id
        .and_then(|tab_id| this.workspace.tabs.index_of(tab_id))
        .is_some_and(|index| {
            this.sync_session_terminal_size_from_metrics(
                index,
                bounds,
                cell_width,
                line_height,
                !metrics_changed,
                cx,
            )
        });

    if metrics_changed || resized {
        cx.notify();
    }

    let focused = pane_id == this.workspace.workspace.active_pane_id && focus.is_focused(window);
    let tab_id = active_tab_id?;
    let input_controller = this.controllers.session.downgrade();
    let snapshot = this
        .controllers
        .session
        .read(cx)
        .terminal_snapshot(tab_id, focused)?;
    let ime_cursor_bounds = terminal_ime_cursor_bounds(
        bounds,
        &snapshot,
        cell_width,
        line_height,
        window.scale_factor(),
    );

    if pane_id == this.workspace.workspace.active_pane_id {
        let ime_anchor = if focused {
            ime_cursor_bounds.map(|bounds| (tab_id, bounds))
        } else {
            None
        };
        if this.workspace.workspace.active_pane.terminal_ime_anchor != ime_anchor {
            this.workspace.workspace.active_pane.terminal_ime_anchor = ime_anchor;
            if ime_anchor.is_some() {
                window.invalidate_character_coordinates();
            }
        }
    }

    Some(TerminalCanvasPrepaint::new(
        snapshot,
        focus,
        input_controller,
        tab_id,
        ime_cursor_bounds,
        drop_target,
    ))
}
