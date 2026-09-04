use gpui_kit::{Div, ParentElement, SharedString, Styled, div, px, rgb};

pub(crate) fn badge(label: impl Into<SharedString>, background: u32, foreground: u32) -> Div {
    badge_base(label, background, foreground)
        .px_2()
        .py_1()
        .text_size(miaominal_settings::FontSize::Body.scaled())
}

pub(crate) fn compact_badge(
    label: impl Into<SharedString>,
    background: u32,
    foreground: u32,
) -> Div {
    badge_base(label, background, foreground)
        .min_w(px(18.0))
        .h(px(18.0))
        .px_1()
        .text_xs()
        .line_height(px(12.0))
}

fn badge_base(label: impl Into<SharedString>, background: u32, foreground: u32) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(999.0))
        .bg(rgb(background))
        .text_color(rgb(foreground))
        .child(label.into())
}
