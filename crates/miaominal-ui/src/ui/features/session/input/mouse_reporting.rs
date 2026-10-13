use crate::ui::contracts::TabId;
use crate::ui::features::session::SessionController;
use gpui_kit::{Context, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent};
use miaominal_terminal::{
    MouseReportButton, MouseReportKind, MouseReportModifiers, encode_mouse_report,
};

/// Application mouse reporting is bound to the terminal that received the press.
#[derive(Default)]
pub(crate) struct TerminalMouseReportingState {
    target: Option<TabId>,
    active: bool,
    last_cell: Option<(usize, usize)>,
}

impl TerminalMouseReportingState {
    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    pub(crate) fn reset(&mut self) {
        self.target = None;
        self.active = false;
        self.last_cell = None;
    }

    fn adopt_target(&mut self, target: Option<TabId>) {
        if self.target != target {
            self.reset();
            self.target = target;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalMouseMotion {
    Ignored,
    Unchanged,
    Reported,
}

fn mouse_report_button_from(button: MouseButton) -> Option<MouseReportButton> {
    match button {
        MouseButton::Left => Some(MouseReportButton::Left),
        MouseButton::Middle => Some(MouseReportButton::Middle),
        MouseButton::Right => Some(MouseReportButton::Right),
        _ => None,
    }
}

impl SessionController {
    pub(crate) fn report_terminal_mouse_press(
        &mut self,
        tab_id: Option<TabId>,
        event: &MouseDownEvent,
        cell: Option<(usize, usize)>,
        state: &mut TerminalMouseReportingState,
        cx: &mut Context<Self>,
    ) -> bool {
        state.adopt_target(tab_id);
        let Some(tab_id) = tab_id else {
            return false;
        };
        let Some(button) = mouse_report_button_from(event.button) else {
            return false;
        };
        let Some((protocol, encoding)) = self.terminal_mouse_mode(tab_id) else {
            return false;
        };
        let Some((line, column)) = cell else {
            return false;
        };
        if event.modifiers.shift || !protocol.is_enabled() {
            return false;
        }
        let modifiers = MouseReportModifiers {
            shift: false,
            alt: event.modifiers.alt,
            control: event.modifiers.control,
        };
        let Some(bytes) = encode_mouse_report(
            protocol,
            encoding,
            MouseReportKind::Press(button),
            column,
            line,
            modifiers,
        ) else {
            return false;
        };
        self.send_terminal_mouse_report(tab_id, bytes, cx);
        state.active = true;
        state.last_cell = Some((line, column));
        true
    }

    pub(crate) fn report_terminal_mouse_motion(
        &mut self,
        tab_id: Option<TabId>,
        event: &MouseMoveEvent,
        cell: Option<(usize, usize)>,
        state: &mut TerminalMouseReportingState,
        cx: &mut Context<Self>,
    ) -> TerminalMouseMotion {
        state.adopt_target(tab_id);
        let Some(tab_id) = tab_id else {
            return TerminalMouseMotion::Ignored;
        };
        let Some((protocol, encoding)) = self.terminal_mouse_mode(tab_id) else {
            return TerminalMouseMotion::Ignored;
        };
        if event.modifiers.shift || !protocol.reports_motion() {
            return TerminalMouseMotion::Ignored;
        }
        let pressed = event.pressed_button.and_then(mouse_report_button_from);
        if pressed.is_none() && !protocol.reports_motion_without_button() {
            return TerminalMouseMotion::Ignored;
        }
        let Some((line, column)) = cell else {
            return TerminalMouseMotion::Ignored;
        };
        if state.last_cell == Some((line, column)) {
            return TerminalMouseMotion::Unchanged;
        }
        let modifiers = MouseReportModifiers {
            shift: false,
            alt: event.modifiers.alt,
            control: event.modifiers.control,
        };
        let Some(bytes) = encode_mouse_report(
            protocol,
            encoding,
            MouseReportKind::Motion(pressed.unwrap_or(MouseReportButton::None)),
            column,
            line,
            modifiers,
        ) else {
            return TerminalMouseMotion::Ignored;
        };
        self.send_terminal_mouse_report(tab_id, bytes, cx);
        state.last_cell = Some((line, column));
        TerminalMouseMotion::Reported
    }

    pub(crate) fn report_terminal_mouse_release(
        &mut self,
        tab_id: Option<TabId>,
        event: &MouseUpEvent,
        cell: Option<(usize, usize)>,
        state: &mut TerminalMouseReportingState,
        cx: &mut Context<Self>,
    ) -> bool {
        state.adopt_target(tab_id);
        let was_active = std::mem::take(&mut state.active);
        let Some(tab_id) = tab_id else {
            return false;
        };
        let Some(button) = mouse_report_button_from(event.button) else {
            return false;
        };
        let Some((protocol, encoding)) = self.terminal_mouse_mode(tab_id) else {
            return false;
        };
        if !was_active || !protocol.is_enabled() {
            return false;
        }
        let (line, column) = cell.or(state.last_cell).unwrap_or((0, 0));
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
            self.send_terminal_mouse_report(tab_id, bytes, cx);
        }
        state.last_cell = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changing_terminal_drops_old_press_and_motion_coordinates() {
        let original = TabId::new(1);
        let mut state = TerminalMouseReportingState {
            target: Some(original),
            active: true,
            last_cell: Some((4, 8)),
        };
        state.adopt_target(Some(original));
        assert!(state.is_active());
        state.adopt_target(Some(TabId::new(2)));
        assert!(!state.is_active());
        assert_eq!(state.last_cell, None);
        state.reset();
        assert_eq!(state.target, None);
    }
}
