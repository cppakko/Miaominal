use super::super::*;
use crate::ui::components::{register_code_editor_input_hint, register_input_hint};

pub(in crate::ui::shell) fn new_input_state(
    placeholder: impl Into<SharedString>,
    default_value: impl Into<SharedString>,
    masked: bool,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> Entity<InputState> {
    let placeholder = placeholder.into();
    let default_value = default_value.into();

    let input = cx.new(move |cx| {
        let input = InputState::new(window, cx)
            .placeholder("")
            .default_value(default_value);

        if masked { input.masked(true) } else { input }
    });
    register_input_hint(&input, placeholder, cx);
    input
}

pub(in crate::ui::shell) fn set_input_value(
    input: &Entity<InputState>,
    value: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    let value = value.into();
    input.update(cx, move |input, cx| input.set_value(value, window, cx));
}

pub(in crate::ui::shell) fn set_input_placeholder(
    input: &Entity<InputState>,
    placeholder: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    let placeholder = placeholder.into();
    register_input_hint(input, placeholder, cx);
    input.update(cx, move |input, cx| input.set_placeholder("", window, cx));
}

pub(in crate::ui::shell) fn set_code_editor_input_placeholder(
    input: &Entity<InputState>,
    placeholder: impl Into<SharedString>,
    folding: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let placeholder = placeholder.into();
    register_code_editor_input_hint(input, placeholder, folding, cx);
    input.update(cx, move |input, cx| input.set_placeholder("", window, cx));
}

pub(in crate::ui::shell) fn set_input_masked(
    input: &Entity<InputState>,
    masked: bool,
    focus: bool,
    window: &mut Window,
    cx: &mut App,
) {
    input.update(cx, move |input, cx| {
        input.set_masked(masked, window, cx);
        if focus {
            input.focus(window, cx);
        }
    });
}
