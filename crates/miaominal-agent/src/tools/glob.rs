use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::path_guard::{cd_prefix, resolve_workspace_path, shell_quote};
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
    let st = channel.shell_type();

    let command = match st {
        ShellType::Posix | ShellType::Fish => {
            glob_posix_command(&root, &name_pattern, max_results, args.include_hidden, st)
        }
        ShellType::PowerShell => {
            glob_powershell_command(&root, &name_pattern, max_results, args.include_hidden)
        }
        ShellType::Cmd => {
            glob_cmd_command(&root, &name_pattern, max_results, args.include_hidden)
        }
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
) -> String {
    let quoted_root = shell_quote(root, st);
    let quoted_pattern = shell_quote(pattern, st);
    let hidden_filter = if include_hidden {
        String::new()
    } else {
        " | awk -F/ '{ for (i=1; i<=NF; i++) if ($i ~ /^\\./) next; print }'".to_string()
    };
    let max = max_results + 1;
    match st {
        ShellType::Posix => format!(
            "cd \"$HOME\" && find {root} -type f -name {pattern} -print{hidden_filter} \
             | sed 's#^./##' | sort | head -n {max}",
            root = quoted_root,
            pattern = quoted_pattern,
            hidden_filter = hidden_filter,
            max = max,
        ),
        ShellType::Fish => format!(
            "cd \"$HOME\"; and find {root} -type f -name {pattern} -print{hidden_filter} \
             | sed 's#^./##' | sort | head -n {max}",
            root = quoted_root,
            pattern = quoted_pattern,
            hidden_filter = hidden_filter,
            max = max,
        ),
        _ => unreachable!(),
    }
}

fn glob_powershell_command(
    root: &str,
    pattern: &str,
    max_results: usize,
    include_hidden: bool,
) -> String {
    let st = ShellType::PowerShell;
    let cd = cd_prefix(st, root);
    let quoted_pattern = shell_quote(pattern, st);
    let max = max_results + 1;
    let hidden_clause = if include_hidden {
        String::new()
    } else {
        " | Where-Object { ($_.FullName -split '[\\\\/]') -notmatch '^\\.' }".to_string()
    };
    let ps_script = format!(
        "{cd}; Get-ChildItem -Recurse -Filter {pattern} -File -ErrorAction SilentlyContinue{hidden_clause} \
         | ForEach-Object {{ $_.FullName.Replace((Get-Location).Path + '\\', '').Replace('\\', '/') }} \
         | Sort-Object | Select-Object -First {max}",
        cd = cd,
        pattern = quoted_pattern,
        hidden_clause = hidden_clause,
        max = max,
    );
    format!("powershell.exe -NoProfile -Command \"{ps_script}\"")
}

fn glob_cmd_command(root: &str, pattern: &str, _max_results: usize, include_hidden: bool) -> String {
    let st = ShellType::Cmd;
    let cd = cd_prefix(st, root);
    // CMD shell_quote strips double-quotes and doubles percents;
    // the pattern is already safe (no % or " from find_name_pattern output).
    let quoted_pattern = shell_quote(pattern, st);
    let attr = if include_hidden {
        "/a:-d"
    } else {
        "/a:-d-h"
    };
    // dir /s /b outputs relative paths. 2>nul suppresses "File Not Found".
    // We deliberately omit | head so that all results flow through Rust-side truncation
    // (CMD has no built-in equivalent to `head`).
    format!(
        "{cd} && dir {attr} /s /b {pattern} 2>nul",
        cd = cd,
        attr = attr,
        pattern = quoted_pattern,
    )
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
        let cmd = glob_powershell_command(".", "*.rs", 200, false);
        assert!(cmd.contains("Set-Location $env:USERPROFILE"));
        assert!(cmd.contains("Get-ChildItem -Recurse -Filter '*.rs' -File"));
        assert!(cmd.contains("Where-Object"));
        assert!(cmd.contains("notmatch"));
        assert!(cmd.contains("Select-Object -First 201"));
    }

    #[test]
    fn powershell_glob_command_with_hidden() {
        let cmd = glob_powershell_command("src", "*.toml", 50, true);
        assert!(cmd.contains("Get-ChildItem -Recurse -Filter '*.toml' -File"));
        assert!(!cmd.contains("Where-Object"));
        assert!(cmd.contains("Select-Object -First 51"));
        assert!(cmd.contains("ForEach-Object"));
        assert!(cmd.contains("Sort-Object"));
    }

    // ── cmd_glob_command ──

    #[test]
    fn cmd_glob_command_without_hidden() {
        let cmd = glob_cmd_command(".", "*.rs", 200, false);
        assert!(cmd.contains("cd /d %USERPROFILE%"));
        assert!(cmd.contains("dir /a:-d-h /s /b *.rs"));
        assert!(cmd.contains("2>nul"));
    }

    #[test]
    fn cmd_glob_command_with_hidden() {
        let cmd = glob_cmd_command("src", "*", 100, true);
        assert!(cmd.contains("cd /d %USERPROFILE%"));
        assert!(cmd.contains("dir /a:-d /s /b * 2>nul"));
        assert!(!cmd.contains("-d-h"));
    }

    // ── posix_glob_command ──

    #[test]
    fn posix_glob_command_without_hidden() {
        let cmd = glob_posix_command(".", "*.rs", 200, false, ShellType::Posix);
        assert!(cmd.starts_with("cd \"$HOME\" && find"));
        assert!(cmd.contains("-name '*.rs'"));
        assert!(cmd.contains("awk"));
        assert!(cmd.contains("head -n 201"));
    }

    #[test]
    fn posix_glob_command_with_hidden() {
        let cmd = glob_posix_command("src", "*.toml", 50, true, ShellType::Posix);
        assert!(cmd.contains("find 'src' -type f -name '*.toml'"));
        assert!(!cmd.contains("awk"));
    }

    #[test]
    fn fish_glob_command_uses_semicolon_and() {
        let cmd = glob_posix_command(".", "*.rs", 200, false, ShellType::Fish);
        assert!(cmd.starts_with("cd \"$HOME\"; and find"));
        assert!(cmd.contains("; and "));
    }
}

