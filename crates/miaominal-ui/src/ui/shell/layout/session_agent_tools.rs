use super::super::*;
use super::session_agent_tool_parse::*;
use super::session_agent_tool_ui::*;
use crate::ui::i18n;

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
        "ask_user" => render_ask_user_tool_body(tool_call, colors),
        "approval" => render_approval_tool_body(tool_call, colors),
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
                tool_placeholder("preparing_command")
            } else {
                tool_placeholder("no_command")
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
                i18n::string_args(
                    "workspace.panel.agent.tool_result.result_with_exit",
                    &[("status", &result.exit_status.to_string())],
                ),
                result.display_text(),
                result.exit_status != 0,
            )
        })
        .or_else(|| {
            tool_display_result(tool_call).map(|result| (tool_field_label("result"), result, false))
        });

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_bash_highlighted_command_block(
            &tool_call.id,
            tool_field_label("command"),
            &command,
            colors,
            syntax_theme,
        ))
        .when_some(result_block, |this, (label, content, error)| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                label,
                None,
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
        .unwrap_or_else(|| tool_placeholder("preparing_patch"));
    let patch_ready = !patch.trim().is_empty() && patch != tool_placeholder("preparing_patch");
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
                    tool_field_label("files"),
                    if files.is_empty() {
                        if patch_ready {
                            tool_placeholder("no_files_detected")
                        } else {
                            tool_placeholder("detecting_files")
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
            tool_field_label("diff"),
            Some("diff"),
            patch,
            colors,
            false,
        ))
        .when_some(
            summary.filter(|summary| !summary.trim().is_empty()),
            |this, summary| {
                this.child(render_tool_terminal_block(
                    &tool_call.id,
                    tool_field_label("patch_output"),
                    Some("diff"),
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
    let path =
        string_field(args.as_ref(), "path").unwrap_or_else(|| tool_placeholder("unknown_path"));
    let range = match (
        number_field(args.as_ref(), "start_line"),
        number_field(args.as_ref(), "end_line"),
    ) {
        (Some(start), Some(end)) => format!("{start}-{end}"),
        (Some(start), None) => format!("{start}+"),
        _ => tool_placeholder("default_range"),
    };
    let content = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "content"));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![
                (tool_field_label("path"), path),
                (tool_field_label("lines"), range),
            ],
            colors,
        ))
        .when_some(content, |this, content| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                tool_field_label("content"),
                None,
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
            vec![(tool_field_label("path"), path)],
            colors,
        ))
        .when_some(entries, |this, entries| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                tool_field_label("entries"),
                None,
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
    let pattern =
        string_field(args.as_ref(), "pattern").unwrap_or_else(|| tool_placeholder("pattern"));
    let entries = list_entries_text(output.as_ref());

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![
                (tool_field_label("root"), root),
                (tool_field_label("pattern"), pattern),
            ],
            colors,
        ))
        .when_some(entries, |this, entries| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                tool_field_label("results"),
                None,
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
    let pattern =
        string_field(args.as_ref(), "pattern").unwrap_or_else(|| tool_placeholder("pattern"));
    let content = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "content"));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![
                (tool_field_label("root"), root),
                (tool_field_label("pattern"), pattern),
            ],
            colors,
        ))
        .when_some(content, |this, content| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                tool_field_label("results"),
                None,
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
    let command =
        string_field(args.as_ref(), "command").unwrap_or_else(|| tool_placeholder("no_command"));
    let cwd = string_field(args.as_ref(), "cwd").unwrap_or_else(|| ".".to_string());
    let mut fields = vec![(tool_field_label("cwd"), cwd)];
    if let Some(job_id) = output
        .as_ref()
        .and_then(|value| value.get("job_id"))
        .map(display_json_value)
    {
        fields.push((tool_field_label("job"), job_id));
    }

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(fields, colors))
        .child(render_tool_terminal_block(
            &tool_call.id,
            tool_field_label("command"),
            Some("bash"),
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
        .unwrap_or_else(|| tool_placeholder("job"));
    let content = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "content"))
        .or_else(|| tool_display_result(tool_call));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![(tool_field_label("job"), job_id)],
            colors,
        ))
        .when_some(content, |this, content| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                tool_field_label("result"),
                None,
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
                tool_field_label("jobs"),
                None,
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
        .unwrap_or_else(|| tool_placeholder("job"));
    let mut fields = vec![(tool_field_label("job"), job_id)];
    if let Some(status) = result.and_then(|value| string_field(Some(value), "status")) {
        fields.push((tool_field_label("status"), status));
    }
    if let Some(exit_status) = result
        .and_then(|value| value.get("exit_status"))
        .filter(|value| !value.is_null())
        .map(display_json_value)
    {
        fields.push((tool_field_label("exit"), exit_status));
    }
    let stdout = result.and_then(|value| string_field(Some(value), "stdout"));
    let stderr = result.and_then(|value| string_field(Some(value), "stderr"));
    let output_truncated = poll_job_output_truncated(result);

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
                    tool_field_label("stdout"),
                    None,
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
                    tool_field_label("stderr"),
                    None,
                    stderr,
                    colors,
                    true,
                ))
            },
        )
        .when(output_truncated, |this| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                tool_field_label("result"),
                None,
                i18n::string("workspace.panel.agent.tool_result.output_truncated"),
                colors,
                false,
            ))
        })
        .when(result.is_none(), |this| {
            this.when_some(tool_display_result(tool_call), |this, content| {
                this.child(render_tool_terminal_block(
                    &tool_call.id,
                    tool_field_label("result"),
                    None,
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
    let query = string_field(args.as_ref(), "query").unwrap_or_else(|| tool_placeholder("query"));
    let results = output
        .as_ref()
        .and_then(|value| value.get("results"))
        .map(display_json_value);

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![(tool_field_label("query"), query)],
            colors,
        ))
        .when_some(results, |this, results| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                tool_field_label("results"),
                None,
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
        .unwrap_or_else(|| tool_placeholder("url"));
    let content = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "content"));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_field_grid(
            vec![(tool_field_label("url"), url)],
            colors,
        ))
        .when_some(content, |this, content| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                tool_field_label("content"),
                None,
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
                    tool_field_label("host"),
                    string_field(Some(value), "host").unwrap_or_default(),
                ),
                (
                    tool_field_label("user"),
                    string_field(Some(value), "user").unwrap_or_default(),
                ),
                (
                    tool_field_label("cwd"),
                    string_field(Some(value), "cwd").unwrap_or_default(),
                ),
                (
                    tool_field_label("shell"),
                    string_field(Some(value), "shell").unwrap_or_default(),
                ),
            ]
        })
        .unwrap_or_else(|| vec![(tool_field_label("status"), pending_or_note(tool_call))]);

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
            tool_field_label("approval"),
            None,
            message,
            colors,
            false,
        ))
        .into_any_element()
}

pub(in crate::ui::shell::layout) fn render_ask_user_tool_body(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
    colors: ToolTerminalColors,
) -> gpui::AnyElement {
    let prompt = parse_ask_user_prompt(tool_call);
    let output = tool_output_value(tool_call);
    let answer = output
        .as_ref()
        .and_then(|value| string_field(Some(value), "answer"));

    v_flex()
        .w_full()
        .gap_2()
        .p_2()
        .child(render_tool_terminal_block(
            &tool_call.id,
            tool_field_label("question"),
            None,
            prompt.message,
            colors,
            false,
        ))
        .when_some(answer, |this, answer| {
            this.child(render_tool_terminal_block(
                &tool_call.id,
                tool_field_label("answer"),
                None,
                answer,
                colors,
                false,
            ))
        })
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
                tool_field_label("result"),
                None,
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
            tool_field_label("request"),
            None,
            preparing_tool_text(&tool_call.name),
            colors,
            false,
        ))
        .into_any_element()
}

fn tool_field_label(key: &str) -> String {
    match key {
        "answer" => i18n::string("workspace.panel.agent.tool_fields.answer"),
        "approval" => i18n::string("workspace.panel.agent.tool_fields.approval"),
        "base" => i18n::string("workspace.panel.agent.tool_fields.base"),
        "command" => i18n::string("workspace.panel.agent.tool_fields.command"),
        "content" => i18n::string("workspace.panel.agent.tool_fields.content"),
        "cwd" => i18n::string("workspace.panel.agent.tool_fields.cwd"),
        "diff" => i18n::string("workspace.panel.agent.tool_fields.diff"),
        "entries" => i18n::string("workspace.panel.agent.tool_fields.entries"),
        "exit" => i18n::string("workspace.panel.agent.tool_fields.exit"),
        "files" => i18n::string("workspace.panel.agent.tool_fields.files"),
        "host" => i18n::string("workspace.panel.agent.tool_fields.host"),
        "job" => i18n::string("workspace.panel.agent.tool_fields.job"),
        "jobs" => i18n::string("workspace.panel.agent.tool_fields.jobs"),
        "lines" => i18n::string("workspace.panel.agent.tool_fields.lines"),
        "patch_output" => i18n::string("workspace.panel.agent.tool_fields.patch_output"),
        "path" => i18n::string("workspace.panel.agent.tool_fields.path"),
        "pattern" => i18n::string("workspace.panel.agent.tool_fields.pattern"),
        "query" => i18n::string("workspace.panel.agent.tool_fields.query"),
        "question" => i18n::string("workspace.panel.agent.tool_fields.question"),
        "request" => i18n::string("workspace.panel.agent.tool_fields.request"),
        "result" => i18n::string("workspace.panel.agent.tool_fields.result"),
        "results" => i18n::string("workspace.panel.agent.tool_fields.results"),
        "root" => i18n::string("workspace.panel.agent.tool_fields.root"),
        "shell" => i18n::string("workspace.panel.agent.tool_fields.shell"),
        "status" => i18n::string("workspace.panel.agent.tool_fields.status"),
        "stderr" => i18n::string("workspace.panel.agent.tool_fields.stderr"),
        "stdout" => i18n::string("workspace.panel.agent.tool_fields.stdout"),
        "url" => i18n::string("workspace.panel.agent.tool_fields.url"),
        "user" => i18n::string("workspace.panel.agent.tool_fields.user"),
        _ => key.to_string(),
    }
}

fn tool_placeholder(key: &str) -> String {
    match key {
        "default_range" => i18n::string("workspace.panel.agent.tool_placeholders.default_range"),
        "detecting_files" => {
            i18n::string("workspace.panel.agent.tool_placeholders.detecting_files")
        }
        "job" => i18n::string("workspace.panel.agent.tool_placeholders.job"),
        "no_command" => i18n::string("workspace.panel.agent.tool_placeholders.no_command"),
        "no_files_detected" => {
            i18n::string("workspace.panel.agent.tool_placeholders.no_files_detected")
        }
        "pattern" => i18n::string("workspace.panel.agent.tool_placeholders.pattern"),
        "preparing_command" => {
            i18n::string("workspace.panel.agent.tool_placeholders.preparing_command")
        }
        "preparing_patch" => {
            i18n::string("workspace.panel.agent.tool_placeholders.preparing_patch")
        }
        "query" => i18n::string("workspace.panel.agent.tool_placeholders.query"),
        "custom_answer" => i18n::string("workspace.panel.agent.tool_placeholders.custom_answer"),
        "unknown_path" => i18n::string("workspace.panel.agent.tool_placeholders.unknown_path"),
        "url" => i18n::string("workspace.panel.agent.tool_placeholders.url"),
        _ => key.to_string(),
    }
}
