use gpui_kit::component::{IconName, Sizable, spinner::Spinner};
use gpui_kit::{px, rgb};

pub(crate) fn md3_spinner(size: f32) -> Spinner {
    let material = miaominal_settings::current_theme().material;
    Spinner::new()
        .with_size(px(size))
        .icon(IconName::LoaderCircle)
        .color(rgb(material.roles.primary).into())
}
