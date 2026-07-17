use super::*;
use crate::ui::i18n;
use alacritty_terminal::index::Side;
use miaominal_settings::TerminalRightClickBehavior;

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

fn should_defer_terminal_text_input_to_ime(keystroke: &gpui::Keystroke) -> bool {
    let modifiers = &keystroke.modifiers;
    !modifiers.control
        && !modifiers.alt
        && !modifiers.platform
        && keystroke.key != "tab"
        && keystroke
            .key_char
            .as_deref()
            .is_some_and(|key_char| !key_char.is_empty() && !key_char.chars().all(char::is_control))
}

fn should_keep_terminal_focus_on_tab(keystroke: &gpui::Keystroke) -> bool {
    let modifiers = &keystroke.modifiers;
    keystroke.key == "tab" && !modifiers.control && !modifiers.alt && !modifiers.platform
}

fn clamp_terminal_pointer_position(
    position: gpui::Point<gpui::Pixels>,
    bounds: gpui::Bounds<gpui::Pixels>,
) -> gpui::Point<gpui::Pixels> {
    let min_x = f32::from(bounds.origin.x);
    let max_x = min_x + f32::from(bounds.size.width);
    let min_y = f32::from(bounds.origin.y);
    let max_y = min_y + f32::from(bounds.size.height);

    gpui::Point::new(
        gpui::Pixels::from(f32::from(position.x).clamp(min_x, max_x)),
        gpui::Pixels::from(f32::from(position.y).clamp(min_y, max_y)),
    )
}

fn terminal_drag_scroll_delta(
    position: gpui::Point<gpui::Pixels>,
    bounds: gpui::Bounds<gpui::Pixels>,
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

fn send_terminal_bytes(this: &mut AppView, bytes: Vec<u8>, cx: &mut Context<AppView>) {
    let Some(tab_id) = this.workspace.workspace.active_tab else {
        return;
    };
    let controller = this.controllers.session.clone();
    controller.update(cx, |controller, cx| {
        controller.send_terminal_input(tab_id, bytes, cx)
    });
}

fn copy_terminal_selection(this: &mut AppView, cx: &mut Context<AppView>) -> bool {
    let Some(tab_id) = this.workspace.workspace.active_tab else {
        return false;
    };
    let controller = this.controllers.session.clone();
    controller.update(cx, |controller, cx| {
        controller.copy_terminal_selection(tab_id, cx)
    })
}

fn paste_into_terminal(this: &mut AppView, cx: &mut Context<AppView>) {
    let Some(tab_id) = this.workspace.workspace.active_tab else {
        return;
    };
    let controller = this.controllers.session.clone();
    controller.update(cx, |controller, cx| {
        controller.paste_terminal_clipboard(tab_id, cx)
    });
}

fn active_terminal_has_selection(this: &AppView, cx: &App) -> bool {
    this.workspace.workspace.active_tab.is_some_and(|tab_id| {
        this.controllers
            .session
            .read(cx)
            .terminal_has_selection(tab_id)
    })
}

fn clear_terminal_selection(this: &mut AppView, cx: &mut Context<AppView>) {
    let Some(tab_id) = this.workspace.workspace.active_tab else {
        return;
    };
    if this
        .controllers
        .session
        .read(cx)
        .clear_terminal_selection(tab_id)
    {
        cx.notify();
    }
}

fn active_terminal_mouse_mode(this: &AppView, cx: &App) -> Option<(MouseProtocol, MouseEncoding)> {
    let tab_id = this.workspace.workspace.active_tab?;
    this.controllers
        .session
        .read(cx)
        .terminal_mouse_mode(tab_id)
}

fn active_terminal_input_modes(this: &AppView, cx: &App) -> Option<TerminalInputModes> {
    let tab_id = this.workspace.workspace.active_tab?;
    this.controllers
        .session
        .read(cx)
        .terminal_input_modes(tab_id)
}

fn active_terminal_alternate_scroll_active(this: &AppView, cx: &App) -> bool {
    this.workspace.workspace.active_tab.is_some_and(|tab_id| {
        this.controllers
            .session
            .read(cx)
            .terminal_alternate_scroll_active(tab_id)
    })
}

fn send_terminal_bytes_no_scroll(this: &mut AppView, bytes: Vec<u8>, cx: &mut Context<AppView>) {
    let Some(tab_id) = this.workspace.workspace.active_tab else {
        return;
    };
    let controller = this.controllers.session.clone();
    controller.update(cx, |controller, cx| {
        controller.send_terminal_mouse_report(tab_id, bytes, cx);
    });
}

pub(in crate::ui::shell) trait WorkspaceTerminalInputExt: Sized {
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

    fn scroll_active_terminal(&mut self, scroll: TerminalScroll, cx: &mut Context<Self>);

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

impl WorkspaceTerminalInputExt for AppView {
    fn handle_terminal_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let should_preserve_focus = should_keep_terminal_focus_on_tab(&event.keystroke);
        let Some(input_modes) = active_terminal_input_modes(self, cx) else {
            if should_preserve_focus {
                window.focus(&self.workspace.workspace.active_pane.terminal_focus, cx);
                cx.stop_propagation();
            }
            return;
        };
        // Plain printable characters (key_char is Some, no Ctrl/Alt/Platform modifier)
        // are delivered exclusively via the IME handler's replace_text_in_range to avoid
        // double input. Only special keys and modifier combos are processed here.
        if should_defer_terminal_text_input_to_ime(&event.keystroke) {
            return;
        }

        let key_event = TerminalKeyEvent::new(
            &event.keystroke,
            if event.is_held {
                TerminalKeyPhase::Repeat
            } else {
                TerminalKeyPhase::Press
            },
        );
        let Some(action) = classify_terminal_key(key_event, input_modes) else {
            if should_preserve_focus {
                window.focus(&self.workspace.workspace.active_pane.terminal_focus, cx);
                cx.stop_propagation();
            }
            return;
        };

        match action {
            TerminalKeyAction::Bytes(bytes) => send_terminal_bytes(self, bytes, cx),
            TerminalKeyAction::Scroll(scroll) => self.scroll_active_terminal(scroll, cx),
            TerminalKeyAction::Copy => {
                copy_terminal_selection(self, cx);
            }
            TerminalKeyAction::Paste => paste_into_terminal(self, cx),
            TerminalKeyAction::OpenSearch => self.open_terminal_search(window, cx),
            TerminalKeyAction::Split(direction) => self.split_active_pane(direction, window, cx),
            TerminalKeyAction::ClosePane => self.close_active_pane(window, cx),
        }

        if should_preserve_focus {
            window.focus(&self.workspace.workspace.active_pane.terminal_focus, cx);
            cx.stop_propagation();
        }
    }

    fn handle_terminal_key_up(
        &mut self,
        event: &KeyUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let should_preserve_focus = should_keep_terminal_focus_on_tab(&event.keystroke);
        let Some(input_modes) = active_terminal_input_modes(self, cx) else {
            if should_preserve_focus {
                window.focus(&self.workspace.workspace.active_pane.terminal_focus, cx);
                cx.stop_propagation();
            }
            return;
        };
        let key_event = TerminalKeyEvent::new(&event.keystroke, TerminalKeyPhase::Release);
        let Some(action) = classify_terminal_key(key_event, input_modes) else {
            if should_preserve_focus {
                window.focus(&self.workspace.workspace.active_pane.terminal_focus, cx);
                cx.stop_propagation();
            }
            return;
        };

        if let TerminalKeyAction::Bytes(bytes) = action {
            send_terminal_bytes(self, bytes, cx);
        }

        if should_preserve_focus {
            window.focus(&self.workspace.workspace.active_pane.terminal_focus, cx);
            cx.stop_propagation();
        }
    }

    fn handle_terminal_scroll_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        line_height: f32,
        cx: &mut Context<Self>,
    ) {
        let pixels = match event.delta {
            ScrollDelta::Pixels(point) => f32::from(point.y),
            ScrollDelta::Lines(point) => point.y * line_height,
        };
        if pixels.abs() < 0.1 {
            return;
        }

        if !event.modifiers.shift
            && let Some((protocol, encoding)) = active_terminal_mouse_mode(self, cx)
            && protocol.is_enabled()
            && let Some((line, column)) = self.event_position_to_viewport_cell(event.position, cx)
        {
            let button = mouse_wheel_button_for_pixels(pixels);
            let steps = terminal_wheel_scroll_lines(event.delta, pixels, line_height)
                .unsigned_abs()
                .max(1) as usize;
            let modifiers = MouseReportModifiers {
                shift: false,
                alt: event.modifiers.alt,
                control: event.modifiers.control,
            };
            let mut sent_any = false;

            for _ in 0..steps {
                if let Some(bytes) = encode_mouse_report(
                    protocol,
                    encoding,
                    MouseReportKind::Press(button),
                    column,
                    line,
                    modifiers,
                ) {
                    send_terminal_bytes_no_scroll(self, bytes, cx);
                    sent_any = true;
                }
            }
            if sent_any {
                return;
            }
        }

        if !event.modifiers.shift && active_terminal_alternate_scroll_active(self, cx) {
            let lines = terminal_wheel_scroll_lines(event.delta, pixels, line_height);
            if lines != 0 {
                send_terminal_bytes_no_scroll(self, terminal_alternate_scroll_bytes(lines), cx);
            }
            return;
        }

        let multiplier = if event.modifiers.shift { 5.0 } else { 1.0 };
        let lines = ((pixels / line_height) * multiplier).round() as i32;
        if lines == 0 {
            return;
        }

        self.scroll_active_terminal(TerminalScroll::Lines(lines), cx);
    }

    fn scroll_active_terminal(&mut self, scroll: TerminalScroll, cx: &mut Context<Self>) {
        let Some(tab_id) = self.workspace.workspace.active_tab else {
            return;
        };
        if !self
            .controllers
            .session
            .read(cx)
            .scroll_terminal(tab_id, scroll)
        {
            return;
        }

        self.touch_terminal_scrollbar_visibility(self.workspace.workspace.active_pane_id, cx);
        cx.notify();
    }

    fn handle_terminal_mouse_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        self.clear_terminal_originated_selection_drag(cx);
        if event.button == MouseButton::Left {
            gpui_component::GlobalState::suppress_text_selection(cx);
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

            let Some(tab_id) = self.workspace.workspace.active_tab else {
                return;
            };
            if !self
                .controllers
                .session
                .read(cx)
                .scroll_terminal_to_display_offset(tab_id, target_offset)
            {
                return;
            }
            self.workspace.workspace.active_pane.terminal_dragging = false;
            self.set_terminal_hover_state(None, false, cx);
            self.workspace.workspace.active_pane.terminal_scrollbar_drag =
                Some(TerminalScrollbarDrag { thumb_grab_offset });
            self.touch_terminal_scrollbar_visibility(self.workspace.workspace.active_pane_id, cx);
            cx.notify();
            return;
        }

        if event.button == MouseButton::Left
            && (event.modifiers.control || event.modifiers.platform)
            && let Some(link) = self.terminal_link_at_position(event.position, cx)
        {
            let uri = link.uri.clone();
            if let Err(error) = open::that(uri.as_ref()) {
                let error = error.to_string();
                self.shell.status_message = i18n::string_args(
                    "session.terminal_messages.failed_to_open_link",
                    &[("uri", uri.as_ref()), ("error", &error)],
                );
            } else {
                self.shell.status_message = i18n::string_args(
                    "session.terminal_messages.opened_link",
                    &[("uri", uri.as_ref())],
                );
            }
            cx.notify();
            return;
        }

        if !event.modifiers.shift
            && let Some(button) = mouse_report_button_from(event.button)
            && let Some((protocol, encoding)) = active_terminal_mouse_mode(self, cx)
            && protocol.is_enabled()
            && let Some((line, column)) = self.event_position_to_viewport_cell(event.position, cx)
        {
            let modifiers = MouseReportModifiers {
                shift: false,
                alt: event.modifiers.alt,
                control: event.modifiers.control,
            };
            if let Some(bytes) = encode_mouse_report(
                protocol,
                encoding,
                MouseReportKind::Press(button),
                column,
                line,
                modifiers,
            ) {
                send_terminal_bytes_no_scroll(self, bytes, cx);
                self.workspace
                    .workspace
                    .active_pane
                    .terminal_mouse_reporting_active = true;
                self.workspace
                    .workspace
                    .active_pane
                    .last_reported_mouse_cell = Some((line, column));
                self.workspace.workspace.active_pane.terminal_dragging = false;
                self.set_terminal_hover_state(None, false, cx);
                self.workspace.workspace.active_pane.terminal_scrollbar_drag = None;
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
                let block = event.modifiers.alt;
                let Some(tab_id) = self.workspace.workspace.active_tab else {
                    return;
                };
                if !self
                    .controllers
                    .session
                    .read(cx)
                    .start_terminal_selection(tab_id, line, column, side, block)
                {
                    return;
                }
                self.workspace.workspace.active_pane.terminal_dragging = true;
                self.start_terminal_originated_selection_drag(cx);
                self.set_terminal_hover_state(None, false, cx);
                self.workspace.workspace.active_pane.terminal_scrollbar_drag = None;
                cx.notify();
            }
            MouseButton::Middle => {
                self.handle_terminal_middle_click(cx);
            }
            _ => {}
        }
    }

    fn handle_terminal_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let open_modifier = event.modifiers.control || event.modifiers.platform;

        if let Some(drag) = self.workspace.workspace.active_pane.terminal_scrollbar_drag {
            self.set_terminal_hover_state(None, open_modifier, cx);
            let Some(metrics) = self.terminal_scrollbar_metrics(cx) else {
                self.workspace.workspace.active_pane.terminal_scrollbar_drag = None;
                return;
            };
            let target_offset = terminal_scrollbar_offset_for_pointer(
                &metrics,
                event.position.y,
                drag.thumb_grab_offset,
            );

            let Some(tab_id) = self.workspace.workspace.active_tab else {
                self.workspace.workspace.active_pane.terminal_scrollbar_drag = None;
                return;
            };
            if !self
                .controllers
                .session
                .read(cx)
                .scroll_terminal_to_display_offset(tab_id, target_offset)
            {
                self.workspace.workspace.active_pane.terminal_scrollbar_drag = None;
                return;
            }
            cx.notify();
            return;
        }

        if event.pressed_button == Some(MouseButton::Left)
            && !self.workspace.workspace.active_pane.terminal_dragging
            && !self
                .workspace
                .workspace
                .active_pane
                .terminal_mouse_reporting_active
        {
            self.set_terminal_hover_state(None, open_modifier, cx);
            return;
        }

        if !self.workspace.workspace.active_pane.terminal_dragging
            && !self
                .workspace
                .workspace
                .active_pane
                .terminal_mouse_reporting_active
        {
            self.set_terminal_hover_state(Some(event.position), open_modifier, cx);
        } else {
            self.set_terminal_hover_state(None, open_modifier, cx);
        }

        if !event.modifiers.shift
            && let Some((protocol, encoding)) = active_terminal_mouse_mode(self, cx)
            && protocol.reports_motion()
        {
            let pressed = event.pressed_button.and_then(mouse_report_button_from);
            let allow = if pressed.is_some() {
                true
            } else {
                protocol.reports_motion_without_button()
            };
            if allow
                && let Some((line, column)) =
                    self.event_position_to_viewport_cell_clamped(event.position, cx)
            {
                if self
                    .workspace
                    .workspace
                    .active_pane
                    .last_reported_mouse_cell
                    == Some((line, column))
                {
                    return;
                }
                let button = pressed.unwrap_or(MouseReportButton::None);
                let modifiers = MouseReportModifiers {
                    shift: false,
                    alt: event.modifiers.alt,
                    control: event.modifiers.control,
                };
                if let Some(bytes) = encode_mouse_report(
                    protocol,
                    encoding,
                    MouseReportKind::Motion(button),
                    column,
                    line,
                    modifiers,
                ) {
                    send_terminal_bytes_no_scroll(self, bytes, cx);
                    self.workspace
                        .workspace
                        .active_pane
                        .last_reported_mouse_cell = Some((line, column));
                    return;
                }
            }
        }

        if !self.workspace.workspace.active_pane.terminal_dragging {
            return;
        }

        let drag_scroll_delta = self
            .workspace
            .workspace
            .active_pane
            .terminal_bounds
            .and_then(|bounds| {
                terminal_drag_scroll_delta(
                    event.position,
                    bounds,
                    self.workspace.workspace.active_pane.terminal_line_height,
                )
            });

        if let Some(delta) = drag_scroll_delta {
            let Some(tab_id) = self.workspace.workspace.active_tab else {
                return;
            };
            if !self
                .controllers
                .session
                .read(cx)
                .scroll_terminal_display_offset_by(tab_id, delta)
            {
                return;
            }
        }

        let Some((line, column, side)) =
            self.event_position_to_cell_and_side_clamped(event.position, cx)
        else {
            return;
        };

        let Some(tab_id) = self.workspace.workspace.active_tab else {
            return;
        };
        if !self
            .controllers
            .session
            .read(cx)
            .update_terminal_selection(tab_id, line, column, side)
        {
            return;
        }
        cx.notify();
    }

    fn handle_terminal_mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if self
            .workspace
            .workspace
            .active_pane
            .terminal_scrollbar_drag
            .take()
            .is_some()
        {
            self.touch_terminal_scrollbar_visibility(self.workspace.workspace.active_pane_id, cx);
            cx.notify();
            return;
        }

        if self
            .workspace
            .workspace
            .active_pane
            .terminal_mouse_reporting_active
            && let Some(button) = mouse_report_button_from(event.button)
            && let Some((protocol, encoding)) = active_terminal_mouse_mode(self, cx)
            && protocol.is_enabled()
        {
            let (line, column) = self
                .event_position_to_viewport_cell_clamped(event.position, cx)
                .or(self
                    .workspace
                    .workspace
                    .active_pane
                    .last_reported_mouse_cell)
                .unwrap_or((0, 0));
            let modifiers = MouseReportModifiers {
                shift: false,
                alt: event.modifiers.alt,
                control: event.modifiers.control,
            };
            if let Some(bytes) = encode_mouse_report(
                protocol,
                encoding,
                MouseReportKind::Release(button),
                column,
                line,
                modifiers,
            ) {
                send_terminal_bytes_no_scroll(self, bytes, cx);
            }
            self.workspace
                .workspace
                .active_pane
                .terminal_mouse_reporting_active = false;
            self.workspace
                .workspace
                .active_pane
                .last_reported_mouse_cell = None;
            return;
        }
        self.workspace
            .workspace
            .active_pane
            .terminal_mouse_reporting_active = false;

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
        if !self.workspace.workspace.active_pane.terminal_dragging {
            self.set_terminal_hover_state(
                Some(event.position),
                event.modifiers.control || event.modifiers.platform,
                cx,
            );
            self.clear_terminal_originated_selection_drag(cx);
            return;
        }
        self.workspace.workspace.active_pane.terminal_dragging = false;
        self.defer_clear_terminal_originated_selection_drag(cx);

        let Some(tab_id) = self.workspace.workspace.active_tab else {
            return;
        };
        let has_selection = self
            .controllers
            .session
            .read(cx)
            .terminal_has_selection(tab_id);
        if !has_selection {
            clear_terminal_selection(self, cx);
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
            self.workspace
                .workspace
                .active_pane
                .terminal_pointer_position,
            event.modifiers.control || event.modifiers.platform,
            cx,
        );
    }

    fn handle_terminal_middle_click(&mut self, cx: &mut Context<Self>) {
        let Some(tab_id) = self.workspace.workspace.active_tab else {
            return;
        };
        let controller = self.controllers.session.clone();
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
        self.workspace.terminal_originated_selection_drag.is_some()
    }

    fn start_terminal_originated_selection_drag(&mut self, cx: &mut Context<Self>) {
        let pane_id = self.workspace.workspace.active_pane_id;
        if self.workspace.terminal_originated_selection_drag != Some(pane_id) {
            self.workspace.terminal_originated_selection_drag = Some(pane_id);
            cx.notify();
        }
    }

    fn clear_terminal_originated_selection_drag(&mut self, cx: &mut Context<Self>) {
        if self
            .workspace
            .terminal_originated_selection_drag
            .take()
            .is_some()
        {
            cx.notify();
        }
    }

    fn defer_clear_terminal_originated_selection_drag(&mut self, cx: &mut Context<Self>) {
        let Some(pane_id) = self.workspace.terminal_originated_selection_drag else {
            return;
        };

        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(0))
                .await;

            this.update(cx, |this, cx| {
                if this.workspace.terminal_originated_selection_drag == Some(pane_id)
                    && !this.workspace.workspace.active_pane.terminal_dragging
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
        let bounds = self.workspace.workspace.active_pane.terminal_bounds?;
        let position = if clamp_to_bounds {
            clamp_terminal_pointer_position(position, bounds)
        } else {
            if !bounds.contains(&position) {
                return None;
            }
            position
        };

        let tab_id = self.workspace.workspace.active_tab?;
        let viewport = self
            .controllers
            .session
            .read(cx)
            .terminal_viewport_state(tab_id)?;
        let columns = viewport.columns;
        let screen_lines = viewport.screen_lines;

        let rel_x = (f32::from(position.x) - f32::from(bounds.origin.x)).max(0.0);
        let rel_y = (f32::from(position.y) - f32::from(bounds.origin.y)).max(0.0);
        let cell_width = self
            .workspace
            .workspace
            .active_pane
            .terminal_cell_width
            .max(1.0);
        let line_height = self
            .workspace
            .workspace
            .active_pane
            .terminal_line_height
            .max(1.0);
        let side = terminal_selection_side(position, position, bounds, cell_width);

        let column = ((rel_x / cell_width).floor() as i32).max(0) as usize;
        let column = column.min(columns.saturating_sub(1));
        let row = (rel_y / line_height).floor() as i32;
        let row = row.min(screen_lines.saturating_sub(1) as i32);

        let display_offset = viewport.display_offset as i32;

        let line = row - display_offset;
        Some((line, column, side))
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
        let bounds = self.workspace.workspace.active_pane.terminal_bounds?;
        let position = if clamp_to_bounds {
            clamp_terminal_pointer_position(position, bounds)
        } else {
            if !bounds.contains(&position) {
                return None;
            }
            position
        };

        let tab_id = self.workspace.workspace.active_tab?;
        let viewport = self
            .controllers
            .session
            .read(cx)
            .terminal_viewport_state(tab_id)?;
        let columns = viewport.columns;
        let screen_lines = viewport.screen_lines;

        let rel_x = (f32::from(position.x) - f32::from(bounds.origin.x)).max(0.0);
        let rel_y = (f32::from(position.y) - f32::from(bounds.origin.y)).max(0.0);
        let cell_width = self
            .workspace
            .workspace
            .active_pane
            .terminal_cell_width
            .max(1.0);
        let line_height = self
            .workspace
            .workspace
            .active_pane
            .terminal_line_height
            .max(1.0);

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
        let tab_id = self.workspace.workspace.active_tab?;
        let link = self
            .controllers
            .session
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
        let previous_modifier = self
            .workspace
            .workspace
            .active_pane
            .terminal_link_open_modifier;
        let previous_link = self
            .workspace
            .workspace
            .active_pane
            .terminal_hovered_link
            .clone();
        let active_terminal = self.workspace.workspace.active_tab.and_then(|tab_id| {
            self.controllers
                .session
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
        let previous_query = self.workspace.workspace.active_pane.terminal_link_query;
        let should_query = open_modifier && query != previous_query;
        let hovered_link = if !open_modifier {
            None
        } else if should_query {
            cell.and_then(|(line, column)| self.terminal_link_at_cell(line, column, cx))
        } else {
            previous_link.clone()
        };
        let changed = previous_modifier != open_modifier || previous_link != hovered_link;

        self.workspace
            .workspace
            .active_pane
            .terminal_pointer_position = position;
        self.workspace.workspace.active_pane.terminal_link_query =
            open_modifier.then_some(query).flatten();
        self.workspace
            .workspace
            .active_pane
            .terminal_link_open_modifier = open_modifier;
        self.workspace.workspace.active_pane.terminal_hovered_link = hovered_link;

        if changed {
            cx.notify();
        }
    }

    fn touch_terminal_scrollbar_visibility(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        let interaction_at = Instant::now();

        if pane_id == self.workspace.workspace.active_pane_id {
            self.workspace
                .workspace
                .active_pane
                .terminal_scrollbar_last_interaction_at = Some(interaction_at);
        } else if let Some(parked) = self.workspace.workspace.parked_panes.get_mut(&pane_id) {
            parked.terminal_scrollbar_last_interaction_at = Some(interaction_at);
        } else {
            return;
        }

        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(TERMINAL_SCROLLBAR_IDLE_HIDE_DELAY)
                .await;

            this.update(cx, |this, cx| {
                let last_interaction_at = if pane_id == this.workspace.workspace.active_pane_id {
                    this.workspace
                        .workspace
                        .active_pane
                        .terminal_scrollbar_last_interaction_at
                } else {
                    this.workspace
                        .workspace
                        .parked_panes
                        .get(&pane_id)
                        .and_then(|pane| pane.terminal_scrollbar_last_interaction_at)
                };

                if last_interaction_at == Some(interaction_at) {
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn terminal_scrollbar_visible(&self, pane_id: PaneId, cx: &App) -> bool {
        let (pointer_position, dragging_scrollbar, last_interaction_at) =
            if pane_id == self.workspace.workspace.active_pane_id {
                (
                    self.workspace
                        .workspace
                        .active_pane
                        .terminal_pointer_position,
                    self.workspace
                        .workspace
                        .active_pane
                        .terminal_scrollbar_drag
                        .is_some(),
                    self.workspace
                        .workspace
                        .active_pane
                        .terminal_scrollbar_last_interaction_at,
                )
            } else {
                let Some(parked) = self.workspace.workspace.parked_panes.get(&pane_id) else {
                    return false;
                };
                (
                    parked.terminal_pointer_position,
                    parked.terminal_scrollbar_drag.is_some(),
                    parked.terminal_scrollbar_last_interaction_at,
                )
            };

        let pointer_over_track = pointer_position.is_some_and(|position| {
            self.terminal_scrollbar_metrics_for_pane(pane_id, cx)
                .is_some_and(|metrics| metrics.track_bounds.contains(&position))
        });

        terminal_scrollbar_is_visible(
            last_interaction_at,
            pointer_over_track,
            dragging_scrollbar,
            Instant::now(),
        )
    }

    fn rebind_terminal_focus_reporting(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let terminal_focus = self.workspace.workspace.active_pane.terminal_focus.clone();
        self.controllers.session.update(cx, |controller, cx| {
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
                .controllers
                .session
                .read(cx)
                .reported_terminal_focus_tab_id()
        {
            return;
        }

        self.release_reported_terminal_focus(cx);

        if let Some(tab_id) = next_reported_tab_id
            && {
                let controller = self.controllers.session.clone();
                controller.update(cx, |controller, cx| {
                    controller.send_terminal_focus_report(tab_id, true, cx)
                })
            }
        {
            self.controllers
                .session
                .read(cx)
                .set_reported_terminal_focus_tab_id(Some(tab_id));
        }
    }

    fn clear_terminal_focus_reporting(&mut self, cx: &mut Context<Self>) {
        self.clear_terminal_originated_selection_drag(cx);
        self.release_reported_terminal_focus(cx);
    }

    fn release_reported_terminal_focus(&mut self, cx: &mut Context<Self>) {
        if let Some(previous_tab_id) = self
            .controllers
            .session
            .read(cx)
            .take_reported_terminal_focus_tab_id()
        {
            let controller = self.controllers.session.clone();
            controller.update(cx, |controller, cx| {
                controller.send_terminal_focus_report(previous_tab_id, false, cx);
            });
        }
    }

    fn current_terminal_focus_report_target(&self, window: &Window, cx: &App) -> Option<TabId> {
        if !window.is_window_active()
            || !self
                .workspace
                .workspace
                .active_pane
                .terminal_focus
                .is_focused(window)
        {
            return None;
        }

        let tab_id = self.workspace.workspace.active_tab?;
        self.controllers
            .session
            .read(cx)
            .terminal_input_modes(tab_id)?
            .focus_in_out
            .then_some(tab_id)
    }

    fn terminal_scrollbar_metrics(&self, cx: &App) -> Option<TerminalScrollbarMetrics> {
        self.terminal_scrollbar_metrics_for_pane(self.workspace.workspace.active_pane_id, cx)
    }

    fn terminal_scrollbar_metrics_for_pane(
        &self,
        pane_id: PaneId,
        cx: &App,
    ) -> Option<TerminalScrollbarMetrics> {
        let (bounds, tab_id) = if pane_id == self.workspace.workspace.active_pane_id {
            (
                self.workspace.workspace.active_pane.terminal_bounds?,
                self.workspace.workspace.active_tab?,
            )
        } else {
            let parked = self.workspace.workspace.parked_panes.get(&pane_id)?;
            (parked.terminal_bounds?, parked.active_tab?)
        };
        let viewport = self
            .controllers
            .session
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

fn mouse_report_button_from(button: MouseButton) -> Option<MouseReportButton> {
    match button {
        MouseButton::Left => Some(MouseReportButton::Left),
        MouseButton::Middle => Some(MouseReportButton::Middle),
        MouseButton::Right => Some(MouseReportButton::Right),
        _ => None,
    }
}

fn mouse_wheel_button_for_pixels(pixels: f32) -> MouseReportButton {
    if pixels.is_sign_positive() {
        MouseReportButton::WheelUp
    } else {
        MouseReportButton::WheelDown
    }
}

fn terminal_wheel_scroll_lines(delta: ScrollDelta, pixels: f32, line_height: f32) -> i32 {
    let lines = match delta {
        ScrollDelta::Pixels(_) => (pixels.abs() / line_height.max(1.0)).round().max(1.0) as i32,
        ScrollDelta::Lines(point) => point.y.abs().round().max(1.0) as i32,
    };

    if pixels.is_sign_positive() {
        lines
    } else {
        -lines
    }
}

fn terminal_alternate_scroll_bytes(lines: i32) -> Vec<u8> {
    let command = if lines > 0 { b'A' } else { b'B' };
    let mut bytes = Vec::with_capacity(lines.unsigned_abs() as usize * 3);
    for _ in 0..lines.unsigned_abs() {
        bytes.extend_from_slice(&[0x1b, b'O', command]);
    }
    bytes
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

    fn key(
        key: &str,
        key_char: Option<&str>,
        shift: bool,
        alt: bool,
        control: bool,
        platform: bool,
    ) -> gpui::Keystroke {
        gpui::Keystroke {
            modifiers: gpui::Modifiers {
                shift,
                control,
                alt,
                platform,
                function: false,
            },
            key: key.into(),
            key_char: key_char.map(Into::into),
        }
    }

    #[test]
    fn positive_scroll_delta_reports_wheel_up() {
        assert_eq!(
            mouse_wheel_button_for_pixels(1.0),
            MouseReportButton::WheelUp
        );
    }

    #[test]
    fn negative_scroll_delta_reports_wheel_down() {
        assert_eq!(
            mouse_wheel_button_for_pixels(-1.0),
            MouseReportButton::WheelDown
        );
    }

    #[test]
    fn positive_alternate_scroll_generates_up_keys() {
        assert_eq!(terminal_alternate_scroll_bytes(2), b"\x1bOA\x1bOA".to_vec());
    }

    #[test]
    fn negative_alternate_scroll_generates_down_keys() {
        assert_eq!(
            terminal_alternate_scroll_bytes(-2),
            b"\x1bOB\x1bOB".to_vec()
        );
    }

    #[test]
    fn small_pixel_scroll_still_generates_one_terminal_line() {
        let delta = ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(3.0)));

        assert_eq!(terminal_wheel_scroll_lines(delta, 3.0, 18.0), 1);
    }

    #[test]
    fn negative_line_scroll_preserves_direction() {
        let delta = ScrollDelta::Lines(gpui::point(0.0, -3.0));

        assert_eq!(terminal_wheel_scroll_lines(delta, -54.0, 18.0), -3);
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

    #[test]
    fn plain_character_input_is_deferred_to_ime() {
        let keystroke = key("a", Some("a"), false, false, false, false);

        assert!(should_defer_terminal_text_input_to_ime(&keystroke));
    }

    #[test]
    fn tab_input_is_not_deferred_to_ime() {
        let keystroke = key("tab", Some("\t"), false, false, false, false);

        assert!(!should_defer_terminal_text_input_to_ime(&keystroke));
    }

    #[test]
    fn return_input_is_not_deferred_to_ime() {
        let keystroke = key("return", Some("\r"), false, false, false, false);

        assert!(!should_defer_terminal_text_input_to_ime(&keystroke));
    }

    #[test]
    fn tab_input_keeps_terminal_focus() {
        let keystroke = key("tab", Some("\t"), false, false, false, false);

        assert!(should_keep_terminal_focus_on_tab(&keystroke));
    }

    #[test]
    fn control_tab_does_not_force_terminal_focus() {
        let keystroke = key("tab", Some("\t"), false, false, true, false);

        assert!(!should_keep_terminal_focus_on_tab(&keystroke));
    }
}
