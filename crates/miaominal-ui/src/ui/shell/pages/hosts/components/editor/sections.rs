use super::super::super::super::super::*;

pub(super) fn proxy_jump_stepper_item(
    icon: IconName,
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
) -> StepperItem {
    let roles = miaominal_settings::current_theme().material.roles;

    StepperItem::new().icon(icon).child(
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_size(miaominal_settings::scaled_font_size(12.0))
                    .text_color(rgb(roles.on_surface))
                    .child(title.into()),
            )
            .child(
                div()
                    .text_size(miaominal_settings::scaled_font_size(11.0))
                    .text_color(rgb(roles.on_surface_variant))
                    .child(detail.into()),
            ),
    )
}
