use super::super::*;
use super::session_agent_tool_ui::*;
use super::session_agent_tool_parse::*;

pub(in crate::ui::shell::layout) fn render_structured_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    if arguments_are_streaming(tool_call) && tool_call.name != "apply_patch" {
        return render_preparing_tool_body(tool_call, colors);
    }

    match tool_call.name.as_str() {
        "apply_patch" => render_apply_patch_tool_body(tool_call, colors),
        "read" => render_read_tool_body(tool_call, colors),
        "list" => render_list_tool_body(tool_call, colors),
        "glob" => render_glob_tool_body(tool_call, colors),
        "grep" => render_grep_tool_body(tool_call, colors),
        "start_job" => render_start_job_tool_body(tool_call, colors),
        "list_jobs" => render_list_jobs_tool_body(tool_call, colors),
        "poll_job" => render_poll_job_tool_body(tool_call, colors),
        "stop_job" => render_job_tool_body(tool_call, colors),
        "web_search" => render_web_search_tool_body(tool_call, colors),
        "web_fetch" => render_web_fetch_tool_body(tool_call, colors),
        "workspace_info" => render_workspace_info_tool_body(tool_call, colors),
        "ask_user" | "approval" => render_approval_tool_body(tool_call, colors),
        _ => render_generic_tool_body(tool_call, colors),
    }
}

pub(in crate::ui::shell::layout) fn render_run_shell_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
    syntax_theme: &::theme::SyntaxTheme,
) -> gpui::AnyElement {
    let command = parse_run_shell_command(&tool_call.arguments)
        .or_else(|| partial_json_string_field(&tool_call.arguments, "command"))
        .unwrap_or_else(|| {
            if arguments_are_streaming(tool_call) {
                "Preparing command...".to_string()
            } else {
                "No command".to_string()
            }
        });
    let result = if tool_has_result_status(tool_call) {
        tool_call
            .confirmation_note
            .as_deref()
            .and_then(parse_run_shell_result)
    } else {
        None
    };
    let result_block = result
        .map(|result| {
            (
                format!("Result - exit {}", result.exit_status),
                result.display_text(),
                result.exit_status != 0,
            )
        })
        .or_else(|| {
            tool_display_result(tool_call).map(|result| ("Result".to_string(), result, false))
        });

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_bash_highlighted_command_block(
            &tool_call.id,
            "Command",
            &command,
            colors,
            syntax_theme,
        ))
        .when_some(result_block, |this, (label, content, error)| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                &label,
                content,
                colors,
                error,
            ))
        })
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_apply_patch_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let patch = string_field(args.as_ref(), "patch")
        .or_else(|| partial_json_string_field(&tool_call.arguments, "patch"))
        .unwrap_or_else(|| "Preparing patch...".to_string());
    let patch_ready = !patch.trim().is_empty() && patch != "Preparing patch...";
    let base_dir = string_field(args.as_ref(), "base_dir")
        .or_else(|| partial_json_string_field(&tool_call.arguments, "base_dir"))
        .unwrap_or_else(|| ".".to_string());
    let files = patch_paths(&patch);
    let summary = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "summary"))
        .or_else(|| tool_display_result(tool_call));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![
                ("Base".to_string(), base_dir),
                (
                    "Files".to_string(),
                    if files.is_empty() {
                        if patch_ready {
                            "No files detected".to_string()
                        } else {
                            "Detecting files...".to_string()
                        }
                    } else {
                        files.join(", ")
                    },
                ),
            ],
            colors,
        ))
        .child(render_tool_terminal_block(
            &tool_call.id,
            "Diff",
            patch,
            colors,
            false,
        ))
        .when_some(
            summary.filter(|summary| !summary.trim().is_empty()),
            |this, summary| {
                this.child(render_tool_terminal_block(
                    &tool_call.id,
                    "Patch Output",
                    summary,
                    colors,
                    false,
                ))
            },
        )
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_read_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let path = string_field(args.as_ref(), "path").unwrap_or_else(|| "(unknown path)".to_string());
    let range = match (
        number_field(args.as_ref(), "start_line"),
        number_field(args.as_ref(), "end_line"),
    ) {
        (Some(start), Some(end)) => format!("{start}-{end}"),
        (Some(start), None) => format!("{start}+"),
        _ => "default".to_string(),
    };
    let content = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "content"));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![("Path".to_string(), path), ("Lines".to_string(), range)],
            colors,
        ))
        .when_some(content, |this, content| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                "Content",
                content,
                colors,
                false,
            ))
        })
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_list_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let path = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "path"))
        .or_else(|| string_field(args.as_ref(), "path"))
        .unwrap_or_else(|| ".".to_string());
    let entries = output
        .as_ref()
        .and_then(|value| value.get("entries"))
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .take(80)
                .filter_map(|entry| {
                    let name = entry.get("name")?.as_str()?;
                    let kind = entry
                        .get("entry_type")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("item");
                    Some(format!("{kind:>9}  {name}"))
                })
                .collect::<Vec<_>>()
                .join("\n")
        });

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![("Path".to_string(), path)],
            colors,
        ))
        .when_some(entries, |this, entries| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                "Entries",
                entries,
                colors,
                false,
            ))
        })
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_glob_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let root = string_field(args.as_ref(), "root").unwrap_or_else(|| ".".to_string());
    let pattern = string_field(args.as_ref(), "pattern").unwrap_or_else(|| "(pattern)".to_string());
    let entries = list_entries_text(output.as_ref());

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![("Root".to_string(), root), ("Pattern".to_string(), pattern)],
            colors,
        ))
        .when_some(entries, |this, entries| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                "Matches",
                entries,
                colors,
                false,
            ))
        })
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_grep_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let root = string_field(args.as_ref(), "root").unwrap_or_else(|| ".".to_string());
    let pattern = string_field(args.as_ref(), "pattern").unwrap_or_else(|| "(pattern)".to_string());
    let content = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "content"));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![("Root".to_string(), root), ("Pattern".to_string(), pattern)],
            colors,
        ))
        .when_some(content, |this, content| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                "Matches",
                content,
                colors,
                false,
            ))
        })
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_start_job_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let command = string_field(args.as_ref(), "command").unwrap_or_else(|| "(command)".to_string());
    let cwd = string_field(args.as_ref(), "cwd").unwrap_or_else(|| ".".to_string());
    let mut fields = vec![("Cwd".to_string(), cwd)];
    if let Some(job_id) = output
        .as_ref()
        .and_then(|value| value.get("job_id"))
        .map(display_json_value)
    {
        fields.push(("Job".to_string(), job_id));
    }

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(fields, colors))
        .child(render_tool_terminal_block(
            &tool_call.id,
            "Command",
            command,
            colors,
            false,
        ))
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_job_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let job_id = args
        .as_ref()
        .and_then(|value| value.get("job_id"))
        .map(display_json_value)
        .unwrap_or_else(|| "(job)".to_string());
    let content = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "content"))
        .or_else(|| tool_display_result(tool_call));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![("Job".to_string(), job_id)],
            colors,
        ))
        .when_some(content, |this, content| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                "Result",
                content,
                colors,
                false,
            ))
        })
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_list_jobs_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let output = tool_output_value(tool_call);
    let content = output
        .as_ref()
        .and_then(|value| value.get("jobs"))
        .map(display_json_value)
        .or_else(|| tool_display_result(tool_call));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .when_some(content, |this, content| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                "Jobs",
                content,
                colors,
                false,
            ))
        })
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_poll_job_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let result = output.as_ref().and_then(|value| value.get("result"));
    let job_id = args
        .as_ref()
        .and_then(|value| value.get("job_id"))
        .or_else(|| result.and_then(|value| value.get("job_id")))
        .map(display_json_value)
        .unwrap_or_else(|| "(job)".to_string());
    let mut fields = vec![("Job".to_string(), job_id)];
    if let Some(status) = result.and_then(|value| string_field(Some(value), "status")) {
        fields.push(("Status".to_string(), status));
    }
    if let Some(exit_status) = result
        .and_then(|value| value.get("exit_status"))
        .filter(|value| !value.is_null())
        .map(display_json_value)
    {
        fields.push(("Exit".to_string(), exit_status));
    }
    let stdout = result.and_then(|value| string_field(Some(value), "stdout"));
    let stderr = result.and_then(|value| string_field(Some(value), "stderr"));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(fields, colors))
        .when_some(
            stdout.filter(|text| !text.trim().is_empty()),
            |this, stdout| {
                this.child(render_tool_terminal_block(
                    &tool_call.id,
                    "Stdout",
                    stdout,
                    colors,
                    false,
                ))
            },
        )
        .when_some(
            stderr.filter(|text| !text.trim().is_empty()),
            |this, stderr| {
                this.child(render_tool_terminal_block(
                    &tool_call.id,
                    "Stderr",
                    stderr,
                    colors,
                    true,
                ))
            },
        )
        .when(result.is_none(), |this| {
            this.when_some(tool_display_result(tool_call), |this, content| {
                this.child(render_tool_terminal_block(
                    &tool_call.id,
                    "Result",
                    content,
                    colors,
                    false,
                ))
            })
        })
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_web_search_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let query = string_field(args.as_ref(), "query").unwrap_or_else(|| "(query)".to_string());
    let results = output
        .as_ref()
        .and_then(|value| value.get("results"))
        .map(display_json_value);

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![("Query".to_string(), query)],
            colors,
        ))
        .when_some(results, |this, results| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                "Results",
                results,
                colors,
                false,
            ))
        })
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_web_fetch_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let url = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "url"))
        .or_else(|| string_field(args.as_ref(), "url"))
        .unwrap_or_else(|| "(url)".to_string());
    let content = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "content"));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![("Url".to_string(), url)],
            colors,
        ))
        .when_some(content, |this, content| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                "Content",
                content,
                colors,
                false,
            ))
        })
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_workspace_info_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let output = tool_output_value(tool_call);
    let fields = output
        .as_ref()
        .map(|value| {
            vec![
                (
                    "Host".to_string(),
                    string_field(Some(value), "host").unwrap_or_default(),
                ),
                (
                    "User".to_string(),
                    string_field(Some(value), "user").unwrap_or_default(),
                ),
                (
                    "Cwd".to_string(),
                    string_field(Some(value), "cwd").unwrap_or_default(),
                ),
                (
                    "Shell".to_string(),
                    string_field(Some(value), "shell").unwrap_or_default(),
                ),
            ]
        })
        .unwrap_or_else(|| vec![("Status".to_string(), pending_or_note(tool_call))]);

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(fields, colors))
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_approval_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let args = tool_arguments_value(&tool_call.arguments);
    let output = tool_output_value(tool_call);
    let message = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "message"))
        .or_else(|| string_field(args.as_ref(), "message"))
        .unwrap_or_else(|| pending_or_note(tool_call));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_terminal_block(
            &tool_call.id,
            "Approval",
            message,
            colors,
            false,
        ))
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_generic_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    if arguments_are_streaming(tool_call) {
        return render_preparing_tool_body(tool_call, colors);
    }

    let args = tool_arguments_value(&tool_call.arguments);
    let fields = args
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .map(|object| {
            object
                .iter()
                .take(8)
                .map(|(key, value)| (title_case_key(key), display_json_value(value)))
                .collect::<Vec<_>>()
        })
        .filter(|fields| !fields.is_empty());
    let result = tool_display_result(tool_call);

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .when_some(fields, |this, fields| {
            this.child(render_tool_field_grid(fields, colors))
        })
        .when_some(result, |this, result| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                "Result",
                result,
                colors,
                false,
            ))
        })
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_preparing_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_terminal_block(
            &tool_call.id,
            "Request",
            preparing_tool_text(&tool_call.name),
            colors,
            false,
        ))
        .into_any_element()
}
