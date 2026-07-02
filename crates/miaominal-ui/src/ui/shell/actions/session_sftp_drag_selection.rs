use super::super::*;
use crate::ui::shell::state::{SftpDragSelectionContext, SftpDragSelectionState};
use gpui_component::scroll::ScrollbarHandle as _;

const SFTP_DRAG_SELECTION_THRESHOLD: f32 = 4.0;
const SFTP_DRAG_AUTO_SCROLL_EDGE_ZONE: f32 = 72.0;
const SFTP_DRAG_AUTO_SCROLL_MIN_STEP: f32 = 0.75;
const SFTP_DRAG_AUTO_SCROLL_MAX_STEP: f32 = 14.0;
const SFTP_DRAG_AUTO_SCROLL_MAX_RATIO: f32 = 2.25;
const SFTP_DRAG_AUTO_SCROLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);

impl AppView {
    pub(in crate::ui::shell) fn begin_sftp_drag_selection(
        &mut self,
        tab_id: usize,
        side: SftpBrowserSide,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        header_height: Pixels,
        cx: &mut Context<Self>,
    ) {
        if position.y <= bounds.origin.y + header_height {
            return;
        }

        let relative_position =
            Point::new(position.x - bounds.origin.x, position.y - bounds.origin.y);
        let scroll_offset = self.sftp_drag_selection_scroll_offset(side, cx);
        let anchor_content_y =
            relative_position.y.as_f32() - header_height.as_f32() + scroll_offset;

        let generation = {
            let Some(sftp) = self
                .workspace_state
                .tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp_mut)
            else {
                return;
            };

            sftp.drag_selection_generation = sftp.drag_selection_generation.wrapping_add(1);
            let generation = sftp.drag_selection_generation;
            sftp.drag_selection_context = Some(SftpDragSelectionContext {
                side,
                tab_id,
                last_position: position,
                panel_bounds: bounds,
                row_height: header_height,
                anchor_content_y,
                generation,
            });

            // Only record the candidate start position. The actual drag state is created lazily
            // in update_sftp_drag_selection once the pointer moves past the threshold, so that
            // a simple click never creates a drag state and never interferes with row selection.
            match side {
                SftpBrowserSide::Local => {
                    sftp.local_drag_candidate = Some(relative_position);
                    sftp.local_drag_selection = None;
                    sftp.suppress_local_clear_click = false;
                }
                SftpBrowserSide::Remote => {
                    sftp.remote_drag_candidate = Some(relative_position);
                    sftp.remote_drag_selection = None;
                    sftp.suppress_remote_clear_click = false;
                }
            }

            generation
        };

        self.start_sftp_drag_selection_auto_scroll(tab_id, generation, cx);
    }

    pub(in crate::ui::shell) fn update_active_sftp_drag_selection(
        &mut self,
        tab_id: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(context) = self.sftp_drag_selection_context_for_tab(tab_id) else {
            return false;
        };

        self.update_sftp_drag_selection(
            context.tab_id,
            context.side,
            position,
            context.panel_bounds,
            context.row_height,
            cx,
        )
    }

    pub(in crate::ui::shell) fn finish_active_sftp_drag_selection(
        &mut self,
        tab_id: usize,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(context) = self.sftp_drag_selection_context_for_tab(tab_id) else {
            return false;
        };

        self.finish_sftp_drag_selection(
            context.tab_id,
            context.side,
            position,
            context.panel_bounds,
            context.row_height,
            cx,
        )
    }

    pub(in crate::ui::shell) fn finish_any_active_sftp_drag_selection(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        let contexts = self
            .workspace_state
            .tabs
            .iter()
            .filter_map(|tab| {
                let context = tab.as_sftp()?.drag_selection_context?;
                (context.tab_id == tab.id).then_some(context)
            })
            .collect::<Vec<_>>();

        if contexts.is_empty() {
            return false;
        }

        for context in contexts {
            self.finish_sftp_drag_selection(
                context.tab_id,
                context.side,
                context.last_position,
                context.panel_bounds,
                context.row_height,
                cx,
            );
        }

        cx.notify();
        true
    }

    fn sftp_drag_selection_context_for_tab(
        &self,
        tab_id: usize,
    ) -> Option<SftpDragSelectionContext> {
        self.workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .and_then(|sftp| sftp.drag_selection_context)
            .filter(|context| context.tab_id == tab_id)
    }

    fn clear_sftp_drag_selection_context(&mut self, tab_id: usize) -> bool {
        let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        else {
            return false;
        };

        let had_context = sftp.drag_selection_context.take().is_some();
        if had_context {
            sftp.drag_selection_generation = sftp.drag_selection_generation.wrapping_add(1);
        }
        had_context
    }

    fn sftp_drag_selection_scroll_offset(&self, side: SftpBrowserSide, cx: &App) -> f32 {
        let offset = match side {
            SftpBrowserSide::Local => {
                self.workspace_forms
                    .sftp_browser
                    .local_table
                    .read(cx)
                    .vertical_scroll_handle
                    .offset()
                    .y
            }
            SftpBrowserSide::Remote => {
                self.workspace_forms
                    .sftp_browser
                    .remote_table
                    .read(cx)
                    .vertical_scroll_handle
                    .offset()
                    .y
            }
        };

        -offset.as_f32()
    }

    fn sftp_drag_selection_relative_position(
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        row_height: Pixels,
    ) -> Point<Pixels> {
        let body_top = bounds.origin.y + row_height;
        let body_bottom = (bounds.origin.y + bounds.size.height).max(body_top);
        let clamped_y = position.y.max(body_top).min(body_bottom);

        Point::new(position.x - bounds.origin.x, clamped_y - bounds.origin.y)
    }

    fn start_sftp_drag_selection_auto_scroll(
        &mut self,
        tab_id: usize,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(SFTP_DRAG_AUTO_SCROLL_INTERVAL)
                    .await;

                let keep_scrolling = this
                    .update(cx, |this, cx| {
                        this.tick_sftp_drag_selection_auto_scroll(tab_id, generation, cx)
                    })
                    .unwrap_or(false);

                if !keep_scrolling {
                    break;
                }
            }
        })
        .detach();
    }

    fn tick_sftp_drag_selection_auto_scroll(
        &mut self,
        tab_id: usize,
        generation: u64,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(context) = self.sftp_drag_selection_context_for_tab(tab_id) else {
            return false;
        };
        if context.generation != generation {
            return false;
        }

        let selection_active = self
            .workspace_state
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp)
            .is_some_and(|sftp| match context.side {
                SftpBrowserSide::Local => sftp.local_drag_selection.is_some(),
                SftpBrowserSide::Remote => sftp.remote_drag_selection.is_some(),
            });
        if !selection_active {
            return true;
        }

        let Some(step) = Self::sftp_drag_selection_auto_scroll_step(context) else {
            return true;
        };

        if self.scroll_sftp_drag_selection_table(context, step, cx) {
            self.update_sftp_drag_selection(
                context.tab_id,
                context.side,
                context.last_position,
                context.panel_bounds,
                context.row_height,
                cx,
            );
        }

        true
    }

    fn sftp_drag_selection_auto_scroll_step(context: SftpDragSelectionContext) -> Option<f32> {
        let body_top = (context.panel_bounds.origin.y + context.row_height).as_f32();
        let body_bottom =
            (context.panel_bounds.origin.y + context.panel_bounds.size.height).as_f32();
        if body_bottom <= body_top {
            return None;
        }

        let edge_zone = SFTP_DRAG_AUTO_SCROLL_EDGE_ZONE.min((body_bottom - body_top) / 2.0);
        if edge_zone < 1.0 {
            return None;
        }

        let pointer_y = context.last_position.y.as_f32();
        let hot_top = body_top + edge_zone;
        let hot_bottom = body_bottom - edge_zone;

        let signed_distance = if pointer_y < hot_top {
            -(hot_top - pointer_y)
        } else if pointer_y > hot_bottom {
            pointer_y - hot_bottom
        } else {
            return None;
        };

        let ratio = (signed_distance.abs() / edge_zone).clamp(0.0, SFTP_DRAG_AUTO_SCROLL_MAX_RATIO);
        let eased = ratio.powf(1.2);
        Some(
            (eased * SFTP_DRAG_AUTO_SCROLL_MAX_STEP).max(SFTP_DRAG_AUTO_SCROLL_MIN_STEP)
                * signed_distance.signum(),
        )
    }

    fn scroll_sftp_drag_selection_table(
        &mut self,
        context: SftpDragSelectionContext,
        step: f32,
        cx: &mut Context<Self>,
    ) -> bool {
        match context.side {
            SftpBrowserSide::Local => self
                .workspace_forms
                .sftp_browser
                .local_table
                .update(cx, |table, cx| {
                    Self::scroll_sftp_table_by_step(table, step, cx)
                }),
            SftpBrowserSide::Remote => self
                .workspace_forms
                .sftp_browser
                .remote_table
                .update(cx, |table, cx| {
                    Self::scroll_sftp_table_by_step(table, step, cx)
                }),
        }
    }

    fn scroll_sftp_table_by_step(
        table: &mut TableState<SftpBrowserTableDelegate>,
        step: f32,
        cx: &mut Context<TableState<SftpBrowserTableDelegate>>,
    ) -> bool {
        let current_offset = table.vertical_scroll_handle.offset();
        let max_offset = table
            .vertical_scroll_handle
            .0
            .borrow()
            .base_handle
            .max_offset();
        let next_y = (current_offset.y.as_f32() - step).clamp(-max_offset.y.as_f32(), 0.0);

        if (next_y - current_offset.y.as_f32()).abs() < 0.5 {
            return false;
        }

        table
            .vertical_scroll_handle
            .set_offset(Point::new(current_offset.x, px(next_y)));
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn update_sftp_drag_selection(
        &mut self,
        tab_id: usize,
        side: SftpBrowserSide,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        row_height: Pixels,
        cx: &mut Context<Self>,
    ) -> bool {
        let relative_position =
            Self::sftp_drag_selection_relative_position(position, bounds, row_height);
        let scroll_offset = self.sftp_drag_selection_scroll_offset(side, cx);

        let drag = {
            let Some(sftp) = self
                .workspace_state
                .tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp_mut)
            else {
                return false;
            };

            let anchor_view_y = if let Some(context) = sftp.drag_selection_context.as_mut()
                && context.tab_id == tab_id
                && context.side == side
            {
                context.last_position = position;
                context.panel_bounds = bounds;
                context.row_height = row_height;
                Some(px(
                    context.anchor_content_y - scroll_offset + row_height.as_f32()
                ))
            } else {
                None
            };

            let drag = match side {
                SftpBrowserSide::Local => sftp.local_drag_selection.as_mut(),
                SftpBrowserSide::Remote => sftp.remote_drag_selection.as_mut(),
            };

            if let Some(drag) = drag {
                if let Some(anchor_view_y) = anchor_view_y {
                    drag.start.y = anchor_view_y;
                }
                drag.update(relative_position);
                Some((*drag, false))
            } else {
                let candidate = match side {
                    SftpBrowserSide::Local => sftp.local_drag_candidate,
                    SftpBrowserSide::Remote => sftp.remote_drag_candidate,
                };

                candidate.and_then(|candidate_start| {
                    let mut state = SftpDragSelectionState::new(candidate_start);
                    if let Some(anchor_view_y) = anchor_view_y {
                        state.start.y = anchor_view_y;
                    }
                    state.update(relative_position);
                    state
                        .exceeds_threshold(px(SFTP_DRAG_SELECTION_THRESHOLD))
                        .then(|| {
                            match side {
                                SftpBrowserSide::Local => {
                                    sftp.local_drag_candidate = None;
                                    sftp.local_drag_selection = Some(state);
                                }
                                SftpBrowserSide::Remote => {
                                    sftp.remote_drag_candidate = None;
                                    sftp.remote_drag_selection = Some(state);
                                }
                            }
                            (state, true)
                        })
                })
            }
        };

        let Some((drag, force_selection_update)) = drag else {
            return false;
        };

        self.apply_sftp_drag_selection(tab_id, side, drag, row_height, force_selection_update, cx);
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn finish_sftp_drag_selection(
        &mut self,
        tab_id: usize,
        side: SftpBrowserSide,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        row_height: Pixels,
        cx: &mut Context<Self>,
    ) -> bool {
        let relative_position =
            Self::sftp_drag_selection_relative_position(position, bounds, row_height);
        let scroll_offset = self.sftp_drag_selection_scroll_offset(side, cx);

        let drag = {
            let Some(sftp) = self
                .workspace_state
                .tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp_mut)
            else {
                return false;
            };

            match side {
                SftpBrowserSide::Local => sftp.local_drag_candidate = None,
                SftpBrowserSide::Remote => sftp.remote_drag_candidate = None,
            }

            let mut drag = match side {
                SftpBrowserSide::Local => sftp.local_drag_selection.take(),
                SftpBrowserSide::Remote => sftp.remote_drag_selection.take(),
            };

            if let Some(context) = sftp.drag_selection_context
                && context.tab_id == tab_id
                && context.side == side
                && let Some(drag) = drag.as_mut()
            {
                drag.start.y = px(context.anchor_content_y - scroll_offset + row_height.as_f32());
            }

            drag
        };

        self.clear_sftp_drag_selection_context(tab_id);

        let Some(mut drag) = drag else {
            return false;
        };

        drag.update(relative_position);
        if !drag.exceeds_threshold(px(SFTP_DRAG_SELECTION_THRESHOLD)) {
            cx.notify();
            return false;
        }

        self.apply_sftp_drag_selection(tab_id, side, drag, row_height, true, cx);
        if let Some(sftp) = self
            .workspace_state
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(TabState::as_sftp_mut)
        {
            match side {
                SftpBrowserSide::Local => sftp.suppress_local_clear_click = true,
                SftpBrowserSide::Remote => sftp.suppress_remote_clear_click = true,
            }
        }
        cx.notify();
        true
    }

    fn apply_sftp_drag_selection(
        &mut self,
        tab_id: usize,
        side: SftpBrowserSide,
        drag: SftpDragSelectionState,
        row_height: Pixels,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        let (row_range, selected_paths) =
            self.sftp_drag_selection_paths(side, drag, row_height, cx);

        let row_range_changed = {
            let Some(sftp) = self
                .workspace_state
                .tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
                .and_then(TabState::as_sftp_mut)
            else {
                return;
            };

            let drag = match side {
                SftpBrowserSide::Local => sftp.local_drag_selection.as_mut(),
                SftpBrowserSide::Remote => sftp.remote_drag_selection.as_mut(),
            };

            if force {
                if let Some(drag) = drag {
                    drag.set_last_row_range(row_range);
                }
                true
            } else {
                drag.is_some_and(|drag| drag.set_last_row_range(row_range))
            }
        };

        if !row_range_changed {
            return;
        }

        match side {
            SftpBrowserSide::Local => {
                let selected_paths: Vec<PathBuf> =
                    selected_paths.into_iter().map(PathBuf::from).collect();
                let primary = selected_paths.first().cloned();
                self.set_sftp_local_selection(tab_id, selected_paths, primary, cx);
            }
            SftpBrowserSide::Remote => {
                let primary = selected_paths.first().cloned();
                self.set_sftp_remote_selection(tab_id, selected_paths, primary, cx);
            }
        }

        self.sync_sftp_selection_for_side(tab_id, side, cx);
    }

    fn sftp_drag_selection_paths(
        &self,
        side: SftpBrowserSide,
        drag: SftpDragSelectionState,
        row_height: Pixels,
        cx: &App,
    ) -> (Option<(usize, usize)>, Vec<String>) {
        match side {
            SftpBrowserSide::Local => {
                let table = self.workspace_forms.sftp_browser.local_table.read(cx);
                let row_range = Self::sftp_drag_selection_row_range(&table, drag, row_height);
                let selected_paths = row_range
                    .map(|(start, end)| table.delegate().paths_in_row_range(start, end))
                    .unwrap_or_default();
                (row_range, selected_paths)
            }
            SftpBrowserSide::Remote => {
                let table = self.workspace_forms.sftp_browser.remote_table.read(cx);
                let row_range = Self::sftp_drag_selection_row_range(&table, drag, row_height);
                let selected_paths = row_range
                    .map(|(start, end)| table.delegate().paths_in_row_range(start, end))
                    .unwrap_or_default();
                (row_range, selected_paths)
            }
        }
    }

    fn sftp_drag_selection_row_range(
        table: &TableState<SftpBrowserTableDelegate>,
        drag: SftpDragSelectionState,
        row_height: Pixels,
    ) -> Option<(usize, usize)> {
        let row_count = table.delegate().row_count();
        if row_count == 0 || row_height <= px(0.0) {
            return None;
        }

        let row_height_px = row_height.as_f32();
        let bounds = drag.bounds();
        let body_top = row_height_px;
        let scroll_offset = -table.vertical_scroll_handle.offset().y.as_f32();
        let content_top = bounds.origin.y.as_f32() - body_top + scroll_offset;
        let content_bottom =
            bounds.origin.y.as_f32() + bounds.size.height.as_f32() - body_top + scroll_offset;
        let total_height = row_count as f32 * row_height_px;

        if content_bottom < 0.0 || content_top >= total_height {
            return None;
        }

        let content_top = content_top.clamp(0.0, total_height);
        let content_bottom = content_bottom.clamp(0.0, total_height);
        let start_row = (content_top / row_height_px)
            .floor()
            .clamp(0.0, row_count.saturating_sub(1) as f32) as usize;
        let end_y = if content_bottom <= content_top {
            content_top
        } else {
            content_bottom - 0.1
        };
        let end_row = (end_y / row_height_px)
            .floor()
            .clamp(0.0, row_count.saturating_sub(1) as f32) as usize;

        Some((start_row, end_row))
    }
}
