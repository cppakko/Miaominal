//! Shared building blocks for catalog cards (hosts, keychain, snippets,
//! port forwarding, trust center) so every page uses the same geometry,
//! typography and chip style.
use super::icon_tile::{IconTileTone, icon_tile};
use super::shell_tokens::{ShellColors, ShellMetrics, ShellTypography, space};
use crate::ui::assets::AppIcon;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, FontWeight, Hsla, IntoElement, SharedString, div};

/// Card surface shared by every catalog page. All cards have the same fixed
/// height regardless of content (subtitle, chips); content is centered
/// vertically inside it.
pub(in crate::ui) fn catalog_card_surface() -> Div {
    div()
        .w_full()
        .min_w_0()
        .h(space(ShellMetrics::CARD_HEIGHT))
        .px(space(ShellMetrics::CARD_PADDING))
        .rounded(space(ShellMetrics::CARD_RADIUS))
        .bg(ShellColors::current().card)
        .flex()
        .flex_col()
        .justify_center()
}

/// Leading icon tile with the shared size, radius and tone.
pub(in crate::ui) fn catalog_card_icon(icon: AppIcon) -> Div {
    catalog_card_icon_content(Icon::new(icon).small())
}

pub(in crate::ui) fn catalog_card_icon_content(content: impl IntoElement) -> Div {
    icon_tile(
        content,
        ShellMetrics::CARD_ICON_SIZE,
        ShellMetrics::CARD_ICON_RADIUS,
        IconTileTone::Primary,
    )
    .flex_shrink_0()
}

pub(in crate::ui) fn catalog_card_title(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .flex_shrink(1.0)
        .truncate()
        .text_size(space(ShellTypography::TITLE_SIZE))
        .line_height(space(ShellTypography::TITLE_LINE_HEIGHT))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(ShellColors::current().foreground)
        .child(text.into())
}

pub(in crate::ui) fn catalog_card_subtitle(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .truncate()
        .text_size(space(ShellTypography::SUPPORTING_SIZE))
        .line_height(space(ShellTypography::SUPPORTING_LINE_HEIGHT))
        .text_color(ShellColors::current().muted)
        .child(text.into())
}

/// Single-line chip row. It never wraps: wrapped rows are measured at their
/// max-content width and would overflow the card, so chips shrink instead.
pub(in crate::ui) fn catalog_card_chips() -> Div {
    h_flex()
        .w_full()
        .min_w_0()
        .min_h(space(ShellMetrics::CARD_CHIP_HEIGHT))
        .overflow_hidden()
        .gap(space(4.0))
}

pub(in crate::ui) fn catalog_card_chip(
    label: impl Into<SharedString>,
    foreground: Option<Hsla>,
) -> Div {
    let colors = ShellColors::current();
    div()
        .min_w_0()
        .max_w(space(140.0))
        .flex_shrink(1.0)
        .truncate()
        .rounded_full()
        .bg(colors.chip)
        .px(space(8.0))
        .text_size(space(ShellTypography::SUPPORTING_SIZE))
        .line_height(space(ShellMetrics::CARD_CHIP_HEIGHT))
        .text_color(foreground.unwrap_or(colors.muted))
        .child(label.into())
}

/// Icon + text block (title, optional subtitle, optional chip row) + optional
/// trailing control. The icon and trailing control are vertically centered
/// against the whole text block, chips included.
pub(in crate::ui) fn catalog_card_content(
    icon: impl IntoElement,
    title: impl IntoElement,
    subtitle: Option<SharedString>,
    trailing: Option<AnyElement>,
    chips: Option<AnyElement>,
) -> Div {
    h_flex()
        .w_full()
        .min_w_0()
        .min_h(space(ShellMetrics::CARD_ICON_SIZE))
        .items_center()
        .gap(space(12.0))
        .child(icon)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .justify_center()
                .child(title)
                .when_some(subtitle, |column, subtitle| {
                    column.child(catalog_card_subtitle(subtitle))
                })
                .when_some(chips, |column, chips| {
                    column.child(div().w_full().min_w_0().pt(space(2.0)).child(chips))
                }),
        )
        .when_some(trailing, |row, trailing| row.child(trailing))
}
