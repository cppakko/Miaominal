use super::custom_glyphs;
use crate::ui::shell::{
    AppView, PaneId, SessionController, TabId, TerminalHoveredLink, WorkspaceTerminalInputExt,
};
use alacritty_terminal::index::Side;
use gpui_kit::{
    Background, Bounds, Context, Corners, DispatchPhase, FocusHandle, FontStyle, FontWeight, Hsla,
    InputHandler, IntoElement, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, SharedString,
    StrikethroughStyle, Styled, TextAlign, TextRun, UTF16Selection, UnderlineStyle, WeakEntity,
    Window, canvas, fill, px, quad, rgba, size,
};
use miaominal_terminal::{
    SearchMatchKind, TerminalFreeTypeTarget, TerminalSnapshot, terminal_font, terminal_font_size,
};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    ops::Range,
    sync::Arc,
};

const TERMINAL_SCROLLBAR_TRACK_WIDTH: f32 = 6.0;
const TERMINAL_SCROLLBAR_MIN_THUMB_HEIGHT: f32 = 20.0;
const TERMINAL_FREE_TYPE_DROP_LINE_ALPHA: f32 = 0.08;

pub(in crate::ui::shell) struct TerminalCanvasPrepaint {
    snapshot: Arc<TerminalSnapshot>,
    focus: FocusHandle,
    input_controller: WeakEntity<SessionController>,
    tab_id: TabId,
    ime_cursor_bounds: Option<Bounds<Pixels>>,
    drop_target: Option<TerminalFreeTypeTarget>,
}

struct TerminalImeHandler {
    controller: WeakEntity<SessionController>,
    tab_id: TabId,
    cursor_bounds: Option<Bounds<Pixels>>,
}

#[derive(Clone, Copy)]
struct TerminalLineShapeKey {
    text_hash: u64,
    text_len: usize,
}

impl InputHandler for TerminalImeHandler {
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut gpui_kit::Window,
        _cx: &mut gpui_kit::App,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &mut self,
        _window: &mut gpui_kit::Window,
        _cx: &mut gpui_kit::App,
    ) -> Option<Range<usize>> {
        None
    }

    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut gpui_kit::Window,
        _cx: &mut gpui_kit::App,
    ) -> Option<String> {
        None
    }

    fn replace_text_in_range(
        &mut self,
        _replacement_range: Option<Range<usize>>,
        text: &str,
        _window: &mut gpui_kit::Window,
        cx: &mut gpui_kit::App,
    ) {
        if text.is_empty() {
            return;
        }
        let tab_id = self.tab_id;
        let text = text.to_string();
        self.controller
            .update(cx, |controller, cx| {
                controller.send_terminal_text_input(tab_id, text, cx);
            })
            .ok();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        _new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut gpui_kit::Window,
        _cx: &mut gpui_kit::App,
    ) {
        // Preedit during IME composition; terminal relies on the OS IME popup for display.
    }

    fn unmark_text(&mut self, _window: &mut gpui_kit::Window, _cx: &mut gpui_kit::App) {}

    fn bounds_for_range(
        &mut self,
        _range: Range<usize>,
        _window: &mut gpui_kit::Window,
        _cx: &mut gpui_kit::App,
    ) -> Option<Bounds<Pixels>> {
        self.cursor_bounds
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut gpui_kit::Window,
        _cx: &mut gpui_kit::App,
    ) -> Option<usize> {
        None
    }
}

#[derive(Clone, Copy)]
pub(in crate::ui::shell) struct TerminalScrollbarMetrics {
    pub(in crate::ui::shell) track_bounds: Bounds<Pixels>,
    pub(in crate::ui::shell) thumb_bounds: Bounds<Pixels>,
    pub(in crate::ui::shell) display_offset: usize,
    pub(in crate::ui::shell) history_size: usize,
    pub(in crate::ui::shell) thumb_max_offset: f32,
}

pub(in crate::ui::shell) fn render_terminal_canvas_for_pane<V: TerminalCanvasHost>(
    hovered_link: Option<TerminalHoveredLink>,
    cell_width: f32,
    line_height: f32,
    view: WeakEntity<V>,
    pane_id: PaneId,
    show_scrollbar: bool,
) -> impl IntoElement {
    let view_for_paint = view.clone();
    canvas(
        move |bounds, window, cx| -> Option<TerminalCanvasPrepaint> {
            view.update(cx, |this, cx| {
                this.prepare_terminal_canvas_prepaint(
                    pane_id,
                    bounds,
                    cell_width,
                    line_height,
                    window,
                    cx,
                )
            })
            .ok()
            .flatten()
        },
        move |bounds, prepaint, window, cx| {
            let Some(prepaint) = prepaint else {
                return;
            };
            paint_snapshot(
                bounds,
                &prepaint.snapshot,
                hovered_link.as_ref(),
                prepaint.drop_target,
                cell_width,
                line_height,
                window,
                cx,
            );
            if show_scrollbar {
                paint_scrollbar(bounds, &prepaint.snapshot, window);
            }
            let focus = Some(prepaint.focus);
            if let Some(focus) = focus.as_ref() {
                window.handle_input(
                    focus,
                    TerminalImeHandler {
                        controller: prepaint.input_controller.clone(),
                        tab_id: prepaint.tab_id,
                        cursor_bounds: prepaint.ime_cursor_bounds,
                    },
                    cx,
                );
            }
            window.on_mouse_event({
                let view = view_for_paint.clone();
                let focus = focus.clone();
                move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble
                        || event.pressed_button.is_none()
                        || cx.has_active_drag()
                        || bounds.contains(&event.position)
                    {
                        return;
                    }

                    let Some(focus) = focus.as_ref() else {
                        return;
                    };
                    if !focus.is_focused(window) {
                        return;
                    }

                    view.update(cx, |this, cx| {
                        this.handle_terminal_outside_mouse_move(pane_id, event, cx);
                    })
                    .ok();
                }
            });
            window.on_mouse_event({
                let view = view_for_paint.clone();
                let focus = focus.clone();
                move |event: &MouseUpEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble || bounds.contains(&event.position) {
                        return;
                    }

                    let Some(focus) = focus.as_ref() else {
                        return;
                    };
                    if !focus.is_focused(window) {
                        return;
                    }

                    view.update(cx, |this, cx| {
                        this.handle_terminal_outside_mouse_up(pane_id, event, cx);
                    })
                    .ok();
                }
            });
        },
    )
    .size_full()
}

pub(in crate::ui::shell) trait TerminalCanvasHost:
    Render + Sized + 'static
{
    fn prepare_terminal_canvas_prepaint(
        &mut self,
        pane_id: PaneId,
        bounds: Bounds<Pixels>,
        cell_width: f32,
        line_height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TerminalCanvasPrepaint>;

    fn handle_terminal_outside_mouse_move(
        &mut self,
        pane_id: PaneId,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    );

    fn handle_terminal_outside_mouse_up(
        &mut self,
        pane_id: PaneId,
        event: &MouseUpEvent,
        cx: &mut Context<Self>,
    );
}

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

    Some(TerminalCanvasPrepaint {
        snapshot,
        focus,
        input_controller,
        tab_id,
        ime_cursor_bounds,
        drop_target,
    })
}

pub(in crate::ui::shell) fn terminal_scrollbar_metrics(
    bounds: Bounds<Pixels>,
    screen_lines: usize,
    history_size: usize,
    display_offset: usize,
) -> Option<TerminalScrollbarMetrics> {
    if history_size == 0 {
        return None;
    }

    let total_lines = history_size + screen_lines;
    if total_lines <= screen_lines {
        return None;
    }

    let track_width = px(TERMINAL_SCROLLBAR_TRACK_WIDTH);
    let track_bounds = Bounds {
        origin: Point {
            x: bounds.origin.x + bounds.size.width - track_width,
            y: bounds.origin.y,
        },
        size: size(track_width, bounds.size.height),
    };

    let total_height = f32::from(bounds.size.height);
    let thumb_height = (screen_lines as f32 / total_lines as f32 * total_height)
        .max(TERMINAL_SCROLLBAR_MIN_THUMB_HEIGHT)
        .min(total_height);
    let thumb_max_offset = (total_height - thumb_height).max(0.0);
    let scroll_ratio = (display_offset as f32 / history_size as f32).clamp(0.0, 1.0);
    let thumb_y = if thumb_max_offset <= f32::EPSILON {
        0.0
    } else {
        thumb_max_offset * (1.0 - scroll_ratio)
    };

    let thumb_bounds = Bounds {
        origin: Point {
            x: bounds.origin.x + bounds.size.width - track_width,
            y: bounds.origin.y + px(thumb_y),
        },
        size: size(track_width, px(thumb_height)),
    };

    Some(TerminalScrollbarMetrics {
        track_bounds,
        thumb_bounds,
        display_offset,
        history_size,
        thumb_max_offset,
    })
}

pub(in crate::ui::shell) fn terminal_scrollbar_offset_for_pointer(
    metrics: &TerminalScrollbarMetrics,
    pointer_y: Pixels,
    thumb_grab_offset: f32,
) -> usize {
    if metrics.thumb_max_offset <= f32::EPSILON {
        return metrics.display_offset;
    }

    let track_origin_y = f32::from(metrics.track_bounds.origin.y);
    let pointer_offset = f32::from(pointer_y) - track_origin_y;
    let thumb_y = (pointer_offset - thumb_grab_offset).clamp(0.0, metrics.thumb_max_offset);
    let scroll_ratio = 1.0 - (thumb_y / metrics.thumb_max_offset);
    (scroll_ratio * metrics.history_size as f32)
        .round()
        .clamp(0.0, metrics.history_size as f32) as usize
}

#[allow(clippy::too_many_arguments)]
fn paint_snapshot(
    bounds: Bounds<Pixels>,
    snapshot: &TerminalSnapshot,
    hovered_link: Option<&TerminalHoveredLink>,
    drop_target: Option<TerminalFreeTypeTarget>,
    cell_width: f32,
    line_height: f32,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
) {
    let cell_width_px = px(cell_width);
    let line_height_px = px(line_height);
    let origin = Point {
        x: px(f32::from(bounds.origin.x).round()),
        y: px(f32::from(bounds.origin.y).round()),
    };
    let terminal_font = terminal_font();
    let font_size = px(terminal_font_size());
    let cursor_unfocused = !snapshot.focused_cursor;

    window.paint_quad(fill(bounds, Background::from(snapshot.default_bg)));
    if let Some(target) = drop_target {
        paint_free_type_drop_line_highlight(
            snapshot,
            target,
            origin,
            cell_width_px,
            line_height_px,
            window,
        );
    }
    paint_backgrounds(snapshot, origin, cell_width_px, line_height_px, window);
    paint_search_highlights(snapshot, origin, cell_width_px, line_height_px, window);
    custom_glyphs::paint_custom_glyphs(snapshot, origin, cell_width_px, line_height_px, window);

    for (row, cells) in snapshot.cells.iter().enumerate() {
        let line_origin = Point {
            x: origin.x,
            y: origin.y + line_height_px * row as f32,
        };

        let (shape_key, runs) =
            build_line_shape_key_and_runs(cells, row, hovered_link, &terminal_font);
        if shape_key.text_len == 0 {
            continue;
        }

        let shaped = window.text_system().shape_line_by_hash(
            shape_key.text_hash,
            shape_key.text_len,
            font_size,
            &runs,
            Some(cell_width_px),
            || SharedString::from(materialize_line_text(cells)),
        );

        if let Err(error) = shaped.paint(
            line_origin,
            line_height_px,
            TextAlign::Left,
            None,
            window,
            cx,
        ) {
            log::warn!("failed to paint terminal line: {error:?}");
        }
    }

    if cursor_unfocused {
        paint_unfocused_cursor(snapshot, origin, cell_width_px, line_height_px, window);
    }
    if let Some(target) = drop_target {
        paint_free_type_drop_caret(
            snapshot,
            target,
            origin,
            cell_width_px,
            line_height_px,
            window,
        );
    }
}

fn paint_free_type_drop_line_highlight(
    snapshot: &TerminalSnapshot,
    target: TerminalFreeTypeTarget,
    origin: Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    window: &mut gpui_kit::Window,
) {
    let Some(row) = terminal_free_type_drop_line_row(target, snapshot.cells.len()) else {
        return;
    };

    let scale_factor = window.scale_factor();
    let bounds = snapped_cell_bounds(
        origin,
        cell_width,
        line_height,
        0,
        snapshot.columns,
        row,
        scale_factor,
    );
    let color = Hsla {
        a: TERMINAL_FREE_TYPE_DROP_LINE_ALPHA,
        ..snapshot.default_fg
    };
    window.paint_quad(fill(bounds, Background::from(color)));
}

fn terminal_free_type_drop_line_row(
    target: TerminalFreeTypeTarget,
    visible_rows: usize,
) -> Option<usize> {
    let row = usize::try_from(target.line).ok()?;
    (row < visible_rows).then_some(row)
}

fn paint_free_type_drop_caret(
    snapshot: &TerminalSnapshot,
    target: TerminalFreeTypeTarget,
    origin: Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    window: &mut gpui_kit::Window,
) {
    let Ok(row) = usize::try_from(target.line) else {
        return;
    };
    let Some(cell) = snapshot
        .cells
        .get(row)
        .and_then(|cells| cells.get(target.column))
    else {
        return;
    };
    let boundary_column = target.column
        + if target.side == Side::Right {
            if cell.wide { 2 } else { 1 }
        } else {
            0
        };
    let caret_width = px(2.0);
    let caret_bounds = Bounds {
        origin: Point {
            x: origin.x + cell_width * boundary_column as f32 - caret_width / 2.0,
            y: origin.y + line_height * row as f32,
        },
        size: size(caret_width, line_height),
    };
    window.paint_quad(fill(caret_bounds, Background::from(snapshot.default_fg)));
}

fn paint_backgrounds(
    snapshot: &TerminalSnapshot,
    origin: Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    window: &mut gpui_kit::Window,
) {
    let scale_factor = window.scale_factor();
    for (row, cells) in snapshot.cells.iter().enumerate() {
        let mut col = 0usize;
        while col < cells.len() {
            let cell = &cells[col];
            if cell.spacer {
                col += 1;
                continue;
            }

            if cell.bg == snapshot.default_bg {
                col += 1;
                continue;
            }

            let mut span = 1usize;
            let mut col_advance = if cell.wide { 2 } else { 1 };
            while col + span < cells.len() {
                let next = &cells[col + span];
                if next.spacer {
                    span += 1;
                    continue;
                }
                if next.bg != cell.bg {
                    break;
                }
                col_advance += if next.wide { 2 } else { 1 };
                span += 1;
            }

            let bounds = snapped_cell_bounds(
                origin,
                cell_width,
                line_height,
                col,
                col_advance,
                row,
                scale_factor,
            );
            window.paint_quad(fill(bounds, Background::from(cell.bg)));

            col += span;
        }
    }
}

fn snap_to_physical(value: Pixels, scale: f32) -> Pixels {
    if scale > 0.0 {
        px((f32::from(value) * scale).round() / scale)
    } else {
        value
    }
}

fn snapped_cell_bounds(
    origin: Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    col: usize,
    advance: usize,
    row: usize,
    scale: f32,
) -> Bounds<Pixels> {
    let left = snap_to_physical(origin.x + cell_width * col as f32, scale);
    let right = snap_to_physical(origin.x + cell_width * (col + advance) as f32, scale);
    let top = snap_to_physical(origin.y + line_height * row as f32, scale);
    let bottom = snap_to_physical(origin.y + line_height * (row + 1) as f32, scale);
    Bounds {
        origin: Point { x: left, y: top },
        size: size(right - left, bottom - top),
    }
}

fn terminal_ime_cursor_bounds(
    bounds: Bounds<Pixels>,
    snapshot: &TerminalSnapshot,
    cell_width: f32,
    line_height: f32,
    scale: f32,
) -> Option<Bounds<Pixels>> {
    if snapshot.columns == 0
        || snapshot.screen_lines == 0
        || !cell_width.is_finite()
        || cell_width <= 0.0
        || !line_height.is_finite()
        || line_height <= 0.0
    {
        return None;
    }

    let max_row = snapshot.screen_lines.saturating_sub(1);
    let row = snapshot.cursor.viewport_line.clamp(0, max_row as i32) as usize;
    let column = snapshot
        .cursor
        .column
        .min(snapshot.columns.saturating_sub(1));

    Some(snapped_cell_bounds(
        bounds.origin,
        px(cell_width),
        px(line_height),
        column,
        1,
        row,
        scale,
    ))
}

fn paint_unfocused_cursor(
    snapshot: &TerminalSnapshot,
    origin: Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    window: &mut gpui_kit::Window,
) {
    let scale_factor = window.scale_factor();
    for (row, cells) in snapshot.cells.iter().enumerate() {
        for (col, cell) in cells.iter().enumerate() {
            if !cell.is_cursor {
                continue;
            }
            let advance = if cell.wide { 2 } else { 1 };
            let bounds = snapped_cell_bounds(
                origin,
                cell_width,
                line_height,
                col,
                advance,
                row,
                scale_factor,
            );
            window.paint_quad(gpui_kit::outline(
                bounds,
                cell.bg,
                gpui_kit::BorderStyle::Solid,
            ));
        }
    }
}

fn paint_search_highlights(
    snapshot: &TerminalSnapshot,
    origin: Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    window: &mut gpui_kit::Window,
) {
    let match_color = Hsla {
        h: 50.0 / 360.0,
        s: 0.85,
        l: 0.55,
        a: 0.55,
    };
    let current_color = Hsla {
        h: 25.0 / 360.0,
        s: 0.95,
        l: 0.55,
        a: 0.75,
    };
    let scale_factor = window.scale_factor();
    for (row, cells) in snapshot.cells.iter().enumerate() {
        for (col, cell) in cells.iter().enumerate() {
            let color = match cell.search_match {
                SearchMatchKind::None => continue,
                SearchMatchKind::Match => match_color,
                SearchMatchKind::Current => current_color,
            };
            let advance = if cell.wide { 2 } else { 1 };
            let bounds = snapped_cell_bounds(
                origin,
                cell_width,
                line_height,
                col,
                advance,
                row,
                scale_factor,
            );
            window.paint_quad(fill(bounds, Background::from(color)));
        }
    }
}

fn terminal_text_character(cell: &miaominal_terminal::TerminalCell) -> char {
    if cell.spacer || cell.character == '\0' || custom_glyphs::is_custom_glyph(cell.character) {
        ' '
    } else {
        cell.character
    }
}

fn hash_terminal_text_char(hasher: &mut DefaultHasher, ch: char) {
    ch.hash(hasher);
}

fn materialize_line_text(cells: &[miaominal_terminal::TerminalCell]) -> String {
    let text_len = line_text_len(cells);
    let mut text = String::with_capacity(text_len);

    for cell in cells {
        text.push(terminal_text_character(cell));
        if !cell.spacer {
            for ch in &cell.zero_width {
                text.push(*ch);
            }
        }
    }

    text
}

fn line_text_len(cells: &[miaominal_terminal::TerminalCell]) -> usize {
    cells
        .iter()
        .map(|cell| {
            let mut len = terminal_text_character(cell).len_utf8();
            if !cell.spacer {
                len += cell
                    .zero_width
                    .iter()
                    .map(|ch| ch.len_utf8())
                    .sum::<usize>();
            }
            len
        })
        .sum()
}

fn build_line_shape_key_and_runs(
    cells: &[miaominal_terminal::TerminalCell],
    row_index: usize,
    hovered_link: Option<&TerminalHoveredLink>,
    base_font: &gpui_kit::Font,
) -> (TerminalLineShapeKey, Vec<TextRun>) {
    let mut hasher = DefaultHasher::new();
    let mut text_len = 0usize;
    let mut runs: Vec<TextRun> = Vec::new();
    let hovered_range = hovered_link_range(cells, row_index, hovered_link);

    for (column, cell) in cells.iter().enumerate() {
        let start = text_len;
        let character = terminal_text_character(cell);
        hash_terminal_text_char(&mut hasher, character);
        text_len += character.len_utf8();
        if !cell.spacer {
            for ch in &cell.zero_width {
                hash_terminal_text_char(&mut hasher, *ch);
                text_len += ch.len_utf8();
            }
        }
        let len = text_len - start;

        let mut font = base_font.clone();
        if cell.bold {
            font.weight = FontWeight::BOLD;
        }
        if cell.italic {
            font.style = FontStyle::Italic;
        }

        let hover_underline = hovered_range
            .map(|(start, end)| (start..end).contains(&column))
            .unwrap_or(false);

        let underline = if cell.underline || hover_underline {
            Some(UnderlineStyle {
                color: Some(cell.fg),
                thickness: px(1.0),
                wavy: false,
            })
        } else {
            None
        };

        let strikethrough = if cell.strikethrough {
            Some(StrikethroughStyle {
                color: Some(cell.fg),
                thickness: px(1.0),
            })
        } else {
            None
        };

        let run = TextRun {
            len,
            font,
            color: cell.fg,
            background_color: None,
            underline,
            strikethrough,
        };

        if let Some(last) = runs.last_mut()
            && runs_compatible(last, &run)
        {
            last.len += run.len;
        } else {
            runs.push(run);
        }
    }

    (
        TerminalLineShapeKey {
            text_hash: hasher.finish(),
            text_len,
        },
        runs,
    )
}

fn hovered_link_range(
    cells: &[miaominal_terminal::TerminalCell],
    row_index: usize,
    hovered_link: Option<&TerminalHoveredLink>,
) -> Option<(usize, usize)> {
    let hovered_link = hovered_link?;
    if hovered_link.line != row_index
        || hovered_link.start_column >= hovered_link.end_column
        || hovered_link.end_column > cells.len()
    {
        return None;
    }

    let hovered_uri = hovered_link.uri.as_ref();
    if cells[hovered_link.start_column..hovered_link.end_column]
        .iter()
        .any(|cell| cell.link.as_deref() != Some(hovered_uri))
    {
        return None;
    }

    Some((hovered_link.start_column, hovered_link.end_column))
}

fn runs_compatible(a: &TextRun, b: &TextRun) -> bool {
    a.font == b.font
        && a.color == b.color
        && a.background_color == b.background_color
        && a.underline == b.underline
        && a.strikethrough == b.strikethrough
}

fn paint_scrollbar(
    bounds: Bounds<Pixels>,
    snapshot: &TerminalSnapshot,
    window: &mut gpui_kit::Window,
) {
    let Some(metrics) = terminal_scrollbar_metrics(
        bounds,
        snapshot.screen_lines,
        snapshot.history_size,
        snapshot.display_offset,
    ) else {
        return;
    };

    let roles = miaominal_settings::current_theme().material.roles;
    let corner_radius = px(TERMINAL_SCROLLBAR_TRACK_WIDTH / 2.0);
    let corners = Corners::all(corner_radius);

    // Keep the track transparent so only the thumb is visible.
    let track_color = gpui_kit::transparent_black();
    window.paint_quad(quad(
        metrics.track_bounds,
        corners,
        Background::from(track_color),
        gpui_kit::Edges::all(px(0.0)),
        gpui_kit::transparent_black(),
        gpui_kit::BorderStyle::default(),
    ));

    // MD3: thumb uses on_surface_variant at medium opacity
    let thumb_color = rgba((roles.on_surface_variant << 8) | 0xb3);
    window.paint_quad(quad(
        metrics.thumb_bounds,
        corners,
        Background::from(thumb_color),
        gpui_kit::Edges::all(px(0.0)),
        gpui_kit::transparent_black(),
        gpui_kit::BorderStyle::default(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_terminal::{TerminalCursorPosition, default_background, default_foreground};

    fn test_bounds(height: f32) -> Bounds<Pixels> {
        Bounds {
            origin: Point {
                x: px(0.0),
                y: px(0.0),
            },
            size: size(px(120.0), px(height)),
        }
    }

    fn test_snapshot(
        viewport_line: i32,
        column: usize,
        columns: usize,
        screen_lines: usize,
    ) -> TerminalSnapshot {
        let fg = default_foreground();
        let bg = default_background();
        TerminalSnapshot {
            cells: (0..screen_lines)
                .map(|_| {
                    (0..columns)
                        .map(|_| miaominal_terminal::TerminalCell::blank(fg, bg))
                        .collect()
                })
                .collect(),
            columns,
            screen_lines,
            display_offset: 0,
            history_size: 0,
            cursor: TerminalCursorPosition {
                viewport_line,
                column,
            },
            default_fg: fg,
            default_bg: bg,
            focused_cursor: true,
            search_total: 0,
            search_current: None,
        }
    }

    #[test]
    fn ime_cursor_bounds_follow_terminal_cursor_and_canvas_origin() {
        let bounds = Bounds {
            origin: Point {
                x: px(10.0),
                y: px(20.0),
            },
            size: size(px(80.0), px(64.0)),
        };
        let snapshot = test_snapshot(2, 3, 10, 4);

        let cursor = terminal_ime_cursor_bounds(bounds, &snapshot, 8.0, 16.0, 1.0)
            .expect("expected IME cursor bounds");

        assert_eq!(cursor.origin.x, px(34.0));
        assert_eq!(cursor.origin.y, px(52.0));
        assert_eq!(cursor.size, size(px(8.0), px(16.0)));
    }

    #[test]
    fn ime_cursor_bounds_clamp_to_visible_terminal_edges() {
        let bounds = test_bounds(64.0);
        let below_and_right = test_snapshot(99, 99, 10, 4);
        let above = test_snapshot(-10, 2, 10, 4);

        let bottom_right = terminal_ime_cursor_bounds(bounds, &below_and_right, 8.0, 16.0, 1.0)
            .expect("expected clamped IME cursor bounds");
        let top = terminal_ime_cursor_bounds(bounds, &above, 8.0, 16.0, 1.0)
            .expect("expected clamped IME cursor bounds");

        assert_eq!(bottom_right.origin, Point::new(px(72.0), px(48.0)));
        assert_eq!(top.origin, Point::new(px(16.0), px(0.0)));
    }

    #[test]
    fn ime_cursor_bounds_require_valid_terminal_metrics() {
        let empty = test_snapshot(0, 0, 0, 0);
        let populated = test_snapshot(0, 0, 10, 4);

        assert!(terminal_ime_cursor_bounds(test_bounds(64.0), &empty, 8.0, 16.0, 1.0).is_none());
        assert!(
            terminal_ime_cursor_bounds(test_bounds(64.0), &populated, 0.0, 16.0, 1.0).is_none()
        );
    }

    #[test]
    fn scrollbar_metrics_absent_without_scrollback() {
        assert!(terminal_scrollbar_metrics(test_bounds(120.0), 20, 0, 0).is_none());
    }

    #[test]
    fn dragging_thumb_center_preserves_current_offset() {
        let metrics = terminal_scrollbar_metrics(test_bounds(120.0), 20, 80, 40)
            .expect("expected scrollbar metrics");
        let thumb_center_y = f32::from(metrics.thumb_bounds.origin.y)
            + f32::from(metrics.thumb_bounds.size.height) / 2.0;
        let target_offset = terminal_scrollbar_offset_for_pointer(
            &metrics,
            px(thumb_center_y),
            f32::from(metrics.thumb_bounds.size.height) / 2.0,
        );

        assert_eq!(target_offset, 40);
    }

    #[test]
    fn dragging_thumb_to_track_extremes_hits_top_and_bottom() {
        let metrics = terminal_scrollbar_metrics(test_bounds(120.0), 20, 80, 40)
            .expect("expected scrollbar metrics");
        let top_offset = terminal_scrollbar_offset_for_pointer(&metrics, px(0.0), 0.0);
        let bottom_offset =
            terminal_scrollbar_offset_for_pointer(&metrics, px(metrics.thumb_max_offset), 0.0);

        assert_eq!(top_offset, 80);
        assert_eq!(bottom_offset, 0);
    }

    #[test]
    fn free_type_drop_line_highlight_requires_a_visible_target_row() {
        assert_eq!(
            terminal_free_type_drop_line_row(TerminalFreeTypeTarget::new(2, 4, Side::Left), 3),
            Some(2)
        );
        assert_eq!(
            terminal_free_type_drop_line_row(TerminalFreeTypeTarget::new(-1, 4, Side::Left), 3),
            None
        );
        assert_eq!(
            terminal_free_type_drop_line_row(TerminalFreeTypeTarget::new(3, 4, Side::Left), 3),
            None
        );
    }

    #[test]
    fn hovered_link_range_stays_on_hovered_run_only() {
        let fg = default_foreground();
        let bg = default_background();
        let mut cells = "a link b link"
            .chars()
            .map(|character| {
                let mut cell = miaominal_terminal::TerminalCell::blank(fg, bg);
                cell.character = character;
                cell
            })
            .collect::<Vec<_>>();

        for cell in cells.iter_mut().take(6).skip(2) {
            cell.link = Some("https://example.test".into());
        }
        for cell in cells.iter_mut().take(13).skip(9) {
            cell.link = Some("https://example.test".into());
        }

        let hovered = TerminalHoveredLink {
            tab_id: TabId::new(7),
            line: 0,
            start_column: 9,
            end_column: 13,
            uri: "https://example.test".into(),
        };

        assert_eq!(hovered_link_range(&cells, 0, Some(&hovered)), Some((9, 13)));
    }

    #[test]
    fn custom_glyphs_are_suppressed_from_text() {
        let fg = default_foreground();
        let bg = default_background();
        let cells = ['a', '\u{2502}', 'b', '\u{2551}', 'c', '\u{2588}']
            .into_iter()
            .map(|character| {
                let mut cell = miaominal_terminal::TerminalCell::blank(fg, bg);
                cell.character = character;
                cell
            })
            .collect::<Vec<_>>();

        let text = materialize_line_text(&cells);

        assert_eq!(text, "a b c ");
    }

    #[test]
    fn shape_key_text_len_matches_materialized_text() {
        let fg = default_foreground();
        let bg = default_background();
        let mut cells = ['a', '你', 'b']
            .into_iter()
            .map(|character| {
                let mut cell = miaominal_terminal::TerminalCell::blank(fg, bg);
                cell.character = character;
                cell
            })
            .collect::<Vec<_>>();
        cells[0].zero_width.push('\u{0301}');

        let text = materialize_line_text(&cells);
        let (shape_key, _) =
            build_line_shape_key_and_runs(&cells, 0, None, &gpui_kit::font("Consolas"));

        assert_eq!(shape_key.text_len, text.len());
    }
}
