use gpui::{px, rgb};
use gpui_component::{IconName, Sizable, spinner::Spinner};

pub(crate) fn md3_spinner(size: f32) -> Spinner {
    let material = miaominal_settings::current_theme().material;
    Spinner::new()
        .with_size(px(size))
        .icon(IconName::LoaderCircle)
        .color(rgb(material.roles.primary).into())
}
