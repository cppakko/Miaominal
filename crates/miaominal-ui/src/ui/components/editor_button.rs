use gpui::{App, IntoElement, SharedString, Styled, Window, px, rgb};
use gpui_component::Disableable;
use gpui_component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
};

use super::editor_footer_actions::EDITOR_FOOTER_ACTION_HEIGHT;

pub(crate) fn editor_button(
    label: impl Into<SharedString>,
    primary: bool,
    large: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    editor_button_with_id(
        SharedString::from(format!("editor-button-{}", label.as_ref())),
        label,
        primary,
        large,
        false,
        on_click,
    )
}

pub(crate) fn editor_button_with_id(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    primary: bool,
    large: bool,
    disabled: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    let label = label.into();
    let roles = miaominal_settings::current_theme().material.roles;
    let background = if disabled {
        roles.surface_container_low
    } else if primary {
        roles.primary
    } else {
        roles.surface_container_highest
    };
    let foreground = if disabled {
        roles.on_surface_variant
    } else if primary {
        roles.on_primary
    } else {
        roles.on_surface
    };
    let mut button = Button::new(id.into())
        .ghost()
        .rounded(px(18.0))
        .border_0()
        .bg(rgb(background))
        .text_color(rgb(foreground))
        .label(label);

    button = if large {
        button
            .large()
            .min_w(px(116.0))
            .min_h(px(EDITOR_FOOTER_ACTION_HEIGHT))
    } else {
        button.small()
    };

    if disabled {
        button = button.opacity(0.58);
    }

    button
        .disabled(disabled)
        .on_click(move |_, window, cx| on_click(window, cx))
}
