pub mod assets;
pub(crate) mod components;
pub mod i18n;
mod shell;
pub(crate) mod theme;
pub(crate) mod utils;

use gpui::App;
use settings::Settings as _;
pub use shell::AppView;

pub fn init_zed_markdown(cx: &mut App) {
    if !cx.has_global::<settings::SettingsStore>() {
        settings::init(cx);
    }

    theme_settings::ThemeSettings::register(cx);

    if !cx.has_global::<::theme::GlobalTheme>() {
        theme_settings::init(::theme::LoadThemes::JustBase, cx);
    }
}
