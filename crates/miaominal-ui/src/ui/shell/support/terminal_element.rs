use crate::ui::shell::{AppView, PaneId, TerminalHoveredLink};
use gpui::{
    Background, Bounds, Corners, DispatchPhase, FocusHandle, FontStyle, FontWeight, Hsla,
    InputHandler, IntoElement, MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString,
    StrikethroughStyle, Styled, TextAlign, TextRun, UTF16Selection, UnderlineStyle, WeakEntity,
    canvas, fill, px, quad, rgba, size,
};
use miaominal_terminal::{SearchMatchKind, TerminalSnapshot, terminal_font, terminal_font_size};
use std::ops::Range;

const TERMINAL_SCROLLBAR_TRACK_WIDTH: f32 = 6.0;
const TERMINAL_SCROLLBAR_MIN_THUMB_HEIGHT: f32 = 20.0;

struct TerminalCanvasPrepaint {
    snapshot: TerminalSnapshot,
    focus: FocusHandle,
}

struct TerminalImeHandler {
    entity: WeakEntity<AppView>,
}

impl InputHandler for TerminalImeHandler {
    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut gpui::Window,
        _cx: &mut gpui::App,
    ) -> Option<UTF16Selection> {
        None
    }

    fn marked_text_range(
        &mut self,
        _window: &mut gpui::Window,
        _cx: &mut gpui::App,
    ) -> Option<Range<usize>> {
        None
    }

    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted_range: &mut Option<Range<usize>>,
        _window: &mut gpui::Window,
        _cx: &mut gpui::App,
    ) -> Option<String> {
        None
    }

    fn replace_text_in_range(
        &mut self,
        _replacement_range: Option<Range<usize>>,
        text: &str,
        _window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) {
        if text.is_empty() {
            return;
        }
        let bytes = text.as_bytes().to_vec();
        self.entity
            .update(cx, |this, cx| this.send_terminal_bytes(bytes, cx))
            .ok();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        _new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut gpui::Window,
        _cx: &mut gpui::App,
    ) {
        // Preedit during IME composition; terminal relies on the OS IME popup for display.
    }

    fn unmark_text(&mut self, _window: &mut gpui::Window, _cx: &mut gpui::App) {}

    fn bounds_for_range(
        &mut self,
        _range: Range<usize>,
        _window: &mut gpui::Window,
        _cx: &mut gpui::App,
    ) -> Option<Bounds<Pixels>> {
        None
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut gpui::Window,
        _cx: &mut gpui::App,
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

#[allow(dead_code)] // single-pane back-compat shim; kept in case external callers reappear
pub(in crate::ui::shell) fn render_terminal_canvas(
    snapshot: TerminalSnapshot,
    hovered_link: Option<TerminalHoveredLink>,
    cell_width: f32,
    line_height: f32,
    view: WeakEntity<AppView>,
) -> impl IntoElement {
    // Back-compat shim: delegates to the pane-aware variant using the active pane.
    canvas(
        move |bounds, _window, cx| {
            view.update(cx, |this, cx| {
                let pane_id = this.active_pane_id();
                this.write_pane_terminal_metrics(pane_id, bounds, cell_width, line_height, cx);
            })
            .ok();
        },
        move |bounds, _state, window, cx| {
            paint_snapshot(
                bounds,
                &snapshot,
                hovered_link.as_ref(),
                cell_width,
                line_height,
                window,
                cx,
            );
            paint_scrollbar(bounds, &snapshot, window);
        },
    )
    .size_full()
}

pub(in crate::ui::shell) fn render_terminal_canvas_for_pane(
    hovered_link: Option<TerminalHoveredLink>,
    cell_width: f32,
    line_height: f32,
    view: WeakEntity<AppView>,
    pane_id: PaneId,
    show_scrollbar: bool,
) -> impl IntoElement {
    let view_for_paint = view.clone();
    canvas(
        move |bounds, window, cx| -> Option<TerminalCanvasPrepaint> {
            view.update(cx, |this, cx| {
                prepare_terminal_canvas_prepaint(
                    this,
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
                        entity: view_for_paint.clone(),
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
                        if this.active_pane_id() != pane_id {
                            return;
                        }

                        if this.workspace_state.workspace.active_pane.terminal_dragging
                            || this
                                .workspace_state
                                .workspace
                                .active_pane
                                .terminal_mouse_reporting_active
                            || this
                                .workspace_state
                                .workspace
                                .active_pane
                                .terminal_scrollbar_drag
                                .is_some()
                        {
                            this.handle_terminal_mouse_move(event, cx);
                        }
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
                        if this.active_pane_id() != pane_id {
                            return;
                        }

                        if this.workspace_state.workspace.active_pane.terminal_dragging
                            || this
                                .workspace_state
                                .workspace
                                .active_pane
                                .terminal_mouse_reporting_active
                            || this
                                .workspace_state
                                .workspace
                                .active_pane
                                .terminal_scrollbar_drag
                                .is_some()
                        {
                            this.handle_terminal_mouse_up(event, cx);
                        }
                    })
                    .ok();
                }
            });
        },
    )
    .size_full()
}

fn prepare_terminal_canvas_prepaint(
    this: &mut AppView,
    pane_id: PaneId,
    bounds: Bounds<Pixels>,
    cell_width: f32,
    line_height: f32,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<AppView>,
) -> Option<TerminalCanvasPrepaint> {
    let (tab_index, focus, metrics_changed) =
        if pane_id == this.workspace_state.workspace.active_pane_id {
            let metrics_changed = this.workspace_state.workspace.active_pane.terminal_bounds
                != Some(bounds)
                || this
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_cell_width
                    != cell_width
                || this
                    .workspace_state
                    .workspace
                    .active_pane
                    .terminal_line_height
                    != line_height;
            this.workspace_state.workspace.active_pane.terminal_bounds = Some(bounds);
            this.workspace_state
                .workspace
                .active_pane
                .terminal_cell_width = cell_width;
            this.workspace_state
                .workspace
                .active_pane
                .terminal_line_height = line_height;

            (
                this.workspace_state.workspace.active_tab,
                this.workspace_state
                    .workspace
                    .active_pane
                    .terminal_focus
                    .clone(),
                metrics_changed,
            )
        } else {
            let parked = this
                .workspace_state
                .workspace
                .parked_panes
                .get_mut(&pane_id)?;
            let metrics_changed = parked.terminal_bounds != Some(bounds)
                || parked.terminal_cell_width != cell_width
                || parked.terminal_line_height != line_height;
            parked.terminal_bounds = Some(bounds);
            parked.terminal_cell_width = cell_width;
            parked.terminal_line_height = line_height;

            (
                parked.active_tab,
                parked.terminal_focus.clone(),
                metrics_changed,
            )
        };

    let resized = tab_index.is_some_and(|index| {
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

    let focused =
        pane_id == this.workspace_state.workspace.active_pane_id && focus.is_focused(window);
    let snapshot = tab_index
        .and_then(|index| this.workspace_state.tabs.get(index))
        .and_then(|tab| tab.as_session())
        .map(|session| session.terminal.snapshot(focused))?;

    Some(TerminalCanvasPrepaint { snapshot, focus })
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

fn paint_snapshot(
    bounds: Bounds<Pixels>,
    snapshot: &TerminalSnapshot,
    hovered_link: Option<&TerminalHoveredLink>,
    cell_width: f32,
    line_height: f32,
    window: &mut gpui::Window,
    cx: &mut gpui::App,
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
    paint_backgrounds(snapshot, origin, cell_width_px, line_height_px, window);
    paint_search_highlights(snapshot, origin, cell_width_px, line_height_px, window);

    for (row, cells) in snapshot.cells.iter().enumerate() {
        let line_origin = Point {
            x: origin.x,
            y: origin.y + line_height_px * row as f32,
        };

        let (text, runs) = build_line_text_and_runs(cells, row, hovered_link, &terminal_font);
        if text.is_empty() {
            continue;
        }

        let shaped = window.text_system().shape_line(
            SharedString::from(text),
            font_size,
            &runs,
            Some(cell_width_px),
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
}

fn paint_backgrounds(
    snapshot: &TerminalSnapshot,
    origin: Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    window: &mut gpui::Window,
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

fn paint_unfocused_cursor(
    snapshot: &TerminalSnapshot,
    origin: Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    window: &mut gpui::Window,
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
            window.paint_quad(gpui::outline(bounds, cell.bg, gpui::BorderStyle::Solid));
        }
    }
}

fn paint_search_highlights(
    snapshot: &TerminalSnapshot,
    origin: Point<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    window: &mut gpui::Window,
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

fn build_line_text_and_runs(
    cells: &[miaominal_terminal::TerminalCell],
    row_index: usize,
    hovered_link: Option<&TerminalHoveredLink>,
    base_font: &gpui::Font,
) -> (String, Vec<TextRun>) {
    let mut text = String::with_capacity(cells.len());
    let mut runs: Vec<TextRun> = Vec::new();
    let hovered_range = hovered_link_range(cells, row_index, hovered_link);

    for (column, cell) in cells.iter().enumerate() {
        let start = text.len();
        let character = if cell.spacer || cell.character == '\0' {
            ' '
        } else {
            cell.character
        };
        text.push(character);
        if !cell.spacer {
            for ch in &cell.zero_width {
                text.push(*ch);
            }
        }
        let len = text.len() - start;

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

    (text, runs)
}

fn hovered_link_range(
    cells: &[miaominal_terminal::TerminalCell],
    row_index: usize,
    hovered_link: Option<&TerminalHoveredLink>,
) -> Option<(usize, usize)> {
    let hovered_link = hovered_link?;
    if hovered_link.line != row_index || hovered_link.column >= cells.len() {
        return None;
    }

    let hovered_uri = hovered_link.uri.as_str();
    if cells[hovered_link.column].link.as_deref() != Some(hovered_uri) {
        return None;
    }

    let mut start = hovered_link.column;
    while start > 0 && cells[start - 1].link.as_deref() == Some(hovered_uri) {
        start -= 1;
    }

    let mut end = hovered_link.column + 1;
    while end < cells.len() && cells[end].link.as_deref() == Some(hovered_uri) {
        end += 1;
    }

    Some((start, end))
}

fn runs_compatible(a: &TextRun, b: &TextRun) -> bool {
    a.font == b.font
        && a.color == b.color
        && a.background_color == b.background_color
        && a.underline == b.underline
        && a.strikethrough == b.strikethrough
}

fn paint_scrollbar(bounds: Bounds<Pixels>, snapshot: &TerminalSnapshot, window: &mut gpui::Window) {
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
    let track_color = gpui::transparent_black();
    window.paint_quad(quad(
        metrics.track_bounds,
        corners,
        Background::from(track_color),
        gpui::Edges::all(px(0.0)),
        gpui::transparent_black(),
        gpui::BorderStyle::default(),
    ));

    // MD3: thumb uses on_surface_variant at medium opacity
    let thumb_color = rgba((roles.on_surface_variant << 8) | 0xb3);
    window.paint_quad(quad(
        metrics.thumb_bounds,
        corners,
        Background::from(thumb_color),
        gpui::Edges::all(px(0.0)),
        gpui::transparent_black(),
        gpui::BorderStyle::default(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use miaominal_terminal::{default_background, default_foreground};

    fn test_bounds(height: f32) -> Bounds<Pixels> {
        Bounds {
            origin: Point {
                x: px(0.0),
                y: px(0.0),
            },
            size: size(px(120.0), px(height)),
        }
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
            tab_id: 7,
            line: 0,
            column: 3,
            uri: "https://example.test".into(),
        };

        assert_eq!(hovered_link_range(&cells, 0, Some(&hovered)), Some((2, 6)));
    }
}
