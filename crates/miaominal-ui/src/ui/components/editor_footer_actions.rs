use gpui::{AnyElement, Div, ParentElement, Styled, div};

pub(crate) const EDITOR_FOOTER_ACTION_HEIGHT: f32 = 32.0;

pub(crate) fn editor_footer_actions(actions: impl IntoIterator<Item = AnyElement>) -> Div {
    div()
        .w_full()
        .flex()
        .items_center()
        .justify_end()
        .gap_2()
        .flex_wrap()
        .children(actions)
}
