use crate::ui::assets::AppIcon;
use gpui::{Div, IntoElement, ParentElement, Styled, div, px, rgb};
use gpui_component::{Icon, Sizable as _};

#[derive(Clone, Copy)]
pub(crate) enum IconTileTone {
    Primary,
    Muted,
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
            gpui::rgba(((roles.primary & 0x00ff_ffff) << 8) | 0x28),
            rgb(roles.primary),
        ),
        IconTileTone::Muted => (
            rgb(roles.surface_container_low),
            rgb(roles.on_surface_variant),
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

pub(crate) fn page_muted_icon_tile(icon: AppIcon, size: f32, corner_radius: f32) -> Div {
    icon_tile(
        Icon::new(icon).small(),
        size,
        corner_radius,
        IconTileTone::Muted,
    )
}
