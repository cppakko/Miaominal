use gpui::{px, rgb};
use gpui_component::{IconName, Sizable, spinner::Spinner};

use crate::settings;

pub(crate) fn md3_spinner(size: f32) -> Spinner {
    let material = settings::current_theme().material;
    Spinner::new()
        .with_size(px(size))
        .icon(IconName::LoaderCircle)
        .color(rgb(material.roles.primary).into())
}
