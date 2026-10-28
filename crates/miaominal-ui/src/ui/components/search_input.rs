use super::HintedInput;
use gpui_kit::component::{Icon, IconName, Sizable as _, input::InputState};
use gpui_kit::{AnyElement, Div, Entity, ParentElement, Styled, div, px, rgb};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchInputStyle {
    Compact,
}

impl SearchInputStyle {
    fn background(self) -> u32 {
        let roles = miaominal_settings::current_theme().material.roles;

        match self {
            Self::Compact => roles.surface,
        }
    }

    fn height(self) -> f32 {
        match self {
            Self::Compact => 38.0,
        }
    }

    fn radius(self) -> f32 {
        match self {
            Self::Compact => 16.0,
        }
    }

    fn build_input(self, input: &Entity<InputState>, icon_color: u32) -> HintedInput {
        let prefix = div().flex().items_center().text_color(rgb(icon_color));

        let field = match self {
            Self::Compact => HintedInput::new(input).small().hint_left(px(34.0)),
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

    div()
        .flex()
        .justify_center()
        .items_center()
        .w_full()
        .h(px(style.height()))
        .rounded(px(style.radius()))
        .bg(rgb(style.background()))
        .overflow_hidden()
        .child(field)
}
