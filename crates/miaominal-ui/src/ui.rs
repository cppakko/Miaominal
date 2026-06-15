pub mod assets;
pub(crate) mod components;
pub mod i18n;
pub(crate) mod markdown_view;
mod shell;
pub(crate) mod theme;
pub(crate) mod utils;

use settings::Settings as _;
pub use shell::AppView;

pub fn init_markdown(cx: &mut gpui::App) {
    if !cx.has_global::<settings::SettingsStore>() {
        settings::init(cx);
    }

    theme_settings::ThemeSettings::register(cx);

    if !cx.has_global::<::theme::GlobalTheme>() {
        theme_settings::init(::theme::LoadThemes::JustBase, cx);
    }
}
