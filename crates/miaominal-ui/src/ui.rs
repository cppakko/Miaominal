pub mod assets;
pub(crate) mod components;
pub mod i18n;
mod shell;
pub(crate) mod theme;
pub(crate) mod utils;

use settings::Settings as _;
pub use shell::AppView;

pub fn init_markdown(_cx: &mut gpui::App) {
    if !_cx.has_global::<settings::SettingsStore>() {
        settings::init(_cx);
    }

    theme_settings::ThemeSettings::register(_cx);

    if !_cx.has_global::<::theme::GlobalTheme>() {
        theme_settings::init(::theme::LoadThemes::JustBase, _cx);
    }

    // Initialize language registry for tree-sitter based code block syntax highlighting
    // Languages are enabled via Cargo features: tree-sitter-rust, tree-sitter-python, etc.
    use gpui_component::highlighter::LanguageRegistry;
    let _ = LanguageRegistry::singleton();
}
