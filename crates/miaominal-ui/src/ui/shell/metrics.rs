pub(in crate::ui::shell) const LEFT_RAIL_WIDTH: f32 = 64.0;
const TOP_BAR_MIN_HEIGHT: f32 = 52.0;
pub(in crate::ui::shell) const TOPBAR_TAB_WIDTH: f32 = 176.0;
pub(in crate::ui::shell) const STATUS_BAR_HEIGHT: f32 = 28.0;
pub(in crate::ui::shell) const FOOTER_HEIGHT: f32 = STATUS_BAR_HEIGHT;
pub(in crate::ui::shell) const NAV_ITEM_HEIGHT: f32 = 48.0;
pub(in crate::ui::shell) const GROUP_CARD_WIDTH: f32 = 194.0;
pub(in crate::ui::shell) const GROUP_CARD_HEIGHT: f32 = 96.0;
pub(in crate::ui::shell) const HOST_CARD_WIDTH: f32 = 284.0;
pub(in crate::ui::shell) const HOST_CARD_HEIGHT: f32 = 82.0;
pub(in crate::ui::shell) const FORWARD_RULE_CARD_WIDTH: f32 = 284.0;
pub(in crate::ui::shell) const FORWARD_RULE_CARD_HEIGHT: f32 = 82.0;
pub(in crate::ui::shell) const TRUSTED_CARD_WIDTH: f32 = 332.0;
pub(in crate::ui::shell) const TERMINAL_PANEL_BORDER: f32 = 2.0;
pub(in crate::ui::shell) const EDITOR_DRAWER_WIDTH: f32 = 468.0;

const TOPBAR_TAB_RENAME_MIN_HEIGHT: f32 = 22.0;
const TOPBAR_TAB_RENAME_VERTICAL_PADDING: f32 = 8.0;
const TOPBAR_CONTENT_VERTICAL_PADDING: f32 = 24.0;

pub(in crate::ui::shell) fn topbar_tab_rename_height(title_line_height: f32) -> f32 {
    TOPBAR_TAB_RENAME_MIN_HEIGHT.max(title_line_height + TOPBAR_TAB_RENAME_VERTICAL_PADDING)
}

pub(in crate::ui::shell) fn top_bar_height_for_title_line_height(title_line_height: f32) -> f32 {
    TOP_BAR_MIN_HEIGHT
        .max(topbar_tab_rename_height(title_line_height) + TOPBAR_CONTENT_VERTICAL_PADDING)
}

pub(in crate::ui::shell) fn top_bar_height() -> f32 {
    top_bar_height_for_title_line_height(miaominal_settings::scaled_line_height(14.0).as_f32())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_bar_height_keeps_default_height_for_compact_titles() {
        for title_line_height in [8.0, 14.0, 20.0] {
            assert_eq!(
                top_bar_height_for_title_line_height(title_line_height),
                52.0
            );
        }
    }

    #[test]
    fn top_bar_height_grows_for_large_titles() {
        assert_eq!(top_bar_height_for_title_line_height(24.0), 56.0);
        assert_eq!(top_bar_height_for_title_line_height(32.0), 64.0);
    }
}
