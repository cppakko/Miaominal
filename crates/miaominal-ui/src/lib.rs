pub mod ui;

pub use ui::AppView;
pub use ui::assets::AppAssets;
pub use ui::bridge_security_platform;
pub use ui::i18n;
pub use ui::init_markdown;
pub use ui::{
    configure_main_window_close, initialize_system_tray, request_main_window_close,
    sync_system_tray,
};
