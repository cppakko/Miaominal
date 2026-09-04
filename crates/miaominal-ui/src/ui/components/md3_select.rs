use gpui_kit::component::select::{Select, SelectDelegate, SelectState};
use gpui_kit::{Styled, px, rgb};

pub(crate) fn md3_select<D>(state: &gpui_kit::Entity<SelectState<D>>) -> Select<D>
where
    D: SelectDelegate + 'static,
{
    let roles = miaominal_settings::current_theme().material.roles;

    Select::new(state)
        .appearance(false)
        .rounded(px(14.0))
        .bg(rgb(roles.surface_container_highest))
        .text_color(rgb(roles.on_surface))
}
