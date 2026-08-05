mod application;
pub mod assets;
pub mod bridge_security_platform;
pub(crate) mod components;
pub mod i18n;
mod shell;
mod system_tray;
pub(crate) mod theme;
pub(crate) mod utils;

pub use application::{initialize_application_state, reload_application_state};
use settings::Settings as _;
pub use shell::AppView;
pub use system_tray::{
    configure_main_window_close, initialize_system_tray, request_main_window_close,
    restore_main_window, sync_system_tray,
};

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
