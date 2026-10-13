use crate::ui::assets::AppIcon;
use crate::ui::components::empty_state::empty_state;
use gpui_kit::prelude::*;
use gpui_kit::{SharedString, div};

pub(in crate::ui::features::forwarding) fn forwarding_section(
    content: impl IntoElement,
) -> impl IntoElement {
    div().w_full().p_5().child(content)
}

pub(in crate::ui::features::forwarding) fn forwarding_empty_state(
    copy: impl Into<SharedString>,
) -> impl IntoElement {
    empty_state(AppIcon::Forward, copy)
}
