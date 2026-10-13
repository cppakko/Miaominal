use super::keymap::{TerminalKeyAction, classify_terminal_key, terminal_key_stroke};
use crate::ui::contracts::{TabId, TerminalWorkspaceRequest};
use crate::ui::features::session::SessionController;
use gpui_kit::{Context, KeyDownEvent, KeyUpEvent, Keystroke, ScrollDelta, ScrollWheelEvent};
use miaominal_terminal::{
    MouseReportButton, MouseReportKind, MouseReportModifiers, TerminalKeyEvent, TerminalKeyPhase,
    TerminalScroll, encode_mouse_report,
};
use std::collections::{HashMap, HashSet};

/// Travels with a pane; only the input feature interprets held local shortcuts.
#[derive(Default)]
pub(crate) struct TerminalKeyboardState {
    suppressed_releases: HashSet<String>,
    pressed_targets: HashMap<String, TabId>,
}

impl TerminalKeyboardState {
    pub(crate) fn pressed_target(&self, key: &str) -> Option<TabId> {
        self.pressed_targets.get(key).copied()
    }

    pub(crate) fn record_press(&mut self, key: &str, target: TabId, repeated: bool) -> TabId {
        if repeated {
            self.pressed_targets.get(key).copied().unwrap_or(target)
        } else {
            self.pressed_targets.insert(key.to_owned(), target);
            target
        }
    }

    pub(crate) fn take_release_target(&mut self, key: &str) -> Option<TabId> {
        self.pressed_targets.remove(key)
    }

    pub(crate) fn suppress_release(&mut self, key: &str) {
        self.suppressed_releases.insert(key.to_owned());
    }

    pub(crate) fn consume_release(&mut self, key: &str) -> bool {
        self.suppressed_releases.remove(key)
    }

    pub(crate) fn reset(&mut self) -> bool {
        let changed = !self.suppressed_releases.is_empty() || !self.pressed_targets.is_empty();
        self.suppressed_releases.clear();
        self.pressed_targets.clear();
        changed
    }
}

#[derive(Default)]
pub(crate) struct TerminalKeyDispatch {
    request: Option<TerminalWorkspaceRequest>,
    suppress_release: bool,
    preserve_focus: bool,
    scrolled: bool,
}

impl TerminalKeyDispatch {
    pub(crate) fn request(&self) -> Option<TerminalWorkspaceRequest> {
        self.request
    }
    pub(crate) fn suppress_release(&self) -> bool {
        self.suppress_release
    }
    pub(crate) fn preserve_focus(&self) -> bool {
        self.preserve_focus
    }
    pub(crate) fn scrolled(&self) -> bool {
        self.scrolled
    }
}

fn should_defer_terminal_text_input_to_ime(keystroke: &Keystroke) -> bool {
    let modifiers = &keystroke.modifiers;
    !modifiers.control
        && !modifiers.alt
        && !modifiers.platform
        && keystroke.key != "tab"
        && keystroke
            .key_char
            .as_deref()
            .is_some_and(|text| !text.is_empty() && !text.chars().all(char::is_control))
}

fn should_keep_terminal_focus_on_tab(keystroke: &Keystroke) -> bool {
    let modifiers = &keystroke.modifiers;
    keystroke.key == "tab" && !modifiers.control && !modifiers.alt && !modifiers.platform
}

impl SessionController {
    pub(crate) fn route_terminal_key_down(
        &mut self,
        tab_id: Option<TabId>,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> TerminalKeyDispatch {
        let mut dispatch = TerminalKeyDispatch {
            preserve_focus: should_keep_terminal_focus_on_tab(&event.keystroke),
            ..Default::default()
        };
        let Some((tab_id, input_modes)) =
            tab_id.and_then(|id| Some((id, self.terminal_input_modes(id)?)))
        else {
            return dispatch;
        };
        // Printable input is delivered by InputHandler, including CJK composition.
        if should_defer_terminal_text_input_to_ime(&event.keystroke) {
            return dispatch;
        }
        let stroke = terminal_key_stroke(&event.keystroke);
        let key_event = TerminalKeyEvent::new(
            &stroke,
            if event.is_held {
                TerminalKeyPhase::Repeat
            } else {
                TerminalKeyPhase::Press
            },
        );
        let Some(action) = classify_terminal_key(key_event, input_modes) else {
            return dispatch;
        };
        if miaominal_settings::current_settings().terminal_free_type_mode
            && self.terminal_has_free_type_selection(tab_id)
        {
            let modifiers = event.keystroke.modifiers;
            let plain =
                !modifiers.shift && !modifiers.control && !modifiers.alt && !modifiers.platform;
            let handled = if plain && matches!(event.keystroke.key.as_str(), "backspace" | "delete")
            {
                self.send_terminal_free_type_delete(tab_id, cx)
            } else if plain && matches!(event.keystroke.key.as_str(), "left" | "right") {
                self.send_terminal_free_type_collapse(tab_id, event.keystroke.key == "right", cx)
            } else {
                false
            };
            if handled {
                dispatch.suppress_release = true;
                return dispatch;
            }
            if !matches!(action, TerminalKeyAction::Copy | TerminalKeyAction::Paste)
                && self.clear_terminal_free_type_selection(tab_id)
            {
                cx.notify();
            }
        }
        dispatch.suppress_release = !matches!(&action, TerminalKeyAction::Bytes(_));
        match action {
            TerminalKeyAction::Bytes(bytes) => {
                self.send_terminal_input(tab_id, bytes, cx);
            }
            TerminalKeyAction::Scroll(scroll) => {
                dispatch.scrolled = self.scroll_terminal(tab_id, scroll);
                if dispatch.scrolled {
                    cx.notify();
                }
            }
            TerminalKeyAction::Copy => {
                self.copy_terminal_selection(tab_id, cx);
            }
            TerminalKeyAction::Paste => {
                self.paste_terminal_clipboard(tab_id, cx);
            }
            TerminalKeyAction::OpenSearch => {
                dispatch.request = Some(TerminalWorkspaceRequest::OpenSearch)
            }
            TerminalKeyAction::Split(direction) => {
                dispatch.request = Some(TerminalWorkspaceRequest::Split(direction))
            }
            TerminalKeyAction::ClosePane => {
                dispatch.request = Some(TerminalWorkspaceRequest::ClosePane)
            }
        }
        dispatch
    }

    pub(crate) fn route_terminal_key_up(
        &mut self,
        tab_id: Option<TabId>,
        event: &KeyUpEvent,
        keyboard: &mut TerminalKeyboardState,
        cx: &mut Context<Self>,
    ) -> bool {
        let preserve_focus = should_keep_terminal_focus_on_tab(&event.keystroke);
        if keyboard.consume_release(event.keystroke.key.as_str()) {
            return preserve_focus;
        }
        let Some((tab_id, modes)) =
            tab_id.and_then(|id| Some((id, self.terminal_input_modes(id)?)))
        else {
            return preserve_focus;
        };
        let stroke = terminal_key_stroke(&event.keystroke);
        if let Some(TerminalKeyAction::Bytes(bytes)) = classify_terminal_key(
            TerminalKeyEvent::new(&stroke, TerminalKeyPhase::Release),
            modes,
        ) {
            self.send_terminal_input(tab_id, bytes, cx);
        }
        preserve_focus
    }

    /// Returns whether viewport scrolling changed; pane geometry comes from Workspace.
    pub(crate) fn route_terminal_wheel(
        &mut self,
        tab_id: Option<TabId>,
        event: &ScrollWheelEvent,
        line_height: f32,
        viewport_cell: Option<(usize, usize)>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab_id) = tab_id else {
            return false;
        };
        let pixels = match event.delta {
            ScrollDelta::Pixels(point) => f32::from(point.y),
            ScrollDelta::Lines(point) => point.y * line_height,
        };
        if pixels.abs() < 0.1 {
            return false;
        }
        let mouse_mode = self.terminal_mouse_mode(tab_id);
        if !event.modifiers.shift
            && let Some((protocol, encoding)) = mouse_mode
            && protocol.is_enabled()
            && let Some((line, column)) = viewport_cell
        {
            let steps = terminal_wheel_scroll_lines(event.delta, pixels, line_height)
                .unsigned_abs()
                .max(1) as usize;
            let modifiers = MouseReportModifiers {
                shift: false,
                alt: event.modifiers.alt,
                control: event.modifiers.control,
            };
            let mut sent = false;
            for _ in 0..steps {
                if let Some(bytes) = encode_mouse_report(
                    protocol,
                    encoding,
                    MouseReportKind::Press(mouse_wheel_button_for_pixels(pixels)),
                    column,
                    line,
                    modifiers,
                ) {
                    self.send_terminal_mouse_report(tab_id, bytes, cx);
                    sent = true;
                }
            }
            if sent {
                return false;
            }
        }
        if !event.modifiers.shift && self.terminal_alternate_scroll_active(tab_id) {
            let lines = terminal_wheel_scroll_lines(event.delta, pixels, line_height);
            if lines != 0 {
                self.send_terminal_mouse_report(tab_id, terminal_alternate_scroll_bytes(lines), cx);
            }
            return false;
        }
        let multiplier = if event.modifiers.shift { 5.0 } else { 1.0 };
        let lines = ((pixels / line_height) * multiplier).round() as i32;
        let changed = lines != 0 && self.scroll_terminal(tab_id, TerminalScroll::Lines(lines));
        if changed {
            cx.notify();
        }
        changed
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
    fn key(
        key: &str,
        key_char: Option<&str>,
        shift: bool,
        alt: bool,
        control: bool,
        platform: bool,
    ) -> gpui_kit::Keystroke {
        gpui_kit::Keystroke {
            modifiers: gpui_kit::Modifiers {
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
        let delta = ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.0), gpui_kit::px(3.0)));

        assert_eq!(terminal_wheel_scroll_lines(delta, 3.0, 18.0), 1);
    }

    #[test]
    fn negative_line_scroll_preserves_direction() {
        let delta = ScrollDelta::Lines(gpui_kit::point(0.0, -3.0));

        assert_eq!(terminal_wheel_scroll_lines(delta, -54.0, 18.0), -3);
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
