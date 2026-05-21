use gpui::{AnyElement, Div, Entity, ParentElement, Styled, div, px, rgb};
use gpui_component::{
    Icon, IconName, Sizable as _,
    input::{Input, InputState},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchInputStyle {
    Pill,
    Compact,
}

impl SearchInputStyle {
    fn background(self) -> u32 {
        let roles = miaominal_settings::current_theme().material.roles;

        match self {
            Self::Pill => roles.surface_container_highest,
            Self::Compact => roles.surface,
        }
    }

    fn height(self) -> f32 {
        match self {
            Self::Pill => 50.0,
            Self::Compact => 38.0,
        }
    }

    fn radius(self) -> f32 {
        match self {
            Self::Pill => 999.0,
            Self::Compact => 16.0,
        }
    }

    fn build_input(self, input: &Entity<InputState>, icon_color: u32) -> Input {
        let mut prefix = div().flex().items_center().text_color(rgb(icon_color));
        if matches!(self, Self::Pill) {
            prefix = prefix.pl_3();
        }

        let field = match self {
            Self::Pill => Input::new(input).large(),
            Self::Compact => Input::new(input).small(),
        };

        field
            .w_full()
            .appearance(false)
            .prefix(prefix.child(Icon::new(IconName::Search).small()))
    }
}

pub(crate) fn search_filter_input(
    input: &Entity<InputState>,
    style: SearchInputStyle,
    suffix: Option<AnyElement>,
) -> Div {
    let material = miaominal_settings::current_theme().material;
    let text_muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );

    let field = match suffix {
        Some(suffix) => style
            .build_input(input, text_muted)
            .suffix(div().pr_2().flex().items_center().child(suffix)),
        None => style.build_input(input, text_muted),
    };

    let mut container = div()
        .flex()
        .justify_center()
        .items_center()
        .w_full()
        .h(px(style.height()))
        .rounded(px(style.radius()))
        .bg(rgb(style.background()))
        .overflow_hidden();

    if matches!(style, SearchInputStyle::Compact) {
        container = container.flex().items_center().justify_center();
    }

    let container = container.child(field);

    if matches!(style, SearchInputStyle::Pill) {
        return div().w_full().pt_2().child(container);
    }

    container
}
