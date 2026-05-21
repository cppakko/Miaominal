use gpui::{IntoElement, ParentElement, SharedString, Styled, div, px, rgb};

pub(crate) fn badge(
    label: impl Into<SharedString>,
    background: u32,
    foreground: u32,
) -> impl IntoElement {
    div()
        .px_2()
        .py_1()
        .rounded(px(999.0))
        .bg(rgb(background))
        .text_size(miaominal_settings::scaled_font_size(10.0))
        .text_color(rgb(foreground))
        .child(label.into())
}
