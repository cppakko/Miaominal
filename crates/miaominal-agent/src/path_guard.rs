use crate::error::{AgentError, AgentResult};
use miaominal_core::profile::ShellType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemotePathKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedRemotePath {
    path: String,
}

impl AuthorizedRemotePath {
    pub fn new(path: String) -> Self {
        Self { path }
    }

    pub fn as_str(&self) -> &str {
        &self.path
    }
}

pub fn resolve_workspace_path(path: &str) -> AgentResult<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "." {
        return Ok(".".into());
    }
    if trimmed.starts_with('~') {
        return Err(AgentError::InvalidPath(
            "home expansion is outside the agent workspace policy".into(),
        ));
    }
    if trimmed.contains('\0') || trimmed.contains('\n') || trimmed.contains('\r') {
        return Err(AgentError::InvalidPath(
            "path contains unsupported control characters".into(),
        ));
    }

    let normalized = trimmed.replace('\\', "/");
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                return Err(AgentError::InvalidPath(
                    "`..` segments cannot escape the agent workspace".into(),
                ));
            }
            part => parts.push(part),
        }
    }

    let prefix = if normalized.starts_with('/') { "/" } else { "" };
    if parts.is_empty() {
        Ok(".".into())
    } else if prefix.is_empty() {
        Ok(parts.join("/"))
    } else {
        Ok(format!("/{parts}", parts = parts.join("/")))
    }
}

/// Quote a string for use inside a shell command, dispatching on shell type.
///
/// Uses the exact quoting logic from `miaominal-ssh/src/ssh/session.rs`.
pub fn shell_quote(value: &str, shell_type: ShellType) -> String {
    match shell_type {
        ShellType::Posix => shell_quote_posix(value),
        ShellType::Fish => shell_quote_fish(value),
        ShellType::PowerShell => shell_quote_powershell(value),
        ShellType::Cmd => {
            panic!("CMD does not have a context-independent quoting rule; use encoded PowerShell")
        }
    }
}

fn shell_quote_posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn shell_quote_fish(value: &str) -> String {
    // Fish single-quoted strings disable variable and command substitution.
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn shell_quote_powershell(value: &str) -> String {
    // PowerShell single-quoted strings: escape ' by doubling it.
    format!("'{}'", value.replace('\'', "''"))
}

pub fn canonical_path_for_shell(path: &str, shell_type: ShellType) -> AgentResult<String> {
    if path.is_empty()
        || path
            .chars()
            .any(|character| matches!(character, '\0' | '\r' | '\n'))
    {
        return Err(AgentError::InvalidPath(
            "canonical path contains unsupported control characters".into(),
        ));
    }

    if !matches!(shell_type, ShellType::PowerShell | ShellType::Cmd) {
        return Ok(path.to_string());
    }

    let normalized = path.replace('\\', "/");
    let bytes = normalized.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/' {
        return Ok(normalized);
    }
    if bytes.len() >= 4
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
        && bytes[3] == b'/'
    {
        return Ok(normalized[1..].to_string());
    }
    if normalized.starts_with("//") {
        return Ok(normalized.replace('/', "\\"));
    }

    Err(AgentError::InvalidPath(format!(
        "SFTP canonical path `{path}` cannot be represented by the Windows exec shell"
    )))
}

// ── Prefix builders ──

/// Generate a `cd` prefix that changes to `$HOME` then to `cwd`.
pub fn cd_prefix(shell_type: ShellType, cwd: &str) -> String {
    let quoted = shell_quote(cwd, shell_type);
    match shell_type {
        ShellType::Posix => format!("cd \"$HOME\" && cd {quoted}"),
        ShellType::Fish => format!("cd \"$HOME\"; and cd {quoted}"),
        ShellType::PowerShell => {
            format!(
                "Set-Location -LiteralPath $env:USERPROFILE; Set-Location -LiteralPath {quoted}"
            )
        }
        ShellType::Cmd => {
            panic!("CMD working directories must use a validated literal or encoded PowerShell")
        }
    }
}

/// Environment variable setup for pager-less, locale-safe, non-interactive execution.
pub fn env_setup(shell_type: ShellType) -> String {
    match shell_type {
        ShellType::Posix => "export PAGER=cat SYSTEMD_PAGER= GIT_PAGER=cat LESS= LANG=C.UTF-8 \
             NO_COLOR=1 CLICOLOR=0 TERM=xterm-256color"
            .into(),
        ShellType::Fish => "set -x PAGER cat; set -x SYSTEMD_PAGER \"\"; set -x GIT_PAGER cat; \
             set -x LESS \"\"; set -x LANG C.UTF-8; set -x NO_COLOR 1; \
             set -x CLICOLOR 0; set -x TERM xterm-256color"
            .into(),
        ShellType::PowerShell => "$env:PAGER='cat'; $env:SYSTEMD_PAGER=''; $env:GIT_PAGER='cat'; \
             $env:LESS=''; $env:LANG='C.UTF-8'; $env:NO_COLOR='1'; \
             $env:CLICOLOR='0'; $env:TERM='xterm-256color'"
            .into(),
        ShellType::Cmd => "SET \"PAGER=cat\" & SET \"SYSTEMD_PAGER=\" & SET \"GIT_PAGER=cat\" & \
             SET \"LESS=\" & SET \"LANG=C.UTF-8\" & SET \"NO_COLOR=1\" & \
             SET \"CLICOLOR=0\" & SET \"TERM=xterm-256color\""
            .into(),
    }
}

/// Generate a command that creates a temporary file and prints its path.
#[allow(dead_code)]
pub fn temp_file(shell_type: ShellType) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => "mktemp".into(),
        ShellType::PowerShell => "(New-TemporaryFile).FullName".into(),
        ShellType::Cmd => "%TEMP%\\miaominal-%RANDOM%.tmp".into(),
    }
}

/// Generate a command that reads the first `max` bytes from `file_var`.
///
/// `file_var` should already be shell-quoted or be a shell variable reference
/// (e.g., `"$tmp"`, `$out`). Returns empty string for CMD (no built-in equivalent).
#[allow(dead_code)]
pub fn head_bytes_cmd(shell_type: ShellType, file_var: &str, max: usize) -> String {
    match shell_type {
        ShellType::Posix | ShellType::Fish => format!("head -c {max} {file_var}"),
        ShellType::PowerShell => {
            format!("[System.IO.File]::ReadAllBytes({file_var})[0..{max}]")
        }
        ShellType::Cmd => String::new(),
    }
}

/// Return the shell variable to read the last exit code.
#[allow(dead_code)]
pub fn exit_code_var(shell_type: ShellType) -> &'static str {
    match shell_type {
        ShellType::Posix | ShellType::Fish => "$?",
        ShellType::PowerShell => "$LASTEXITCODE",
        ShellType::Cmd => "%ERRORLEVEL%",
    }
}

/// Return the line-ending separator between chained commands.
#[allow(dead_code)]
pub fn line_ending(shell_type: ShellType) -> &'static str {
    match shell_type {
        ShellType::Posix | ShellType::Fish => "\n",
        ShellType::PowerShell | ShellType::Cmd => "\r",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_are_normalized() {
        assert_eq!(
            resolve_workspace_path("./src//main.rs").unwrap(),
            "src/main.rs"
        );
        assert_eq!(resolve_workspace_path(".").unwrap(), ".");
    }

    #[test]
    fn parent_paths_are_rejected_but_absolute_paths_normalize() {
        assert!(resolve_workspace_path("../secret").is_err());
        assert!(resolve_workspace_path("src/../../secret").is_err());
        assert_eq!(resolve_workspace_path("/etc//nginx").unwrap(), "/etc/nginx");
        assert!(resolve_workspace_path("~/secret").is_err());
    }

    #[test]
    fn windows_backslash_parent_traversal_rejected() {
        assert!(resolve_workspace_path("..\\secret").is_err());
        assert!(resolve_workspace_path("src\\..\\..\\secret").is_err());
        assert!(resolve_workspace_path("..\\..\\Windows\\System32").is_err());
    }

    #[test]
    fn windows_backslash_paths_normalized() {
        assert_eq!(
            resolve_workspace_path("src\\main.rs").unwrap(),
            "src/main.rs"
        );
        assert_eq!(
            resolve_workspace_path("src\\lib\\mod.rs").unwrap(),
            "src/lib/mod.rs"
        );
    }

    // ── shell_quote tests ──

    #[test]
    fn posix_quote_wraps_in_single_quotes() {
        assert_eq!(shell_quote("hello", ShellType::Posix), "'hello'");
        assert_eq!(shell_quote("foo bar", ShellType::Posix), "'foo bar'");
    }

    #[test]
    fn posix_quote_escapes_single_quote() {
        assert_eq!(shell_quote("it's ok", ShellType::Posix), "'it'\"'\"'s ok'");
    }

    #[test]
    fn fish_quote_wraps_in_single_quotes() {
        assert_eq!(shell_quote("hello", ShellType::Fish), "'hello'");
        assert_eq!(shell_quote("foo bar", ShellType::Fish), "'foo bar'");
    }

    #[test]
    fn fish_quote_escapes_backslash_and_single_quote() {
        assert_eq!(shell_quote("path\\to", ShellType::Fish), "'path\\\\to'");
        assert_eq!(
            shell_quote("it's $HOME (whoami)", ShellType::Fish),
            "'it\\'s $HOME (whoami)'"
        );
    }

    #[test]
    fn windows_sftp_paths_convert_to_native_exec_paths() {
        assert_eq!(
            canonical_path_for_shell("/C:/Users/akko/project", ShellType::Cmd).unwrap(),
            "C:/Users/akko/project"
        );
        assert_eq!(
            canonical_path_for_shell("D:/work", ShellType::PowerShell).unwrap(),
            "D:/work"
        );
        assert!(canonical_path_for_shell("/home/akko", ShellType::Cmd).is_err());
    }

    #[test]
    fn powershell_quote_wraps_in_single_quotes() {
        assert_eq!(shell_quote("hello", ShellType::PowerShell), "'hello'");
        assert_eq!(shell_quote("it's ok", ShellType::PowerShell), "'it''s ok'");
    }

    #[test]
    #[should_panic(expected = "context-independent quoting rule")]
    fn cmd_quote_requires_an_encoded_command() {
        let _ = shell_quote("x & whoami", ShellType::Cmd);
    }

    // ── cd_prefix tests ──

    #[test]
    fn cd_prefix_posix() {
        let result = cd_prefix(ShellType::Posix, "/home/user/project");
        assert_eq!(result, "cd \"$HOME\" && cd '/home/user/project'");
    }

    #[test]
    fn cd_prefix_fish() {
        let result = cd_prefix(ShellType::Fish, "/home/user/project");
        assert_eq!(result, "cd \"$HOME\"; and cd '/home/user/project'");
    }

    #[test]
    fn cd_prefix_powershell() {
        let result = cd_prefix(ShellType::PowerShell, "C:\\Users\\user\\project");
        assert_eq!(
            result,
            "Set-Location -LiteralPath $env:USERPROFILE; Set-Location -LiteralPath 'C:\\Users\\user\\project'"
        );
    }

    #[test]
    #[should_panic(expected = "CMD does not have a context-independent quoting rule")]
    fn cd_prefix_cmd_requires_an_encoded_command() {
        let _ = cd_prefix(ShellType::Cmd, "C:\\Users\\user\\project");
    }

    #[test]
    fn cd_prefix_quotes_path_with_spaces() {
        // Posix
        let posix = cd_prefix(ShellType::Posix, "/home/user/my project");
        assert_eq!(posix, "cd \"$HOME\" && cd '/home/user/my project'");
        // Fish
        let fish = cd_prefix(ShellType::Fish, "/home/user/my project");
        assert_eq!(fish, "cd \"$HOME\"; and cd '/home/user/my project'");
        // PowerShell
        let ps = cd_prefix(ShellType::PowerShell, "C:\\Users\\user\\my project");
        assert_eq!(
            ps,
            "Set-Location -LiteralPath $env:USERPROFILE; Set-Location -LiteralPath 'C:\\Users\\user\\my project'"
        );
    }

    // ── env_setup tests ──

    #[test]
    fn env_setup_posix() {
        let result = env_setup(ShellType::Posix);
        assert!(result.starts_with("export "));
        assert!(result.contains("PAGER=cat"));
        assert!(result.contains("SYSTEMD_PAGER="));
        assert!(result.contains("LANG=C.UTF-8"));
        assert!(result.contains("TERM=xterm-256color"));
        assert!(!result.contains(';'));
    }

    #[test]
    fn env_setup_fish() {
        let result = env_setup(ShellType::Fish);
        assert!(result.starts_with("set -x "));
        assert!(result.contains("PAGER cat"));
        assert!(result.contains(";"));
        assert!(result.contains("LANG C.UTF-8"));
    }

    #[test]
    fn env_setup_powershell() {
        let result = env_setup(ShellType::PowerShell);
        assert!(result.starts_with("$env:"));
        assert!(result.contains("PAGER='cat'"));
        assert!(result.contains("LANG='C.UTF-8'"));
        assert!(result.contains(";"));
    }

    #[test]
    fn env_setup_cmd() {
        let result = env_setup(ShellType::Cmd);
        assert!(result.starts_with("SET "));
        assert!(result.contains("PAGER=cat"));
        assert!(result.contains(" & "));
    }

    // ── temp_file tests ──

    #[test]
    fn temp_file_posix() {
        assert_eq!(temp_file(ShellType::Posix), "mktemp");
    }

    #[test]
    fn temp_file_fish() {
        assert_eq!(temp_file(ShellType::Fish), "mktemp");
    }

    #[test]
    fn temp_file_powershell() {
        assert_eq!(
            temp_file(ShellType::PowerShell),
            "(New-TemporaryFile).FullName"
        );
    }

    #[test]
    fn temp_file_cmd() {
        let result = temp_file(ShellType::Cmd);
        assert!(result.starts_with("%TEMP%\\"));
        assert!(result.ends_with(".tmp"));
    }

    // ── head_bytes_cmd tests ──

    #[test]
    fn head_bytes_cmd_posix() {
        assert_eq!(
            head_bytes_cmd(ShellType::Posix, "\"$tmp\"", 4096),
            "head -c 4096 \"$tmp\""
        );
    }

    #[test]
    fn head_bytes_cmd_fish() {
        assert_eq!(
            head_bytes_cmd(ShellType::Fish, "\"$tmp\"", 4096),
            "head -c 4096 \"$tmp\""
        );
    }

    #[test]
    fn head_bytes_cmd_powershell() {
        let result = head_bytes_cmd(ShellType::PowerShell, "$tmp", 4096);
        assert!(result.contains("ReadAllBytes"));
        assert!(result.contains("4096"));
    }

    #[test]
    fn head_bytes_cmd_cmd() {
        assert_eq!(head_bytes_cmd(ShellType::Cmd, "", 4096), "");
    }

    // ── exit_code_var tests ──

    #[test]
    fn exit_code_var_posix() {
        assert_eq!(exit_code_var(ShellType::Posix), "$?");
    }

    #[test]
    fn exit_code_var_fish() {
        assert_eq!(exit_code_var(ShellType::Fish), "$?");
    }

    #[test]
    fn exit_code_var_powershell() {
        assert_eq!(exit_code_var(ShellType::PowerShell), "$LASTEXITCODE");
    }

    #[test]
    fn exit_code_var_cmd() {
        assert_eq!(exit_code_var(ShellType::Cmd), "%ERRORLEVEL%");
    }

    // ── line_ending tests ──

    #[test]
    fn line_ending_posix() {
        assert_eq!(line_ending(ShellType::Posix), "\n");
    }

    #[test]
    fn line_ending_fish() {
        assert_eq!(line_ending(ShellType::Fish), "\n");
    }

    #[test]
    fn line_ending_powershell() {
        assert_eq!(line_ending(ShellType::PowerShell), "\r");
    }

    #[test]
    fn line_ending_cmd() {
        assert_eq!(line_ending(ShellType::Cmd), "\r");
    }
}
