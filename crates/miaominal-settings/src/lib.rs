pub mod theme;

mod data;
#[cfg(feature = "desktop-ui")]
mod desktop;
mod global;
mod model;
mod rdp;
pub use rdp::{RdpClientKind, RdpClientSettings};

#[cfg(feature = "desktop-ui")]
pub use desktop::{
    FontSize, interface_font, scaled_font_size, scaled_line_height, sync_component_theme,
};
pub use global::{
    cell_width_default, current_settings, current_theme, font_fallbacks, font_family, install,
    interface_font_size, line_height_default, terminal_font_family, terminal_font_size,
};
#[cfg(feature = "desktop-ui")]
pub use model::available_font_families;
pub use model::{
    AI_PROVIDER_POSITIVE_INTEGER_MIN, AI_PROVIDER_TEMPERATURE_MAX, AI_PROVIDER_TEMPERATURE_MIN,
    AiAgentMode, AiProviderConfig, AiProviderKind, AiReasoningEffort, AppLanguage, AppSettings,
    CURRENT_ONBOARDING_VERSION, FONT_SIZE_MAX, FONT_SIZE_MIN, KeyBinding, LINE_HEIGHT_MAX,
    LINE_HEIGHT_MIN, LastTabCloseBehavior, LocalVaultAutoLockDuration, MonitorHistoryDuration,
    OpenSshIntegrationMode, PLATFORM_DEFAULT_FONT, RECENT_CONNECTIONS_COUNT_MAX,
    RECENT_CONNECTIONS_COUNT_MIN, SSH_CONNECT_TIMEOUT_SECS_DEFAULT, SSH_CONNECT_TIMEOUT_SECS_MAX,
    SSH_CONNECT_TIMEOUT_SECS_MIN, SSH_KEEPALIVE_INTERVAL_SECS_DEFAULT,
    SSH_KEEPALIVE_INTERVAL_SECS_MAX, SSH_KEEPALIVE_MAX_DEFAULT, SSH_KEEPALIVE_MAX_MAX,
    SSH_KEEPALIVE_MAX_MIN, STEP, SshBridgeConfig, SyncedSettings, TerminalColorScheme,
    TerminalCursorStyle, TerminalKeyBindings, TerminalPalette, TerminalRightClickBehavior, Theme,
    ThemeId, WEB_SEARCH_MAX_RESULTS_MAX, WEB_SEARCH_MAX_RESULTS_MIN, WebSearchConfig,
    WebSearchProviderKind, WindowCloseBehavior, ai_provider_kind_label, changed,
    default_font_fallbacks, default_font_family,
};
