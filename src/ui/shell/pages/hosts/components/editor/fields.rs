use super::super::super::super::super::*;
use crate::ui::i18n;

pub(super) fn editor_static_field(
    label: impl Into<SharedString>,
    detail: impl Into<SharedString>,
) -> impl IntoElement {
    let label = label.into();
    let detail = detail.into();
    let material = settings::current_theme().material;
    let roles = material.roles;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );

    div()
        .w_full()
        .rounded(px(14.0))
        .bg(rgb(roles.surface_container_low))
        .px_3()
        .py_2()
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .text_size(settings::scaled_font_size(12.0))
                        .text_color(rgb(text_muted))
                        .child(label),
                )
                .child(
                    div()
                        .text_size(settings::scaled_font_size(12.0))
                        .text_color(rgb(roles.on_surface_variant))
                        .child(detail),
                ),
        )
}

pub(super) fn editor_environment_variable_row(
    _index: usize,
    name_input: Entity<InputState>,
    value_input: Entity<InputState>,
    show_remove: bool,
    on_remove: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let roles = settings::current_theme().material.roles;

    v_flex().w_full().gap_2().child(
        h_flex()
            .w_full()
            .gap_2()
            .items_end()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(surface_text_input_stack(
                        i18n::string("hosts.editor.environment_variables.name_label"),
                        name_input,
                        TextInputSurface::Low,
                        false,
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .child(surface_text_input_stack(
                        i18n::string("hosts.editor.environment_variables.value_label"),
                        value_input,
                        TextInputSurface::Low,
                        false,
                    )),
            )
            .when(show_remove, |this| {
                this.child(div().flex_shrink_0().child(icon_button(
                    AppIcon::Close,
                    30.0,
                    10.0,
                    None,
                    None,
                    Some(roles.outline_variant),
                    move |window, cx| {
                        on_remove(window, cx);
                    },
                )))
            }),
    )
}
