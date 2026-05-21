use super::super::workspace::SplitDirection;
use super::super::*;
use miaominal_terminal as terminal;
use miaominal_terminal::{
    TerminalInputModes, TerminalScroll, terminal_cell_width_default, terminal_font,
    terminal_font_size, terminal_line_height_default,
};

pub(in crate::ui::shell) use miaominal_terminal::{TerminalKeyEvent, TerminalKeyPhase};

pub(in crate::ui::shell) enum TerminalKeyAction {
    Bytes(Vec<u8>),
    Scroll(TerminalScroll),
    Copy,
    Paste,
    OpenSearch,
    Split(SplitDirection),
    ClosePane,
}

pub(in crate::ui::shell) fn terminal_cell_width(window: &Window) -> f32 {
    let terminal_font = terminal_font();
    let terminal_font_size = px(terminal_font_size());
    let terminal_font_id = window.text_system().resolve_font(&terminal_font);
    let fallback = terminal_cell_width_default();

    window
        .text_system()
        .ch_advance(terminal_font_id, terminal_font_size)
        .map(f32::from)
        .unwrap_or(fallback)
        .max(1.0)
}

fn resolved_terminal_line_height(configured: f32, measured: f32) -> f32 {
    if configured.is_finite() && configured > 0.0 {
        configured
    } else if measured.is_finite() && measured > 0.0 {
        measured
    } else {
        terminal_line_height_default().max(1.0)
    }
}

pub(in crate::ui::shell) fn terminal_line_height(window: &Window) -> f32 {
    let configured = terminal_line_height_default();
    if configured.is_finite() && configured > 0.0 {
        return configured;
    }

    let terminal_font = terminal_font();
    let terminal_font_size = px(terminal_font_size());
    let terminal_font_id = window.text_system().resolve_font(&terminal_font);

    let measured = f32::from(
        window
            .text_system()
            .bounding_box(terminal_font_id, terminal_font_size)
            .size
            .height,
    ) + 4.0;
    resolved_terminal_line_height(configured, measured)
}

pub(in crate::ui::shell) fn classify_terminal_key(
    event: TerminalKeyEvent<'_>,
    input_modes: TerminalInputModes,
) -> Option<TerminalKeyAction> {
    let keystroke = event.keystroke;
    let modifiers = keystroke.modifiers;

    if event.phase == TerminalKeyPhase::Press {
        let bindings = miaominal_settings::current_settings().key_bindings;
        if bindings.copy.matches_keystroke(keystroke) {
            return Some(TerminalKeyAction::Copy);
        }
        if bindings.paste.matches_keystroke(keystroke) {
            return Some(TerminalKeyAction::Paste);
        }
        if bindings.search.matches_keystroke(keystroke) {
            return Some(TerminalKeyAction::OpenSearch);
        }
        if bindings.split_right.matches_keystroke(keystroke) {
            return Some(TerminalKeyAction::Split(SplitDirection::Right));
        }
        if bindings.split_down.matches_keystroke(keystroke) {
            return Some(TerminalKeyAction::Split(SplitDirection::Down));
        }
        if bindings.close_pane.matches_keystroke(keystroke) {
            return Some(TerminalKeyAction::ClosePane);
        }
    }

    if event.phase != TerminalKeyPhase::Release
        && modifiers.shift
        && !modifiers.control
        && !modifiers.alt
        && !modifiers.platform
    {
        match keystroke.key.as_str() {
            "pageup" => return Some(TerminalKeyAction::Scroll(TerminalScroll::PageUp)),
            "pagedown" => return Some(TerminalKeyAction::Scroll(TerminalScroll::PageDown)),
            "home" => return Some(TerminalKeyAction::Scroll(TerminalScroll::Top)),
            "end" => return Some(TerminalKeyAction::Scroll(TerminalScroll::Bottom)),
            "up" => return Some(TerminalKeyAction::Scroll(TerminalScroll::Lines(1))),
            "down" => return Some(TerminalKeyAction::Scroll(TerminalScroll::Lines(-1))),
            _ => {}
        }
    }

    terminal::encode_terminal_input(event, input_modes).map(TerminalKeyAction::Bytes)
}

#[cfg(test)]
mod tests {
    use super::resolved_terminal_line_height;

    #[test]
    fn terminal_line_height_uses_configured_value_even_when_measured_is_larger() {
        assert_eq!(resolved_terminal_line_height(14.0, 20.0), 14.0);
        assert_eq!(resolved_terminal_line_height(24.0, 20.0), 24.0);
    }

    #[test]
    fn terminal_line_height_falls_back_to_measured_value_when_config_is_invalid() {
        assert_eq!(resolved_terminal_line_height(0.0, 20.0), 20.0);
        assert_eq!(resolved_terminal_line_height(f32::NAN, 20.0), 20.0);
    }
}
