use crate::{settings, ui::assets::AppIcon};
use gpui::{
    App, Div, InteractiveElement, MouseButton, ParentElement, Styled, Window, div, px, rgb,
};
use gpui_component::{Icon, Sizable as _};

pub(crate) fn icon_button(
    icon: AppIcon,
    size: f32,
    corner_radius: f32,
    background: Option<u32>,
    foreground: Option<u32>,
    border: Option<u32>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Div {
    icon_button_base(
        size,
        corner_radius,
        background,
        foreground,
        border,
        on_click,
    )
    .child(Icon::new(icon).small())
}

pub(crate) fn icon_button_with_icon_size(
    icon: AppIcon,
    size: f32,
    icon_size: f32,
    corner_radius: f32,
    background: Option<u32>,
    foreground: Option<u32>,
    border: Option<u32>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Div {
    icon_button_base(
        size,
        corner_radius,
        background,
        foreground,
        border,
        on_click,
    )
    .child(Icon::new(icon).size(px(icon_size)))
}

fn icon_button_base(
    size: f32,
    corner_radius: f32,
    background: Option<u32>,
    foreground: Option<u32>,
    border: Option<u32>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Div {
    let mut button = div()
        .size(px(size))
        .rounded(px(corner_radius))
        .bg(rgb(if let Some(background) = background {
            background
        } else {
            settings::current_theme().material.roles.secondary_container
        }))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_color(rgb(if let Some(foreground) = foreground {
            foreground
        } else {
            settings::current_theme()
                .material
                .roles
                .on_secondary_container
        }));

    if let Some(border) = border {
        button = button.border_color(rgb(border));
    }

    button.on_mouse_down(MouseButton::Left, move |_, window, cx| on_click(window, cx))
}
