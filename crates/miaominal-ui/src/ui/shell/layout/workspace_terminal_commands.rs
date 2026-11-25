use super::super::*;
use crate::ui::assets::AppIcon;
use crate::ui::components::shell_tokens::space;
use crate::ui::components::tokens::color_with_alpha;
use crate::ui::features::session::TerminalInputHost as _;
use crate::ui::i18n;
use gpui_kit::component::button::{Button, ButtonCustomVariant};
use gpui_kit::component::{Disableable as _, Icon, h_flex};
use gpui_kit::{FontWeight, Hsla};
use miaominal_terminal::{CommandBlockStatus, TerminalCommandBlockContent, TerminalCommandMarker};

const MAX_TOOLTIP_COMMAND_CHARS: usize = 48;
const MAX_DIAGNOSTIC_OUTPUT_CHARS: usize = 6000;
const COMMAND_BAR_HEIGHT: f32 = 32.0;
const COMMAND_BAR_CONTROL_HEIGHT: f32 = 24.0;
const COMMAND_BAR_MIN_TOP: f32 = 4.0;

fn command_bar_top_offset(
    top_pad: f32,
    viewport_line: usize,
    line_height: f32,
    bar_height: f32,
) -> f32 {
    (top_pad + viewport_line as f32 * line_height + (line_height - bar_height) * 0.5)
        .round()
        .max(COMMAND_BAR_MIN_TOP)
}

fn truncate_command_preview(command: &str, max_chars: usize) -> String {
    let single_line = command.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = single_line.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}

fn marker_tooltip(marker: &TerminalCommandMarker) -> String {
    let status_text = match marker.status {
        CommandBlockStatus::Finished(0) => i18n::string("session.command_blocks.exit_ok"),
        CommandBlockStatus::Finished(code) => {
            let code = code.to_string();
            i18n::string_args("session.command_blocks.exit_failed", &[("code", &code)])
        }
        CommandBlockStatus::Running => i18n::string("session.command_blocks.running"),
        CommandBlockStatus::FinishedUnknown => i18n::string("session.command_blocks.finished"),
    };
    let preview = truncate_command_preview(&marker.command, MAX_TOOLTIP_COMMAND_CHARS);
    if preview.is_empty() {
        status_text
    } else {
        format!("{status_text} · {preview}")
    }
}

fn hovered_row_for_pane(app: &AppView, pane_id: PaneId, line_height: f32) -> Option<usize> {
    let pane = app.terminal_pane_presentation(pane_id)?;
    let bounds = pane.bounds?;
    let pos = pane.pointer_position?;
    let min_x = f32::from(bounds.origin.x) - 24.0;
    let max_x = f32::from(bounds.origin.x) + f32::from(bounds.size.width) + 24.0;
    let min_y = f32::from(bounds.origin.y);
    let max_y = min_y + f32::from(bounds.size.height);
    let px_x = f32::from(pos.x);
    let px_y = f32::from(pos.y);
    if px_x < min_x || px_x > max_x || px_y < min_y || px_y > max_y {
        return None;
    }
    Some(((px_y - min_y) / line_height.max(1.0)).floor() as usize)
}

/// Computes the left padding inside `terminal_surface` so the status dot sits
/// visually centered in the seamless gutter between the card edge and column 0.
fn command_dot_left_offset(
    left_pad: f32,
    dot_size: f32,
    embedded_in_split: bool,
    rem_size: f32,
) -> f32 {
    let outer_left_pad = if embedded_in_split {
        0.0
    } else {
        rem_size * 0.5
    };
    let visual_center_x = (left_pad - outer_left_pad) * 0.5 + 0.5;
    let max_left = (left_pad - dot_size).max(0.0);
    let min_left = 0.5_f32.min(max_left);
    (visual_center_x - dot_size * 0.5).clamp(min_left, max_left)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::ui::shell::layout) fn render_terminal_command_overlay(
    app: &AppView,
    pane_id: PaneId,
    tab_id: TabId,
    interactive: bool,
    embedded_in_split: bool,
    allow_agent: bool,
    focused: bool,
    cursor_visible: bool,
    line_height: f32,
    window: &Window,
    cx: &mut Context<AppView>,
) -> Option<gpui_kit::AnyElement> {
    let snapshot = app
        .controllers
        .session
        .read(cx)
        .terminal_snapshot_with_cursor(tab_id, focused, cursor_visible)?;
    if snapshot.command_markers.is_empty() {
        return None;
    }

    let theme = miaominal_settings::current_theme();
    let roles = theme.material.roles;
    let palette = theme.terminal;
    let rem_size = f32::from(window.rem_size());
    let left_pad = rem_size;
    let top_pad = if embedded_in_split { 0.0 } else { rem_size };

    let pinned_id = if interactive {
        app.workspace
            .workspace
            .active_pane
            .terminal_pinned_command_id
            .and_then(|(pinned_tab, id)| (pinned_tab == tab_id).then_some(id))
    } else {
        None
    };

    let has_active_gesture = interactive
        && app
            .workspace
            .workspace
            .active_pane
            .terminal_mouse_gesture
            .is_some();

    let hovered_bar_id = if interactive && !has_active_gesture {
        app.workspace
            .workspace
            .active_pane
            .terminal_hovered_command_bar
            .and_then(|(hovered_tab, id)| (hovered_tab == tab_id).then_some(id))
    } else {
        None
    };

    let hovered_id = if interactive && !has_active_gesture {
        hovered_row_for_pane(app, pane_id, line_height).and_then(|row| {
            snapshot
                .command_markers
                .iter()
                .rfind(|marker| {
                    (marker.visible_start_line..=marker.visible_end_line).contains(&row)
                })
                .map(|marker| marker.id)
        })
    } else {
        None
    };

    let action_marker = if interactive {
        pinned_id
            .and_then(|id| snapshot.command_markers.iter().find(|m| m.id == id))
            .or_else(|| {
                hovered_bar_id.and_then(|id| snapshot.command_markers.iter().find(|m| m.id == id))
            })
            .or_else(|| {
                snapshot
                    .focused_command_id
                    .and_then(|id| snapshot.command_markers.iter().find(|m| m.id == id))
            })
            .or_else(|| {
                hovered_id.and_then(|id| snapshot.command_markers.iter().find(|m| m.id == id))
            })
    } else {
        None
    };

    let app_entity = app.controllers.session.clone();

    let mut overlay = div().absolute().top_0().right_0().bottom_0().left_0();

    for marker in &snapshot.command_markers {
        let is_highlighted =
            pinned_id == Some(marker.id) || snapshot.focused_command_id == Some(marker.id);
        let dot_size = match marker.status {
            CommandBlockStatus::FinishedUnknown => {
                if is_highlighted {
                    7.0
                } else {
                    6.0
                }
            }
            _ => {
                if is_highlighted {
                    8.0
                } else {
                    7.0
                }
            }
        };

        let dot_inner = match marker.status {
            CommandBlockStatus::Finished(0) => div()
                .size(px(dot_size))
                .rounded_full()
                .bg(rgb(palette.ansi[2])),
            CommandBlockStatus::Finished(_) => div()
                .size(px(dot_size))
                .rounded_full()
                .bg(rgb(palette.ansi[1])),
            CommandBlockStatus::Running => div()
                .size(px(dot_size))
                .rounded_full()
                .border(px(1.5))
                .border_color(rgb(roles.primary)),
            CommandBlockStatus::FinishedUnknown => div()
                .size(px(dot_size))
                .rounded_full()
                .bg(color_with_alpha(roles.on_surface_variant, 115)),
        }
        .flex_shrink_0();

        let dot_left = command_dot_left_offset(left_pad, dot_size, embedded_in_split, rem_size);
        let row_top = (top_pad + marker.viewport_line as f32 * line_height).round();
        let dot_container = div()
            .id(SharedString::from(format!(
                "terminal-cmd-dot-{}-{}",
                pane_id.raw(),
                marker.id
            )))
            .absolute()
            .left_0()
            .top(px(row_top))
            .w(px(left_pad))
            .h(px(line_height))
            .flex()
            .items_center()
            .pl(px(dot_left))
            .child(dot_inner);

        let dot_container = if interactive {
            let marker_id = marker.id;
            let tooltip: SharedString = marker_tooltip(marker).into();
            dot_container
                .cursor_pointer()
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        let next = if this
                            .workspace
                            .workspace
                            .active_pane
                            .terminal_pinned_command_id
                            == Some((tab_id, marker_id))
                        {
                            None
                        } else {
                            Some((tab_id, marker_id))
                        };
                        this.workspace
                            .workspace
                            .active_pane
                            .terminal_pinned_command_id = next;
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .on_mouse_up(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
        } else {
            dot_container
        };

        overlay = overlay.child(dot_container);
    }

    if let Some(marker) = action_marker {
        let extended = theme.material.extended;
        let marker_id = marker.id;
        let is_pinned = pinned_id == Some(marker_id);
        let bar_height = f32::from(space(COMMAND_BAR_HEIGHT).to_pixels(window.rem_size()));
        let bar_top =
            command_bar_top_offset(top_pad, marker.viewport_line, line_height, bar_height);
        let copy_source = app_entity.clone();
        let diagnose_source = app_entity;

        let mut bar = h_flex()
            .id(SharedString::from(format!(
                "terminal-cmd-bar-{}-{}",
                pane_id.raw(),
                marker_id
            )))
            .absolute()
            .right(space(16.0))
            .top(px(bar_top))
            .h(space(COMMAND_BAR_HEIGHT))
            .px(space(4.0))
            .gap(space(4.0))
            .items_center()
            .rounded_full()
            .bg(rgb(roles.surface_container_high))
            .border_1()
            .border_color(if is_pinned {
                color_with_alpha(roles.primary, 150)
            } else {
                color_with_alpha(roles.outline_variant, 170)
            })
            .shadow_md()
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                let target = Some((tab_id, marker_id));
                let pane = &mut this.workspace.workspace.active_pane;
                if *hovered {
                    if pane.terminal_hovered_command_bar != target {
                        pane.terminal_hovered_command_bar = target;
                        cx.notify();
                    }
                } else if pane.terminal_hovered_command_bar == target {
                    pane.terminal_hovered_command_bar = None;
                    cx.notify();
                }
            }))
            .on_mouse_move(|_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_mouse_up(MouseButton::Right, |_, _, cx| cx.stop_propagation());

        let status_chip = |bg: u32, fg: u32, leading: gpui_kit::AnyElement, label: SharedString| {
            h_flex()
                .h(space(COMMAND_BAR_CONTROL_HEIGHT))
                .px(space(8.0))
                .gap(space(4.0))
                .items_center()
                .rounded_full()
                .flex_shrink_0()
                .bg(rgb(bg))
                .text_color(rgb(fg))
                .font_weight(FontWeight::MEDIUM)
                .text_size(miaominal_settings::FontSize::Body.scaled())
                .child(leading)
                .child(label)
        };

        let status_pill = match marker.status {
            CommandBlockStatus::Finished(0) => Some(status_chip(
                extended.success.color_container,
                extended.success.on_color_container,
                Icon::new(AppIcon::Check)
                    .size(space(12.0))
                    .into_any_element(),
                SharedString::from("0"),
            )),
            CommandBlockStatus::Finished(code) => Some(status_chip(
                roles.error_container,
                roles.on_error_container,
                Icon::new(AppIcon::Close)
                    .size(space(12.0))
                    .into_any_element(),
                SharedString::from(format!("exit {code}")),
            )),
            CommandBlockStatus::Running => Some(status_chip(
                roles.secondary_container,
                roles.on_secondary_container,
                div()
                    .size(space(6.0))
                    .rounded_full()
                    .bg(rgb(roles.primary))
                    .flex_shrink_0()
                    .into_any_element(),
                i18n::string("session.command_blocks.running").into(),
            )),
            CommandBlockStatus::FinishedUnknown => None,
        };

        if let Some(status_pill) = status_pill {
            bar = bar.child(status_pill).child(
                div()
                    .w(px(1.0))
                    .h(space(14.0))
                    .mx(space(2.0))
                    .bg(color_with_alpha(roles.outline_variant, 160))
                    .flex_shrink_0(),
            );
        }

        let bar_surface = Hsla::from(rgb(roles.surface_container_high));
        let on_surface = Hsla::from(rgb(roles.on_surface));
        let on_surface_variant = Hsla::from(rgb(roles.on_surface_variant));
        let copy_label: SharedString = i18n::string("session.command_blocks.copy_output").into();

        bar = bar.child(
            Button::new(SharedString::from(format!("terminal-cmd-copy-{marker_id}")))
                .size(space(COMMAND_BAR_CONTROL_HEIGHT))
                .p_0()
                .rounded_full()
                .tab_stop(false)
                .accessibility_label(copy_label.clone())
                .tooltip(copy_label)
                .disabled(!marker.has_output)
                .custom(
                    ButtonCustomVariant::new(cx)
                        .color(bar_surface.opacity(0.0))
                        .foreground(if marker.has_output {
                            on_surface_variant
                        } else {
                            on_surface_variant.opacity(0.38)
                        })
                        .hover(if marker.has_output {
                            bar_surface.blend(on_surface.opacity(0.08))
                        } else {
                            bar_surface.opacity(0.0)
                        })
                        .active(if marker.has_output {
                            bar_surface.blend(on_surface.opacity(0.12))
                        } else {
                            bar_surface.opacity(0.0)
                        }),
                )
                .child(Icon::new(AppIcon::Copy).size(space(13.0)))
                .on_click(move |_, _, cx| {
                    copy_source.update(cx, |controller, cx| {
                        controller.emit(
                            AppCommand::TerminalMenuRequested {
                                pane_id: Some(pane_id),
                                command: TerminalMenuCommand::CopyCommandOutput(marker_id),
                            },
                            cx,
                        );
                    });
                }),
        );

        if matches!(marker.status, CommandBlockStatus::Finished(code) if code != 0) && allow_agent {
            let primary_container = Hsla::from(rgb(roles.primary_container));
            let on_primary_container = Hsla::from(rgb(roles.on_primary_container));
            let diagnose_label: SharedString =
                i18n::string("session.command_blocks.diagnose_ai").into();
            bar = bar.child(
                Button::new(SharedString::from(format!(
                    "terminal-cmd-diagnose-{marker_id}"
                )))
                .h(space(COMMAND_BAR_CONTROL_HEIGHT))
                .pl(space(8.0))
                .pr(space(10.0))
                .py_0()
                .rounded_full()
                .tab_stop(false)
                .accessibility_label(diagnose_label.clone())
                .font_weight(FontWeight::MEDIUM)
                .text_size(miaominal_settings::FontSize::Body.scaled())
                .custom(
                    ButtonCustomVariant::new(cx)
                        .color(primary_container)
                        .foreground(on_primary_container)
                        .hover(primary_container.blend(on_primary_container.opacity(0.08)))
                        .active(primary_container.blend(on_primary_container.opacity(0.12))),
                )
                .bg(primary_container)
                .text_color(on_primary_container)
                .child(
                    h_flex()
                        .items_center()
                        .gap(space(4.0))
                        .text_size(miaominal_settings::FontSize::Body.scaled())
                        .child(Icon::new(AppIcon::Sparkles).size(space(12.0)))
                        .child(diagnose_label),
                )
                .on_click(move |_, _, cx| {
                    diagnose_source.update(cx, |controller, cx| {
                        controller.emit(
                            AppCommand::TerminalMenuRequested {
                                pane_id: Some(pane_id),
                                command: TerminalMenuCommand::DiagnoseCommandWithAi(marker_id),
                            },
                            cx,
                        );
                    });
                }),
            );
        }

        overlay = overlay.child(bar);
    }

    Some(overlay.into_any_element())
}

pub(in crate::ui::shell) fn format_command_diagnostic_prompt(
    block: &TerminalCommandBlockContent,
) -> String {
    let code = match block.status {
        CommandBlockStatus::Finished(code) => code.to_string(),
        _ => "non-zero".to_string(),
    };
    let command = if block.command.trim().is_empty() {
        "<unknown>".to_string()
    } else {
        block.command.trim().to_string()
    };
    let output = truncate_output_tail(&block.output, MAX_DIAGNOSTIC_OUTPUT_CHARS);

    if let Some(cwd) = block
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let output_text = if output.is_empty() {
            i18n::string("session.command_blocks.no_output")
        } else {
            output
        };
        return i18n::string_args(
            "session.command_blocks.diagnose_prompt_with_cwd",
            &[
                ("code", &code),
                ("cwd", cwd),
                ("command", &command),
                ("output", &output_text),
            ],
        );
    }

    if output.is_empty() {
        i18n::string_args(
            "session.command_blocks.diagnose_prompt_no_output",
            &[("code", &code), ("command", &command)],
        )
    } else {
        i18n::string_args(
            "session.command_blocks.diagnose_prompt",
            &[("code", &code), ("command", &command), ("output", &output)],
        )
    }
}

fn truncate_output_tail(output: &str, max_chars: usize) -> String {
    let trimmed = output.trim();
    let total_chars = trimmed.chars().count();
    if total_chars <= max_chars {
        return trimmed.to_string();
    }
    let skip_chars = total_chars.saturating_sub(max_chars);
    let byte_start = trimmed
        .char_indices()
        .nth(skip_chars)
        .map(|(index, _)| index)
        .unwrap_or(0);
    let tail = &trimmed[byte_start..];
    let aligned = tail
        .find('\n')
        .map(|newline| &tail[newline + 1..])
        .filter(|rest| !rest.is_empty())
        .unwrap_or(tail);
    format!("... (truncated)\n{aligned}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_prompt_includes_command_exit_code_cwd_and_tail_truncates_output() {
        let mut long_output = String::new();
        for i in 0..500 {
            long_output.push_str(&format!("line-{i:04}: something happened during build\n"));
        }
        let block = TerminalCommandBlockContent {
            id: 7,
            command: "cargo build --release".to_string(),
            output: long_output,
            status: CommandBlockStatus::Finished(101),
            cwd: Some("/srv/app".to_string()),
        };

        let prompt = format_command_diagnostic_prompt(&block);
        assert!(prompt.contains("101"));
        assert!(prompt.contains("/srv/app"));
        assert!(prompt.contains("cargo build --release"));
        assert!(prompt.contains("... (truncated)"));
        assert!(prompt.contains("line-0499"));
        assert!(!prompt.contains("line-0000"));
    }

    #[test]
    fn diagnostic_prompt_handles_empty_output_without_cwd() {
        let block = TerminalCommandBlockContent {
            id: 8,
            command: "false".to_string(),
            output: String::new(),
            status: CommandBlockStatus::Finished(1),
            cwd: None,
        };

        let prompt = format_command_diagnostic_prompt(&block);
        assert!(prompt.contains("1"));
        assert!(prompt.contains("false"));
    }

    #[test]
    fn command_dot_left_offset_centers_in_single_and_split_gutters() {
        let single_default = command_dot_left_offset(14.0, 7.0, false, 14.0);
        assert!((single_default - 0.5).abs() < 1e-5);

        let single_highlighted = command_dot_left_offset(14.0, 8.0, false, 14.0);
        assert!(single_highlighted >= 0.5);
        assert!(single_highlighted <= 14.0 - 8.0);

        let single_large_rem = command_dot_left_offset(20.0, 7.0, false, 20.0);
        assert!((single_large_rem - 2.0).abs() < 1e-5);
        let left_margin = 20.0 * 0.5 + single_large_rem;
        let right_margin = 20.0 - (single_large_rem + 7.0);
        assert!((left_margin - right_margin - 1.0).abs() < 1e-5);

        let split_default = command_dot_left_offset(14.0, 7.0, true, 14.0);
        assert!((split_default - 4.0).abs() < 1e-5);
        assert!(split_default > single_default);
    }

    #[test]
    fn command_bar_top_offset_centers_on_row_and_clamps_top_edge() {
        // Single-pane row 0 with top_pad=14, line_height=18, bar_height=32 => 14 + (18 - 32)/2 = 7
        assert_eq!(command_bar_top_offset(14.0, 0, 18.0, 32.0), 7.0);

        // Split-pane row 0 with top_pad=0 => -7, clamped to COMMAND_BAR_MIN_TOP (4.0)
        assert_eq!(
            command_bar_top_offset(0.0, 0, 18.0, 32.0),
            COMMAND_BAR_MIN_TOP
        );

        // Subsequent rows center vertically on the prompt row
        assert_eq!(command_bar_top_offset(0.0, 2, 18.0, 32.0), 29.0);
    }
}
