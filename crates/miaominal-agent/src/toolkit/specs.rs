//! Tool definitions sent to the model (design doc §9).

use crate::model::ToolSpec;
use crate::policy::ApprovalMode;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolClass {
    /// Never modifies anything; safe to run concurrently.
    ReadOnly,
    /// Risk depends on the arguments (shell).
    Command,
    /// Writes files.
    FileWrite,
    /// Runs inside the engine (ask_user, update_plan).
    Engine,
}

pub fn tool_class(name: &str) -> Option<ToolClass> {
    Some(match name {
        "read_file" | "list_dir" | "glob" | "grep" | "host_info" | "terminal_read"
        | "web_search" | "web_fetch" | "job_poll" => ToolClass::ReadOnly,
        "shell" | "job_start" | "job_stop" => ToolClass::Command,
        "edit_file" | "write_file" | "apply_patch" => ToolClass::FileWrite,
        "ask_user" | "update_plan" => ToolClass::Engine,
        _ => return None,
    })
}

#[derive(Clone, Debug, Default)]
pub struct ToolSetOptions {
    pub mode: Option<ApprovalMode>,
    pub has_targets: bool,
    pub target_names: Vec<String>,
    pub web_search: bool,
    pub terminal: bool,
}

pub fn tool_specs(options: &ToolSetOptions) -> Vec<ToolSpec> {
    let read_only = options.mode == Some(ApprovalMode::ReadOnly);
    let target = target_schema(&options.target_names);
    let mut specs = Vec::new();
    if options.has_targets {
        specs.push(spec(
            "shell",
            "Run a non-interactive command on a target and return exit code, stdout and stderr.\n\
Use for inspection (status, logs, config checks) and for changes when a file tool does not fit.\n\
- Runs via the target's exec_shell from <environment>; write commands in that syntax.\n\
- No TTY: never run editors, pagers, or anything that prompts. Add -y / --no-pager.\n\
- Pipes, &&, and redirects are fine. Independent read-only checks can be separate parallel calls.\n\
- Times out after timeout_s (default 60, max 600). For builds, installs or anything longer, use job_start.\n\
- Output is truncated to head+tail when large; narrow with grep/tail/head instead of re-running.\n\
Example: {\"command\": \"systemctl status nginx --no-pager -l && journalctl -u nginx -n 50 --no-pager\"}",
            json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "The command line to run."},
                    "target": target,
                    "cwd": {"type": "string", "description": "Working directory (default: the target's home/cwd)."},
                    "timeout_s": {"type": "integer", "minimum": 1, "maximum": 600, "description": "Timeout in seconds (default 60)."}
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        ));
        if !read_only {
            specs.push(spec(
                "job_start",
                "Start a long-running command in the background on a POSIX target and return a job_id immediately.\n\
Use for builds, installs, downloads, or watching output over time. Then call job_poll to read new output.\n\
Example: {\"command\": \"apt-get install -y nginx\"}",
                json!({
                    "type": "object",
                    "properties": {
                        "command": {"type": "string"},
                        "target": target,
                        "cwd": {"type": "string"}
                    },
                    "required": ["command"],
                    "additionalProperties": false
                }),
            ));
            specs.push(spec(
                "job_stop",
                "Stop a background job started with job_start.",
                json!({
                    "type": "object",
                    "properties": {"job_id": {"type": "string"}, "target": target},
                    "required": ["job_id"],
                    "additionalProperties": false
                }),
            ));
        }
        specs.push(spec(
            "job_poll",
            "Return status and output produced since the previous poll for a background job.\n\
Optionally wait up to wait_s seconds (max 60) for the job to finish before returning.",
            json!({
                "type": "object",
                "properties": {
                    "job_id": {"type": "string"},
                    "target": target,
                    "wait_s": {"type": "integer", "minimum": 0, "maximum": 60}
                },
                "required": ["job_id"],
                "additionalProperties": false
            }),
        ));
        specs.push(spec(
            "read_file",
            "Read a text file on a target. Returns numbered lines (`  12│text`).\n\
Use offset/limit for large files (default: first 400 lines). Read a file before editing it.\n\
Example: {\"path\": \"/etc/nginx/nginx.conf\"}",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Absolute path, or relative to the target's home."},
                    "target": target,
                    "offset": {"type": "integer", "minimum": 1, "description": "1-based first line."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 2000, "description": "Number of lines."}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        ));
        if !read_only {
            specs.push(spec(
                "edit_file",
                "Replace an exact string in a file. old_string must match the file exactly (including \
indentation) and be unique unless replace_all is true; include surrounding lines to make it unique. \
Do not include the line-number prefixes from read_file. Returns a diff.\n\
Example: {\"path\": \"/etc/nginx/conf.d/app.conf\", \"old_string\": \"listen 80;\", \"new_string\": \"listen 8080;\"}",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "target": target,
                        "old_string": {"type": "string"},
                        "new_string": {"type": "string"},
                        "replace_all": {"type": "boolean"}
                    },
                    "required": ["path", "old_string", "new_string"],
                    "additionalProperties": false
                }),
            ));
            specs.push(spec(
                "write_file",
                "Create a file or replace its entire content. Parent directories are created. \
Prefer edit_file for changing existing files.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "target": target,
                        "content": {"type": "string"}
                    },
                    "required": ["path", "content"],
                    "additionalProperties": false
                }),
            ));
            specs.push(spec(
                "apply_patch",
                "Apply a unified diff (---/+++ headers with paths relative to base_dir, @@ hunks). \
Use for coordinated changes across several files; prefer edit_file for single edits.",
                json!({
                    "type": "object",
                    "properties": {
                        "patch": {"type": "string"},
                        "base_dir": {"type": "string", "description": "Directory the patch paths are relative to."},
                        "target": target
                    },
                    "required": ["patch", "base_dir"],
                    "additionalProperties": false
                }),
            ));
        }
        specs.push(spec(
            "list_dir",
            "List a directory on a target (name, type, size).",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "target": target,
                    "include_hidden": {"type": "boolean"}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        ));
        specs.push(spec(
            "glob",
            "Find files by name pattern under a directory, e.g. {\"root\": \"/etc/nginx\", \"pattern\": \"*.conf\"}.",
            json!({
                "type": "object",
                "properties": {
                    "root": {"type": "string"},
                    "pattern": {"type": "string"},
                    "target": target,
                    "include_hidden": {"type": "boolean"}
                },
                "required": ["root", "pattern"],
                "additionalProperties": false
            }),
        ));
        specs.push(spec(
            "grep",
            "Search file contents with a regular expression under a directory. Returns path:line:text matches.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string"},
                    "root": {"type": "string"},
                    "target": target,
                    "include": {"type": "array", "items": {"type": "string"}, "description": "File globs, e.g. [\"*.conf\"]."},
                    "case_insensitive": {"type": "boolean"}
                },
                "required": ["pattern", "root"],
                "additionalProperties": false
            }),
        ));
        specs.push(spec(
            "host_info",
            "Refresh and show details of a target: OS, shell, user, cwd, available tools. \
<environment> already has the basics; call this only when you need more detail.",
            json!({
                "type": "object",
                "properties": {"target": target},
                "additionalProperties": false
            }),
        ));
    }
    if options.terminal {
        specs.push(spec(
            "terminal_read",
            "Read recent text from the user's terminal (scrollback, ANSI removed). Use when the user refers \
to something on their screen. Optional search returns only matching lines with context.",
            json!({
                "type": "object",
                "properties": {
                    "lines": {"type": "integer", "minimum": 1, "maximum": 2000, "description": "Number of trailing lines (default 200)."},
                    "search": {"type": "string", "description": "Case-insensitive substring to look for."}
                },
                "additionalProperties": false
            }),
        ));
    }
    if options.web_search {
        specs.push(spec(
            "web_search",
            "Search the web. Use for current information, error messages, or documentation not on the host.",
            json!({
                "type": "object",
                "properties": {"query": {"type": "string"}},
                "required": ["query"],
                "additionalProperties": false
            }),
        ));
    }
    specs.push(spec(
        "web_fetch",
        "Fetch a URL and return its text content (HTML converted to text).",
        json!({
            "type": "object",
            "properties": {"url": {"type": "string"}},
            "required": ["url"],
            "additionalProperties": false
        }),
    ));
    specs.push(spec(
        "ask_user",
        "Ask the user a question and wait for the answer. Use only when the next step depends on their \
intent or on information you cannot find yourself. Offer up to 4 short choices when possible.",
        json!({
            "type": "object",
            "properties": {
                "question": {"type": "string"},
                "choices": {"type": "array", "items": {"type": "string"}, "maxItems": 4}
            },
            "required": ["question"],
            "additionalProperties": false
        }),
    ));
    specs.push(spec(
        "update_plan",
        "Create or update the task plan shown to the user. Send the full list each time; keep exactly one \
step in_progress while working.",
        json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "step": {"type": "string"},
                            "status": {"type": "string", "enum": ["pending", "in_progress", "completed"]}
                        },
                        "required": ["step", "status"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["items"],
            "additionalProperties": false
        }),
    ));
    specs
}

fn spec(name: &str, description: &str, parameters: Value) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
    }
}

fn target_schema(names: &[String]) -> Value {
    if names.len() > 1 {
        json!({
            "type": "string",
            "enum": names,
            "description": "Execution target name from <environment> (without @). Omit for the default target."
        })
    } else {
        json!({
            "type": "string",
            "description": "Execution target name from <environment> (without @). Omit for the default target."
        })
    }
}

/// Validates arguments against the tool's JSON schema subset (required
/// properties, primitive types, enums, no unknown properties). Returns a
/// short, model-facing error.
pub fn validate_arguments(spec: &ToolSpec, arguments: &Value) -> Result<(), String> {
    validate(&spec.parameters, arguments, "")
}

fn validate(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    let at = |path: &str| {
        if path.is_empty() {
            "arguments".to_string()
        } else {
            format!("`{path}`")
        }
    };
    if let Some(kind) = schema.get("type").and_then(Value::as_str) {
        let ok = match kind {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            _ => true,
        };
        if !ok {
            return Err(format!("{} must be of type {kind}", at(path)));
        }
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array)
        && !options.contains(value)
    {
        let list = options
            .iter()
            .map(|option| option.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!("{} must be one of: {list}", at(path)));
    }
    if let Some(object) = value.as_object() {
        let properties = schema.get("properties").and_then(Value::as_object);
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for name in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(name) {
                    return Err(format!("missing required argument `{}`", join(path, name)));
                }
            }
        }
        if let Some(properties) = properties {
            for (name, item) in object {
                match properties.get(name) {
                    Some(property) => {
                        if item.is_null() {
                            continue;
                        }
                        validate(property, item, &join(path, name))?;
                    }
                    None if schema.get("additionalProperties") == Some(&Value::Bool(false)) => {
                        let known = properties.keys().cloned().collect::<Vec<_>>().join(", ");
                        return Err(format!(
                            "unknown argument `{}`; allowed: {known}",
                            join(path, name)
                        ));
                    }
                    None => {}
                }
            }
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        if let Some(max) = schema.get("maxItems").and_then(Value::as_u64)
            && array.len() as u64 > max
        {
            return Err(format!("{} allows at most {max} items", at(path)));
        }
        for (index, item) in array.iter().enumerate() {
            validate(items, item, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_string()
    } else {
        format!("{path}.{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> ToolSetOptions {
        ToolSetOptions {
            mode: Some(ApprovalMode::Standard),
            has_targets: true,
            target_names: vec!["a".into(), "b".into()],
            web_search: true,
            terminal: true,
        }
    }

    #[test]
    fn every_spec_has_a_class() {
        for spec in tool_specs(&options()) {
            assert!(tool_class(&spec.name).is_some(), "{}", spec.name);
        }
    }

    #[test]
    fn read_only_mode_hides_write_tools() {
        let mut options = options();
        options.mode = Some(ApprovalMode::ReadOnly);
        let names: Vec<_> = tool_specs(&options).into_iter().map(|s| s.name).collect();
        assert!(names.contains(&"shell".to_string()));
        assert!(!names.contains(&"edit_file".to_string()));
        assert!(!names.contains(&"job_start".to_string()));
    }

    #[test]
    fn validation_reports_useful_errors() {
        let specs = tool_specs(&options());
        let shell = specs.iter().find(|s| s.name == "shell").unwrap();
        assert!(validate_arguments(shell, &json!({"command": "ls"})).is_ok());
        assert_eq!(
            validate_arguments(shell, &json!({})).unwrap_err(),
            "missing required argument `command`"
        );
        assert!(
            validate_arguments(shell, &json!({"command": "ls", "cmd": 1}))
                .unwrap_err()
                .starts_with("unknown argument `cmd`")
        );
        assert!(
            validate_arguments(shell, &json!({"command": "ls", "target": "c"}))
                .unwrap_err()
                .contains("must be one of")
        );
        assert!(validate_arguments(shell, &json!({"command": "ls", "timeout_s": null})).is_ok());
        let plan = specs.iter().find(|s| s.name == "update_plan").unwrap();
        assert!(
            validate_arguments(plan, &json!({"items": [{"step": "x", "status": "done"}]}))
                .unwrap_err()
                .contains("items[0].status")
        );
    }
}
