use material_colors::{
    color::Argb,
    dynamic_color::variant::Variant,
    palette::TonalPalette,
    scheme::Scheme,
    theme::{ColorGroup, CustomColor, Theme as MaterialLibTheme, ThemeBuilder},
};
use std::str::FromStr;

pub const DEFAULT_SEED_COLOR: &str = "#2f6fed";

const SUCCESS_COLOR_NAME: &str = "success";
const WARNING_COLOR_NAME: &str = "warning";
const INFO_COLOR_NAME: &str = "info";

const SUCCESS_COLOR: u32 = 0x2e7d32;
const WARNING_COLOR: u32 = 0xb26a00;
const INFO_COLOR: u32 = 0x00639b;

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct Md3Roles {
    pub background: u32,
    pub on_background: u32,
    pub surface: u32,
    pub surface_dim: u32,
    pub surface_bright: u32,
    pub surface_container_lowest: u32,
    pub surface_container_low: u32,
    pub surface_container: u32,
    pub surface_container_high: u32,
    pub surface_container_highest: u32,
    pub surface_variant: u32,
    pub on_surface: u32,
    pub on_surface_variant: u32,
    pub outline: u32,
    pub outline_variant: u32,
    pub primary: u32,
    pub on_primary: u32,
    pub primary_container: u32,
    pub on_primary_container: u32,
    pub inverse_primary: u32,
    pub secondary: u32,
    pub on_secondary: u32,
    pub secondary_container: u32,
    pub on_secondary_container: u32,
    pub tertiary: u32,
    pub on_tertiary: u32,
    pub tertiary_container: u32,
    pub on_tertiary_container: u32,
    pub error: u32,
    pub on_error: u32,
    pub error_container: u32,
    pub on_error_container: u32,
    pub surface_tint: u32,
    pub inverse_surface: u32,
    pub inverse_on_surface: u32,
    pub shadow: u32,
    pub scrim: u32,
}

impl Md3Roles {
    fn from_scheme(scheme: Scheme) -> Self {
        Self {
            background: rgb_u32(scheme.background),
            on_background: rgb_u32(scheme.on_background),
            surface: rgb_u32(scheme.surface),
            surface_dim: rgb_u32(scheme.surface_dim),
            surface_bright: rgb_u32(scheme.surface_bright),
            surface_container_lowest: rgb_u32(scheme.surface_container_lowest),
            surface_container_low: rgb_u32(scheme.surface_container_low),
            surface_container: rgb_u32(scheme.surface_container),
            surface_container_high: rgb_u32(scheme.surface_container_high),
            surface_container_highest: rgb_u32(scheme.surface_container_highest),
            surface_variant: rgb_u32(scheme.surface_variant),
            on_surface: rgb_u32(scheme.on_surface),
            on_surface_variant: rgb_u32(scheme.on_surface_variant),
            outline: rgb_u32(scheme.outline),
            outline_variant: rgb_u32(scheme.outline_variant),
            primary: rgb_u32(scheme.primary),
            on_primary: rgb_u32(scheme.on_primary),
            primary_container: rgb_u32(scheme.primary_container),
            on_primary_container: rgb_u32(scheme.on_primary_container),
            inverse_primary: rgb_u32(scheme.inverse_primary),
            secondary: rgb_u32(scheme.secondary),
            on_secondary: rgb_u32(scheme.on_secondary),
            secondary_container: rgb_u32(scheme.secondary_container),
            on_secondary_container: rgb_u32(scheme.on_secondary_container),
            tertiary: rgb_u32(scheme.tertiary),
            on_tertiary: rgb_u32(scheme.on_tertiary),
            tertiary_container: rgb_u32(scheme.tertiary_container),
            on_tertiary_container: rgb_u32(scheme.on_tertiary_container),
            error: rgb_u32(scheme.error),
            on_error: rgb_u32(scheme.on_error),
            error_container: rgb_u32(scheme.error_container),
            on_error_container: rgb_u32(scheme.on_error_container),
            surface_tint: rgb_u32(scheme.surface_tint),
            inverse_surface: rgb_u32(scheme.inverse_surface),
            inverse_on_surface: rgb_u32(scheme.inverse_on_surface),
            shadow: rgb_u32(scheme.shadow),
            scrim: rgb_u32(scheme.scrim),
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct RoleGroup {
    pub color: u32,
    pub on_color: u32,
    pub color_container: u32,
    pub on_color_container: u32,
}

impl RoleGroup {
    fn from_group(group: &ColorGroup) -> Self {
        Self {
            color: rgb_u32(group.color),
            on_color: rgb_u32(group.on_color),
            color_container: rgb_u32(group.color_container),
            on_color_container: rgb_u32(group.on_color_container),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExtendedRoles {
    pub success: RoleGroup,
    pub warning: RoleGroup,
    pub info: RoleGroup,
}

#[derive(Debug, Clone, Copy)]
pub struct ThemePalettes {
    pub primary: TonalPalette,
    pub secondary: TonalPalette,
    pub tertiary: TonalPalette,
    pub neutral: TonalPalette,
    pub neutral_variant: TonalPalette,
    pub error: TonalPalette,
}

#[derive(Debug, Clone, Copy)]
pub struct MaterialTheme {
    pub source: u32,
    pub dark: bool,
    pub roles: Md3Roles,
    pub extended: ExtendedRoles,
    pub palettes: ThemePalettes,
}

pub fn normalize_seed_color(input: &str) -> Option<String> {
    parse_seed_color(input).map(seed_to_hex)
}

pub fn parse_seed_color(input: &str) -> Option<Argb> {
    Argb::from_str(input.trim()).ok().map(opaque)
}

pub fn parse_seed_color_or_default(input: &str) -> Argb {
    parse_seed_color(input).unwrap_or_else(default_seed_argb)
}

pub fn default_seed_argb() -> Argb {
    Argb::from_str(DEFAULT_SEED_COLOR).unwrap_or_else(|_| Argb::from_u32(0xff2f6fed))
}

pub fn build_theme(source: Argb, dark: bool) -> MaterialTheme {
    let theme = ThemeBuilder::with_source(source)
        .variant(Variant::TonalSpot)
        .custom_colors(vec![
            custom_color(SUCCESS_COLOR_NAME, SUCCESS_COLOR),
            custom_color(WARNING_COLOR_NAME, WARNING_COLOR),
            custom_color(INFO_COLOR_NAME, INFO_COLOR),
        ])
        .build();

    let extended = ExtendedRoles {
        success: role_group_from_custom(&theme, SUCCESS_COLOR_NAME, dark),
        warning: role_group_from_custom(&theme, WARNING_COLOR_NAME, dark),
        info: role_group_from_custom(&theme, INFO_COLOR_NAME, dark),
    };

    let scheme = if dark {
        theme.schemes.dark
    } else {
        theme.schemes.light
    };

    MaterialTheme {
        source: rgb_u32(source),
        dark,
        roles: Md3Roles::from_scheme(scheme),
        extended,
        palettes: ThemePalettes {
            primary: theme.palettes.primary,
            secondary: theme.palettes.secondary,
            tertiary: theme.palettes.tertiary,
            neutral: theme.palettes.neutral,
            neutral_variant: theme.palettes.neutral_variant,
            error: theme.palettes.error,
        },
    }
}

pub fn palette_tone_rgb(palette: TonalPalette, tone: i32) -> u32 {
    rgb_u32(palette.tone(tone))
}

pub fn terminal_ansi(theme: &MaterialTheme) -> [u32; 16] {
    let normal_tone = if theme.dark { 80 } else { 40 };
    let bright_tone = if theme.dark { 90 } else { 50 };
    let neutral_base = 20;
    let neutral_soft = if theme.dark { 80 } else { 60 };
    let neutral_bright = if theme.dark { 40 } else { 35 };
    let neutral_high = if theme.dark { 95 } else { 80 };

    [
        palette_tone_rgb(theme.palettes.neutral_variant, neutral_base),
        palette_tone_rgb(theme.palettes.error, normal_tone),
        tone_rgb_from_argb(Argb::from_u32(0xff000000 | SUCCESS_COLOR), normal_tone),
        tone_rgb_from_argb(Argb::from_u32(0xff000000 | WARNING_COLOR), normal_tone),
        palette_tone_rgb(theme.palettes.primary, normal_tone),
        palette_tone_rgb(theme.palettes.tertiary, normal_tone),
        palette_tone_rgb(theme.palettes.secondary, normal_tone),
        palette_tone_rgb(theme.palettes.neutral, neutral_soft),
        palette_tone_rgb(theme.palettes.neutral_variant, neutral_bright),
        palette_tone_rgb(theme.palettes.error, bright_tone),
        tone_rgb_from_argb(Argb::from_u32(0xff000000 | SUCCESS_COLOR), bright_tone),
        tone_rgb_from_argb(Argb::from_u32(0xff000000 | WARNING_COLOR), bright_tone),
        palette_tone_rgb(theme.palettes.primary, bright_tone),
        palette_tone_rgb(theme.palettes.tertiary, bright_tone),
        palette_tone_rgb(theme.palettes.secondary, bright_tone),
        palette_tone_rgb(theme.palettes.neutral, neutral_high),
    ]
}

fn custom_color(name: &str, value: u32) -> CustomColor {
    CustomColor {
        value: Argb::from_u32(0xff000000 | value),
        name: name.to_string(),
        blend: true,
    }
}

fn role_group_from_custom(theme: &MaterialLibTheme, name: &str, dark: bool) -> RoleGroup {
    if let Some(group) = theme
        .custom_colors
        .iter()
        .find(|group| group.color.name == name)
    {
        if dark {
            RoleGroup::from_group(&group.dark)
        } else {
            RoleGroup::from_group(&group.light)
        }
    } else {
        let fallback_seed = match name {
            SUCCESS_COLOR_NAME => Argb::from_u32(0xff000000 | SUCCESS_COLOR),
            WARNING_COLOR_NAME => Argb::from_u32(0xff000000 | WARNING_COLOR),
            _ => Argb::from_u32(0xff000000 | INFO_COLOR),
        };
        let palette = TonalPalette::from_hct(fallback_seed.into());
        let fallback_group = fallback_color_group(palette, dark);
        RoleGroup::from_group(&fallback_group)
    }
}

fn fallback_color_group(palette: TonalPalette, dark: bool) -> ColorGroup {
    if dark {
        ColorGroup {
            color: palette.tone(80),
            on_color: palette.tone(20),
            color_container: palette.tone(30),
            on_color_container: palette.tone(90),
        }
    } else {
        ColorGroup {
            color: palette.tone(40),
            on_color: palette.tone(100),
            color_container: palette.tone(90),
            on_color_container: palette.tone(10),
        }
    }
}

fn tone_rgb_from_argb(seed: Argb, tone: i32) -> u32 {
    let palette = TonalPalette::from_hct(seed.into());
    palette_tone_rgb(palette, tone)
}

fn opaque(color: Argb) -> Argb {
    Argb::new(255, color.red, color.green, color.blue)
}

fn seed_to_hex(color: Argb) -> String {
    format!("#{:02x}{:02x}{:02x}", color.red, color.green, color.blue)
}

fn rgb_u32(color: Argb) -> u32 {
    ((color.red as u32) << 16) | ((color.green as u32) << 8) | (color.blue as u32)
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_SEED_COLOR, build_theme, normalize_seed_color, parse_seed_color_or_default,
        terminal_ansi,
    };

    #[test]
    fn normalizes_seed_color_to_lowercase_hex() {
        assert_eq!(
            normalize_seed_color("2F6FED").as_deref(),
            Some(DEFAULT_SEED_COLOR)
        );
    }

    #[test]
    fn falls_back_for_invalid_seed_color() {
        let seed = parse_seed_color_or_default("not-a-color");
        let theme = build_theme(seed, false);
        assert_eq!(theme.source, 0x2f6fed);
    }

    #[test]
    fn builds_non_zero_terminal_ansi_palette() {
        let seed = parse_seed_color_or_default(DEFAULT_SEED_COLOR);
        let theme = build_theme(seed, true);
        let ansi = terminal_ansi(&theme);

        assert_eq!(ansi.len(), 16);
        assert!(ansi.iter().all(|color| *color != 0));
    }
}
