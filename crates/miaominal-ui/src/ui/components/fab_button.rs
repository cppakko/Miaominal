use crate::ui::assets::AppIcon;
use crate::ui::components::shell_tokens::{ShellColors, ShellMetrics, ShellTypography, space};
use gpui_kit::component::{
    Icon, Sizable as _,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    h_flex,
};
use gpui_kit::{
    App, ElementId, FontWeight, InteractiveElement, ParentElement, SharedString, Styled, Window,
};

/// The page's primary create command, docked into the bottom navigation bar.
/// It shares the navigation items' height and pill shape, so the bar reads as
/// one control surface; the tonal container alone carries its emphasis.
pub(crate) fn dock_action_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    let label = label.into();
    fab_base(id, label.clone(), cx, on_click)
        .h(space(ShellMetrics::NAV_INDICATOR_HEIGHT))
        .rounded_full()
        .pl(space(16.0))
        .pr(space(20.0))
        .child(
            h_flex()
                .gap(space(8.0))
                .child(Icon::new(AppIcon::Plus).size(space(ShellMetrics::DOCK_ACTION_ICON_SIZE)))
                .child(label),
        )
}

/// An icon-only page command docked into the bottom navigation bar. It uses the
/// navigation items' footprint. An emphasized action is the page's primary
/// command (tonal container); otherwise it reads like a navigation item so a
/// secondary command, such as refresh, does not compete with the primary one.
pub(crate) fn dock_icon_action_button(
    id: impl Into<ElementId>,
    icon: AppIcon,
    label: impl Into<SharedString>,
    emphasized: bool,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    let label = label.into();
    let colors = ShellColors::current();
    let button = fab_base(id, label.clone(), cx, on_click)
        .tooltip(label)
        .w(space(ShellMetrics::NAV_INDICATOR_WIDTH))
        .h(space(ShellMetrics::NAV_INDICATOR_HEIGHT))
        .rounded_full()
        .p_0()
        .child(Icon::new(icon).size(space(ShellMetrics::DOCK_ACTION_ICON_SIZE)));
    if emphasized {
        return button;
    }
    let background = colors.navigation_bar;
    let foreground = colors.on_navigation_bar;
    button
        .custom(
            ButtonCustomVariant::new(cx)
                .color(background)
                .foreground(foreground)
                .hover(background.blend(foreground.opacity(0.08)))
                .active(background.blend(foreground.opacity(0.12))),
        )
        .bg(background)
        .text_color(foreground)
}

pub(crate) fn fab_icon_button(
    id: impl Into<ElementId>,
    icon: AppIcon,
    label: impl Into<SharedString>,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    fab_base(id, label.into(), cx, on_click)
        .size(space(ShellMetrics::FAB_SIZE))
        .rounded(miaominal_settings::scaled_font_size(
            ShellMetrics::FAB_RADIUS,
        ))
        .shadow_md()
        .p_0()
        .child(Icon::new(icon).size(space(ShellMetrics::FAB_ICON_SIZE)))
}

fn fab_base(
    id: impl Into<ElementId>,
    label: SharedString,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    let colors = ShellColors::current();
    Button::new(id)
        .large()
        .line_height(space(ShellTypography::BODY_LINE_HEIGHT))
        .font_weight(FontWeight::MEDIUM)
        .occlude()
        .custom(
            ButtonCustomVariant::new(cx)
                .color(colors.primary_container)
                .foreground(colors.on_primary_container)
                .hover(
                    colors
                        .primary_container
                        .blend(colors.on_primary_container.opacity(0.08)),
                )
                .active(
                    colors
                        .primary_container
                        .blend(colors.on_primary_container.opacity(0.12)),
                ),
        )
        .accessibility_label(label)
        .bg(colors.primary_container)
        .text_color(colors.on_primary_container)
        .on_click(move |_, window, cx| {
            on_click(window, cx);
            cx.stop_propagation();
        })
}
