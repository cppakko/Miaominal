use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::path_guard::{resolve_workspace_path, shell_quote};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct GlobArgs {
    #[serde(default = "default_dot")]
    pub root: String,
    pub pattern: String,
    pub max_results: Option<usize>,
    #[serde(default)]
    pub include_hidden: bool,
}

pub async fn glob(channel: &AgentExecChannel, args: GlobArgs) -> AgentResult<ToolOutput> {
    let root = resolve_workspace_path(&args.root)?;
    channel
        .policy()
        .enforce_path(crate::policy::AgentPathAccess::Read, &root, false)?;
    if is_overbroad_root(&root) {
        return Err(AgentError::InvalidPath(
            "glob requires a narrowed workspace root".into(),
        ));
    }
    let max_results = args.max_results.unwrap_or(200);
    let name_pattern = find_name_pattern(&args.pattern);
    let hidden_filter = if args.include_hidden {
        ""
    } else {
        " | awk -F/ '{ for (i=1; i<=NF; i++) if ($i ~ /^\\./) next; print }'"
    };
    let command = format!(
        "cd \"$HOME\" && find {root} -type f -name {pattern} -print{hidden_filter} | sed 's#^./##' | sort | head -n {max}",
        root = shell_quote(&root),
        pattern = shell_quote(&name_pattern),
        hidden_filter = hidden_filter,
        max = max_results + 1,
    );
    let output = channel.exec(command).await?;
    let mut entries = output.lines().map(str::to_string).collect::<Vec<_>>();
    let truncated = entries.len() > max_results;
    entries.truncate(max_results);
    Ok(ToolOutput::List { entries, truncated })
}

fn find_name_pattern(pattern: &str) -> String {
    pattern
        .rsplit('/')
        .next()
        .filter(|part| !part.is_empty() && *part != "**")
        .unwrap_or(pattern)
        .to_string()
}

fn default_dot() -> String {
    ".".into()
}

fn is_overbroad_root(root: &str) -> bool {
    matches!(
        root,
        "/" | "/home" | "/root" | "/var" | "/etc" | "home" | "root"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_find_name_pattern_from_globstar() {
        assert_eq!(find_name_pattern("**/*.conf"), "*.conf");
        assert_eq!(
            find_name_pattern("docker-compose*.yml"),
            "docker-compose*.yml"
        );
    }

    #[test]
    fn rejects_overbroad_roots() {
        assert!(is_overbroad_root("/"));
        assert!(is_overbroad_root("/home"));
        assert!(!is_overbroad_root("/var/log/nginx"));
    }
}
