use super::super::*;
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

fn terminal_read_only_status_message() -> String {
    i18n::string("session.terminal_messages.read_only_history")
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

impl AppView {
    pub(in crate::ui::shell) fn handle_terminal_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let should_preserve_focus = should_keep_terminal_focus_on_tab(&event.keystroke);
        let Some(input_modes) = self.active_terminal_input_modes() else {
            if should_preserve_focus {
                window.focus(
                    &self.workspace_state.workspace.active_pane.terminal_focus,
                    cx,
                );
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
                window.focus(
                    &self.workspace_state.workspace.active_pane.terminal_focus,
                    cx,
                );
                cx.stop_propagation();
            }
            return;
        };

        match action {
            TerminalKeyAction::Bytes(bytes) => self.send_terminal_bytes(bytes, cx),
            TerminalKeyAction::Scroll(scroll) => self.scroll_active_terminal(scroll, cx),
            TerminalKeyAction::Copy => {
                self.copy_terminal_selection(cx);
            }
            TerminalKeyAction::Paste => self.paste_into_terminal(cx),
            TerminalKeyAction::OpenSearch => self.open_terminal_search(window, cx),
            TerminalKeyAction::Split(direction) => self.split_active_pane(direction, window, cx),
            TerminalKeyAction::ClosePane => self.close_active_pane(window, cx),
        }

        if should_preserve_focus {
            window.focus(
                &self.workspace_state.workspace.active_pane.terminal_focus,
                cx,
            );
            cx.stop_propagation();
        }
    }

    pub(in crate::ui::shell) fn handle_terminal_key_up(
        &mut self,
        event: &KeyUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let should_preserve_focus = should_keep_terminal_focus_on_tab(&event.keystroke);
        let Some(input_modes) = self.active_terminal_input_modes() else {
            if should_preserve_focus {
                window.focus(
                    &self.workspace_state.workspace.active_pane.terminal_focus,
                    cx,
                );
                cx.stop_propagation();
            }
            return;
        };
        let key_event = TerminalKeyEvent::new(&event.keystroke, TerminalKeyPhase::Release);
        let Some(action) = classify_terminal_key(key_event, input_modes) else {
            if should_preserve_focus {
                window.focus(
                    &self.workspace_state.workspace.active_pane.terminal_focus,
                    cx,
                );
                cx.stop_propagation();
            }
            return;
        };

        if let TerminalKeyAction::Bytes(bytes) = action {
            self.send_terminal_bytes(bytes, cx);
        }

        if should_preserve_focus {
            window.focus(
                &self.workspace_state.workspace.active_pane.terminal_focus,
                cx,
            );
            cx.stop_propagation();
        }
    }

    pub(in crate::ui::shell) fn handle_terminal_scroll_wheel(
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
            && let Some((protocol, encoding)) = self.active_terminal_mouse_mode()
            && protocol.is_enabled()
            && let Some((line, column)) = self.event_position_to_viewport_cell(event.position)
        {
            let button = mouse_wheel_button_for_pixels(pixels);
            let steps = match event.delta {
                ScrollDelta::Pixels(_) => (pixels.abs() / line_height).round().max(1.0) as usize,
                ScrollDelta::Lines(point) => point.y.abs().round().max(1.0) as usize,
            };
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
                    self.send_terminal_bytes_no_scroll(bytes, cx);
                    sent_any = true;
                }
            }
            if sent_any {
                return;
            }
        }

        let multiplier = if event.modifiers.shift { 5.0 } else { 1.0 };
        let lines = ((pixels / line_height) * multiplier).round() as i32;
        if lines == 0 {
            return;
        }

        self.scroll_active_terminal(TerminalScroll::Lines(lines), cx);
    }

    fn scroll_active_terminal(&mut self, scroll: TerminalScroll, cx: &mut Context<Self>) {
        let Some(index) = self.workspace_state.workspace.active_tab else {
            return;
        };
        let Some(tab) = self.workspace_state.tabs.get_mut(index) else {
            return;
        };
        let Some(session) = tab.as_session_mut() else {
            return;
        };

        session.terminal.scroll(scroll);
        self.touch_terminal_scrollbar_visibility(self.workspace_state.workspace.active_pane_id, cx);
        cx.notify();
    }

    pub(in crate::ui::shell) fn send_terminal_bytes(
        &mut self,
        bytes: Vec<u8>,
        cx: &mut Context<Self>,
    ) {
        let active_pane_id = self.workspace_state.workspace.active_pane_id;
        let Some(index) = self.workspace_state.workspace.active_tab else {
            return;
        };
        let (was_scrolled, needs_notify) = {
            let Some(tab) = self.workspace_state.tabs.get_mut(index) else {
                return;
            };
            let Some(session) = tab.as_session_mut() else {
                return;
            };

            if session.is_terminal_read_only() {
                self.status_message = terminal_read_only_status_message();
                cx.notify();
                return;
            }

            let Some(commands) = session.commands.as_ref() else {
                self.status_message = i18n::string("session.terminal_messages.connection_starting");
                cx.notify();
                return;
            };

            let was_scrolled = session.terminal.display_offset() != 0;
            session.terminal.scroll_to_bottom();

            let len = bytes.len() as u64;
            let mut needs_notify = was_scrolled;
            if let Err(error) = commands.send_bytes(bytes) {
                session.set_connection_state(SessionConnectionState::Disconnected);
                tab.status = i18n::string("session.status.disconnected");
                let error = error.to_string();
                self.status_message = i18n::string_args(
                    "session.terminal_messages.input_failed",
                    &[("error", &error)],
                );
                needs_notify = true;
            } else {
                session.bytes_out = session.bytes_out.saturating_add(len);
            }

            (was_scrolled, needs_notify)
        };

        if was_scrolled {
            self.touch_terminal_scrollbar_visibility(active_pane_id, cx);
        }
        if needs_notify {
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn copy_terminal_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.workspace_state.workspace.active_tab else {
            return false;
        };
        let Some(tab) = self.workspace_state.tabs.get(index) else {
            return false;
        };
        let Some(session) = tab.as_session() else {
            return false;
        };

        let Some(text) = session.terminal.selection_text() else {
            self.status_message = i18n::string("session.terminal_messages.no_selection_to_copy");
            cx.notify();
            return false;
        };

        if text.is_empty() {
            self.status_message = i18n::string("session.terminal_messages.no_selection_to_copy");
            cx.notify();
            return false;
        }

        let length = text.chars().count();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        let length = length.to_string();
        self.status_message = i18n::string_args(
            "session.terminal_messages.copied_characters",
            &[("count", &length)],
        );
        cx.notify();
        true
    }

    pub(in crate::ui::shell) fn paste_into_terminal(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            self.status_message = i18n::string("session.terminal_messages.clipboard_empty");
            cx.notify();
            return;
        };

        self.send_paste_text(text, cx);
    }

    pub(in crate::ui::shell) fn send_paste_text(&mut self, text: String, cx: &mut Context<Self>) {
        let active_pane_id = self.workspace_state.workspace.active_pane_id;
        let Some(index) = self.workspace_state.workspace.active_tab else {
            return;
        };
        let was_scrolled = {
            let Some(tab) = self.workspace_state.tabs.get_mut(index) else {
                return;
            };
            let Some(session) = tab.as_session_mut() else {
                return;
            };

            if session.is_terminal_read_only() {
                self.status_message = terminal_read_only_status_message();
                cx.notify();
                return;
            }

            let Some(commands) = session.commands.as_ref() else {
                self.status_message = i18n::string("session.terminal_messages.connection_starting");
                cx.notify();
                return;
            };

            let bracketed = session.terminal.bracketed_paste_enabled();
            let bytes = sanitize_paste(&text, bracketed);
            let len = bytes.len() as u64;
            let was_scrolled = session.terminal.display_offset() != 0;
            session.terminal.scroll_to_bottom();
            if let Err(error) = commands.send_bytes(bytes) {
                session.set_connection_state(SessionConnectionState::Disconnected);
                tab.status = i18n::string("session.status.disconnected");
                let error = error.to_string();
                self.status_message = i18n::string_args(
                    "session.terminal_messages.paste_failed",
                    &[("error", &error)],
                );
            } else {
                session.bytes_out = session.bytes_out.saturating_add(len);
                let count = text.chars().count().to_string();
                self.status_message = i18n::string_args(
                    "session.terminal_messages.pasted_characters",
                    &[("count", &count)],
                );
            }

            was_scrolled
        };

        if was_scrolled {
            self.touch_terminal_scrollbar_visibility(active_pane_id, cx);
        }
        cx.notify();
    }

    pub(in crate::ui::shell) fn handle_terminal_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        if event.button == MouseButton::Left
            && let Some(metrics) = self.terminal_scrollbar_metrics()
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

            let Some(index) = self.workspace_state.workspace.active_tab else {
                return;
            };
            let Some(tab) = self.workspace_state.tabs.get_mut(index) else {
                return;
            };
            let Some(session) = tab.as_session_mut() else {
                return;
            };

            session.terminal.scroll_to_display_offset(target_offset);
            self.workspace_state.workspace.active_pane.terminal_dragging = false;
            self.set_terminal_hover_state(None, false, cx);
            self.workspace_state
                .workspace
                .active_pane
                .terminal_scrollbar_drag = Some(TerminalScrollbarDrag { thumb_grab_offset });
            self.touch_terminal_scrollbar_visibility(
                self.workspace_state.workspace.active_pane_id,
                cx,
            );
            cx.notify();
            return;
        }

        if event.button == MouseButton::Left
            && (event.modifiers.control || event.modifiers.platform)
            && let Some(link) = self.terminal_link_at_position(event.position)
        {
            let uri = link.uri.clone();
            if let Err(error) = open::that(&link.uri) {
                let error = error.to_string();
                self.status_message = i18n::string_args(
                    "session.terminal_messages.failed_to_open_link",
                    &[("uri", &uri), ("error", &error)],
                );
            } else {
                self.status_message =
                    i18n::string_args("session.terminal_messages.opened_link", &[("uri", &uri)]);
            }
            cx.notify();
            return;
        }

        if !event.modifiers.shift
            && let Some(button) = mouse_report_button_from(event.button)
            && let Some((protocol, encoding)) = self.active_terminal_mouse_mode()
            && protocol.is_enabled()
            && let Some((line, column)) = self.event_position_to_viewport_cell(event.position)
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
                self.send_terminal_bytes_no_scroll(bytes, cx);
                self.workspace_state
                    .workspace
                    .active_pane
                    .terminal_mouse_reporting_active = true;
                self.workspace_state
                    .workspace
                    .active_pane
                    .last_reported_mouse_cell = Some((line, column));
                self.workspace_state.workspace.active_pane.terminal_dragging = false;
                self.set_terminal_hover_state(None, false, cx);
                self.workspace_state
                    .workspace
                    .active_pane
                    .terminal_scrollbar_drag = None;
                return;
            }
        }

        match event.button {
            MouseButton::Left => {
                let Some((line, column, side)) =
                    self.event_position_to_cell_and_side(event.position)
                else {
                    return;
                };
                let block = event.modifiers.alt;
                let Some(index) = self.workspace_state.workspace.active_tab else {
                    return;
                };
                let Some(tab) = self.workspace_state.tabs.get_mut(index) else {
                    return;
                };
                let Some(session) = tab.as_session_mut() else {
                    return;
                };

                session.terminal.start_selection(line, column, side, block);
                self.workspace_state.workspace.active_pane.terminal_dragging = true;
                self.set_terminal_hover_state(None, false, cx);
                self.workspace_state
                    .workspace
                    .active_pane
                    .terminal_scrollbar_drag = None;
                cx.notify();
            }
            MouseButton::Middle => {
                self.handle_terminal_middle_click(cx);
            }
            _ => {}
        }
    }

    pub(in crate::ui::shell) fn handle_terminal_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        let open_modifier = event.modifiers.control || event.modifiers.platform;

        if let Some(drag) = self
            .workspace_state
            .workspace
            .active_pane
            .terminal_scrollbar_drag
        {
            self.set_terminal_hover_state(None, open_modifier, cx);
            let Some(metrics) = self.terminal_scrollbar_metrics() else {
                self.workspace_state
                    .workspace
                    .active_pane
                    .terminal_scrollbar_drag = None;
                return;
            };
            let target_offset = terminal_scrollbar_offset_for_pointer(
                &metrics,
                event.position.y,
                drag.thumb_grab_offset,
            );

            let Some(index) = self.workspace_state.workspace.active_tab else {
                self.workspace_state
                    .workspace
                    .active_pane
                    .terminal_scrollbar_drag = None;
                return;
            };
            let Some(tab) = self.workspace_state.tabs.get_mut(index) else {
                self.workspace_state
                    .workspace
                    .active_pane
                    .terminal_scrollbar_drag = None;
                return;
            };
            let Some(session) = tab.as_session_mut() else {
                self.workspace_state
                    .workspace
                    .active_pane
                    .terminal_scrollbar_drag = None;
                return;
            };

            session.terminal.scroll_to_display_offset(target_offset);
            cx.notify();
            return;
        }

        if event.pressed_button == Some(MouseButton::Left)
            && !self.workspace_state.workspace.active_pane.terminal_dragging
            && !self
                .workspace_state
                .workspace
                .active_pane
                .terminal_mouse_reporting_active
        {
            self.set_terminal_hover_state(None, open_modifier, cx);
            return;
        }

        if !self.workspace_state.workspace.active_pane.terminal_dragging
            && !self
                .workspace_state
                .workspace
                .active_pane
                .terminal_mouse_reporting_active
        {
            self.set_terminal_hover_state(Some(event.position), open_modifier, cx);
        } else {
            self.set_terminal_hover_state(None, open_modifier, cx);
        }

        if !event.modifiers.shift
            && let Some((protocol, encoding)) = self.active_terminal_mouse_mode()
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
                    self.event_position_to_viewport_cell_clamped(event.position)
            {
                if self
                    .workspace_state
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
                    self.send_terminal_bytes_no_scroll(bytes, cx);
                    self.workspace_state
                        .workspace
                        .active_pane
                        .last_reported_mouse_cell = Some((line, column));
                    return;
                }
            }
        }

        if !self.workspace_state.workspace.active_pane.terminal_dragging {
            return;
        }

        let drag_scroll_delta = self
            .workspace_state
            .workspace
            .active_pane
            .terminal_bounds
            .and_then(|bounds| {
                terminal_drag_scroll_delta(
                    event.position,
                    bounds,
                    self.workspace_state
                        .workspace
                        .active_pane
                        .terminal_line_height,
                )
            });

        if let Some(delta) = drag_scroll_delta {
            let Some(index) = self.workspace_state.workspace.active_tab else {
                return;
            };
            let Some(tab) = self.workspace_state.tabs.get_mut(index) else {
                return;
            };
            let Some(session) = tab.as_session_mut() else {
                return;
            };

            let current_offset = session.terminal.display_offset() as i32;
            let target_offset = (current_offset + delta).max(0) as usize;
            session.terminal.scroll_to_display_offset(target_offset);
        }

        let Some((line, column, side)) =
            self.event_position_to_cell_and_side_clamped(event.position)
        else {
            return;
        };

        let Some(index) = self.workspace_state.workspace.active_tab else {
            return;
        };
        let Some(tab) = self.workspace_state.tabs.get_mut(index) else {
            return;
        };
        let Some(session) = tab.as_session_mut() else {
            return;
        };

        session.terminal.update_selection(line, column, side);
        cx.notify();
    }

    pub(in crate::ui::shell) fn handle_terminal_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        cx: &mut Context<Self>,
    ) {
        if self
            .workspace_state
            .workspace
            .active_pane
            .terminal_scrollbar_drag
            .take()
            .is_some()
        {
            self.touch_terminal_scrollbar_visibility(
                self.workspace_state.workspace.active_pane_id,
                cx,
            );
            cx.notify();
            return;
        }

        if self
            .workspace_state
            .workspace
            .active_pane
            .terminal_mouse_reporting_active
            && let Some(button) = mouse_report_button_from(event.button)
            && let Some((protocol, encoding)) = self.active_terminal_mouse_mode()
            && protocol.is_enabled()
        {
            let (line, column) = self
                .event_position_to_viewport_cell_clamped(event.position)
                .or(self
                    .workspace_state
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
                self.send_terminal_bytes_no_scroll(bytes, cx);
            }
            self.workspace_state
                .workspace
                .active_pane
                .terminal_mouse_reporting_active = false;
            self.workspace_state
                .workspace
                .active_pane
                .last_reported_mouse_cell = None;
            return;
        }
        self.workspace_state
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
            return;
        }
        if !self.workspace_state.workspace.active_pane.terminal_dragging {
            self.set_terminal_hover_state(
                Some(event.position),
                event.modifiers.control || event.modifiers.platform,
                cx,
            );
            return;
        }
        self.workspace_state.workspace.active_pane.terminal_dragging = false;

        let Some(index) = self.workspace_state.workspace.active_tab else {
            return;
        };
        let Some(tab) = self.workspace_state.tabs.get(index) else {
            return;
        };
        let Some(session) = tab.as_session() else {
            return;
        };

        if !session.terminal.has_selection() {
            self.clear_terminal_selection(cx);
        }
        self.set_terminal_hover_state(
            Some(event.position),
            event.modifiers.control || event.modifiers.platform,
            cx,
        );
        cx.notify();
    }

    pub(in crate::ui::shell) fn handle_terminal_hover(
        &mut self,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        if hovered {
            return;
        }

        self.set_terminal_hover_state(None, false, cx);
    }

    pub(in crate::ui::shell) fn handle_terminal_modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        cx: &mut Context<Self>,
    ) {
        self.set_terminal_hover_state(
            self.workspace_state
                .workspace
                .active_pane
                .terminal_pointer_position,
            event.modifiers.control || event.modifiers.platform,
            cx,
        );
    }

    pub(in crate::ui::shell) fn handle_terminal_middle_click(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.workspace_state.workspace.active_tab else {
            return;
        };

        let selection_text = self
            .workspace_state
            .tabs
            .get(index)
            .and_then(TabState::as_session)
            .and_then(|session| session.terminal.selection_text())
            .filter(|text| !text.is_empty());

        let text = match selection_text {
            Some(text) => text,
            None => match cx.read_from_clipboard().and_then(|item| item.text()) {
                Some(text) => text,
                None => {
                    self.status_message =
                        i18n::string("session.terminal_messages.nothing_to_paste");
                    cx.notify();
                    return;
                }
            },
        };

        self.send_paste_text(text, cx);
    }

    fn handle_terminal_secondary_click_action(&mut self, cx: &mut Context<Self>) {
        if self.active_terminal_has_selection() {
            if self.copy_terminal_selection(cx) {
                self.clear_terminal_selection(cx);
            }
        } else {
            self.paste_into_terminal(cx);
        }
    }

    fn active_terminal_has_selection(&self) -> bool {
        self.workspace_state
            .workspace
            .active_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(TabState::as_session)
            .is_some_and(|session| session.terminal.has_selection())
    }

    fn clear_terminal_selection(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.workspace_state.workspace.active_tab else {
            return;
        };
        let Some(tab) = self.workspace_state.tabs.get_mut(index) else {
            return;
        };
        let Some(session) = tab.as_session_mut() else {
            return;
        };

        if session.terminal.has_selection() {
            session.terminal.clear_selection();
            cx.notify();
        }
    }

    fn event_position_to_cell_and_side(
        &self,
        position: Point<Pixels>,
    ) -> Option<(i32, usize, Side)> {
        self.position_to_cell_and_side(position, false)
    }

    fn event_position_to_cell_and_side_clamped(
        &self,
        position: Point<Pixels>,
    ) -> Option<(i32, usize, Side)> {
        self.position_to_cell_and_side(position, true)
    }

    fn position_to_cell_and_side(
        &self,
        position: Point<Pixels>,
        clamp_to_bounds: bool,
    ) -> Option<(i32, usize, Side)> {
        let bounds = self.workspace_state.workspace.active_pane.terminal_bounds?;
        let position = if clamp_to_bounds {
            clamp_terminal_pointer_position(position, bounds)
        } else {
            if !bounds.contains(&position) {
                return None;
            }
            position
        };

        let session = self
            .workspace_state
            .workspace
            .active_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(TabState::as_session)?;
        let columns = session.terminal.columns();
        let screen_lines = session.terminal.screen_lines();

        let rel_x = (f32::from(position.x) - f32::from(bounds.origin.x)).max(0.0);
        let rel_y = (f32::from(position.y) - f32::from(bounds.origin.y)).max(0.0);
        let cell_width = self
            .workspace_state
            .workspace
            .active_pane
            .terminal_cell_width
            .max(1.0);
        let line_height = self
            .workspace_state
            .workspace
            .active_pane
            .terminal_line_height
            .max(1.0);
        let side = terminal_selection_side(position, position, bounds, cell_width);

        let column = ((rel_x / cell_width).floor() as i32).max(0) as usize;
        let column = column.min(columns.saturating_sub(1));
        let row = (rel_y / line_height).floor() as i32;
        let row = row.min(screen_lines.saturating_sub(1) as i32);

        let display_offset = session.terminal.display_offset() as i32;

        let line = row - display_offset;
        Some((line, column, side))
    }

    fn event_position_to_viewport_cell(&self, position: Point<Pixels>) -> Option<(usize, usize)> {
        self.position_to_viewport_cell(position, false)
    }

    fn event_position_to_viewport_cell_clamped(
        &self,
        position: Point<Pixels>,
    ) -> Option<(usize, usize)> {
        self.position_to_viewport_cell(position, true)
    }

    fn position_to_viewport_cell(
        &self,
        position: Point<Pixels>,
        clamp_to_bounds: bool,
    ) -> Option<(usize, usize)> {
        let bounds = self.workspace_state.workspace.active_pane.terminal_bounds?;
        let position = if clamp_to_bounds {
            clamp_terminal_pointer_position(position, bounds)
        } else {
            if !bounds.contains(&position) {
                return None;
            }
            position
        };

        let session = self
            .workspace_state
            .workspace
            .active_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(TabState::as_session)?;
        let columns = session.terminal.columns();
        let screen_lines = session.terminal.screen_lines();

        let rel_x = (f32::from(position.x) - f32::from(bounds.origin.x)).max(0.0);
        let rel_y = (f32::from(position.y) - f32::from(bounds.origin.y)).max(0.0);
        let cell_width = self
            .workspace_state
            .workspace
            .active_pane
            .terminal_cell_width
            .max(1.0);
        let line_height = self
            .workspace_state
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

    fn terminal_link_at_position(&self, position: Point<Pixels>) -> Option<TerminalHoveredLink> {
        let index = self.workspace_state.workspace.active_tab?;
        let tab = self.workspace_state.tabs.get(index)?;
        let session = tab.as_session()?;
        let (line, column) = self.event_position_to_viewport_cell(position)?;
        let uri = session.terminal.link_at(line, column)?;

        Some(TerminalHoveredLink {
            tab_id: tab.id,
            line,
            column,
            uri,
        })
    }

    fn set_terminal_hover_state(
        &mut self,
        position: Option<Point<Pixels>>,
        open_modifier: bool,
        cx: &mut Context<Self>,
    ) {
        let hovered_link = position.and_then(|position| self.terminal_link_at_position(position));
        let changed = self
            .workspace_state
            .workspace
            .active_pane
            .terminal_pointer_position
            != position
            || self
                .workspace_state
                .workspace
                .active_pane
                .terminal_link_open_modifier
                != open_modifier
            || self
                .workspace_state
                .workspace
                .active_pane
                .terminal_hovered_link
                != hovered_link;

        self.workspace_state
            .workspace
            .active_pane
            .terminal_pointer_position = position;
        self.workspace_state
            .workspace
            .active_pane
            .terminal_link_open_modifier = open_modifier;
        self.workspace_state
            .workspace
            .active_pane
            .terminal_hovered_link = hovered_link;

        if changed {
            cx.notify();
        }
    }

    fn active_terminal_mouse_mode(&self) -> Option<(MouseProtocol, MouseEncoding)> {
        let session = self
            .workspace_state
            .workspace
            .active_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(TabState::as_session)?;
        Some((
            session.terminal.mouse_protocol(),
            session.terminal.mouse_encoding(),
        ))
    }

    fn active_terminal_input_modes(&self) -> Option<TerminalInputModes> {
        let session = self
            .workspace_state
            .workspace
            .active_tab
            .and_then(|index| self.workspace_state.tabs.get(index))
            .and_then(TabState::as_session)?;
        Some(session.terminal.input_modes())
    }

    fn touch_terminal_scrollbar_visibility(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        let interaction_at = Instant::now();

        if pane_id == self.workspace_state.workspace.active_pane_id {
            self.workspace_state
                .workspace
                .active_pane
                .terminal_scrollbar_last_interaction_at = Some(interaction_at);
        } else if let Some(parked) = self
            .workspace_state
            .workspace
            .parked_panes
            .get_mut(&pane_id)
        {
            parked.terminal_scrollbar_last_interaction_at = Some(interaction_at);
        } else {
            return;
        }

        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(TERMINAL_SCROLLBAR_IDLE_HIDE_DELAY)
                .await;

            this.update(cx, |this, cx| {
                let last_interaction_at =
                    if pane_id == this.workspace_state.workspace.active_pane_id {
                        this.workspace_state
                            .workspace
                            .active_pane
                            .terminal_scrollbar_last_interaction_at
                    } else {
                        this.workspace_state
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

    pub(in crate::ui::shell) fn terminal_scrollbar_visible(&self, pane_id: PaneId) -> bool {
        let (pointer_position, dragging_scrollbar, last_interaction_at) =
            if pane_id == self.workspace_state.workspace.active_pane_id {
                (
                    self.workspace_state
                        .workspace
                        .active_pane
                        .terminal_pointer_position,
                    self.workspace_state
                        .workspace
                        .active_pane
                        .terminal_scrollbar_drag
                        .is_some(),
                    self.workspace_state
                        .workspace
                        .active_pane
                        .terminal_scrollbar_last_interaction_at,
                )
            } else {
                let Some(parked) = self.workspace_state.workspace.parked_panes.get(&pane_id) else {
                    return false;
                };
                (
                    parked.terminal_pointer_position,
                    parked.terminal_scrollbar_drag.is_some(),
                    parked.terminal_scrollbar_last_interaction_at,
                )
            };

        let pointer_over_track = pointer_position.is_some_and(|position| {
            self.terminal_scrollbar_metrics_for_pane(pane_id)
                .is_some_and(|metrics| metrics.track_bounds.contains(&position))
        });

        terminal_scrollbar_is_visible(
            last_interaction_at,
            pointer_over_track,
            dragging_scrollbar,
            Instant::now(),
        )
    }

    pub(in crate::ui::shell) fn rebind_terminal_focus_reporting(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self._subscriptions._terminal_focus_in_subscription = cx.on_focus_in(
            &self.workspace_state.workspace.active_pane.terminal_focus,
            window,
            |this, window, cx| {
                this.sync_terminal_focus_reporting(window, cx);
            },
        );
        self._subscriptions._terminal_focus_out_subscription = cx.on_focus_out(
            &self.workspace_state.workspace.active_pane.terminal_focus,
            window,
            |this, _, window, cx| {
                this.sync_terminal_focus_reporting(window, cx);
            },
        );
    }

    pub(in crate::ui::shell) fn sync_terminal_focus_reporting(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next_reported_tab_id = self.current_terminal_focus_report_target(window);
        if next_reported_tab_id == self.workspace_state.reported_terminal_focus_tab_id {
            return;
        }

        if let Some(previous_tab_id) = self.workspace_state.reported_terminal_focus_tab_id.take() {
            self.send_focus_event_to_tab(previous_tab_id, false, cx);
        }

        if let Some(tab_id) = next_reported_tab_id
            && self.send_focus_event_to_tab(tab_id, true, cx)
        {
            self.workspace_state.reported_terminal_focus_tab_id = Some(tab_id);
        }
    }

    fn current_terminal_focus_report_target(&self, window: &Window) -> Option<usize> {
        if !window.is_window_active()
            || !self
                .workspace_state
                .workspace
                .active_pane
                .terminal_focus
                .is_focused(window)
        {
            return None;
        }

        let tab = self
            .workspace_state
            .workspace
            .active_tab
            .and_then(|index| self.workspace_state.tabs.get(index))?;
        let session = tab.as_session()?;
        session
            .terminal
            .input_modes()
            .focus_in_out
            .then_some(tab.id)
    }

    fn send_focus_event_to_tab(
        &mut self,
        tab_id: usize,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab_index) = self
            .workspace_state
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
        else {
            return false;
        };
        let Some(tab) = self.workspace_state.tabs.get_mut(tab_index) else {
            return false;
        };
        let Some(session) = tab.as_session_mut() else {
            return false;
        };
        if session.is_terminal_read_only() || !session.terminal.input_modes().focus_in_out {
            return false;
        }

        let Some(commands) = session.commands.as_ref() else {
            return false;
        };

        let bytes = if focused {
            b"\x1b[I".to_vec()
        } else {
            b"\x1b[O".to_vec()
        };
        let len = bytes.len() as u64;

        if let Err(error) = commands.send_bytes(bytes) {
            session.set_connection_state(SessionConnectionState::Disconnected);
            tab.status = i18n::string("session.status.disconnected");
            let error = error.to_string();
            self.status_message = i18n::string_args(
                "session.terminal_messages.focus_report_failed",
                &[("error", &error)],
            );
            cx.notify();
            false
        } else {
            session.bytes_out = session.bytes_out.saturating_add(len);
            true
        }
    }

    fn send_terminal_bytes_no_scroll(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        let Some(index) = self.workspace_state.workspace.active_tab else {
            return;
        };
        let Some(tab) = self.workspace_state.tabs.get_mut(index) else {
            return;
        };
        let Some(session) = tab.as_session_mut() else {
            return;
        };

        if session.is_terminal_read_only() {
            return;
        }

        let Some(commands) = session.commands.as_ref() else {
            self.status_message = i18n::string("session.terminal_messages.connection_starting");
            cx.notify();
            return;
        };

        let len = bytes.len() as u64;
        if let Err(error) = commands.send_bytes(bytes) {
            session.set_connection_state(SessionConnectionState::Disconnected);
            tab.status = i18n::string("session.status.disconnected");
            let error = error.to_string();
            self.status_message = i18n::string_args(
                "session.terminal_messages.mouse_report_failed",
                &[("error", &error)],
            );
            cx.notify();
        } else {
            session.bytes_out = session.bytes_out.saturating_add(len);
        }
    }

    fn terminal_scrollbar_metrics(&self) -> Option<TerminalScrollbarMetrics> {
        self.terminal_scrollbar_metrics_for_pane(self.workspace_state.workspace.active_pane_id)
    }

    fn terminal_scrollbar_metrics_for_pane(
        &self,
        pane_id: PaneId,
    ) -> Option<TerminalScrollbarMetrics> {
        let (bounds, tab_index) = if pane_id == self.workspace_state.workspace.active_pane_id {
            (
                self.workspace_state.workspace.active_pane.terminal_bounds?,
                self.workspace_state.workspace.active_tab?,
            )
        } else {
            let parked = self.workspace_state.workspace.parked_panes.get(&pane_id)?;
            (parked.terminal_bounds?, parked.active_tab?)
        };
        let session = self
            .workspace_state
            .tabs
            .get(tab_index)
            .and_then(TabState::as_session)?;

        terminal_scrollbar_metrics(
            bounds,
            session.terminal.screen_lines(),
            session.terminal.history_size(),
            session.terminal.display_offset(),
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

#[cfg(test)]
mod tests {
    use super::*;

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
