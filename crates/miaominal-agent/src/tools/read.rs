use crate::channel::{AgentExecChannel, DEFAULT_MAX_OUTPUT_BYTES, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::path_guard::{resolve_workspace_path, shell_quote};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ReadArgs {
    pub path: String,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
    pub max_bytes: Option<usize>,
}

pub async fn read(channel: &AgentExecChannel, args: ReadArgs) -> AgentResult<ToolOutput> {
    let path = resolve_workspace_path(&args.path)?;
    channel
        .policy()
        .enforce_path(crate::policy::AgentPathAccess::Read, &path, false)?;
    let max_bytes = args.max_bytes.unwrap_or(DEFAULT_MAX_OUTPUT_BYTES);
    let start = args.start_line.unwrap_or(1).max(1);
    let end = args.end_line.unwrap_or(start + 199).max(start);
    if end.saturating_sub(start) > 2_000 {
        return Err(AgentError::InvalidArguments(
            "read line range is too large; request 2000 lines or fewer".into(),
        ));
    }
    let command = format!(
        "cd \"$HOME\" && if [ -f {path} ]; then \
         tmp=$(mktemp); sed -n '{start},{end}p' {path} >\"$tmp\"; \
         bytes=$(wc -c <\"$tmp\"); head -c {max} \"$tmp\"; rm -f \"$tmp\"; \
         if [ \"$bytes\" -gt {max} ]; then printf '\\n[MIAOMINAL_TRUNCATED]'; fi; \
         else printf 'not a regular file: %s' {path} >&2; exit 1; fi",
        path = shell_quote(&path, channel.shell_type()),
        start = start,
        end = end,
        max = max_bytes,
    );
    let output = channel.exec(command).await?;
    Ok(ToolOutput::Text {
        truncated: output.contains("[MIAOMINAL_TRUNCATED]"),
        content: output.replace("\n[MIAOMINAL_TRUNCATED]", ""),
    })
}
