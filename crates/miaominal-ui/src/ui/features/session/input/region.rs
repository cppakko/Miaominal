//! Production input-handler composition shared by terminal presentation and its
//! headless interaction tests. Window assembly supplies styling and focus scope.
use super::gestures::{TerminalInputHost, WorkspaceTerminalInputExt};
use gpui_kit::{
    Context, ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ScrollWheelEvent, StatefulInteractiveElement,
};

pub(crate) fn bind_terminal_input_handlers<V: TerminalInputHost, E: StatefulInteractiveElement>(
    surface: E,
    line_height: f32,
    cx: &Context<V>,
) -> E {
    surface
        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
            this.handle_terminal_hover(*hovered, cx);
        }))
        .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
            this.handle_terminal_modifiers_changed(event, cx);
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                window.focus(&this.active_terminal_pane().terminal_focus, cx);
                this.handle_terminal_mouse_down(event, cx);
            }),
        )
        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
            this.handle_terminal_mouse_move(event, cx);
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, event: &MouseUpEvent, _, cx| {
                this.handle_terminal_mouse_up(event, cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Middle,
            cx.listener(|this, event: &MouseDownEvent, _, cx| {
                this.handle_terminal_mouse_down(event, cx);
            }),
        )
        .on_mouse_up(
            MouseButton::Middle,
            cx.listener(|this, event: &MouseUpEvent, _, cx| {
                this.handle_terminal_mouse_up(event, cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                window.focus(&this.active_terminal_pane().terminal_focus, cx);
                this.handle_terminal_mouse_down(event, cx);
            }),
        )
        .on_mouse_up(
            MouseButton::Right,
            cx.listener(|this, event: &MouseUpEvent, _, cx| {
                this.handle_terminal_mouse_up(event, cx);
            }),
        )
        .on_key_down(cx.listener(V::handle_terminal_key_down))
        .on_key_up(cx.listener(V::handle_terminal_key_up))
        .on_scroll_wheel(cx.listener(move |this, event: &ScrollWheelEvent, _, cx| {
            this.handle_terminal_scroll_wheel(event, line_height, cx);
        }))
}
