use crate::channel::{AgentExecChannel, DEFAULT_MAX_OUTPUT_BYTES, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::path_guard::{env_setup, resolve_workspace_path, shell_quote};
use miaominal_core::profile::ShellType;
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

    let st = channel.shell_type();
    let command = match st {
        ShellType::Posix | ShellType::Fish => posix_read_command(&path, start, end, max_bytes, st),
        ShellType::PowerShell => powershell_read_command(&path, start, end, max_bytes),
        ShellType::Cmd => cmd_read_command(&path, start, end, max_bytes),
    };

    let output = channel.exec(command).await?;
    Ok(ToolOutput::Text {
        truncated: output.contains("[MIAOMINAL_TRUNCATED]"),
        content: output.replace("\n[MIAOMINAL_TRUNCATED]", ""),
    })
}

// ── Command builders ──

/// Build the POSIX/Fish read command (unchanged from original logic).
fn posix_read_command(
    path: &str,
    start: usize,
    end: usize,
    max_bytes: usize,
    st: ShellType,
) -> String {
    let quoted = shell_quote(path, st);
    format!(
        "cd \"$HOME\" && if [ -f {path} ]; then \
         tmp=$(mktemp); sed -n '{start},{end}p' {path} >\"$tmp\"; \
         bytes=$(wc -c <\"$tmp\"); head -c {max} \"$tmp\"; rm -f \"$tmp\"; \
         if [ \"$bytes\" -gt {max} ]; then printf '\\n[MIAOMINAL_TRUNCATED]'; fi; \
         else printf 'not a regular file: %s' {path} >&2; exit 1; fi",
        path = quoted,
        start = start,
        end = end,
        max = max_bytes,
    )
}

/// Build a PowerShell read command using `Get-Content` for line ranges
/// and `[System.Text.Encoding]::UTF8` for byte-level truncation.
///
/// Output format matches POSIX: content followed by `\n[MIAOMINAL_TRUNCATED]`
/// when truncated, no trailing newline otherwise.
fn powershell_read_command(path: &str, start: usize, end: usize, max_bytes: usize) -> String {
    let quoted = shell_quote(path, ShellType::PowerShell);
    let env = env_setup(ShellType::PowerShell);
    let lines_to_read = end;
    let skip = start.saturating_sub(1);
    let take = end - start + 1;
    let max_byte_index = max_bytes.saturating_sub(1);
    let ps_script = format!(
        "{env}; \
         if (-not (Test-Path -LiteralPath {path} -PathType Leaf)) {{ \
         Write-Error ('not a regular file: ' + {path}); exit 1 }}; \
         $content = Get-Content -LiteralPath {path} -TotalCount {lines_to_read} \
         | Select-Object -Skip {skip} -First {take}; \
         $text = $content -join [char]10; \
         $bytes = [System.Text.Encoding]::UTF8.GetByteCount($text); \
         if ($bytes -gt {max}) {{ \
           $raw = [System.Text.Encoding]::UTF8.GetBytes($text); \
           $truncated = [System.Text.Encoding]::UTF8.GetString($raw[0..{max_byte_index}]); \
           [Console]::Write($truncated + [char]10 + '[MIAOMINAL_TRUNCATED]') \
         }} else {{ [Console]::Write($text) }}",
        env = env,
        path = quoted,
        lines_to_read = lines_to_read,
        skip = skip,
        take = take,
        max = max_bytes,
        max_byte_index = max_byte_index,
    );
    format!("powershell.exe -NoProfile -Command \"{ps_script}\"")
}

/// Build a CMD read command using PowerShell as fallback.
///
/// CMD sessions spawn `powershell.exe -NoProfile -Command "..."` to execute
/// the same PowerShell read logic. Single-quoted strings inside the script
/// pass cleanly through CMD's double-quote argument wrapping.
fn cmd_read_command(path: &str, start: usize, end: usize, max_bytes: usize) -> String {
    let ps_quoted = shell_quote(path, ShellType::PowerShell);
    let env = env_setup(ShellType::PowerShell);
    let lines_to_read = end;
    let skip = start.saturating_sub(1);
    let take = end - start + 1;
    let max_byte_index = max_bytes.saturating_sub(1);
    format!(
        "powershell.exe -NoProfile -Command \"{env}; \
         if (-not (Test-Path -LiteralPath {path} -PathType Leaf)) {{ \
         Write-Error 'not a regular file: {path}'; exit 1 }}; \
         $content = Get-Content -LiteralPath {path} -TotalCount {lines_to_read} \
         | Select-Object -Skip {skip} -First {take}; \
         $text = $content -join [char]10; \
         $bytes = [System.Text.Encoding]::UTF8.GetByteCount($text); \
         if ($bytes -gt {max}) {{ \
           $raw = [System.Text.Encoding]::UTF8.GetBytes($text); \
           $truncated = [System.Text.Encoding]::UTF8.GetString($raw[0..{max_byte_index}]); \
           [Console]::Write($truncated + [char]10 + '[MIAOMINAL_TRUNCATED]') \
         }} else {{ [Console]::Write($text) }}\"",
        env = env,
        path = ps_quoted,
        lines_to_read = lines_to_read,
        skip = skip,
        take = take,
        max = max_bytes,
        max_byte_index = max_byte_index,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── POSIX regression ──

    #[test]
    fn posix_read_command_unchanged() {
        let cmd = posix_read_command("/home/user/file.txt", 5, 25, 4096, ShellType::Posix);
        assert_eq!(
            cmd,
            "cd \"$HOME\" && if [ -f '/home/user/file.txt' ]; then \
             tmp=$(mktemp); sed -n '5,25p' '/home/user/file.txt' >\"$tmp\"; \
             bytes=$(wc -c <\"$tmp\"); head -c 4096 \"$tmp\"; rm -f \"$tmp\"; \
             if [ \"$bytes\" -gt 4096 ]; then printf '\\n[MIAOMINAL_TRUNCATED]'; fi; \
             else printf 'not a regular file: %s' '/home/user/file.txt' >&2; exit 1; fi"
        );
    }

    #[test]
    fn posix_read_command_handles_single_quote_in_path() {
        let cmd = posix_read_command("/home/user/it's file.txt", 1, 10, 1024, ShellType::Posix);
        // POSIX quoting wraps the path so single quotes don't break the shell
        // The path should appear quoted (not raw with unescaped single quotes)
        assert!(cmd.contains("it"), "path fragment missing");
        assert!(cmd.contains("s file.txt"), "path fragment missing");
        // Single quote should be escaped via '"'"' pattern
        assert!(
            cmd.contains("\"'\"'"),
            "expected POSIX single-quote escape pattern"
        );
    }

    #[test]
    fn fish_read_command_uses_fish_quoting() {
        let cmd = posix_read_command("/home/user/project", 1, 5, 2048, ShellType::Fish);
        // Fish uses double-quote wrapping
        assert!(cmd.contains("\"/home/user/project\""));
    }

    // ── PowerShell command generation ──

    #[test]
    fn powershell_read_command_generation() {
        let cmd = powershell_read_command("C:\\Users\\akko\\file.txt", 1, 10, 65536);
        // Must contain PowerShell env setup
        assert!(cmd.contains("$env:PAGER='cat'"));
        assert!(cmd.contains("$env:LANG='C.UTF-8'"));
        // Must use Test-Path for regular file check
        assert!(cmd.contains("Test-Path -LiteralPath"));
        assert!(cmd.contains("-PathType Leaf"));
        // Must use Get-Content with line range
        assert!(cmd.contains("Get-Content -LiteralPath"));
        assert!(cmd.contains("-TotalCount 10"));
        assert!(cmd.contains("Select-Object -Skip 0 -First 10"));
        // Must use UTF-8 byte-level truncation
        assert!(cmd.contains("[System.Text.Encoding]::UTF8.GetByteCount"));
        assert!(cmd.contains("[System.Text.Encoding]::UTF8.GetBytes"));
        assert!(cmd.contains("[System.Text.Encoding]::UTF8.GetString"));
        // Must output truncated marker matching POSIX format
        assert!(cmd.contains("[MIAOMINAL_TRUNCATED]"));
        assert!(cmd.contains("[Console]::Write"));
        // Path must be PowerShell single-quoted
        assert!(cmd.contains("'C:\\Users\\akko\\file.txt'"));
    }

    #[test]
    fn powershell_read_command_handles_single_quote_in_path() {
        let cmd = powershell_read_command("C:\\Users\\akko\\it's file.txt", 1, 10, 65536);
        assert!(cmd.contains("'C:\\Users\\akko\\it''s file.txt'"));
    }

    #[test]
    fn powershell_read_command_skip_calculation() {
        // start=5, end=25 → skip=4, take=21
        let cmd = powershell_read_command("C:\\f.txt", 5, 25, 4096);
        assert!(cmd.contains("-TotalCount 25"));
        assert!(cmd.contains("Select-Object -Skip 4 -First 21"));
    }

    #[test]
    fn powershell_read_command_start_at_1() {
        // start=1, end=200 → skip=0, take=200
        let cmd = powershell_read_command("C:\\f.txt", 1, 200, 65536);
        assert!(cmd.contains("-TotalCount 200"));
        assert!(cmd.contains("Select-Object -Skip 0 -First 200"));
    }

    // ── CMD command generation ──

    #[test]
    fn cmd_read_command_generation() {
        let cmd = cmd_read_command("C:\\Users\\akko\\file.txt", 1, 10, 65536);
        // Must wrap in powershell.exe
        assert!(cmd.starts_with("powershell.exe -NoProfile -Command \""));
        assert!(cmd.ends_with('"'));
        // Must contain PowerShell env setup
        assert!(cmd.contains("$env:PAGER='cat'"));
        // Must use Test-Path
        assert!(cmd.contains("Test-Path -LiteralPath"));
        // Must use single-quoted error message (CMD-safe)
        assert!(cmd.contains("'not a regular file: "));
        // Must use Get-Content
        assert!(cmd.contains("Get-Content -LiteralPath"));
        // Must use [char]10 for newline (CMD-safe, no embedded double-quotes)
        assert!(cmd.contains("$content -join [char]10"));
        // Must output marker
        assert!(cmd.contains("[MIAOMINAL_TRUNCATED]"));
        // Path must be PowerShell single-quoted
        assert!(cmd.contains("'C:\\Users\\akko\\file.txt'"));
    }

    #[test]
    fn cmd_read_command_handles_single_quote_in_path() {
        let cmd = cmd_read_command("C:\\it's\\file.txt", 1, 5, 1024);
        assert!(cmd.contains("'C:\\it''s\\file.txt'"));
    }

    // ── All shell types produce valid commands ──

    #[test]
    fn all_shell_types_produce_non_empty_commands() {
        for (st, path) in [
            (ShellType::Posix, "/tmp/file.txt"),
            (ShellType::Fish, "/tmp/file.txt"),
            (ShellType::PowerShell, "C:\\tmp\\file.txt"),
            (ShellType::Cmd, "C:\\tmp\\file.txt"),
        ] {
            let cmd = match st {
                ShellType::Posix | ShellType::Fish => posix_read_command(path, 1, 10, 4096, st),
                ShellType::PowerShell => powershell_read_command(path, 1, 10, 4096),
                ShellType::Cmd => cmd_read_command(path, 1, 10, 4096),
            };
            assert!(!cmd.is_empty(), "command for {st:?} must not be empty");
        }
    }

    #[test]
    fn all_commands_contain_truncation_marker() {
        for (st, path) in [
            (ShellType::Posix, "/tmp/file.txt"),
            (ShellType::Fish, "/tmp/file.txt"),
            (ShellType::PowerShell, "C:\\tmp\\file.txt"),
            (ShellType::Cmd, "C:\\tmp\\file.txt"),
        ] {
            let cmd = match st {
                ShellType::Posix | ShellType::Fish => posix_read_command(path, 1, 10, 4096, st),
                ShellType::PowerShell => powershell_read_command(path, 1, 10, 4096),
                ShellType::Cmd => cmd_read_command(path, 1, 10, 4096),
            };
            assert!(
                cmd.contains("[MIAOMINAL_TRUNCATED]"),
                "command for {st:?} must contain truncation marker"
            );
        }
    }
}
