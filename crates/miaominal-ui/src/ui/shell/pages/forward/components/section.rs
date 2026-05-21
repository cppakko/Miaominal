use super::super::super::super::*;
use super::super::super::empty_state::shell_empty_state;

pub(in crate::ui::shell::pages::forward) fn forwarding_section(
    content: impl IntoElement,
) -> impl IntoElement {
    div().w_full().p_5().child(content)
}

pub(in crate::ui::shell::pages::forward) fn forwarding_empty_state(
    copy: impl Into<SharedString>,
) -> impl IntoElement {
    shell_empty_state(AppIcon::Forward, copy)
}
