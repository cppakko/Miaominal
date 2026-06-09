use crate::channel::{AgentExecChannel, DEFAULT_MAX_OUTPUT_BYTES, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::path_guard::{resolve_workspace_path, shell_quote};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct GrepArgs {
    pub pattern: String,
    #[serde(default = "default_dot")]
    pub root: String,
    #[serde(default)]
    pub include: Vec<String>,
    pub max_results: Option<usize>,
    pub max_bytes: Option<usize>,
    #[serde(default)]
    pub case_insensitive: bool,
}

pub async fn grep(channel: &AgentExecChannel, args: GrepArgs) -> AgentResult<ToolOutput> {
    let root = resolve_workspace_path(&args.root)?;
    channel
        .policy()
        .enforce_path(crate::policy::AgentPathAccess::Read, &root, false)?;
    if crate::policy::is_sensitive_grep_pattern(&args.pattern) {
        return Err(crate::error::AgentError::Denied {
            tool_name: "grep".into(),
            reason: "grep pattern targets sensitive secret material".into(),
        });
    }
    if root == "/" || root == "/root" || root == "/home" {
        return Err(AgentError::InvalidPath(
            "grep requires a narrowed root".into(),
        ));
    }
    let max_results = args.max_results.unwrap_or(100);
    let max_bytes = args.max_bytes.unwrap_or(DEFAULT_MAX_OUTPUT_BYTES);
    let case_flag = if args.case_insensitive { "-i " } else { "" };
    let include_args = args
        .include
        .iter()
        .map(|include| format!(" --glob {}", shell_quote(include)))
        .collect::<String>();
    let find_name_filter = args
        .include
        .first()
        .map(|include| format!(" -name {}", shell_quote(include)))
        .unwrap_or_default();
    let command = format!(
        "cd \"$HOME\" && if command -v rg >/dev/null 2>&1; then \
         rg -n {case_flag}--max-count {max_results} --max-columns 300{include_args} -- {pattern} {root}; \
         else find {root} -type f{find_name_filter} -exec grep -n {case_flag}-E -- {pattern} {{}} \\; | head -n {max_results}; fi | head -c {max_bytes}",
        case_flag = case_flag,
        max_results = max_results,
        include_args = include_args,
        find_name_filter = find_name_filter,
        pattern = shell_quote(&args.pattern),
        root = shell_quote(&root),
        max_bytes = max_bytes,
    );
    Ok(ToolOutput::Text {
        content: channel.exec(command).await?,
        truncated: false,
    })
}

fn default_dot() -> String {
    ".".into()
}
