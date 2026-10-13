use crate::ui::shell::pages::empty_state::shell_empty_state;
use crate::ui::shell::*;

pub(in crate::ui::shell::forwarding) fn forwarding_section(
    content: impl IntoElement,
) -> impl IntoElement {
    div().w_full().p_5().child(content)
}

pub(in crate::ui::shell::forwarding) fn forwarding_empty_state(
    copy: impl Into<SharedString>,
) -> impl IntoElement {
    shell_empty_state(AppIcon::Forward, copy)
}
