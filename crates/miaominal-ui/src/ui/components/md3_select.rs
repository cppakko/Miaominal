use gpui::{Styled, px, rgb};
use gpui_component::select::{Select, SelectDelegate, SelectState};

pub(crate) fn md3_select<D>(state: &gpui::Entity<SelectState<D>>) -> Select<D>
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
