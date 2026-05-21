use super::super::*;

pub(in crate::ui::shell) fn shell_empty_state(icon: AppIcon, copy: impl Into<SharedString>) -> Div {
    let copy = copy.into();
    let roles = miaominal_settings::current_theme().material.roles;

    div()
        .w_full()
        .min_h(px(240.0))
        .px_4()
        .py_6()
        .flex()
        .items_center()
        .justify_center()
        .child(
            v_flex()
                .max_w(px(380.0))
                .items_center()
                .justify_center()
                .gap_3()
                .child(
                    div()
                        .size(px(128.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(roles.primary))
                        .child(Icon::new(icon).size(px(128.0)))
                        .text_color(rgb(roles.on_surface_variant)),
                )
                .child(
                    div()
                        .text_size(miaominal_settings::scaled_font_size(13.0))
                        .line_height(miaominal_settings::scaled_line_height(20.0))
                        .text_center()
                        .text_color(rgb(roles.on_surface_variant))
                        .child(copy),
                ),
        )
}

pub(in crate::ui::shell) fn shell_empty_page(icon: AppIcon, copy: impl Into<SharedString>) -> Div {
    div()
        .size_full()
        .px_5()
        .py_4()
        .child(shell_empty_state(icon, copy).h_full().min_h(px(420.0)))
}
