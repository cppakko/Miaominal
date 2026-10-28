use gpui_kit::{Div, IntoElement, ParentElement, Styled, div, px, rgb};

#[derive(Clone, Copy)]
pub(crate) enum IconTileTone {
    Primary,
}

pub(crate) fn icon_tile(
    content: impl IntoElement,
    size: f32,
    corner_radius: f32,
    tone: IconTileTone,
) -> Div {
    let roles = miaominal_settings::current_theme().material.roles;
    let (background, foreground) = match tone {
        IconTileTone::Primary => (
            gpui_kit::rgba(((roles.primary & 0x00ff_ffff) << 8) | 0x28),
            rgb(roles.primary),
        ),
    };

    div()
        .size(px(size))
        .rounded(px(corner_radius))
        .bg(background)
        .flex()
        .items_center()
        .justify_center()
        .text_color(foreground)
        .child(content)
}
