use super::super::*;

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
            lines.push("Command timed out.".to_string());
        }
        if self.truncated {
            lines.push("Output truncated.".to_string());
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
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
) -> Option<serde_json::Value> {
    if !tool_has_result_status(tool_call) {
        return None;
    }

    let note = tool_call.confirmation_note.as_deref()?;
    serde_json::from_str(note).ok()
}

pub(in crate::ui::shell::layout) fn tool_output_value(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
) -> Option<serde_json::Value> {
    let response = tool_response_value(tool_call)?;
    let output = response.get("output")?;
    if let Some(kind) = output.get("kind").and_then(serde_json::Value::as_str) {
        if kind == "patch" {
            return Some(output.clone());
        }
    }
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
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
) -> String {
    tool_display_result(tool_call).unwrap_or_else(|| pending_result_text(tool_call))
}

pub(in crate::ui::shell::layout) fn tool_has_result_status(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
) -> bool {
    matches!(
        tool_call.status,
        SessionAgentToolStatus::Completed
            | SessionAgentToolStatus::Failed
            | SessionAgentToolStatus::Rejected
    )
}

pub(in crate::ui::shell::layout) fn pending_result_text(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
) -> String {
    match tool_call.status {
        SessionAgentToolStatus::Pending => "Preparing request...".to_string(),
        SessionAgentToolStatus::WaitingForConfirmation => "Waiting for approval...".to_string(),
        SessionAgentToolStatus::InProgress => "Waiting for result...".to_string(),
        SessionAgentToolStatus::Completed => "No output".to_string(),
        SessionAgentToolStatus::Failed => "Tool failed before returning output.".to_string(),
        SessionAgentToolStatus::Rejected => "Tool was rejected.".to_string(),
    }
}

pub(in crate::ui::shell::layout) fn arguments_are_streaming(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
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
        "read" => "Preparing file read...".to_string(),
        "list" => "Preparing directory listing...".to_string(),
        "glob" => "Preparing file search...".to_string(),
        "grep" => "Preparing text search...".to_string(),
        "start_job" => "Preparing background job...".to_string(),
        "poll_job" => "Preparing job status request...".to_string(),
        "stop_job" => "Preparing job stop request...".to_string(),
        "web_search" => "Preparing web search...".to_string(),
        "web_fetch" => "Preparing web fetch...".to_string(),
        "workspace_info" => "Preparing workspace info request...".to_string(),
        "ask_user" | "approval" => "Preparing approval prompt...".to_string(),
        _ => "Preparing request...".to_string(),
    }
}

pub(in crate::ui::shell::layout) fn tool_display_result(
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
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
        "path" => "Path".to_string(),
        "root" => "Root".to_string(),
        "pattern" => "Pattern".to_string(),
        "query" => "Query".to_string(),
        "url" => "Url".to_string(),
        "command" => "Command".to_string(),
        "cwd" => "Cwd".to_string(),
        "job_id" => "Job".to_string(),
        "base_dir" => "Base".to_string(),
        "content" => "Content".to_string(),
        "summary" => "Summary".to_string(),
        "message" => "Message".to_string(),
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
    tool_call: &crate::ui::shell::state::SessionAgentToolCall,
) -> String {
    let mut text = format!(
        "Tool: {}\nStatus: {:?}\nArguments:\n{}",
        tool_call.name, tool_call.status, tool_call.arguments
    );
    if let Some(result) = tool_call.confirmation_note.as_ref() {
        text.push_str("\n\nResult:\n");
        text.push_str(result);
    }
    text
}
