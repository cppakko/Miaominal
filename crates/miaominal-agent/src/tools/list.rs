use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::AgentResult;
use crate::path_guard::{RemotePathKind, shell_quote};
use crate::policy::{AgentPathAccess, posix_find_sensitive_predicate};
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

    let path = channel
        .authorize_existing_path(&args.path, AgentPathAccess::Read, RemotePathKind::Directory)
        .await?;
    let path = path.as_str();
    let max_entries = args.max_entries.unwrap_or(500);
    let command = list_command(
        channel.shell_type(),
        path,
        args.include_hidden,
        max_entries,
        !channel.policy_bypass_enabled(),
    );
    let output = channel.exec(command).await?;
    let mut entries = output
        .lines()
        .filter_map(parse_entry)
        .filter(|entry| {
            channel.policy_bypass_enabled()
                || !crate::policy::is_sensitive_path(&join_child_path(path, &entry.name))
        })
        .collect::<Vec<ListEntry>>();
    let truncated = entries.len() > max_entries;
    entries.truncate(max_entries);

    Ok(ToolOutput::DirectoryList {
        path: path.to_string(),
        entries,
        truncated,
    })
}

fn list_command(
    shell_type: ShellType,
    path: &str,
    include_hidden: bool,
    max_entries: usize,
    guard_sensitive: bool,
) -> String {
    let max = max_entries + 1;
    match shell_type {
        ShellType::PowerShell | ShellType::Cmd => {
            let hidden_filter = if include_hidden {
                String::new()
            } else {
                " | Where-Object { -not $_.Name.StartsWith('.') }".to_string()
            };
            let sensitive_filter = if guard_sensitive {
                " | Where-Object { -not (Test-MiaominalSensitivePath $_.FullName) }"
            } else {
                ""
            };
            let root = shell_quote(path, ShellType::PowerShell);
            let ps_script = format!(
                "{sensitive_function}; $root={root}; Get-ChildItem -LiteralPath $root -Force{hidden_filter}{sensitive_filter} | ForEach-Object {{ $isLink=($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0; $type = if ($isLink) {{ 'l' }} elseif ($_.PSIsContainer) {{ 'd' }} else {{ 'f' }}; $length=if($_.PSIsContainer){{0}}else{{$_.Length}}; $_.Name + [char]9 + $type + [char]9 + $length + [char]9 + [DateTimeOffset]::new($_.LastWriteTime).ToUnixTimeSeconds() }} | Sort-Object | Select-Object -First {max}",
                sensitive_function = super::windows::powershell_sensitive_path_function(),
            );
            super::windows::powershell_command_for_shell(shell_type, &ps_script)
        }
        _ => {
            let hidden_filter = if include_hidden {
                ""
            } else {
                " | awk -F'\\t' '$1 !~ /^\\./'"
            };
            let sensitive_filter = if guard_sensitive {
                format!(" ! {}", posix_find_sensitive_predicate())
            } else {
                String::new()
            };
            format!(
                "cd \"$HOME\" && find -P {path} -mindepth 1 -maxdepth 1{sensitive_filter} -printf '%f\\t%y\\t%s\\t%T@\\n'{hidden_filter} | sort | head -n {max}",
                path = shell_quote(path, shell_type),
            )
        }
    }
}

fn join_child_path(parent: &str, child: &str) -> String {
    format!("{}/{}", parent.trim_end_matches(['/', '\\']), child)
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
    use base64::Engine as _;

    fn decode_powershell_command(command: &str) -> String {
        let payload = command
            .strip_prefix("powershell.exe -NoProfile -EncodedCommand ")
            .expect("encoded PowerShell command");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .expect("valid base64");
        let units = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        String::from_utf16(&units).expect("valid UTF-16LE")
    }

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
        let command = list_command(ShellType::PowerShell, "src", false, 500, true);
        let cmd = decode_powershell_command(&command);
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
            cmd.contains("Get-ChildItem -LiteralPath $root"),
            "expected literal root path, got: {cmd}"
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
        let visible_all = decode_powershell_command(&list_command(
            ShellType::PowerShell,
            "src",
            true,
            500,
            false,
        ));
        assert!(
            !visible_all.contains("Where-Object"),
            "include_hidden=true should skip Where-Object, got: {visible_all}"
        );
    }

    #[test]
    fn cmd_list_command_structure() {
        let cmd = list_command(ShellType::Cmd, "x & whoami & rem", false, 500, true);
        assert!(cmd.starts_with("set MIAOMINAL_AGENT_PS_GZIP="));
        assert!(cmd.contains("powershell.exe -NoProfile -EncodedCommand "));
        assert!(!cmd.contains("whoami"));
    }

    #[test]
    fn cmd_list_command_include_hidden() {
        let cmd = list_command(ShellType::Cmd, "src", true, 500, false);
        assert!(cmd.starts_with("set MIAOMINAL_AGENT_PS_GZIP="));
    }

    #[cfg(windows)]
    #[test]
    fn cmd_list_executes_literal_metacharacter_path_without_injection() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("x & echo MIAOMINAL_INJECTED & rem");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("safe.txt"), "safe").unwrap();
        std::fs::create_dir(root.join(".ssh")).unwrap();
        std::fs::write(root.join(".ssh").join("id_rsa"), "private").unwrap();

        let command = list_command(
            ShellType::Cmd,
            root.to_string_lossy().as_ref(),
            true,
            100,
            true,
        );
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", &command])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert!(stdout.contains("safe.txt"));
        assert!(!stdout.contains("MIAOMINAL_INJECTED"));
        assert!(!stdout.contains(".ssh"));
    }

    #[test]
    fn posix_list_filters_sensitive_entries_before_truncation() {
        let cmd = list_command(ShellType::Posix, "src", false, 500, true);
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
        assert!(cmd.contains("-ipath '/etc/shadow'"));
        assert!(cmd.contains("-iname '*.env.*'"));
        assert!(cmd.contains("-iname '*.rdp'"));
        assert!(cmd.find("-iname '*.key'").unwrap() < cmd.find("-printf").unwrap());
        assert!(cmd.find("-printf").unwrap() < cmd.find("head -n").unwrap());
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
        let cmd = list_command(ShellType::Posix, "src", true, 500, true);
        assert!(
            !cmd.contains("awk"),
            "include_hidden should skip awk: {cmd}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn posix_list_limit_counts_only_safe_entries() {
        let temp = tempfile::tempdir().unwrap();
        for name in [
            "00-secret.key",
            "01-secret.PEM",
            "02-secret.env.local",
            "03-secret.RDP",
            "90-safe.txt",
            "91-safe.txt",
        ] {
            std::fs::write(temp.path().join(name), name).unwrap();
        }

        let cmd = list_command(
            ShellType::Posix,
            temp.path().to_string_lossy().as_ref(),
            true,
            1,
            true,
        );
        let output = std::process::Command::new("sh")
            .args(["-c", &cmd])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        let names = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(parse_entry)
            .map(|entry| entry.name)
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["90-safe.txt", "91-safe.txt"]);
        assert!(names.len() > 1, "the caller should report truncated=true");
    }

    #[test]
    fn fish_list_uses_posix_path() {
        let cmd = list_command(ShellType::Fish, "src", false, 500, true);
        // Fish currently falls through to the Posix find command
        assert!(
            cmd.contains("cd \"$HOME\""),
            "expected cd $HOME for fish, got: {cmd}"
        );
        assert!(cmd.contains("find "), "expected find for fish, got: {cmd}");
    }
}
