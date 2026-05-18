#[path = "settings/global.rs"]
mod global;
#[path = "settings/model.rs"]
mod model;

pub use crate::infra::settings_store::SettingsStore;
pub(crate) use global::install;
#[allow(unused_imports)]
pub use global::{
    cell_width_default, current_settings, current_theme, font_fallbacks, font_family, font_size,
    line_height_default, scaled_font_size, scaled_line_height, sync_component_theme,
};
pub(crate) use model::changed;
#[allow(unused_imports)]
pub use model::{
    AppLanguage, AppSettings, CURRENT_ONBOARDING_VERSION, FONT_SIZE_MAX, FONT_SIZE_MIN, KeyBinding,
    LINE_HEIGHT_MAX, LINE_HEIGHT_MIN, LastTabCloseBehavior, LocalVaultAutoLockDuration,
    MonitorHistoryDuration, PLATFORM_DEFAULT_FONT, RECENT_CONNECTIONS_COUNT_MAX,
    RECENT_CONNECTIONS_COUNT_MIN, STEP, SyncedSettings, TerminalKeyBindings, TerminalPalette,
    TerminalRightClickBehavior, Theme, ThemeId, available_font_families, default_font_fallbacks,
    default_font_family,
};
