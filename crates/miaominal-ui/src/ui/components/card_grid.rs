use super::shell_tokens::{ShellMetrics, space};
use gpui_kit::{Div, Styled, div};

/// Equal-width catalog tracks. Callers supply the content width in the shell's
/// 14px reference scale, after page insets and any editor drawer are removed.
pub(in crate::ui) fn card_grid(available_width: f32) -> Div {
    div()
        .w_full()
        .min_w_0()
        .grid()
        .grid_cols(card_grid_columns(available_width))
        .gap(space(ShellMetrics::CARD_GAP))
}

pub(in crate::ui) fn card_grid_columns(available_width: f32) -> u16 {
    let width = available_width.clamp(1.0, ShellMetrics::CONTENT_MAX_WIDTH);
    ((width + ShellMetrics::CARD_GAP) / (ShellMetrics::CARD_MIN_WIDTH + ShellMetrics::CARD_GAP))
        .floor()
        .max(1.0) as u16
}
