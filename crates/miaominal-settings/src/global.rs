use crate::model::{AppSettings, DEFAULT_CELL_WIDTH, Theme, ThemeId};
use std::sync::RwLock;

static GLOBAL: RwLock<GlobalState> = RwLock::new(GlobalState {
    settings: None,
    theme: None,
});

struct GlobalState {
    settings: Option<AppSettings>,
    theme: Option<Theme>,
}

fn ensure_initialized() {
    let needs_init = {
        let guard = GLOBAL.read().expect("settings poisoned");
        guard.settings.is_none()
    };
    if needs_init {
        let defaults = AppSettings::default();
        let theme = Theme::from_settings(&defaults);
        let mut guard = GLOBAL.write().expect("settings poisoned");
        if guard.settings.is_none() {
            guard.settings = Some(defaults);
            guard.theme = Some(theme);
        }
    }
}

pub fn install(settings: AppSettings) {
    let theme = Theme::from_settings(&settings);
    let mut guard = GLOBAL.write().expect("settings poisoned");
    guard.settings = Some(settings);
    guard.theme = Some(theme);
}

pub fn current_settings() -> AppSettings {
    ensure_initialized();
    let guard = GLOBAL.read().expect("settings poisoned");
    guard.settings.as_ref().cloned().unwrap_or_default()
}

pub fn current_theme() -> Theme {
    ensure_initialized();
    let guard = GLOBAL.read().expect("settings poisoned");
    guard
        .theme
        .as_ref()
        .cloned()
        .unwrap_or_else(|| Theme::from_id(ThemeId::Light))
}

pub fn font_family() -> String {
    current_settings().effective_font_family().to_string()
}

pub fn terminal_font_family() -> String {
    current_settings()
        .effective_terminal_font_family()
        .to_string()
}

pub fn font_fallbacks() -> Vec<String> {
    current_settings().effective_font_fallbacks().to_vec()
}

pub fn font_size() -> f32 {
    current_settings().font_size
}

pub fn line_height_default() -> f32 {
    current_settings().line_height
}

pub fn cell_width_default() -> f32 {
    DEFAULT_CELL_WIDTH
}
