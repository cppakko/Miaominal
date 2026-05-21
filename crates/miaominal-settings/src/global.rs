use crate::model::{AppSettings, DEFAULT_CELL_WIDTH, DEFAULT_FONT_SIZE, Theme, ThemeId};
use crate::theme as material_theme;
use gpui::{App, Hsla, Pixels, px, rgb};
use gpui_component::theme::{Theme as ComponentTheme, ThemeMode};
use std::sync::{Arc, RwLock};

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

pub fn sync_component_theme(cx: &mut App) {
    let app_settings = current_settings();
    let theme = current_theme();
    let material = theme.material;
    let is_dark = app_settings.theme_id.is_dark();
    let primary_hover_tone = if is_dark { 70 } else { 35 };
    let primary_active_tone = if is_dark { 60 } else { 30 };
    let secondary_hover_tone = if is_dark { 80 } else { 88 };
    let secondary_active_tone = if is_dark { 70 } else { 82 };
    let danger_hover_tone = if is_dark { 70 } else { 35 };
    let danger_active_tone = if is_dark { 60 } else { 30 };

    let component_theme = ComponentTheme::global_mut(cx);
    component_theme.mode = if is_dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    component_theme.font_family = app_settings.effective_font_family().to_string().into();
    component_theme.mono_font_family = app_settings.effective_font_family().to_string().into();
    component_theme.font_size = px(app_settings.font_size);
    component_theme.mono_font_size = px(app_settings.font_size);

    let colors = &mut component_theme.colors;
    let hsla = |color: u32| rgb(color).into();

    colors.background = hsla(material.roles.background);
    colors.foreground = hsla(material.roles.on_background);
    colors.border = hsla(material.roles.outline_variant);
    colors.input = hsla(material.roles.outline);
    colors.caret = hsla(material.roles.on_surface);
    colors.ring = hsla(material.roles.primary);
    colors.selection = hsla(material.roles.primary_container);
    colors.accent = hsla(material.roles.primary_container);
    colors.accent_foreground = hsla(material.roles.on_primary_container);
    colors.primary = hsla(material.roles.primary);
    colors.primary_hover = hsla(material_theme::palette_tone_rgb(
        material.palettes.primary,
        primary_hover_tone,
    ));
    colors.primary_active = hsla(material_theme::palette_tone_rgb(
        material.palettes.primary,
        primary_active_tone,
    ));
    colors.primary_foreground = hsla(material.roles.on_primary);
    colors.secondary = hsla(material.roles.secondary_container);
    colors.secondary_hover = hsla(material_theme::palette_tone_rgb(
        material.palettes.secondary,
        secondary_hover_tone,
    ));
    colors.secondary_active = hsla(material_theme::palette_tone_rgb(
        material.palettes.secondary,
        secondary_active_tone,
    ));
    colors.secondary_foreground = hsla(material.roles.on_secondary_container);
    colors.muted = hsla(material.roles.surface_container_low);
    colors.muted_foreground = hsla(material.roles.on_surface_variant);
    colors.popover = hsla(material.roles.surface_container_high);
    colors.popover_foreground = hsla(material.roles.on_surface);
    colors.overlay = hsla(material.roles.scrim);
    colors.sidebar = hsla(material.roles.surface_container_low);
    colors.sidebar_foreground = hsla(material.roles.on_surface);
    colors.sidebar_border = hsla(material.roles.outline_variant);
    colors.sidebar_accent = hsla(material.roles.secondary_container);
    colors.sidebar_accent_foreground = hsla(material.roles.on_secondary_container);
    colors.sidebar_primary = hsla(material.roles.primary);
    colors.sidebar_primary_foreground = hsla(material.roles.on_primary);
    colors.scrollbar = Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.0,
        a: 0.0,
    };
    colors.scrollbar_thumb = hsla(material.roles.outline_variant);
    colors.scrollbar_thumb_hover = hsla(material.roles.outline);
    colors.link = hsla(material.roles.primary);
    colors.link_hover = hsla(material_theme::palette_tone_rgb(
        material.palettes.primary,
        primary_hover_tone,
    ));
    colors.link_active = hsla(material_theme::palette_tone_rgb(
        material.palettes.primary,
        primary_active_tone,
    ));
    colors.list = hsla(material.roles.surface);
    colors.list_even = hsla(material.roles.surface_container_low);
    colors.list_head = hsla(material.roles.surface_container_high);
    colors.list_hover = hsla(material.roles.surface_container);
    colors.list_active = hsla(material.roles.primary_container);
    colors.list_active_border = hsla(material.roles.primary);
    colors.table = hsla(material.roles.surface);
    colors.table_even = hsla(material.roles.surface_container_low);
    colors.table_head = hsla(material.roles.surface_container_high);
    colors.table_head_foreground = hsla(material.roles.on_surface);
    colors.table_hover = hsla(material.roles.surface_container);
    colors.table_active = hsla(material.roles.primary_container);
    colors.table_active_border = hsla(material.roles.primary);
    colors.table_row_border = hsla(material.roles.outline_variant);
    colors.tab_bar = hsla(material.roles.surface);
    colors.tab_bar_segmented = hsla(material.roles.surface_container_high);
    colors.tab = hsla(material.roles.surface);
    colors.tab_foreground = hsla(material.roles.on_surface_variant);
    colors.tab_active = hsla(material.roles.secondary_container);
    colors.tab_active_foreground = hsla(material.roles.on_secondary_container);
    colors.title_bar = hsla(material.roles.surface_container_low);
    colors.title_bar_border = hsla(material.roles.outline_variant);
    colors.danger = hsla(material.roles.error);
    colors.danger_hover = hsla(material_theme::palette_tone_rgb(
        material.palettes.error,
        danger_hover_tone,
    ));
    colors.danger_active = hsla(material_theme::palette_tone_rgb(
        material.palettes.error,
        danger_active_tone,
    ));
    colors.danger_foreground = hsla(material.roles.on_error);
    colors.success = hsla(material.extended.success.color);
    colors.success_hover = hsla(material.extended.success.color);
    colors.success_active = hsla(material.extended.success.color_container);
    colors.success_foreground = hsla(material.extended.success.on_color);
    colors.warning = hsla(material.extended.warning.color);
    colors.warning_hover = hsla(material.extended.warning.color);
    colors.warning_active = hsla(material.extended.warning.color_container);
    colors.warning_foreground = hsla(material.extended.warning.on_color);
    colors.info = hsla(material.extended.info.color);
    colors.info_hover = hsla(material.extended.info.color);
    colors.info_active = hsla(material.extended.info.color_container);
    colors.info_foreground = hsla(material.extended.info.on_color);
    colors.progress_bar = hsla(material.roles.primary_container);
    colors.switch = hsla(material.roles.on_secondary_container);
    colors.switch_thumb = hsla(material.roles.on_primary);
    colors.slider_bar = hsla(material.roles.secondary_container);
    colors.slider_thumb = hsla(material.roles.primary);

    let highlight_theme = Arc::make_mut(&mut component_theme.highlight_theme);
    highlight_theme.name = format!("miaominal_{}", if is_dark { "dark" } else { "light" });
    highlight_theme.appearance = component_theme.mode;
    highlight_theme.style.editor_background = Some(hsla(material.roles.surface_container_highest));
    highlight_theme.style.editor_foreground = Some(hsla(material.roles.on_surface));
    highlight_theme.style.editor_active_line = Some(hsla(material.roles.surface_container_high));
    highlight_theme.style.editor_line_number = Some(hsla(material.roles.on_surface_variant));
    highlight_theme.style.editor_active_line_number = Some(hsla(material.roles.on_surface));
    highlight_theme.style.editor_invisible = Some(hsla(material.roles.outline_variant));
}

pub fn font_family() -> String {
    current_settings().effective_font_family().to_string()
}

pub fn font_fallbacks() -> Vec<String> {
    current_settings().effective_font_fallbacks().to_vec()
}

pub fn font_size() -> f32 {
    current_settings().font_size
}

pub fn scaled_font_size(base_size: f32) -> Pixels {
    px(base_size / DEFAULT_FONT_SIZE * current_settings().font_size)
}

pub fn scaled_line_height(base_height: f32) -> Pixels {
    px(base_height / DEFAULT_FONT_SIZE * current_settings().font_size)
}

pub fn line_height_default() -> f32 {
    current_settings().line_height
}

pub fn cell_width_default() -> f32 {
    DEFAULT_CELL_WIDTH
}
