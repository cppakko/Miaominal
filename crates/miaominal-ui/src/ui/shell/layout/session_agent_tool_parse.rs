use super::super::*;
use crate::ui::i18n;

#[derive(Debug, Clone)]
pub(in crate::ui::shell::layout) struct RunShellDisplayResult {
    pub(in crate::ui::shell::layout) stdout: String,
    pub(in crate::ui::shell::layout) stderr: String,
    pub(in crate::ui::shell::layout) exit_status: i64,
    pub(in crate::ui::shell::layout) timed_out: bool,
    pub(in crate::ui::shell::layout) truncated: bool,
}

impl RunShellDisplayResult {
    pub(in crate::ui::shell::layout) fn display_text(&self) -> String {
        let mut lines = Vec::new();
        if !self.stdout.trim().is_empty() {
            lines.push(self.stdout.clone());
        }
        if !self.stderr.trim().is_empty() {
            lines.push(self.stderr.clone());
        }
        if self.timed_out {
            lines.push(i18n::string(
                "workspace.panel.agent.tool_result.command_timed_out",
            ));
        }
        if self.truncated {
            lines.push(i18n::string(
                "workspace.panel.agent.tool_result.output_truncated",
            ));
        }
        lines.join("\n")
    }
}

pub(in crate::ui::shell::layout) fn parse_run_shell_command(arguments: &str) -> Option<String> {
    tool_arguments_value(arguments)?
        .get("command")?
        .as_str()
        .map(ToOwned::to_owned)
}

pub(in crate::ui::shell::layout) fn parse_run_shell_result(
    note: &str,
) -> Option<RunShellDisplayResult> {
    let value: serde_json::Value = serde_json::from_str(note).ok()?;
    let output = value.get("output")?;
    let result = output.get("result")?;
    Some(RunShellDisplayResult {
        stdout: result.get("stdout")?.as_str()?.to_string(),
        stderr: result.get("stderr")?.as_str()?.to_string(),
        exit_status: result.get("exit_status")?.as_i64()?,
        timed_out: result.get("timed_out")?.as_bool()?,
        truncated: result.get("truncated")?.as_bool()?,
    })
}

pub(in crate::ui::shell::layout) fn tool_arguments_value(
    arguments: &str,
) -> Option<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(arguments).ok()?;
    Some(value.get("arguments").unwrap_or(&value).clone())
}

pub(in crate::ui::shell::layout) fn tool_response_value(
    tool_call: &crate::ui::shell::SessionAgentToolCall,
) -> Option<serde_json::Value> {
    if !tool_has_result_status(tool_call) {
        return None;
    }

    let note = tool_call.confirmation_note.as_deref()?;
    serde_json::from_str(note).ok()
}

pub(in crate::ui::shell::layout) fn tool_output_value(
    tool_call: &crate::ui::shell::SessionAgentToolCall,
) -> Option<serde_json::Value> {
    let response = tool_response_value(tool_call)?;
    let output = response.get("output")?;
    Some(output.clone())
}

pub(in crate::ui::shell::layout) fn string_field(
    value: Option<&serde_json::Value>,
    key: &str,
) -> Option<String> {
    value?
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

pub(in crate::ui::shell::layout) fn number_field(
    value: Option<&serde_json::Value>,
    key: &str,
) -> Option<i64> {
    value?.get(key).and_then(serde_json::Value::as_i64)
}

pub(in crate::ui::shell::layout) fn poll_job_output_truncated(
    result: Option<&serde_json::Value>,
) -> bool {
    result
        .and_then(|value| value.get("truncated"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

pub(in crate::ui::shell::layout) fn list_entries_text(
    value: Option<&serde_json::Value>,
) -> Option<String> {
    let entries = value?.get("entries")?.as_array()?;
    Some(
        entries
            .iter()
            .take(100)
            .map(display_json_value)
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

pub(in crate::ui::shell::layout) fn pending_or_note(
    tool_call: &crate::ui::shell::SessionAgentToolCall,
) -> String {
    tool_display_result(tool_call).unwrap_or_else(|| pending_result_text(tool_call))
}

pub(in crate::ui::shell::layout) fn tool_has_result_status(
    tool_call: &crate::ui::shell::SessionAgentToolCall,
) -> bool {
    matches!(
        tool_call.status,
        SessionAgentToolStatus::Completed
            | SessionAgentToolStatus::Failed
            | SessionAgentToolStatus::Rejected
    )
}

pub(in crate::ui::shell::layout) fn pending_result_text(
    tool_call: &crate::ui::shell::SessionAgentToolCall,
) -> String {
    match tool_call.status {
        SessionAgentToolStatus::Pending => {
            i18n::string("workspace.panel.agent.tool_status.preparing_request")
        }
        SessionAgentToolStatus::WaitingForConfirmation => {
            if tool_call.name == "ask_user" {
                i18n::string("workspace.panel.agent.tool_status.waiting_for_answer")
            } else {
                i18n::string("workspace.panel.agent.tool_status.waiting_for_approval")
            }
        }
        SessionAgentToolStatus::InProgress => {
            i18n::string("workspace.panel.agent.tool_status.waiting_for_result")
        }
        SessionAgentToolStatus::Completed => {
            i18n::string("workspace.panel.agent.tool_status.no_output")
        }
        SessionAgentToolStatus::Failed => {
            i18n::string("workspace.panel.agent.tool_status.tool_failed")
        }
        SessionAgentToolStatus::Rejected => {
            i18n::string("workspace.panel.agent.tool_status.tool_rejected")
        }
    }
}

pub(in crate::ui::shell::layout) fn arguments_are_streaming(
    tool_call: &crate::ui::shell::SessionAgentToolCall,
) -> bool {
    matches!(
        tool_call.status,
        SessionAgentToolStatus::Pending
            | SessionAgentToolStatus::WaitingForConfirmation
            | SessionAgentToolStatus::InProgress
    ) && !tool_call.arguments.trim().is_empty()
        && tool_call.arguments.trim() != "No arguments"
        && tool_arguments_value(&tool_call.arguments).is_none()
}

pub(in crate::ui::shell::layout) fn preparing_tool_text(tool_name: &str) -> String {
    match tool_name {
        "read" => i18n::string("workspace.panel.agent.tool_status.preparing_file_read"),
        "list" => i18n::string("workspace.panel.agent.tool_status.preparing_directory_listing"),
        "glob" => i18n::string("workspace.panel.agent.tool_status.preparing_file_search"),
        "grep" => i18n::string("workspace.panel.agent.tool_status.preparing_text_search"),
        "start_job" => i18n::string("workspace.panel.agent.tool_status.preparing_background_job"),
        "poll_job" => i18n::string("workspace.panel.agent.tool_status.preparing_job_status"),
        "stop_job" => i18n::string("workspace.panel.agent.tool_status.preparing_job_stop"),
        "web_search" => i18n::string("workspace.panel.agent.tool_status.preparing_web_search"),
        "web_fetch" => i18n::string("workspace.panel.agent.tool_status.preparing_web_fetch"),
        "workspace_info" => {
            i18n::string("workspace.panel.agent.tool_status.preparing_workspace_info")
        }
        "ask_user" => i18n::string("workspace.panel.agent.tool_status.preparing_question"),
        "approval" => i18n::string("workspace.panel.agent.tool_status.preparing_approval_prompt"),
        _ => i18n::string("workspace.panel.agent.tool_status.preparing_request"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui::shell::layout) struct AskUserChoiceDisplay {
    pub(in crate::ui::shell::layout) label: String,
    pub(in crate::ui::shell::layout) description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui::shell::layout) struct AskUserPromptDisplay {
    pub(in crate::ui::shell::layout) message: String,
    pub(in crate::ui::shell::layout) choices: Vec<AskUserChoiceDisplay>,
    pub(in crate::ui::shell::layout) allow_custom: bool,
}

pub(in crate::ui::shell::layout) fn parse_ask_user_prompt(
    tool_call: &crate::ui::shell::SessionAgentToolCall,
) -> AskUserPromptDisplay {
    let args = tool_arguments_value(&tool_call.arguments);
    let message = (tool_call.status == SessionAgentToolStatus::WaitingForConfirmation)
        .then(|| tool_call.confirmation_note.clone())
        .flatten()
        .or_else(|| string_field(args.as_ref(), "message"))
        .or_else(|| partial_json_string_field(&tool_call.arguments, "message"))
        .unwrap_or_else(|| pending_or_note(tool_call));
    let choices = args
        .as_ref()
        .and_then(|value| value.get("choices"))
        .and_then(serde_json::Value::as_array)
        .map(|choices| {
            choices
                .iter()
                .filter_map(|choice| {
                    if let Some(label) = choice.as_str() {
                        let label = label.trim();
                        return (!label.is_empty()).then(|| AskUserChoiceDisplay {
                            label: label.to_string(),
                            description: None,
                        });
                    }

                    let label = choice.get("label")?.as_str()?.trim();
                    if label.is_empty() {
                        return None;
                    }
                    let description = choice
                        .get("description")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|description| !description.is_empty())
                        .map(ToOwned::to_owned);
                    Some(AskUserChoiceDisplay {
                        label: label.to_string(),
                        description,
                    })
                })
                .take(3)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    AskUserPromptDisplay {
        message,
        choices,
        allow_custom: true,
    }
}

pub(in crate::ui::shell::layout) fn tool_display_result(
    tool_call: &crate::ui::shell::SessionAgentToolCall,
) -> Option<String> {
    if !tool_has_result_status(tool_call) {
        return None;
    }

    tool_output_value(tool_call)
        .map(|value| display_json_value(&value))
        .or_else(|| {
            let note = tool_call.confirmation_note.as_deref()?;
            if note.trim().is_empty() {
                return None;
            }
            serde_json::from_str::<serde_json::Value>(note)
                .ok()
                .map(|value| display_json_value(&value))
                .or_else(|| Some(note.to_string()))
        })
        .filter(|text| !text.trim().is_empty())
}

pub(in crate::ui::shell::layout) fn partial_json_string_field(
    arguments: &str,
    key: &str,
) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = arguments.find(&needle)?;
    let after_key = &arguments[start + needle.len()..];
    let colon = after_key.find(':')?;
    let after_colon = after_key[colon + 1..].trim_start();
    let mut chars = after_colon.chars();
    if chars.next()? != '"' {
        return None;
    }

    let mut value = String::new();
    let mut escaped = false;
    for ch in chars {
        if escaped {
            match ch {
                '"' => value.push('"'),
                '\\' => value.push('\\'),
                '/' => value.push('/'),
                'b' => value.push('\u{0008}'),
                'f' => value.push('\u{000C}'),
                'n' => value.push('\n'),
                'r' => value.push('\r'),
                't' => value.push('\t'),
                _ => value.push(ch),
            }
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '"' => break,
            _ => value.push(ch),
        }
    }

    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

pub(in crate::ui::shell::layout) fn display_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Array(values) => values
            .iter()
            .take(100)
            .map(display_json_value)
            .collect::<Vec<_>>()
            .join("\n"),
        serde_json::Value::Object(object) => object
            .iter()
            .take(20)
            .map(|(key, value)| format!("{}: {}", title_case_key(key), display_json_value(value)))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

pub(in crate::ui::shell::layout) fn title_case_key(key: &str) -> String {
    match key {
        "path" => i18n::string("workspace.panel.agent.tool_fields.path"),
        "root" => i18n::string("workspace.panel.agent.tool_fields.root"),
        "pattern" => i18n::string("workspace.panel.agent.tool_fields.pattern"),
        "query" => i18n::string("workspace.panel.agent.tool_fields.query"),
        "url" => i18n::string("workspace.panel.agent.tool_fields.url"),
        "command" => i18n::string("workspace.panel.agent.tool_fields.command"),
        "cwd" => i18n::string("workspace.panel.agent.tool_fields.cwd"),
        "job_id" => i18n::string("workspace.panel.agent.tool_fields.job"),
        "base_dir" => i18n::string("workspace.panel.agent.tool_fields.base"),
        "content" => i18n::string("workspace.panel.agent.tool_fields.content"),
        "summary" => i18n::string("workspace.panel.agent.tool_fields.summary"),
        "message" => i18n::string("workspace.panel.agent.tool_fields.message"),
        _ => key.replace('_', " "),
    }
}

pub(in crate::ui::shell::layout) fn patch_paths(patch: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in patch.lines() {
        let path = line
            .strip_prefix("--- ")
            .or_else(|| line.strip_prefix("+++ "))
            .map(str::trim)
            .and_then(|path| {
                if path == "/dev/null" {
                    None
                } else {
                    Some(path.trim_start_matches("a/").trim_start_matches("b/"))
                }
            });
        if let Some(path) = path
            && !paths.iter().any(|existing| existing == path)
        {
            paths.push(path.to_string());
        }
    }
    paths
}

pub(in crate::ui::shell::layout) fn format_tool_call_copy_text(
    tool_call: &crate::ui::shell::SessionAgentToolCall,
) -> String {
    let arguments = if tool_call.arguments.trim() == "No arguments" {
        i18n::string("workspace.panel.agent.tool_placeholders.no_arguments")
    } else {
        tool_call.arguments.clone()
    };
    let mut text = format!(
        "{}: {}\n{}: {:?}\n{}:\n{}",
        i18n::string("workspace.panel.agent.tool_copy.header_tool"),
        tool_call.name,
        i18n::string("workspace.panel.agent.tool_copy.header_status"),
        tool_call.status,
        i18n::string("workspace.panel.agent.tool_copy.header_arguments"),
        arguments
    );
    if let Some(result) = tool_call.confirmation_note.as_ref() {
        text.push_str(&format!(
            "\n\n{}:\n",
            i18n::string("workspace.panel.agent.tool_copy.header_result")
        ));
        text.push_str(result);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_job_output_truncated_requires_true_boolean_flag() {
        let truncated = serde_json::json!({ "truncated": true });
        let complete = serde_json::json!({ "truncated": false });
        let legacy = serde_json::json!({});

        assert!(poll_job_output_truncated(Some(&truncated)));
        assert!(!poll_job_output_truncated(Some(&complete)));
        assert!(!poll_job_output_truncated(Some(&legacy)));
        assert!(!poll_job_output_truncated(None));
    }
}
