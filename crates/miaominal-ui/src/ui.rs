mod application;
pub mod assets;
pub mod bridge_security_platform;
pub(crate) mod components;
pub mod i18n;
mod shell;
mod system_tray;
pub(crate) mod theme;
pub(crate) mod utils;
mod windowing;

pub use application::{initialize_application_state, reload_application_state};
pub use shell::{AppView, AppWindowRole, open_settings_from_menu};
pub use system_tray::{
    configure_detached_window_close, configure_main_window_close, initialize_system_tray,
    quit_application, request_main_window_close, restore_main_window, sync_system_tray,
};
pub use windowing::{DetachedWindowTarget, register_detached_window_opener};

pub fn init_markdown(_cx: &mut gpui_kit::App) {
    // Initialize language registry for tree-sitter based code block syntax highlighting
    // Languages are enabled via Cargo features: tree-sitter-rust, tree-sitter-python, etc.
    use gpui_kit::component::highlighter::LanguageRegistry;
    let _ = LanguageRegistry::singleton();
}
