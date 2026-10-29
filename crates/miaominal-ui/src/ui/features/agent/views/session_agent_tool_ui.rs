use super::super::*;
use super::session_agent_highlight::{
    Token, diff_tokens, highlight_styles, output_tokens, shell_tokens,
};
use crate::ui::i18n;

#[cfg(test)]
use gpui_kit::test::TestSupportExt as _;

#[cfg(test)]
#[path = "session_agent_tool_ui_tests.rs"]
mod tests;

#[derive(Clone, Copy)]
pub(in crate::ui::features::agent) struct ToolTerminalColors {
    pub(in crate::ui::features::agent) surface: u32,
    pub(in crate::ui::features::agent) surface_container_lowest: u32,
    pub(in crate::ui::features::agent) on_surface: u32,
    pub(in crate::ui::features::agent) error: u32,
    pub(in crate::ui::features::agent) text_muted: u32,
}

/// Tallest a command/output block grows before it scrolls.
const TOOL_BLOCK_MAX_HEIGHT_REM: f32 = 15.0;

pub(in crate::ui::features::agent) fn render_tool_terminal_block(
    tool_call_id: &str,
    label: String,
    language: Option<&str>,
    content: String,
    colors: ToolTerminalColors,
    error: bool,
) -> gpui_kit::AnyElement {
    if content.trim().is_empty() {
        let empty = i18n::string("workspace.panel.agent.tool_result.empty_output");
        return render_tool_terminal_block_content(
            tool_call_id,
            label,
            highlighted_text(empty, Vec::new(), colors, false),
            colors,
        );
    }
    let content = content.trim_end().to_string();
    let tokens = match language {
        Some("bash" | "sh" | "shell" | "powershell") => shell_tokens(&content),
        Some("diff" | "patch") => diff_tokens(&content),
        _ => output_tokens(&content),
    };
    render_tool_terminal_block_content(
        tool_call_id,
        label,
        highlighted_text(content, tokens, colors, error),
        colors,
    )
}

/// Monospace, wrapping text with token highlights.
fn highlighted_text(
    text: String,
    tokens: Vec<(std::ops::Range<usize>, Token)>,
    colors: ToolTerminalColors,
    error: bool,
) -> gpui_kit::AnyElement {
    let styles = highlight_styles(tokens);
    div()
        .w_full()
        .min_w_0()
        .font_family(miaominal_settings::terminal_font_family())
        .text_size(miaominal_settings::FontSize::Body.scaled())
        .line_height(miaominal_settings::scaled_line_height(18.0))
        .text_color(rgb(if error {
            colors.error
        } else {
            colors.on_surface
        }))
        .child(gpui_kit::StyledText::new(text).with_highlights(styles))
        .into_any_element()
}

/// MD3 "surface container" block: a small label above content that grows
/// with its text up to [`TOOL_BLOCK_MAX_HEIGHT_REM`], then scrolls.
pub(in crate::ui::features::agent) fn render_tool_terminal_block_content(
    tool_call_id: &str,
    label: String,
    content: gpui_kit::AnyElement,
    colors: ToolTerminalColors,
) -> gpui_kit::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let block_bg = if material.dark {
        colors.surface_container_lowest
    } else {
        colors.surface
    };
    let scroll_id = SharedString::from(format!("session-agent-tool-block-{tool_call_id}-{label}"));

    v_flex()
        .id(SharedString::from(format!("{scroll_id}-block")))
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .overflow_hidden()
        .rounded_md()
        .bg(rgb(block_bg))
        .child(
            div()
                .w_full()
                .px_3()
                .pt_2()
                .text_size(miaominal_settings::FontSize::Body.scaled())
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(colors.text_muted))
                .child(label),
        )
        .child(
            div()
                .id(scroll_id.clone())
                .w_full()
                .min_w_0()
                .max_h(gpui_kit::rems(TOOL_BLOCK_MAX_HEIGHT_REM))
                .overflow_y_scrollbar()
                .id(scroll_id)
                .px_3()
                .pt_1()
                .pb_2()
                .child(content),
        )
        .map(|block| {
            #[cfg(test)]
            let block = block.test_support();
            block
        })
        .into_any_element()
}

pub(in crate::ui::features::agent) fn render_tool_field_grid(
    fields: Vec<(String, String)>,
    colors: ToolTerminalColors,
) -> gpui_kit::AnyElement {
    v_flex()
        .w_full()
        .gap_1()
        .children(fields.into_iter().map(|(label, value)| {
            let field_id =
                SharedString::from(format!("session-agent-tool-field-{}", label.as_str()));
            h_flex()
                .id(field_id)
                .w_full()
                .gap_2()
                .items_start()
                .child(
                    div()
                        .w(px(62.0))
                        .flex_shrink_0()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(colors.text_muted))
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .line_height(miaominal_settings::scaled_line_height(18.0))
                        .text_color(rgb(colors.on_surface))
                        .child(value),
                )
                .into_any_element()
        }))
        .into_any_element()
}

/// Command block: `$ command` with shell highlighting.
pub(in crate::ui::features::agent) fn render_bash_highlighted_command_block(
    tool_call_id: &str,
    label: String,
    command: &str,
    colors: ToolTerminalColors,
) -> gpui_kit::AnyElement {
    if command.trim().is_empty() {
        let text = i18n::string("workspace.panel.agent.tool_result.empty_command");
        return render_tool_terminal_block_content(
            tool_call_id,
            label,
            highlighted_text(text, Vec::new(), colors, false),
            colors,
        );
    }
    let text = format!("$ {}", command.trim_end());
    let tokens = shell_tokens(&text);
    render_tool_terminal_block_content(
        tool_call_id,
        label,
        highlighted_text(text, tokens, colors, false),
        colors,
    )
}

/// One-line preview shown under a collapsed tool card title.
pub(in crate::ui::features::agent) fn tool_call_summary_line(
    tool_call: &crate::ui::features::agent::SessionAgentToolCall,
) -> Option<(String, bool)> {
    let value: serde_json::Value = serde_json::from_str(&tool_call.arguments).ok()?;
    for key in [
        "command", "path", "pattern", "url", "query", "base_dir", "job_id",
    ] {
        if let Some(text) = value.get(key).and_then(|value| value.as_str()) {
            let Some(line) = text.lines().map(str::trim).find(|line| !line.is_empty()) else {
                continue;
            };
            return Some(if key == "command" {
                (format!("$ {line}"), true)
            } else {
                (line.to_string(), false)
            });
        }
    }
    None
}

pub(in crate::ui::features::agent) fn render_tool_summary_line(
    (text, is_command): (String, bool),
    colors: ToolTerminalColors,
) -> gpui_kit::AnyElement {
    let tokens = if is_command {
        shell_tokens(&text)
    } else {
        Vec::new()
    };
    let styles = highlight_styles(tokens);
    div()
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .font_family(miaominal_settings::terminal_font_family())
        .text_size(miaominal_settings::FontSize::Body.scaled())
        .line_height(miaominal_settings::scaled_line_height(18.0))
        .text_color(rgb(colors.text_muted))
        .child(gpui_kit::StyledText::new(text).with_highlights(styles))
        .into_any_element()
}
