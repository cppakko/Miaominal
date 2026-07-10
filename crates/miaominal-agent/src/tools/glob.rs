use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::path_guard::{RemotePathKind, shell_quote};
use crate::policy::{AgentPathAccess, posix_find_sensitive_predicate};
use miaominal_core::profile::ShellType;
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
    if matches!(channel.shell_type(), ShellType::PowerShell | ShellType::Cmd) {
        super::workspace_info::ensure_exec_shell_detected(channel).await;
    }

    let root = channel
        .authorize_existing_path(&args.root, AgentPathAccess::Read, RemotePathKind::Directory)
        .await?;
    let root = root.as_str();
    if is_overbroad_root(root) {
        return Err(AgentError::InvalidPath(
            "glob requires a narrowed workspace root".into(),
        ));
    }
    let max_results = args.max_results.unwrap_or(200);
    let name_pattern = find_name_pattern(&args.pattern);
    let st = channel.shell_type();

    let command = match st {
        ShellType::Posix | ShellType::Fish => glob_posix_command(
            root,
            &name_pattern,
            max_results,
            args.include_hidden,
            st,
            !channel.policy_bypass_enabled(),
        ),
        ShellType::PowerShell | ShellType::Cmd => glob_windows_command(
            st,
            root,
            &name_pattern,
            max_results,
            args.include_hidden,
            !channel.policy_bypass_enabled(),
        ),
    };

    let output = channel.exec(command).await?;
    let mut entries = output.lines().map(str::to_string).collect::<Vec<_>>();
    let truncated = entries.len() > max_results;
    entries.truncate(max_results);
    Ok(ToolOutput::List { entries, truncated })
}

// ── Shell-specific glob command builders ──

fn glob_posix_command(
    root: &str,
    pattern: &str,
    max_results: usize,
    include_hidden: bool,
    st: ShellType,
    guard_sensitive: bool,
) -> String {
    let quoted_root = shell_quote(root, st);
    let quoted_pattern = shell_quote(pattern, st);
    let hidden_filter = if include_hidden {
        String::new()
    } else {
        " | awk -F/ '{ for (i=1; i<=NF; i++) if ($i ~ /^\\./) next; print }'".to_string()
    };
    let sensitive_guard = if guard_sensitive {
        format!("{} -prune -o -type f", posix_find_sensitive_predicate())
    } else {
        "-type f".to_string()
    };
    let max = max_results + 1;
    match st {
        ShellType::Posix => format!(
            "cd \"$HOME\" && find -P {root} {sensitive_guard} -name {pattern} -print{hidden_filter} \
             | sed 's#^./##' | sort | head -n {max}",
            root = quoted_root,
            pattern = quoted_pattern,
            hidden_filter = hidden_filter,
            max = max,
        ),
        ShellType::Fish => format!(
            "cd \"$HOME\"; and find -P {root} {sensitive_guard} -name {pattern} -print{hidden_filter} \
             | sed 's#^./##' | sort | head -n {max}",
            root = quoted_root,
            pattern = quoted_pattern,
            hidden_filter = hidden_filter,
            max = max,
        ),
        _ => unreachable!(),
    }
}

fn glob_windows_command(
    shell_type: ShellType,
    root: &str,
    pattern: &str,
    max_results: usize,
    include_hidden: bool,
    guard_sensitive: bool,
) -> String {
    let quoted_root = shell_quote(root, ShellType::PowerShell);
    let quoted_pattern = shell_quote(pattern, ShellType::PowerShell);
    let max = max_results + 1;
    let ps_script = format!(
        "{sensitive_function}; $root={root}; $pattern={pattern}; $includeHidden=${include_hidden}; $guardSensitive=${guard_sensitive}; $max={max}; $stack=[Collections.Generic.Stack[string]]::new(); $results=[Collections.Generic.List[string]]::new(); $stack.Push($root); while($stack.Count -gt 0 -and $results.Count -lt $max){{ $dir=$stack.Pop(); foreach($item in @(Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue)){{ if(-not $includeHidden -and $item.Name.StartsWith('.')){{continue}}; if(($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0){{continue}}; if($guardSensitive -and (Test-MiaominalSensitivePath $item.FullName)){{continue}}; if($item.PSIsContainer){{$stack.Push($item.FullName); continue}}; if($item.Name -like $pattern){{$relative=$item.FullName.Substring($root.TrimEnd('\\','/').Length).TrimStart('\\','/').Replace('\\','/'); $results.Add($relative)}} }} }}; $results | Sort-Object | Select-Object -First $max",
        sensitive_function = super::windows::powershell_sensitive_path_function(),
        root = quoted_root,
        pattern = quoted_pattern,
        include_hidden = if include_hidden { "true" } else { "false" },
        guard_sensitive = if guard_sensitive { "true" } else { "false" },
        max = max,
    );
    super::windows::powershell_command_for_shell(shell_type, &ps_script)
}

// ── Helpers ──

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

    // ── find_name_pattern ──

    #[test]
    fn extracts_find_name_pattern_from_globstar() {
        assert_eq!(find_name_pattern("**/*.conf"), "*.conf");
        assert_eq!(
            find_name_pattern("docker-compose*.yml"),
            "docker-compose*.yml"
        );
    }

    // ── is_overbroad_root ──

    #[test]
    fn rejects_overbroad_roots() {
        assert!(is_overbroad_root("/"));
        assert!(is_overbroad_root("/home"));
        assert!(!is_overbroad_root("/var/log/nginx"));
    }

    // ── powershell_glob_command ──

    #[test]
    fn powershell_glob_command_without_hidden() {
        let command =
            glob_windows_command(ShellType::PowerShell, "C:/work", "*.rs", 200, false, true);
        let cmd = decode_powershell_command(&command);
        assert!(cmd.contains("Get-ChildItem -LiteralPath $dir -Force"));
        assert!(cmd.contains("ReparsePoint"));
        assert!(cmd.contains("Test-MiaominalSensitivePath"));
        assert!(cmd.contains("$pattern='*.rs'"));
        assert!(cmd.contains("$max=201"));
    }

    #[test]
    fn powershell_glob_command_with_hidden() {
        let command =
            glob_windows_command(ShellType::PowerShell, "C:/work", "*.toml", 50, true, false);
        let cmd = decode_powershell_command(&command);
        assert!(cmd.contains("$includeHidden=$true"));
        assert!(cmd.contains("$guardSensitive=$false"));
        assert!(cmd.contains("$max=51"));
    }

    // ── cmd_glob_command ──

    #[test]
    fn cmd_glob_command_without_hidden() {
        let cmd = glob_windows_command(
            ShellType::Cmd,
            "C:/x & whoami & rem",
            "*.rs & whoami",
            200,
            false,
            true,
        );
        assert!(cmd.starts_with("set MIAOMINAL_AGENT_PS_GZIP="));
        assert!(!cmd.contains("whoami"));
    }

    #[test]
    fn cmd_glob_command_with_hidden() {
        let cmd = glob_windows_command(ShellType::Cmd, "C:/src", "*", 100, true, false);
        assert!(cmd.contains("powershell.exe -NoProfile -EncodedCommand "));
    }

    #[cfg(windows)]
    #[test]
    fn cmd_glob_prunes_sensitive_tree_and_does_not_inject() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("x & echo MIAOMINAL_INJECTED & rem");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("safe.rs"), "fn safe() {}").unwrap();
        std::fs::create_dir(root.join(".ssh")).unwrap();
        std::fs::write(root.join(".ssh").join("secret.rs"), "private").unwrap();
        let outside = temp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("linked-secret.rs"), "private").unwrap();
        let junction = root.join("linked");
        let junction_command = format!(
            r#"mklink /J "{}" "{}""#,
            junction.display(),
            outside.display()
        );
        let junction_created = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", &junction_command])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);

        let command = glob_windows_command(
            ShellType::Cmd,
            root.to_string_lossy().as_ref(),
            "*.rs",
            100,
            true,
            true,
        );
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", &command])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert!(stdout.contains("safe.rs"));
        assert!(!stdout.contains("secret.rs"));
        if junction_created {
            assert!(!stdout.contains("linked-secret.rs"));
        }
        assert!(!stdout.contains("MIAOMINAL_INJECTED"));
    }

    // ── posix_glob_command ──

    #[test]
    fn posix_glob_command_without_hidden() {
        let cmd = glob_posix_command(".", "*.rs", 200, false, ShellType::Posix, true);
        assert!(cmd.starts_with("cd \"$HOME\" && find -P"));
        assert!(cmd.contains("-name '*.rs'"));
        assert!(cmd.contains("-iname '.ssh'"));
        assert!(cmd.contains("-ipath '/etc/shadow'"));
        assert!(cmd.contains("-ipath '/etc/sudoers'"));
        assert!(cmd.contains("-iname '*.env.*'"));
        assert!(cmd.contains("-iname '*.rdp'"));
        assert!(cmd.contains("-iname '*.kdbx'"));
        assert!(cmd.contains("awk"));
        assert!(cmd.contains("head -n 201"));
    }

    #[test]
    fn posix_glob_command_with_hidden() {
        let cmd = glob_posix_command("src", "*.toml", 50, true, ShellType::Posix, false);
        assert!(cmd.contains("find -P 'src' -type f -name '*.toml'"));
        assert!(!cmd.contains("awk"));
    }

    #[test]
    fn fish_glob_command_uses_semicolon_and() {
        let cmd = glob_posix_command(".", "*.rs", 200, false, ShellType::Fish, true);
        assert!(cmd.starts_with("cd \"$HOME\"; and find -P"));
        assert!(cmd.contains("; and "));
    }
}
