use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::AgentResult;
use crate::path_guard::{cd_prefix, resolve_workspace_path, shell_quote};
use miaominal_core::profile::ShellType;
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
    if matches!(channel.shell_type(), ShellType::PowerShell | ShellType::Cmd) {
        super::workspace_info::ensure_exec_shell_detected(channel).await;
    }

    let path = resolve_workspace_path(&args.path)?;
    if !channel.policy_bypass_enabled() {
        channel
            .policy()
            .enforce_path(crate::policy::AgentPathAccess::Read, &path, false)?;
    }
    let max_entries = args.max_entries.unwrap_or(500);
    let command = list_command(
        channel.shell_type(),
        &path,
        args.include_hidden,
        max_entries,
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

fn list_command(
    shell_type: ShellType,
    path: &str,
    include_hidden: bool,
    max_entries: usize,
) -> String {
    let max = max_entries + 1;
    match shell_type {
        ShellType::PowerShell => {
            let cd = cd_prefix(shell_type, path);
            let hidden_filter = if include_hidden {
                String::new()
            } else {
                " | Where-Object { -not $_.Name.StartsWith('.') }".to_string()
            };
            let ps_script = format!(
                "{cd}; Get-ChildItem -Path '.' -Force{hidden_filter} | ForEach-Object {{ $type = if ($_.PSIsContainer) {{ 'd' }} elseif ($_.LinkType) {{ 'l' }} else {{ 'f' }}; $_.Name + [char]9 + $type + [char]9 + $_.Length + [char]9 + [DateTimeOffset]::new($_.LastWriteTime).ToUnixTimeSeconds() }} | Sort-Object | Select-Object -First {max}",
            );
            format!("powershell.exe -NoProfile -Command \"{ps_script}\"")
        }
        ShellType::Cmd => {
            let cd = cd_prefix(shell_type, path);
            let hidden_switch = if include_hidden { "/a" } else { "/a-h" };
            format!(
                "{cd} && for /f \"delims=\" %i in ('dir /b {hidden_switch} 2^>nul') do @echo %i\tf\t0\t0"
            )
        }
        _ => {
            let hidden_filter = if include_hidden {
                ""
            } else {
                " | awk -F'\\t' '$1 !~ /^\\./'"
            };
            format!(
                "cd \"$HOME\" && find {path} -mindepth 1 -maxdepth 1 -printf '%f\\t%y\\t%s\\t%T@\\n'{hidden_filter} | sort | head -n {max}",
                path = shell_quote(path, shell_type),
            )
        }
    }
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

    #[test]
    fn powershell_list_parse_format() {
        // Directory
        let entry = parse_entry("src\td\t4096\t1780940000").unwrap();
        assert_eq!(entry.name, "src");
        assert_eq!(entry.entry_type, ListEntryType::Directory);
        assert_eq!(entry.size, Some(4096));
        assert_eq!(entry.mtime, Some(1780940000));

        // File
        let entry = parse_entry("main.rs\tf\t2048\t1780940001").unwrap();
        assert_eq!(entry.name, "main.rs");
        assert_eq!(entry.entry_type, ListEntryType::File);
        assert_eq!(entry.size, Some(2048));
        assert_eq!(entry.mtime, Some(1780940001));

        // Symlink
        let entry = parse_entry("link\tl\t0\t1780940002").unwrap();
        assert_eq!(entry.name, "link");
        assert_eq!(entry.entry_type, ListEntryType::Symlink);
        assert_eq!(entry.size, Some(0));
        assert_eq!(entry.mtime, Some(1780940002));
    }

    #[test]
    fn powershell_list_command_structure() {
        let cmd = list_command(ShellType::PowerShell, "src", false, 500);
        // Must use PowerShell cmdlets, not find
        assert!(
            cmd.contains("Get-ChildItem"),
            "expected Get-ChildItem, got: {cmd}"
        );
        assert!(
            cmd.contains("ForEach-Object"),
            "expected ForEach-Object, got: {cmd}"
        );
        assert!(
            cmd.contains("Sort-Object"),
            "expected Sort-Object, got: {cmd}"
        );
        assert!(
            cmd.contains("Select-Object -First"),
            "expected Select-Object -First, got: {cmd}"
        );
        assert!(
            cmd.contains("Set-Location $env:USERPROFILE"),
            "expected cd to USERPROFILE, got: {cmd}"
        );
        // Must NOT contain POSIX find
        assert!(!cmd.contains("find "), "should not use POSIX find: {cmd}");
        // Must filter dotfiles when include_hidden=false
        assert!(
            cmd.contains("Where-Object"),
            "expected dotfile filter when include_hidden=false, got: {cmd}"
        );
        assert!(
            cmd.contains("$_.Name.StartsWith('.')"),
            "expected dotfile name check, got: {cmd}"
        );
    }

    #[test]
    fn powershell_list_command_include_hidden() {
        let visible_all = list_command(ShellType::PowerShell, "src", true, 500);
        assert!(
            !visible_all.contains("Where-Object"),
            "include_hidden=true should skip Where-Object, got: {visible_all}"
        );
    }

    #[test]
    fn cmd_list_command_structure() {
        let cmd = list_command(ShellType::Cmd, "src", false, 500);
        // Must use CMD dir + for loop
        assert!(cmd.contains("dir /b"), "expected dir /b, got: {cmd}");
        assert!(cmd.contains("for /f"), "expected for /f, got: {cmd}");
        assert!(
            cmd.contains("cd /d %USERPROFILE%"),
            "expected cd to USERPROFILE, got: {cmd}"
        );
        // Must filter hidden files when include_hidden=false
        assert!(cmd.contains("/a-h"), "expected /a-h switch, got: {cmd}");
        // Must NOT contain PowerShell cmdlets
        assert!(
            !cmd.contains("Get-ChildItem"),
            "should not use PowerShell: {cmd}"
        );
    }

    #[test]
    fn cmd_list_command_include_hidden() {
        let cmd = list_command(ShellType::Cmd, "src", true, 500);
        assert!(
            cmd.contains("/a ") || cmd.contains("/a\"") || cmd.ends_with("/a"),
            "include_hidden=true should use /a not /a-h, got: {cmd}"
        );
        assert!(
            !cmd.contains("/a-h"),
            "should not filter hidden, got: {cmd}"
        );
    }

    #[test]
    fn posix_list_unchanged() {
        let cmd = list_command(ShellType::Posix, "src", false, 500);
        assert!(
            cmd.contains("cd \"$HOME\""),
            "expected cd $HOME, got: {cmd}"
        );
        assert!(cmd.contains("find "), "expected find, got: {cmd}");
        assert!(
            cmd.contains("-mindepth 1 -maxdepth 1"),
            "expected depth limits, got: {cmd}"
        );
        assert!(cmd.contains("-printf"), "expected -printf, got: {cmd}");
        assert!(
            cmd.contains("| sort | head -n"),
            "expected sort|head, got: {cmd}"
        );
        assert!(
            cmd.contains("awk -F'\\t'"),
            "expected awk filter for hidden, got: {cmd}"
        );
    }

    #[test]
    fn posix_list_include_hidden_skips_awk() {
        let cmd = list_command(ShellType::Posix, "src", true, 500);
        assert!(
            !cmd.contains("awk"),
            "include_hidden should skip awk: {cmd}"
        );
    }

    #[test]
    fn fish_list_uses_posix_path() {
        let cmd = list_command(ShellType::Fish, "src", false, 500);
        // Fish currently falls through to the Posix find command
        assert!(
            cmd.contains("cd \"$HOME\""),
            "expected cd $HOME for fish, got: {cmd}"
        );
        assert!(cmd.contains("find "), "expected find for fish, got: {cmd}");
    }
}
