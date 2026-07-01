use super::super::*;

#[derive(Clone, Copy)]
struct ShellEmptyStateLayout {
    min_height: f32,
    icon_size: f32,
    max_width: f32,
    gap: f32,
    compact: bool,
}

impl ShellEmptyStateLayout {
    fn page() -> Self {
        Self {
            min_height: 240.0,
            icon_size: 128.0,
            max_width: 380.0,
            gap: 12.0,
            compact: false,
        }
    }

    fn compact(min_height: f32) -> Self {
        Self {
            min_height,
            icon_size: 48.0,
            max_width: 260.0,
            gap: 8.0,
            compact: true,
        }
    }
}

pub(in crate::ui::shell) fn shell_empty_state(icon: AppIcon, copy: impl Into<SharedString>) -> Div {
    shell_empty_state_with_layout(icon, copy.into(), ShellEmptyStateLayout::page())
}

pub(in crate::ui::shell) fn shell_compact_empty_state(
    icon: AppIcon,
    copy: impl Into<SharedString>,
    min_height: f32,
) -> Div {
    shell_empty_state_with_layout(icon, copy.into(), ShellEmptyStateLayout::compact(min_height))
}

fn shell_empty_state_with_layout(
    icon: AppIcon,
    copy: SharedString,
    layout: ShellEmptyStateLayout,
) -> Div {
    let roles = miaominal_settings::current_theme().material.roles;
    let text_size = if layout.compact {
        miaominal_settings::FontSize::Body.scaled()
    } else {
        miaominal_settings::FontSize::Subheading.scaled()
    };
    let line_height = if layout.compact { 18.0 } else { 20.0 };

    let container = div()
        .w_full()
        .h_full()
        .min_h(px(layout.min_height))
        .flex()
        .items_center()
        .justify_center();

    let container = if layout.compact {
        container.px_3().py_2()
    } else {
        container.px_4().py_6()
    };

    container
        .child(
            v_flex()
                .max_w(px(layout.max_width))
                .items_center()
                .justify_center()
                .gap(px(layout.gap))
                .child(
                    div()
                        .size(px(layout.icon_size))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(roles.primary))
                        .child(Icon::new(icon).size(px(layout.icon_size)))
                        .text_color(rgb(roles.on_surface_variant)),
                )
                .child(
                    div()
                        .text_size(text_size)
                        .line_height(miaominal_settings::scaled_line_height(line_height))
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
