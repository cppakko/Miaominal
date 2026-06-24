use super::super::*;

#[derive(Clone, Copy)]
pub(in crate::ui::shell::layout) struct ToolTerminalColors {
    pub(in crate::ui::shell::layout) surface: u32,
    pub(in crate::ui::shell::layout) surface_container_lowest: u32,
    pub(in crate::ui::shell::layout) outline_variant: u32,
    pub(in crate::ui::shell::layout) on_surface: u32,
    pub(in crate::ui::shell::layout) error: u32,
    pub(in crate::ui::shell::layout) text_muted: u32,
    pub(in crate::ui::shell::layout) selectable: bool,
}

pub(in crate::ui::shell::layout) fn render_tool_terminal_block(
    tool_call_id: &str,
    label: &str,
    content: String,
    colors: ToolTerminalColors,
    error: bool,
) -> gpui::AnyElement {
    if content.trim().is_empty() {
        let empty_content = div()
            .font_family(miaominal_settings::font_family())
            .text_size(miaominal_settings::FontSize::Body.scaled())
            .line_height(miaominal_settings::scaled_line_height(18.0))
            .text_color(rgb(if error {
                colors.error
            } else {
                colors.on_surface
            }))
            .child("(no output)")
            .into_any_element();
        return render_tool_terminal_block_content(label, empty_content, colors);
    }

    // Use markdown code block for syntax highlighting via tree-sitter
    // Try to infer language from label
    let language = match label {
        "Diff" | "Patch Output" => "diff",
        "Command" => "bash",
        _ => "", // Plain text, no highlighting
    };

    let markdown_code = if language.is_empty() {
        // For plain text or unknown content, use plain div
        let highlighted_content = div()
            .font_family(miaominal_settings::font_family())
            .text_size(miaominal_settings::FontSize::Body.scaled())
            .line_height(miaominal_settings::scaled_line_height(18.0))
            .text_color(rgb(if error {
                colors.error
            } else {
                colors.on_surface
            }))
            .child(content.clone())
            .into_any_element();
        return render_tool_terminal_block_content(label, highlighted_content, colors);
    } else {
        format!("```{}\n{}\n```", language, content)
    };

    let highlighted_content = div()
        .w_full()
        .child(
            gpui_component::text::TextView::markdown(
                tool_terminal_markdown_id(tool_call_id, label, language),
                markdown_code,
            )
            .selectable(colors.selectable),
        )
        .when(error, |this| this.text_color(rgb(colors.error)))
        .into_any_element();

    render_tool_terminal_block_content(label, highlighted_content, colors)
}

pub(in crate::ui::shell::layout) fn tool_terminal_markdown_id(
    tool_call_id: &str,
    label: &str,
    language: &str,
) -> String {
    format!("session-agent-tool-markdown-{tool_call_id}-{label}-{language}")
}

pub(in crate::ui::shell::layout) fn render_tool_terminal_block_content(
    label: &str,
    content: gpui::AnyElement,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let material = miaominal_settings::current_theme().material;
    let terminal_bg = if material.dark {
        colors.surface_container_lowest
    } else {
        colors.surface
    };

    v_flex()
        .w_full()
        .overflow_hidden()
        .rounded(px(6.0))
        .bg(rgb(terminal_bg))
        .child(
            div()
                .w_full()
                .px_2()
                .py_1()
                .border_b_1()
                .border_color(rgb(colors.outline_variant))
                .text_size(miaominal_settings::FontSize::Body.scaled())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(colors.text_muted))
                .child(label.to_string()),
        )
        .child(
            div()
                .w_full()
                .min_h(px(34.0))
                .max_h(px(220.0))
                .overflow_y_scrollbar()
                .px_2()
                .py_2()
                .child(content),
        )
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_tool_field_grid(
    fields: Vec<(String, String)>,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    v_flex()
        .w_full()
        .gap_1()
        .children(fields.into_iter().map(|(label, value)| {
            h_flex()
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

/// Renders a terminal-style block with syntax-highlighted bash command text.
pub(in crate::ui::shell::layout) fn render_bash_highlighted_command_block(
    tool_call_id: &str,
    label: &str,
    command: &str,
    colors: ToolTerminalColors,
    _syntax_theme: &::theme::SyntaxTheme,
) -> gpui::AnyElement {
    if command.trim().is_empty() {
        let base_color = gpui::Hsla::from(rgb(colors.on_surface));
        let content = div()
            .font_family(miaominal_settings::font_family())
            .text_size(miaominal_settings::FontSize::Body.scaled())
            .line_height(miaominal_settings::scaled_line_height(18.0))
            .text_color(base_color)
            .child("(no command)")
            .into_any_element();
        return render_tool_terminal_block_content(label, content, colors);
    }

    // Use markdown code block for syntax highlighting via tree-sitter
    let markdown_code = format!("```bash\n{}\n```", command);
    let content = div()
        .w_full()
        .child(
            gpui_component::text::TextView::markdown(
                tool_terminal_markdown_id(tool_call_id, label, "bash"),
                markdown_code,
            )
            .selectable(colors.selectable),
        )
        .into_any_element();
    render_tool_terminal_block_content(label, content, colors)
}
