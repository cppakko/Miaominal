use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::AgentResult;
use crate::path_guard::{resolve_workspace_path, shell_quote};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct ListArgs {
    #[serde(default = "default_dot")]
    pub path: String,
    #[serde(default)]
    pub include_hidden: bool,
    pub max_entries: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ListEntryType {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListEntry {
    pub name: String,
    pub entry_type: ListEntryType,
    pub size: Option<u64>,
    pub mtime: Option<u64>,
}

pub async fn list(channel: &AgentExecChannel, args: ListArgs) -> AgentResult<ToolOutput> {
    let path = resolve_workspace_path(&args.path)?;
    channel
        .policy()
        .enforce_path(crate::policy::AgentPathAccess::Read, &path, false)?;
    let max_entries = args.max_entries.unwrap_or(500);
    let hidden_filter = if args.include_hidden {
        ""
    } else {
        " | awk -F'\\t' '$1 !~ /^\\./'"
    };
    let command = format!(
        "cd \"$HOME\" && find {path} -mindepth 1 -maxdepth 1 -printf '%f\\t%y\\t%s\\t%T@\\n'{hidden_filter} | sort | head -n {max}",
        path = shell_quote(&path),
        hidden_filter = hidden_filter,
        max = max_entries + 1,
    );
    let output = channel.exec(command).await?;
    let mut entries = output
        .lines()
        .filter_map(parse_entry)
        .collect::<Vec<ListEntry>>();
    let truncated = entries.len() > max_entries;
    entries.truncate(max_entries);

    Ok(ToolOutput::DirectoryList {
        path,
        entries,
        truncated,
    })
}

fn parse_entry(line: &str) -> Option<ListEntry> {
    let mut parts = line.split('\t');
    let name = parts.next()?.to_string();
    let entry_type = match parts.next()? {
        "f" => ListEntryType::File,
        "d" => ListEntryType::Directory,
        "l" => ListEntryType::Symlink,
        _ => ListEntryType::Other,
    };
    let size = parts.next().and_then(|value| value.parse().ok());
    let mtime = parts
        .next()
        .and_then(|value| value.split('.').next())
        .and_then(|value| value.parse().ok());
    Some(ListEntry {
        name,
        entry_type,
        size,
        mtime,
    })
}

fn default_dot() -> String {
    ".".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_find_entry() {
        let entry = parse_entry("src\td\t4096\t1780940000.0").unwrap();

        assert_eq!(entry.name, "src");
        assert_eq!(entry.entry_type, ListEntryType::Directory);
        assert_eq!(entry.size, Some(4096));
        assert_eq!(entry.mtime, Some(1780940000));
    }
}
