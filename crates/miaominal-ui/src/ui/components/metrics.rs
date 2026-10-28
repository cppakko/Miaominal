pub(in crate::ui) const TOPBAR_TAB_WIDTH: f32 = 208.0;
pub(in crate::ui) const FOOTER_HEIGHT: f32 = super::shell_tokens::ShellMetrics::FOOTER_HEIGHT;
pub(in crate::ui) const TERMINAL_PANEL_BORDER: f32 = 2.0;
pub(in crate::ui) const EDITOR_DRAWER_WIDTH: f32 = 468.0;

pub(in crate::ui) fn top_bar_height() -> f32 {
    miaominal_settings::scaled_font_size(super::shell_tokens::ShellMetrics::TOPBAR_HEIGHT).as_f32()
}
