use super::card_surface;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Div, InteractiveElement, MouseButton, ParentElement, Styled, Window, px,
};
use gpui_component::h_flex;

pub(crate) fn list_item_card(
    leading: AnyElement,
    body: AnyElement,
    trailing: Option<AnyElement>,
    actions: Option<AnyElement>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Div {
    let roles = miaominal_settings::current_theme().material.roles;

    card_surface(roles.surface_container, 16.0)
        .w_full()
        .px_4()
        .py_3()
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_3()
                .child(
                    h_flex()
                        .flex_1()
                        .min_w(px(0.0))
                        .items_center()
                        .gap_3()
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| on_click(window, cx))
                        .child(leading)
                        .child(body)
                        .when_some(trailing, |this, trailing| this.child(trailing)),
                )
                .when_some(actions, |this, actions| this.child(actions)),
        )
}
