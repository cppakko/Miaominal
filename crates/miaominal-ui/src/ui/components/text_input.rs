use super::icon_button;
use crate::ui::assets::AppIcon;
use gpui::{
    AnyElement, App, Div, Entity, IntoElement, ParentElement, SharedString, Styled, Window, div,
    prelude::FluentBuilder as _, px, rgb,
};
use gpui_component::{
    Icon, Sizable as _, Size, h_flex,
    input::{Input, InputState},
    v_flex,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextInputSurface {
    Low,
    Highest,
}

impl TextInputSurface {
    fn background(self) -> u32 {
        let roles = miaominal_settings::current_theme().material.roles;

        match self {
            Self::Low => roles.surface_container_low,
            Self::Highest => roles.surface_container_highest,
        }
    }
}

pub(crate) fn surface_text_input(input: &Entity<InputState>, surface: TextInputSurface) -> Input {
    let roles = miaominal_settings::current_theme().material.roles;

    Input::new(input)
        .w_full()
        .border_0()
        .rounded(px(14.0))
        .text_color(rgb(roles.on_surface))
        .bg(rgb(surface.background()))
}

pub(crate) fn field_label(label: impl Into<SharedString>, required: bool) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;

    h_flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .text_size(miaominal_settings::FontSize::Input.scaled())
                .text_color(rgb(roles.on_surface_variant))
                .child(label.into()),
        )
        .when(required, |this| {
            this.child(
                div()
                    .text_size(miaominal_settings::FontSize::Input.scaled())
                    .text_color(rgb(roles.error))
                    .child("*"),
            )
        })
}

pub(crate) fn field_stack(
    label: impl Into<SharedString>,
    required: bool,
    field: impl IntoElement,
) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap_2()
        .child(field_label(label, required))
        .child(field)
}

pub(crate) fn surface_text_input_stack(
    label: impl Into<SharedString>,
    input: Entity<InputState>,
    surface: TextInputSurface,
    required: bool,
) -> impl IntoElement {
    field_stack(label, required, surface_text_input(&input, surface).large())
}

fn render_secret_toggle_button(
    icon: AppIcon,
    disabled: bool,
    on_toggle: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let roles = miaominal_settings::current_theme().material.roles;

    if disabled {
        div()
            .size(px(30.0))
            .rounded(px(10.0))
            .bg(rgb(roles.surface_container_highest))
            .opacity(0.5)
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(roles.on_surface_variant))
            .child(Icon::new(icon).small())
            .into_any_element()
    } else {
        icon_button(
            icon,
            30.0,
            10.0,
            Some(roles.surface_container_highest),
            Some(roles.on_surface_variant),
            Some(roles.outline_variant),
            on_toggle,
        )
        .into_any_element()
    }
}

pub(crate) fn surface_secret_text_input(
    input: Entity<InputState>,
    surface: TextInputSurface,
    size: impl Into<Size>,
    disabled: bool,
    reveal_icon: AppIcon,
    on_toggle: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let size = size.into();

    h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .child(
            div().flex_1().min_w(px(0.0)).child(
                surface_text_input(&input, surface)
                    .with_size(size)
                    .disabled(disabled),
            ),
        )
        .child(div().flex_shrink_0().child(render_secret_toggle_button(
            reveal_icon,
            disabled,
            on_toggle,
        )))
}

pub(crate) struct SecretTextInputStackOptions {
    pub surface: TextInputSurface,
    pub size: Size,
    pub required: bool,
    pub disabled: bool,
    pub reveal_icon: AppIcon,
}

pub(crate) fn surface_secret_text_input_stack(
    label: impl Into<SharedString>,
    input: Entity<InputState>,
    options: SecretTextInputStackOptions,
    on_toggle: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    field_stack(
        label,
        options.required,
        surface_secret_text_input(
            input,
            options.surface,
            options.size,
            options.disabled,
            options.reveal_icon,
            on_toggle,
        ),
    )
}

pub(crate) fn surface_text_editor(
    input: &Entity<InputState>,
    height: f32,
    surface: TextInputSurface,
) -> Div {
    div()
        .w_full()
        .h(px(height))
        .rounded(px(16.0))
        .bg(rgb(surface.background()))
        .overflow_hidden()
        .child(
            Input::new(input)
                .size_full()
                .appearance(false)
                .focus_bordered(false)
                .p_3(),
        )
}

pub(crate) fn surface_text_editor_stack(
    label: impl Into<SharedString>,
    input: Entity<InputState>,
    height: f32,
    surface: TextInputSurface,
    required: bool,
) -> impl IntoElement {
    field_stack(
        label,
        required,
        surface_text_editor(&input, height, surface),
    )
}
