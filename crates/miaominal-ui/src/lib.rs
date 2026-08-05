pub mod ui;

pub use ui::assets::AppAssets;
pub use ui::bridge_security_platform;
pub use ui::i18n;
pub use ui::init_markdown;
pub use ui::{AppView, AppWindowRole};
pub use ui::{
    DetachedWindowTarget, configure_detached_window_close, configure_main_window_close,
    initialize_application_state, initialize_system_tray, register_detached_window_opener,
    reload_application_state, request_main_window_close, restore_main_window, sync_system_tray,
};
