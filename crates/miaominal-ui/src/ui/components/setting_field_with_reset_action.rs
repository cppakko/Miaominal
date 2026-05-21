use super::icon_button;
use crate::ui::assets::AppIcon;
use gpui::{App, Div, IntoElement, ParentElement, Styled, Window};
use gpui_component::h_flex;

pub(crate) fn setting_field_with_reset_action(
    field: impl IntoElement,
    wrap: bool,
    on_reset: impl Fn(&mut Window, &mut App) + 'static,
) -> Div {
    let roles = miaominal_settings::current_theme().material.roles;
    let mut row = h_flex().w_full().gap_3().items_center();

    if wrap {
        row = row.flex_wrap();
    }

    row.child(field).child(icon_button(
        AppIcon::Rotate,
        28.0,
        8.0,
        Some(roles.surface_container_highest),
        Some(roles.on_surface_variant),
        None,
        on_reset,
    ))
}
