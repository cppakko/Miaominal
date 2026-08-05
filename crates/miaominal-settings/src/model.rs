use crate::theme::{self as material_theme, MaterialTheme};

pub(crate) use crate::data::DEFAULT_CELL_WIDTH;
#[cfg(feature = "desktop-ui")]
pub(crate) use crate::data::DEFAULT_FONT_SIZE;
#[cfg(feature = "desktop-ui")]
pub use crate::data::available_font_families;
pub use crate::data::{
    AI_PROVIDER_POSITIVE_INTEGER_MIN, AI_PROVIDER_TEMPERATURE_MAX, AI_PROVIDER_TEMPERATURE_MIN,
    AiAgentMode, AiProviderConfig, AiProviderKind, AiReasoningEffort, AppLanguage, AppSettings,
    CURRENT_ONBOARDING_VERSION, FONT_SIZE_MAX, FONT_SIZE_MIN, KeyBinding, LINE_HEIGHT_MAX,
    LINE_HEIGHT_MIN, LastTabCloseBehavior, LocalVaultAutoLockDuration, MonitorHistoryDuration,
    OpenSshIntegrationMode, PLATFORM_DEFAULT_FONT, RECENT_CONNECTIONS_COUNT_MAX,
    RECENT_CONNECTIONS_COUNT_MIN, STEP, SshBridgeConfig, SyncedSettings, TerminalKeyBindings,
    TerminalRightClickBehavior, ThemeId, WEB_SEARCH_MAX_RESULTS_MAX, WEB_SEARCH_MAX_RESULTS_MIN,
    WebSearchConfig, WebSearchProviderKind, WindowCloseBehavior, ai_provider_kind_label,
    default_font_fallbacks, default_font_family,
};

impl AppLanguage {
    pub fn detect_system() -> Self {
        sys_locale::get_locale()
            .as_deref()
            .map(Self::from_locale_code)
            .unwrap_or(Self::English)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TerminalPalette {
    pub default_fg: u32,
    pub default_bg: u32,
    pub cursor: u32,
    pub selection: u32,
    pub ansi: [u32; 16],
}

impl TerminalPalette {
    pub fn from_material(theme: &MaterialTheme) -> Self {
        Self {
            default_fg: theme.roles.on_surface,
            default_bg: theme.roles.surface,
            cursor: theme.roles.primary,
            selection: theme.roles.primary_container,
            ansi: material_theme::terminal_ansi(theme),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub material: MaterialTheme,
    pub terminal: TerminalPalette,
}

impl Theme {
    pub fn from_settings(settings: &AppSettings) -> Self {
        let material =
            material_theme::build_theme(settings.seed_argb(), settings.theme_id.is_dark());

        Self {
            terminal: TerminalPalette::from_material(&material),
            material,
        }
    }

    pub fn from_id(id: ThemeId) -> Self {
        let settings = AppSettings {
            theme_id: id,
            ..AppSettings::default()
        };
        Self::from_settings(&settings)
    }
}

impl AppSettings {
    pub fn default_for_system() -> Self {
        Self {
            language: AppLanguage::detect_system(),
            ..Self::default()
        }
    }

    pub fn seed_argb(&self) -> material_colors::color::Argb {
        material_theme::parse_seed_color_or_default(&self.seed_color)
    }
}

pub fn changed(a: &AppSettings, b: &AppSettings) -> bool {
    a.language != b.language
        || a.font_family != b.font_family
        || a.terminal_font_family != b.terminal_font_family
        || a.font_fallbacks != b.font_fallbacks
        || (a.font_size - b.font_size).abs() > f32::EPSILON
        || (a.line_height - b.line_height).abs() > f32::EPSILON
        || a.theme_id != b.theme_id
        || a.seed_color != b.seed_color
        || a.recent_connections_count != b.recent_connections_count
        || a.key_bindings != b.key_bindings
        || a.terminal_right_click_behavior != b.terminal_right_click_behavior
        || a.terminal_shift_right_click_context_menu != b.terminal_shift_right_click_context_menu
        || a.terminal_free_type_mode != b.terminal_free_type_mode
        || a.auto_collect_session_monitoring != b.auto_collect_session_monitoring
        || a.last_tab_close_behavior != b.last_tab_close_behavior
        || a.window_close_behavior != b.window_close_behavior
        || a.monitor_history_duration != b.monitor_history_duration
        || a.local_sftp_hidden_columns != b.local_sftp_hidden_columns
        || a.remote_sftp_hidden_columns != b.remote_sftp_hidden_columns
        || a.completed_onboarding_version != b.completed_onboarding_version
        || a.local_vault_enabled != b.local_vault_enabled
        || a.local_vault_auto_lock_duration != b.local_vault_auto_lock_duration
        || a.ai_providers != b.ai_providers
        || a.selected_ai_provider_id != b.selected_ai_provider_id
        || a.agent_mode != b.agent_mode
        || a.web_search != b.web_search
        || a.open_ssh_integration_mode != b.open_ssh_integration_mode
        || a.ssh_bridge != b.ssh_bridge
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_clamps_numeric_values_and_normalizes_seed() {
        let mut settings = AppSettings {
            language: AppLanguage::English,
            font_family: " ".into(),
            terminal_font_family: "monospace".into(),
            font_size: 3.0,
            line_height: 100.0,
            theme_id: ThemeId::Light,
            seed_color: "#6750A4".into(),
            ..AppSettings::default()
        };

        settings.sanitize();

        assert_eq!(settings.font_family, default_font_family());
        assert_eq!(settings.terminal_font_family, default_font_family());
        assert_eq!(settings.font_size, FONT_SIZE_MIN);
        assert_eq!(settings.line_height, LINE_HEIGHT_MAX);
        assert_eq!(settings.seed_color, "#6750a4");
    }

    #[test]
    fn sanitize_rewrites_legacy_zed_mono_default() {
        let mut settings = AppSettings {
            font_family: ".ZedMono".into(),
            ..AppSettings::default()
        };

        settings.sanitize();

        assert_eq!(settings.font_family, default_font_family());
    }

    #[test]
    fn sanitize_rewrites_generic_monospace_alias() {
        let mut settings = AppSettings {
            font_family: "monospace".into(),
            ..AppSettings::default()
        };

        settings.sanitize();

        assert_eq!(settings.font_family, default_font_family());
    }

    #[test]
    fn locale_detection_prefers_simplified_chinese_for_zh_codes() {
        assert_eq!(
            AppLanguage::from_locale_code("zh-CN"),
            AppLanguage::SimplifiedChinese
        );
        assert_eq!(
            AppLanguage::from_locale_code("zh_TW"),
            AppLanguage::SimplifiedChinese
        );
        assert_eq!(AppLanguage::from_locale_code("en-US"), AppLanguage::English);
    }

    #[test]
    fn deserialize_missing_last_tab_close_behavior_uses_default() {
        let settings: AppSettings = toml::from_str("").expect("settings should deserialize");

        assert_eq!(
            settings.last_tab_close_behavior,
            LastTabCloseBehavior::ExitApplication
        );
        assert_eq!(
            settings.window_close_behavior,
            WindowCloseBehavior::ExitApplication
        );
        assert_eq!(settings.local_sftp_hidden_columns, vec![4, 5]);
        assert_eq!(settings.remote_sftp_hidden_columns, vec![4, 5]);
        assert!(!settings.terminal_free_type_mode);
    }

    #[test]
    fn deserialize_missing_local_vault_auto_lock_duration_uses_default() {
        let settings: AppSettings = toml::from_str("").expect("settings should deserialize");

        assert_eq!(
            settings.local_vault_auto_lock_duration,
            LocalVaultAutoLockDuration::Off
        );
    }

    #[test]
    fn deserialize_missing_agent_mode_uses_execute() {
        let settings: AppSettings = toml::from_str("").expect("settings should deserialize");

        assert_eq!(settings.agent_mode, AiAgentMode::Execute);
    }

    #[test]
    fn changed_detects_agent_mode() {
        let original = AppSettings::default();
        let modified = AppSettings {
            agent_mode: AiAgentMode::FullAuto,
            ..AppSettings::default()
        };

        assert!(changed(&original, &modified));
    }

    #[test]
    fn window_close_behavior_round_trips_and_is_local_only() {
        let settings = AppSettings {
            window_close_behavior: WindowCloseBehavior::MinimizeToTray,
            ..AppSettings::default()
        };

        let serialized = toml::to_string(&settings).expect("serialize settings");
        assert!(serialized.contains("window_close_behavior = \"minimize_to_tray\""));

        let decoded: AppSettings = toml::from_str(&serialized).expect("deserialize settings");
        assert_eq!(
            decoded.window_close_behavior,
            WindowCloseBehavior::MinimizeToTray
        );

        let synced = toml::to_string(&settings.synced_settings()).expect("serialize sync settings");
        assert!(!synced.contains("window_close_behavior"));
    }

    #[test]
    fn changed_detects_window_close_behavior() {
        let original = AppSettings::default();
        let modified = AppSettings {
            window_close_behavior: WindowCloseBehavior::MinimizeToTray,
            ..AppSettings::default()
        };

        assert!(changed(&original, &modified));
    }

    #[test]
    fn changed_detects_local_vault_auto_lock_duration() {
        let original = AppSettings::default();
        let modified = AppSettings {
            local_vault_auto_lock_duration: LocalVaultAutoLockDuration::FiveMinutes,
            ..AppSettings::default()
        };

        assert!(changed(&original, &modified));
    }

    #[test]
    fn changed_detects_terminal_font_family_and_fallbacks() {
        let original = AppSettings::default();
        let mut modified = original.clone();
        modified.terminal_font_family = "JetBrains Mono".into();

        assert!(changed(&original, &modified));

        modified = original.clone();
        modified.font_fallbacks = vec!["Noto Sans CJK SC".into()];

        assert!(changed(&original, &modified));
    }

    #[test]
    fn changed_detects_terminal_free_type_mode() {
        let original = AppSettings::default();
        let modified = AppSettings {
            terminal_free_type_mode: true,
            ..AppSettings::default()
        };

        assert!(changed(&original, &modified));
    }

    #[test]
    fn bridge_settings_are_local_only_and_detect_changes() {
        let original = AppSettings::default();
        let mut modified = original.clone();
        modified.open_ssh_integration_mode = OpenSshIntegrationMode::Bridge;
        modified.ssh_bridge.max_connections = 32;

        assert!(changed(&original, &modified));

        let serialized = toml::to_string(&modified.synced_settings()).expect("sync settings");
        assert!(!serialized.contains("open_ssh_integration_mode"));
        assert!(!serialized.contains("ssh_bridge"));
        assert!(!serialized.contains("security_policy"));
        assert!(!serialized.contains("managed_open_ssh_integration_enabled"));
        assert!(!serialized.contains("bridge_security_policy"));
        assert!(!serialized.contains("bridge_audit_records"));
    }

    #[test]
    fn bridge_settings_default_disabled_and_sanitize_limits() {
        let mut settings: AppSettings = toml::from_str(
            r#"
[ssh_bridge]
max_connections = 0
max_channels_per_connection = 1000
"#,
        )
        .expect("settings");
        settings.sanitize();

        assert_eq!(
            settings.open_ssh_integration_mode,
            OpenSshIntegrationMode::Disabled
        );
        assert_eq!(settings.ssh_bridge.max_connections, 1);
        assert_eq!(settings.ssh_bridge.max_channels_per_connection, 256);
    }

    #[test]
    fn sanitize_normalizes_sftp_browser_hidden_columns() {
        let mut settings = AppSettings {
            local_sftp_hidden_columns: vec![5, 4, 99, 5],
            remote_sftp_hidden_columns: vec![0, 1, 2, 3, 4, 5],
            ..AppSettings::default()
        };

        settings.sanitize();

        assert_eq!(settings.local_sftp_hidden_columns, vec![4, 5]);
        assert_eq!(settings.remote_sftp_hidden_columns, vec![4, 5]);
    }

    #[test]
    fn onboarding_defaults_to_incomplete() {
        let settings = AppSettings::default_for_system();

        assert!(settings.should_show_onboarding());
        assert_eq!(settings.completed_onboarding_version, 0);
    }

    #[test]
    fn onboarding_completion_uses_current_version() {
        let mut settings = AppSettings::default_for_system();

        settings.mark_current_onboarding_completed();

        assert!(!settings.should_show_onboarding());
        assert_eq!(
            settings.completed_onboarding_version,
            CURRENT_ONBOARDING_VERSION
        );
    }
}
