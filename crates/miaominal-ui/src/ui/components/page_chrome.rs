use super::icon_tile::{IconTileTone, icon_tile};
use super::view_mode_motion::view_mode_progress;
use crate::ui::assets::AppIcon;
use crate::ui::components::shell_tokens::{ShellColors, ShellMetrics, space};
use crate::ui::contracts::navigation::ProfileViewMode;
use crate::ui::i18n;
use gpui_kit::component::{
    Icon, Sizable as _,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    h_flex,
};
use gpui_kit::{
    App, Div, InteractiveElement as _, IntoElement, ParentElement, RenderOnce, Role, SharedString,
    StatefulInteractiveElement as _, Styled, TestSupportExt as _, Window, div, rgb,
};
use std::rc::Rc;

pub(crate) fn page_section_title(title: impl Into<SharedString>) -> impl IntoElement {
    let roles = miaominal_settings::current_theme().material.roles;

    div()
        .text_size(miaominal_settings::FontSize::SectionTitle.scaled())
        .text_color(rgb(roles.on_surface))
        .child(title.into())
}

pub(in crate::ui) fn view_mode_switcher(
    id: &'static str,
    selected_mode: ProfileViewMode,
    on_change: impl Fn(ProfileViewMode, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    ViewModeSwitcher {
        id,
        selected_mode,
        on_change: Rc::new(on_change),
    }
}

type ViewModeChange = Rc<dyn Fn(ProfileViewMode, &mut Window, &mut App)>;

#[derive(IntoElement)]
struct ViewModeSwitcher {
    id: &'static str,
    selected_mode: ProfileViewMode,
    on_change: ViewModeChange,
}

impl RenderOnce for ViewModeSwitcher {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            selected_mode,
            on_change,
        } = self;
        let progress = view_mode_progress(id, selected_mode, window, cx);
        let colors = ShellColors::current();
        h_flex()
            .id(id)
            .role(Role::Group)
            .aria_label(i18n::string("view_mode.label"))
            .test_support()
            .relative()
            .flex_shrink_0()
            .gap(space(4.0))
            .p(space(4.0))
            .rounded_full()
            .bg(colors.card)
            .child(
                div()
                    .id(SharedString::from(format!("{id}-indicator")))
                    .test_support()
                    .absolute()
                    .top(space(4.0))
                    .left(space(
                        4.0 + progress * (ShellMetrics::VIEW_MODE_ITEM_WIDTH + 4.0),
                    ))
                    .w(space(ShellMetrics::VIEW_MODE_ITEM_WIDTH))
                    .h(space(ShellMetrics::VIEW_MODE_ITEM_HEIGHT))
                    .rounded_full()
                    .bg(colors.selected),
            )
            .children(
                [
                    (ProfileViewMode::Grid, AppIcon::Grid, "grid"),
                    (ProfileViewMode::List, AppIcon::List, "list"),
                ]
                .into_iter()
                .map(|(mode, icon, suffix)| {
                    let selected = selected_mode == mode;
                    let emphasis = if mode == ProfileViewMode::List {
                        progress
                    } else {
                        1.0 - progress
                    };
                    let foreground = colors.muted.blend(colors.on_selected.opacity(emphasis));
                    let label = i18n::string(if mode == ProfileViewMode::Grid {
                        "view_mode.grid"
                    } else {
                        "view_mode.list"
                    });
                    let on_change = on_change.clone();
                    Button::new(SharedString::from(format!("{id}-{suffix}")))
                        .ghost()
                        .toggled(selected)
                        .accessibility_label(label.clone())
                        .tooltip(label)
                        .w(space(ShellMetrics::VIEW_MODE_ITEM_WIDTH))
                        .h(space(ShellMetrics::VIEW_MODE_ITEM_HEIGHT))
                        .rounded_full()
                        .custom(
                            ButtonCustomVariant::new(cx)
                                .color(gpui_kit::transparent_black())
                                .foreground(foreground)
                                .hover(foreground.opacity(0.08))
                                .active(foreground.opacity(0.12)),
                        )
                        .child(Icon::new(icon).size(space(ShellMetrics::VIEW_MODE_ICON_SIZE)))
                        .on_click(move |_, window, cx| {
                            if mode != selected_mode {
                                on_change(mode, window, cx);
                            }
                        })
                }),
            )
    }
}

pub(crate) fn page_primary_icon_tile(icon: AppIcon, size: f32, corner_radius: f32) -> Div {
    icon_tile(
        Icon::new(icon).small(),
        size,
        corner_radius,
        IconTileTone::Primary,
    )
}
