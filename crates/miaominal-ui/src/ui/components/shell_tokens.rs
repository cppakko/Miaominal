//! Product presentation projected from Material 3 roles. Geometry uses the
//! interface font as its scale, matching the React reference at the default size.
use gpui_kit::{Hsla, Rems, rems, rgb};

pub(in crate::ui) fn space(value: f32) -> Rems {
    rems(value / 14.0)
}

/// React's title-medium, body-medium and supporting typography at 14px base.
/// Font families continue to come from the user's interface font settings.
pub(in crate::ui) struct ShellTypography;
impl ShellTypography {
    pub const TITLE_SIZE: f32 = 16.0;
    pub const TITLE_LINE_HEIGHT: f32 = 24.0;
    pub const BODY_SIZE: f32 = 14.0;
    pub const BODY_LINE_HEIGHT: f32 = 21.0;
    pub const SUPPORTING_SIZE: f32 = 12.0;
    pub const SUPPORTING_LINE_HEIGHT: f32 = 18.0;
    pub const SEARCH_GROUP_LINE_HEIGHT: f32 = 20.0;
    pub const SEARCH_CATEGORY_LINE_HEIGHT: f32 = 16.0;
}

#[derive(Clone, Copy)]
pub(in crate::ui) struct ShellColors {
    pub chrome: Hsla,
    pub canvas: Hsla,
    pub navigation: Hsla,
    pub navigation_bar: Hsla,
    pub on_navigation_bar: Hsla,
    pub selected: Hsla,
    pub on_selected: Hsla,
    pub card: Hsla,
    pub hover: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub primary: Hsla,
    pub on_primary: Hsla,
    pub danger: Hsla,
    pub primary_container: Hsla,
    pub on_primary_container: Hsla,
    pub footer: Hsla,
    pub chip: Hsla,
    pub scrim: Hsla,
}

impl ShellColors {
    pub fn current() -> Self {
        let roles = miaominal_settings::current_theme().material.roles;
        Self {
            chrome: rgb(roles.surface_container_high).into(),
            canvas: rgb(roles.surface).into(),
            navigation: rgb(roles.surface_container).into(),
            navigation_bar: rgb(roles.inverse_surface).into(),
            on_navigation_bar: rgb(roles.inverse_on_surface).into(),
            selected: rgb(roles.secondary_container).into(),
            on_selected: rgb(roles.on_secondary_container).into(),
            card: rgb(roles.surface_container_high).into(),
            hover: rgb(roles.surface_container_highest).into(),
            foreground: rgb(roles.on_surface).into(),
            muted: rgb(roles.on_surface_variant).into(),
            primary: rgb(roles.primary).into(),
            on_primary: rgb(roles.on_primary).into(),
            danger: rgb(roles.error).into(),
            primary_container: rgb(roles.primary_container).into(),
            on_primary_container: rgb(roles.on_primary_container).into(),
            footer: rgb(roles.surface).into(),
            chip: rgb(roles.surface_container_low).into(),
            scrim: rgb(roles.scrim).into(),
        }
    }
}

/// Reference geometry in logical pixels at the default interface font size.
pub(in crate::ui) struct ShellMetrics;
impl ShellMetrics {
    pub fn management_inset(window_width: f32) -> f32 {
        if window_width <= 850.0 {
            20.0
        } else if window_width <= 1100.0 {
            24.0
        } else {
            32.0
        }
    }

    pub fn titlebar_inset(window_width: f32) -> f32 {
        if window_width <= 850.0 { 8.0 } else { 16.0 }
    }

    pub const TOPBAR_HEIGHT: f32 = 56.0;
    pub const TAB_WIDTH: f32 = 208.0;
    pub const COMPACT_TAB_WIDTH: f32 = 176.0;
    pub const TAB_HEIGHT: f32 = 48.0;
    pub const NAV_BAR_HEIGHT: f32 = 72.0;
    pub const NAV_CONTENT_SAFE_AREA: f32 = 96.0;
    pub const SESSION_PANEL_MENU_CLEARANCE: f32 = 64.0;
    pub const NAV_INDICATOR_WIDTH: f32 = 56.0;
    pub const NAV_INDICATOR_HEIGHT: f32 = 40.0;
    pub const DOCK_ACTION_ICON_SIZE: f32 = 20.0;
    pub const SEARCH_HEIGHT: f32 = 56.0;
    pub const FAB_SIZE: f32 = 56.0;
    pub const FAB_RADIUS: f32 = 16.0;
    pub const FAB_ICON_SIZE: f32 = 24.0;
    pub const FAB_RIGHT_INSET: f32 = 32.0;
    pub const FAB_BOTTOM_INSET: f32 = 24.0;
    pub const EDITOR_ACTION_HEIGHT: f32 = 40.0;
    pub const EDITOR_ACTION_RADIUS: f32 = 20.0;
    pub const EDITOR_ACTION_ICON_SIZE: f32 = 18.0;
    /// Every catalog card has exactly this height (title + subtitle + chips fit).
    pub const CARD_HEIGHT: f32 = 88.0;
    pub const CARD_RADIUS: f32 = 20.0;
    pub const CARD_PADDING: f32 = 12.0;
    pub const CARD_ICON_SIZE: f32 = 40.0;
    pub const CARD_ICON_RADIUS: f32 = 12.0;
    pub const CARD_CHIP_HEIGHT: f32 = 20.0;
    pub const CARD_MIN_WIDTH: f32 = 264.0;
    pub const CARD_GAP: f32 = 12.0;
    pub const CONTENT_MAX_WIDTH: f32 = 1280.0;
    pub const CATALOG_INSET: f32 = 20.0;
    // Outer margins (8 + 8) and trailing shell inset (12) after the rail moved
    // into the bottom navigation bar.
    pub const MANAGEMENT_CHROME_WIDTH: f32 = 28.0;
    pub const FOOTER_HEIGHT: f32 = 44.0;
    pub const TOOL_SIZE: f32 = 32.0;
    pub const VIEW_MODE_ITEM_WIDTH: f32 = 44.0;
    pub const VIEW_MODE_ITEM_HEIGHT: f32 = 32.0;
    pub const VIEW_MODE_ICON_SIZE: f32 = 20.0;
}
