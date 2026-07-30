pub mod theme;

mod data;
#[cfg(feature = "desktop-ui")]
mod desktop;
mod global;
mod model;

#[cfg(feature = "desktop-ui")]
pub use desktop::{
    FontSize, interface_font, scaled_font_size, scaled_line_height, sync_component_theme,
};
pub use global::{
    cell_width_default, current_settings, current_theme, font_fallbacks, font_family, font_size,
    install, line_height_default, terminal_font_family,
};
#[cfg(feature = "desktop-ui")]
pub use model::available_font_families;
pub use model::{
    AI_PROVIDER_POSITIVE_INTEGER_MIN, AI_PROVIDER_TEMPERATURE_MAX, AI_PROVIDER_TEMPERATURE_MIN,
    AiAgentMode, AiProviderConfig, AiProviderKind, AiReasoningEffort, AppLanguage, AppSettings,
    CURRENT_ONBOARDING_VERSION, FONT_SIZE_MAX, FONT_SIZE_MIN, KeyBinding, LINE_HEIGHT_MAX,
    LINE_HEIGHT_MIN, LastTabCloseBehavior, LocalVaultAutoLockDuration, MonitorHistoryDuration,
    PLATFORM_DEFAULT_FONT, RECENT_CONNECTIONS_COUNT_MAX, RECENT_CONNECTIONS_COUNT_MIN, STEP,
    SyncedSettings, TerminalKeyBindings, TerminalPalette, TerminalRightClickBehavior, Theme,
    ThemeId, WEB_SEARCH_MAX_RESULTS_MAX, WEB_SEARCH_MAX_RESULTS_MIN, WebSearchConfig,
    WebSearchProviderKind, ai_provider_kind_label, changed, default_font_fallbacks,
    default_font_family,
};
