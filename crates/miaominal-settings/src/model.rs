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
    RECENT_CONNECTIONS_COUNT_MIN, SSH_CONNECT_TIMEOUT_SECS_DEFAULT, SSH_CONNECT_TIMEOUT_SECS_MAX,
    SSH_CONNECT_TIMEOUT_SECS_MIN, SSH_KEEPALIVE_INTERVAL_SECS_DEFAULT,
    SSH_KEEPALIVE_INTERVAL_SECS_MAX, SSH_KEEPALIVE_MAX_DEFAULT, SSH_KEEPALIVE_MAX_MAX,
    SSH_KEEPALIVE_MAX_MIN, STEP, SshBridgeConfig, SyncedSettings, TerminalColorScheme,
    TerminalCursorStyle, TerminalKeyBindings, TerminalRightClickBehavior, ThemeId,
    WEB_SEARCH_MAX_RESULTS_MAX, WEB_SEARCH_MAX_RESULTS_MIN, WebSearchConfig, WebSearchProviderKind,
    WindowCloseBehavior, ai_provider_kind_label, default_font_fallbacks, default_font_family,
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

    pub fn from_scheme_and_material(scheme: TerminalColorScheme, theme: &MaterialTheme) -> Self {
        match scheme {
            TerminalColorScheme::ThemeDefault => Self::from_material(theme),
            TerminalColorScheme::Dracula => Self {
                default_fg: 0xf8f8f2,
                default_bg: 0x282a36,
                cursor: 0xf8f8f2,
                selection: 0x44475a,
                ansi: [
                    0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2,
                    0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
                ],
            },
            TerminalColorScheme::Nord => Self {
                default_fg: 0xd8dee9,
                default_bg: 0x2e3440,
                cursor: 0xd8dee9,
                selection: 0x434c5e,
                ansi: [
                    0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0,
                    0x4c566a, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
                ],
            },
            TerminalColorScheme::CatppuccinMocha => Self {
                default_fg: 0xcdd6f4,
                default_bg: 0x1e1e2e,
                cursor: 0xf5e0dc,
                selection: 0x585b70,
                ansi: [
                    0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
                    0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
                ],
            },
            TerminalColorScheme::CatppuccinLatte => Self {
                default_fg: 0x4c4f69,
                default_bg: 0xeff1f5,
                cursor: 0xdc8a78,
                selection: 0xacb0be,
                ansi: [
                    0x5c5f77, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xacb0be,
                    0x6c6f85, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0xbcc0cc,
                ],
            },
            TerminalColorScheme::SolarizedDark => Self {
                default_fg: 0x839496,
                default_bg: 0x002b36,
                cursor: 0x93a1a1,
                selection: 0x073642,
                ansi: [
                    0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
                    0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
                ],
            },
            TerminalColorScheme::SolarizedLight => Self {
                default_fg: 0x657b83,
                default_bg: 0xfdf6e3,
                cursor: 0x586e75,
                selection: 0xeee8d5,
                ansi: [
                    0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
                    0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
                ],
            },
            TerminalColorScheme::TokyoNight => Self {
                default_fg: 0xc0caf5,
                default_bg: 0x1a1b26,
                cursor: 0xc0caf5,
                selection: 0x33467c,
                ansi: [
                    0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
                    0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
                ],
            },
            TerminalColorScheme::OneDark => Self {
                default_fg: 0xabb2bf,
                default_bg: 0x282c34,
                cursor: 0x528bff,
                selection: 0x3e4451,
                ansi: [
                    0x282c34, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf,
                    0x5c6370, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xffffff,
                ],
            },
        }
    }
}

#[derive(Clone, Copy)]
struct SchemeSurfacePreset {
    dark: bool,
    seed: u32,
    surface: u32,
    surface_dim: u32,
    surface_container_lowest: u32,
    surface_container_low: u32,
    surface_container: u32,
    surface_container_high: u32,
    surface_container_highest: u32,
    surface_bright: u32,
    surface_variant: u32,
    on_surface: u32,
    on_surface_variant: u32,
    outline: u32,
    outline_variant: u32,
}

impl SchemeSurfacePreset {
    fn for_scheme(scheme: TerminalColorScheme) -> Option<Self> {
        match scheme {
            TerminalColorScheme::ThemeDefault => None,
            TerminalColorScheme::Dracula => Some(Self {
                dark: true,
                seed: 0xbd93f9,
                surface_container_lowest: 0x21222c,
                surface: 0x282a36,
                surface_dim: 0x282a36,
                surface_container_low: 0x2e303e,
                surface_container: 0x343746,
                surface_container_high: 0x3b3f52,
                surface_container_highest: 0x44475a,
                surface_bright: 0x4e5268,
                surface_variant: 0x383a59,
                on_surface: 0xf8f8f2,
                on_surface_variant: 0xb6b9d1,
                outline: 0x6272a4,
                outline_variant: 0x44475a,
            }),
            TerminalColorScheme::Nord => Some(Self {
                dark: true,
                seed: 0x88c0d0,
                surface_container_lowest: 0x272c36,
                surface: 0x2e3440,
                surface_dim: 0x2e3440,
                surface_container_low: 0x333a47,
                surface_container: 0x373e4c,
                surface_container_high: 0x3b4252,
                surface_container_highest: 0x434c5e,
                surface_bright: 0x4c566a,
                surface_variant: 0x3b4252,
                on_surface: 0xeceff4,
                on_surface_variant: 0xbac4d4,
                outline: 0x6c7a96,
                outline_variant: 0x434c5e,
            }),
            TerminalColorScheme::CatppuccinMocha => Some(Self {
                dark: true,
                seed: 0x89b4fa,
                surface_container_lowest: 0x181825,
                surface: 0x1e1e2e,
                surface_dim: 0x1e1e2e,
                surface_container_low: 0x242536,
                surface_container: 0x2a2b3d,
                surface_container_high: 0x313244,
                surface_container_highest: 0x45475a,
                surface_bright: 0x585b70,
                surface_variant: 0x313244,
                on_surface: 0xcdd6f4,
                on_surface_variant: 0xa6adc8,
                outline: 0x6c7086,
                outline_variant: 0x45475a,
            }),
            TerminalColorScheme::CatppuccinLatte => Some(Self {
                dark: false,
                seed: 0x1e66f5,
                surface_container_lowest: 0xf7f8fb,
                surface: 0xeff1f5,
                surface_bright: 0xeff1f5,
                surface_container_low: 0xe8ebf1,
                surface_container: 0xe2e6ee,
                surface_container_high: 0xdce0e8,
                surface_container_highest: 0xd2d6e2,
                surface_dim: 0xccd0da,
                surface_variant: 0xdce0e8,
                on_surface: 0x4c4f69,
                on_surface_variant: 0x6c6f85,
                outline: 0x8c8fa1,
                outline_variant: 0xc6cad6,
            }),
            TerminalColorScheme::SolarizedDark => Some(Self {
                dark: true,
                seed: 0x2aa198,
                surface_container_lowest: 0x00222b,
                surface: 0x002b36,
                surface_dim: 0x002b36,
                surface_container_low: 0x04313d,
                surface_container: 0x073642,
                surface_container_high: 0x0c3f4d,
                surface_container_highest: 0x144b59,
                surface_bright: 0x1c5766,
                surface_variant: 0x073642,
                on_surface: 0xeee8d5,
                on_surface_variant: 0x93a1a1,
                outline: 0x586e75,
                outline_variant: 0x1a4b57,
            }),
            TerminalColorScheme::SolarizedLight => Some(Self {
                dark: false,
                seed: 0x268bd2,
                surface_container_lowest: 0xfffdf5,
                surface: 0xfdf6e3,
                surface_bright: 0xfdf6e3,
                surface_container_low: 0xf7f0dd,
                surface_container: 0xf2ecd9,
                surface_container_high: 0xeee8d5,
                surface_container_highest: 0xe4decb,
                surface_dim: 0xdcd6c2,
                surface_variant: 0xeee8d5,
                on_surface: 0x586e75,
                on_surface_variant: 0x657b83,
                outline: 0x93a1a1,
                outline_variant: 0xd5ceb9,
            }),
            TerminalColorScheme::TokyoNight => Some(Self {
                dark: true,
                seed: 0x7aa2f7,
                surface_container_lowest: 0x16161e,
                surface: 0x1a1b26,
                surface_dim: 0x1a1b26,
                surface_container_low: 0x1f2335,
                surface_container: 0x24283b,
                surface_container_high: 0x292e42,
                surface_container_highest: 0x333a54,
                surface_bright: 0x3b4261,
                surface_variant: 0x24283b,
                on_surface: 0xc0caf5,
                on_surface_variant: 0x9aa5ce,
                outline: 0x565f89,
                outline_variant: 0x3b4261,
            }),
            TerminalColorScheme::OneDark => Some(Self {
                dark: true,
                seed: 0x61afef,
                surface_container_lowest: 0x21252b,
                surface: 0x282c34,
                surface_dim: 0x282c34,
                surface_container_low: 0x2d323b,
                surface_container: 0x323742,
                surface_container_high: 0x373d49,
                surface_container_highest: 0x3e4451,
                surface_bright: 0x484f5e,
                surface_variant: 0x333844,
                on_surface: 0xd7dae0,
                on_surface_variant: 0x9da5b4,
                outline: 0x5c6370,
                outline_variant: 0x3e4451,
            }),
        }
    }

    fn apply_to_material(self, material: &mut MaterialTheme) {
        // Keep background and surface identical (Material 3 invariant) so shell
        // canvas, active tab, component background, and status footer meet with
        // no visible seam.
        material.roles.background = self.surface;
        material.roles.on_background = self.on_surface;
        material.roles.surface = self.surface;
        material.roles.surface_dim = self.surface_dim;
        material.roles.surface_bright = self.surface_bright;
        material.roles.surface_container_lowest = self.surface_container_lowest;
        material.roles.surface_container_low = self.surface_container_low;
        material.roles.surface_container = self.surface_container;
        material.roles.surface_container_high = self.surface_container_high;
        material.roles.surface_container_highest = self.surface_container_highest;
        material.roles.surface_variant = self.surface_variant;
        material.roles.on_surface = self.on_surface;
        material.roles.on_surface_variant = self.on_surface_variant;
        material.roles.outline = self.outline;
        material.roles.outline_variant = self.outline_variant;
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
            if let Some(preset) = SchemeSurfacePreset::for_scheme(settings.terminal_color_scheme) {
                let seed = material_colors::color::Argb::from_u32(0xff00_0000 | preset.seed);
                let mut theme = material_theme::build_theme(seed, preset.dark);
                preset.apply_to_material(&mut theme);
                theme
            } else {
                material_theme::build_theme(settings.seed_argb(), settings.theme_id.is_dark())
            };

        Self {
            terminal: TerminalPalette::from_scheme_and_material(
                settings.terminal_color_scheme,
                &material,
            ),
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
    a.rdp_client != b.rdp_client
        || a.language != b.language
        || a.font_family != b.font_family
        || a.terminal_font_family != b.terminal_font_family
        || a.font_fallbacks != b.font_fallbacks
        || (a.interface_font_size - b.interface_font_size).abs() > f32::EPSILON
        || (a.terminal_font_size - b.terminal_font_size).abs() > f32::EPSILON
        || (a.line_height - b.line_height).abs() > f32::EPSILON
        || a.theme_id != b.theme_id
        || a.seed_color != b.seed_color
        || a.terminal_color_scheme != b.terminal_color_scheme
        || a.terminal_cursor_style != b.terminal_cursor_style
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
        || a.agent_auto_approve_dangerous != b.agent_auto_approve_dangerous
        || a.web_search != b.web_search
        || a.open_ssh_integration_mode != b.open_ssh_integration_mode
        || a.ssh_bridge != b.ssh_bridge
        || a.ssh_keepalive_interval_secs != b.ssh_keepalive_interval_secs
        || a.ssh_keepalive_max != b.ssh_keepalive_max
        || a.ssh_connect_timeout_secs != b.ssh_connect_timeout_secs
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
            interface_font_size: 3.0,
            terminal_font_size: 40.0,
            line_height: 100.0,
            theme_id: ThemeId::Light,
            seed_color: "#6750A4".into(),
            ..AppSettings::default()
        };

        settings.sanitize();

        assert_eq!(settings.font_family, default_font_family());
        assert_eq!(settings.terminal_font_family, default_font_family());
        assert_eq!(settings.interface_font_size, FONT_SIZE_MIN);
        assert_eq!(settings.terminal_font_size, FONT_SIZE_MAX);
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
        let defaults = AppSettings::default();

        assert_eq!(settings.interface_font_size, defaults.interface_font_size);
        assert_eq!(settings.terminal_font_size, defaults.terminal_font_size);
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
    fn changed_detects_interface_and_terminal_font_sizes_independently() {
        let original = AppSettings::default();
        let mut modified = original.clone();
        modified.interface_font_size += STEP;

        assert!(changed(&original, &modified));

        modified = original.clone();
        modified.terminal_font_size += STEP;

        assert!(changed(&original, &modified));
    }

    #[test]
    fn interface_and_terminal_font_sizes_round_trip_independently() {
        let settings = AppSettings {
            interface_font_size: 16.0,
            terminal_font_size: 18.0,
            ..AppSettings::default()
        };

        let serialized = toml::to_string(&settings).expect("settings should serialize");
        assert!(serialized.contains("interface_font_size = 16.0"));
        assert!(serialized.contains("terminal_font_size = 18.0"));
        assert!(
            !serialized
                .lines()
                .any(|line| line.starts_with("font_size ="))
        );

        let restored: AppSettings =
            toml::from_str(&serialized).expect("settings should deserialize");
        assert_eq!(restored.interface_font_size, 16.0);
        assert_eq!(restored.terminal_font_size, 18.0);
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

    #[test]
    fn terminal_color_scheme_preset_harmonizes_app_and_terminal_surfaces() {
        let luminance = |rgb: u32| -> u32 {
            let r = (rgb >> 16) & 0xff;
            let g = (rgb >> 8) & 0xff;
            let b = rgb & 0xff;
            299 * r + 587 * g + 114 * b
        };

        for scheme in TerminalColorScheme::ALL {
            let theme = Theme::from_settings(&AppSettings {
                theme_id: ThemeId::Light,
                terminal_color_scheme: scheme,
                ..AppSettings::default()
            });
            let roles = theme.material.roles;
            assert_eq!(
                roles.background, roles.surface,
                "{scheme:?}: background must equal surface to avoid shell/page seams"
            );
            assert_eq!(roles.surface, theme.terminal.default_bg);

            if let Some(expected_theme_id) = scheme.preset_theme_id() {
                assert_eq!(theme.material.dark, expected_theme_id.is_dark());
                if expected_theme_id.is_dark() {
                    assert_eq!(roles.surface_dim, roles.surface);
                    assert!(luminance(roles.surface_container_lowest) < luminance(roles.surface));
                    assert!(luminance(roles.surface) < luminance(roles.surface_container_low));
                    assert!(
                        luminance(roles.surface_container_low) < luminance(roles.surface_container)
                    );
                    assert!(
                        luminance(roles.surface_container)
                            < luminance(roles.surface_container_high)
                    );
                    assert!(
                        luminance(roles.surface_container_high)
                            < luminance(roles.surface_container_highest)
                    );
                    assert!(
                        luminance(roles.surface_container_highest)
                            <= luminance(roles.surface_bright)
                    );
                } else {
                    assert_eq!(roles.surface_bright, roles.surface);
                    assert!(luminance(roles.surface_container_lowest) > luminance(roles.surface));
                    assert!(luminance(roles.surface) > luminance(roles.surface_container_low));
                    assert!(
                        luminance(roles.surface_container_low) > luminance(roles.surface_container)
                    );
                    assert!(
                        luminance(roles.surface_container)
                            > luminance(roles.surface_container_high)
                    );
                    assert!(
                        luminance(roles.surface_container_high)
                            > luminance(roles.surface_container_highest)
                    );
                    assert!(
                        luminance(roles.surface_container_highest) >= luminance(roles.surface_dim)
                    );
                }
            }
        }
    }
}
