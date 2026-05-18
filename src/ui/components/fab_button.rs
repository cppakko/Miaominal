use crate::{settings, ui::assets::AppIcon};
use gpui::{
    App, InteractiveElement, IntoElement, MouseButton, ParentElement, Styled, Window, div, px, rgb,
};
use gpui_component::{Icon, Sizable as _};

const FAB_SIZE: f32 = 52.0;

pub(crate) fn fab_button(on_click: impl Fn(&mut Window, &mut App) + 'static) -> impl IntoElement {
    fab_icon_button(AppIcon::Plus, on_click)
}

pub(crate) fn fab_icon_button(
    icon: AppIcon,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let roles = settings::current_theme().material.roles;

    div()
        .size(px(FAB_SIZE))
        .rounded(px(18.0))
        .bg(rgb(roles.primary))
        .border_color(rgb(roles.outline))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |_, window: &mut Window, cx| {
            on_click(window, cx);
        })
        .child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(roles.on_primary))
                .child(Icon::new(icon).large()),
        )
}
