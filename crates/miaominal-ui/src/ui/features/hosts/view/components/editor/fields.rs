use crate::ui::assets::AppIcon;
use crate::ui::components::{
    TextInputSurface, icon_button, surface_text_input_action_stack, surface_text_input_stack,
};
use crate::ui::i18n;
use gpui_kit::component::input::InputState;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, SharedString, Window, div, px, rgb};

pub(super) fn editor_static_field(
    label: impl Into<SharedString>,
    detail: impl Into<SharedString>,
) -> impl IntoElement {
    let label = label.into();
    let detail = detail.into();
    let material = miaominal_settings::current_theme().material;
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
                        .text_size(miaominal_settings::FontSize::Input.scaled())
                        .text_color(rgb(text_muted))
                        .child(label),
                )
                .child(
                    div()
                        .text_size(miaominal_settings::FontSize::Input.scaled())
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
    let roles = miaominal_settings::current_theme().material.roles;
    let item_id = SharedString::from(format!(
        "host-editor-environment-variable-row-{:?}",
        name_input.entity_id()
    ));

    v_flex().id(item_id).w_full().gap_2().child(
        h_flex()
            .w_full()
            .gap_2()
            .items_start()
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
            .child(div().flex_1().min_w(px(0.0)).child(if show_remove {
                surface_text_input_action_stack(
                    i18n::string("hosts.editor.environment_variables.value_label"),
                    value_input,
                    TextInputSurface::Low,
                    false,
                    icon_button(
                        AppIcon::Close,
                        30.0,
                        10.0,
                        None,
                        None,
                        Some(roles.outline_variant),
                        move |window, cx| {
                            on_remove(window, cx);
                        },
                    )
                    .into_any_element(),
                )
                .into_any_element()
            } else {
                surface_text_input_stack(
                    i18n::string("hosts.editor.environment_variables.value_label"),
                    value_input,
                    TextInputSurface::Low,
                    false,
                )
                .into_any_element()
            })),
    )
}
