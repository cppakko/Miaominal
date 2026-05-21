use gpui::{AnyElement, Div, ParentElement, Styled, div};

pub(crate) fn editor_footer_actions(actions: impl IntoIterator<Item = AnyElement>) -> Div {
    div()
        .w_full()
        .flex()
        .justify_end()
        .gap_2()
        .flex_wrap()
        .children(actions)
}
