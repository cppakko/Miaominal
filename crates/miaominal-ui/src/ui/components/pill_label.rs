use gpui::{Div, ParentElement, SharedString, Styled, div, px, rgb};

pub(crate) fn pill_label(label: impl Into<SharedString>, background: u32, foreground: u32) -> Div {
    div()
        .flex_shrink_0()
        .px_3()
        .py_2()
        .rounded(px(999.0))
        .bg(rgb(background))
        .text_color(rgb(foreground))
        .text_size(miaominal_settings::FontSize::Body.scaled())
        .child(label.into())
}
