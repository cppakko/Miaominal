use crate::ui::assets::AppIcon;
use gpui::{
    App, Div, InteractiveElement, MouseButton, ParentElement, SharedString, Styled, Window, div,
    px, rgb,
};
use gpui_component::{
    Icon, Sizable as _,
    button::{Button, ButtonVariants as _},
};

pub(crate) struct IconButtonStyle {
    pub size: f32,
    pub corner_radius: f32,
    pub background: Option<u32>,
    pub foreground: Option<u32>,
    pub border: Option<u32>,
}

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

pub(crate) fn icon_button_with_tooltip(
    icon: AppIcon,
    tooltip: impl Into<SharedString>,
    size: f32,
    corner_radius: f32,
    background: Option<u32>,
    foreground: Option<u32>,
    border: Option<u32>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Div {
    let tooltip = tooltip.into();
    let button_id = SharedString::from(format!("icon-button-tooltip-{tooltip}"));
    let background = background.unwrap_or_else(default_icon_button_background);
    let foreground = foreground.unwrap_or_else(default_icon_button_foreground);

    let mut button = Button::new(button_id)
        .text()
        .tab_stop(false)
        .tooltip(tooltip)
        .size(px(size))
        .p_0()
        .rounded(px(corner_radius))
        .bg(rgb(background))
        .cursor_pointer()
        .text_color(rgb(foreground))
        .child(Icon::new(icon).small())
        .on_click(move |_, window, cx| on_click(window, cx));

    if let Some(border) = border {
        button = button.border_color(rgb(border));
    }

    div().size(px(size)).child(button)
}

pub(crate) fn icon_button_with_icon_size(
    icon: AppIcon,
    icon_size: f32,
    style: IconButtonStyle,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Div {
    icon_button_base(
        style.size,
        style.corner_radius,
        style.background,
        style.foreground,
        style.border,
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
        .bg(rgb(
            background.unwrap_or_else(default_icon_button_background)
        ))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_color(rgb(
            foreground.unwrap_or_else(default_icon_button_foreground)
        ));

    if let Some(border) = border {
        button = button.border_color(rgb(border));
    }

    button.on_mouse_down(MouseButton::Left, move |_, window, cx| on_click(window, cx))
}

fn default_icon_button_background() -> u32 {
    miaominal_settings::current_theme()
        .material
        .roles
        .secondary_container
}

fn default_icon_button_foreground() -> u32 {
    miaominal_settings::current_theme()
        .material
        .roles
        .on_secondary_container
}
