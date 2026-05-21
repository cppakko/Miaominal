use super::{
    icon_button::icon_button,
    icon_tile::{IconTileTone, icon_tile},
};
use crate::ui::assets::AppIcon;
use gpui::{App, Div, IntoElement, ParentElement, SharedString, Styled, Window, div, rgb};
use gpui_component::{Icon, Sizable as _};

pub(crate) fn page_section_title(title: impl Into<SharedString>) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;

    div()
        .text_size(miaominal_settings::scaled_font_size(18.0))
        .text_color(rgb(roles.on_surface))
        .child(title.into())
}

pub(crate) fn page_view_mode_toolbar_item(
    icon: AppIcon,
    selected: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );

    icon_button(
        icon,
        34.0,
        12.0,
        if selected {
            Some(roles.primary)
        } else {
            Some(roles.surface_container_highest)
        },
        if selected {
            Some(roles.on_primary)
        } else {
            Some(text_muted)
        },
        Some(if selected {
            roles.outline
        } else {
            roles.outline_variant
        }),
        move |window, cx| on_click(window, cx),
    )
}

pub(crate) fn page_primary_icon_tile(icon: AppIcon, size: f32, corner_radius: f32) -> Div {
    icon_tile(
        Icon::new(icon).small(),
        size,
        corner_radius,
        IconTileTone::Primary,
    )
}
