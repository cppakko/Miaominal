use crate::backend::{BackendRoute, ExecMode};
use crate::channel::{AgentExecChannel, DEFAULT_MAX_OUTPUT_BYTES, ShellCommandResult, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::path_guard::{RemotePathKind, cd_prefix, env_setup, shell_quote};
use crate::policy::AgentPathAccess;
use anyhow::anyhow;
use miaominal_core::profile::ShellType;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct RunShellArgs {
    pub command: String,
    pub cwd: Option<String>,
    pub timeout_seconds: Option<u64>,
    pub max_bytes: Option<usize>,
    pub shell: Option<String>,
}

pub async fn run_shell(channel: &AgentExecChannel, args: RunShellArgs) -> AgentResult<ToolOutput> {
    if matches!(channel.shell_type(), ShellType::PowerShell | ShellType::Cmd) {
        super::workspace_info::ensure_exec_shell_detected(channel).await;
    }

    let cwd = channel
        .authorize_existing_path(
            args.cwd.as_deref().unwrap_or("."),
            AgentPathAccess::Read,
            RemotePathKind::Directory,
        )
        .await?;
    let cwd = cwd.as_str().to_string();
    let timeout_secs = args.timeout_seconds.unwrap_or(20).max(1);
    let max_bytes = args.max_bytes.unwrap_or(DEFAULT_MAX_OUTPUT_BYTES);
    let explicit_shell = args.shell.is_some();
    let shell = args.shell.as_deref().unwrap_or(channel.shell_label());
    let st = shell_type_from_label(shell).ok_or_else(|| {
        AgentError::PosixOnly("run_shell v1 supports posix-sh, sh, fish, powershell, or cmd".into())
    })?;
    let sentinel = format!(
        "MIAOMINAL_{:016x}_",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0),
    );

    let exec_path = select_exec_path_for_shell(
        st,
        channel.terminal_exec().is_some(),
        channel.uses_pty(),
        explicit_shell,
        &cwd,
    );
    let command = match st {
        ShellType::PowerShell => {
            build_powershell_non_terminal(&args.command, &cwd, timeout_secs, max_bytes, st)
        }
        ShellType::Cmd => build_cmd_non_terminal(&args.command, &cwd, timeout_secs, max_bytes, st),
        _ => build_posix_non_terminal(&args.command, &cwd, timeout_secs, max_bytes, st),
    };
    let terminal_command = build_terminal_command(exec_path, st, &args.command, &cwd, &sentinel);
    let output = match exec_path {
        ShellExecPath::Terminal => {
            channel
                .exec_via_terminal(
                    terminal_command.expect("terminal path must have a terminal wrapper"),
                    &sentinel,
                    timeout_secs + 5,
                )
                .await?
        }
        ShellExecPath::Pty => {
            channel
                .exec_with_mode(BackendRoute::Pty, command, ExecMode::pty_default())
                .await?
        }
        ShellExecPath::Exec => channel.exec(command).await?,
    };

    let result = if matches!(exec_path, ShellExecPath::Terminal) {
        parse_terminal_shell_result(&output, &sentinel)?
    } else {
        parse_shell_result(&output)?
    };
    Ok(ToolOutput::Shell { result })
}

// ── Command wrapper builders ──

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShellExecPath {
    Terminal,
    Pty,
    Exec,
}

fn select_exec_path(
    terminal_available: bool,
    pty_enabled: bool,
    explicit_shell: bool,
) -> ShellExecPath {
    if explicit_shell {
        ShellExecPath::Exec
    } else if terminal_available {
        ShellExecPath::Terminal
    } else if pty_enabled {
        ShellExecPath::Pty
    } else {
        ShellExecPath::Exec
    }
}

fn select_exec_path_for_shell(
    shell_type: ShellType,
    terminal_available: bool,
    pty_enabled: bool,
    explicit_shell: bool,
    cwd: &str,
) -> ShellExecPath {
    let selected = select_exec_path(terminal_available, pty_enabled, explicit_shell);
    if matches!(shell_type, ShellType::Cmd)
        && matches!(selected, ShellExecPath::Terminal)
        && !cmd_terminal_cwd_is_safe(cwd)
    {
        ShellExecPath::Exec
    } else {
        selected
    }
}

fn build_terminal_command(
    exec_path: ShellExecPath,
    shell_type: ShellType,
    user_command: &str,
    cwd: &str,
    sentinel: &str,
) -> Option<String> {
    if !matches!(exec_path, ShellExecPath::Terminal) {
        return None;
    }

    Some(match shell_type {
        ShellType::PowerShell => build_powershell_terminal(user_command, cwd, sentinel, shell_type),
        ShellType::Cmd => build_cmd_terminal(user_command, cwd, sentinel, shell_type),
        ShellType::Fish => format!(
            "cd \"$HOME\"; and cd {cwd}; and set -x PAGER cat; set -x SYSTEMD_PAGER \"\"; set -x GIT_PAGER cat; set -x LESS \"\"; set -x LANG C.UTF-8; set -x NO_COLOR 1; set -x CLICOLOR 0; set -x TERM xterm-256color; {user_command}; printf '\\n{sentinel}%d:%s\\n' $status $PWD",
            cwd = shell_quote(cwd, shell_type),
        ),
        ShellType::Posix => format!(
            "cd \"$HOME\" && cd {cwd} && export PAGER=cat SYSTEMD_PAGER= GIT_PAGER=cat LESS= LANG=C.UTF-8 NO_COLOR=1 CLICOLOR=0 TERM=xterm-256color; {user_command}; printf '\\n{sentinel}%d:%s\\n' \"$?\" \"$PWD\"",
            cwd = shell_quote(cwd, shell_type),
        ),
    })
}

fn shell_type_from_label(label: &str) -> Option<ShellType> {
    match label {
        "posix-sh" | "sh" => Some(ShellType::Posix),
        "fish" => Some(ShellType::Fish),
        "powershell" => Some(ShellType::PowerShell),
        "cmd" => Some(ShellType::Cmd),
        _ => None,
    }
}

/// Build a POSIX non-terminal wrapper with mktemp, timeout, head -c truncation.
fn build_posix_non_terminal(
    user_command: &str,
    cwd: &str,
    timeout_secs: u64,
    max_bytes: usize,
    shell_type: ShellType,
) -> String {
    format!(
        concat!(
            "cd \"$HOME\" && cd {cwd} && ",
            "export PAGER=cat SYSTEMD_PAGER= GIT_PAGER=cat LESS= LANG=C.UTF-8 NO_COLOR=1 CLICOLOR=0 TERM=xterm-256color; ",
            "out=$(mktemp) && err=$(mktemp) && ",
            "timeout {timeout_secs} sh -lc {user_command} >\"$out\" 2>\"$err\"; ",
            "miaominal_status=$?; ",
            "printf 'MIAOMINAL_STATUS=%s\\n' \"$miaominal_status\"; ",
            "printf 'MIAOMINAL_STDOUT_BEGIN\\n'; head -c {max} \"$out\"; ",
            "printf '\\nMIAOMINAL_STDOUT_END\\n'; ",
            "printf 'MIAOMINAL_STDERR_BEGIN\\n'; head -c {max} \"$err\"; ",
            "printf '\\nMIAOMINAL_STDERR_END\\n'; ",
            "stdout_bytes=$(wc -c <\"$out\"); stderr_bytes=$(wc -c <\"$err\"); ",
            "rm -f \"$out\" \"$err\"; ",
            "if [ \"$stdout_bytes\" -gt {max} ] || [ \"$stderr_bytes\" -gt {max} ]; then ",
            "printf 'MIAOMINAL_TRUNCATED=1\\n'; ",
            "else printf 'MIAOMINAL_TRUNCATED=0\\n'; fi"
        ),
        cwd = shell_quote(cwd, shell_type),
        timeout_secs = timeout_secs,
        user_command = shell_quote(user_command, shell_type),
        max = max_bytes,
    )
}

/// Build a PowerShell non-terminal wrapper.
///
/// Uses an outer PowerShell process to launch the actual shell command with
/// redirected temp files, byte-limited output replay, and a hard timeout.
fn build_powershell_non_terminal(
    user_command: &str,
    cwd: &str,
    timeout_secs: u64,
    max_bytes: usize,
    _shell_type: ShellType,
) -> String {
    build_windows_non_terminal(
        "powershell.exe",
        &["-NoProfile", "-Command", user_command],
        cwd,
        timeout_secs,
        max_bytes,
    )
}

/// Build a PowerShell terminal-mode wrapper (WinkTerm sentinel style).
///
/// Sent directly to the interactive PowerShell PTY — no outer `powershell.exe` wrapper.
fn build_powershell_terminal(
    user_command: &str,
    cwd: &str,
    sentinel: &str,
    shell_type: ShellType,
) -> String {
    let cd = cd_prefix(shell_type, cwd);
    let env = env_setup(shell_type);
    format!(
        "{cd}; {env}; {user_command}; Write-Host `n{sentinel}$LASTEXITCODE:$PWD",
        cd = cd,
        env = env,
        user_command = user_command,
        sentinel = sentinel,
    )
}

/// Build a CMD non-terminal wrapper.
///
/// CMD cannot enforce timeouts or byte caps itself, so an outer PowerShell
/// wrapper launches `cmd.exe` and replays bounded stdout/stderr sentinels.
fn build_cmd_non_terminal(
    user_command: &str,
    cwd: &str,
    timeout_secs: u64,
    max_bytes: usize,
    _shell_type: ShellType,
) -> String {
    build_windows_non_terminal(
        "cmd.exe",
        &["/d", "/c", user_command],
        cwd,
        timeout_secs,
        max_bytes,
    )
}

fn build_windows_non_terminal(
    program: &str,
    arguments: &[&str],
    cwd: &str,
    timeout_secs: u64,
    max_bytes: usize,
) -> String {
    let cd = cd_prefix(ShellType::PowerShell, cwd);
    let env = env_setup(ShellType::PowerShell);
    let quoted_program = shell_quote(program, ShellType::PowerShell);
    let argument_string = windows_command_line_args(arguments);
    let quoted_arguments = shell_quote(&argument_string, ShellType::PowerShell);
    let timeout_ms = timeout_secs.saturating_mul(1000);

    let ps_script = format!(
        concat!(
            "{cd}; {env}; ",
            "$out=(New-TemporaryFile).FullName; $err=(New-TemporaryFile).FullName; $caughtError=$null; ",
            "$outStream=$null; $errStream=$null; $process=$null; ",
            "function Read-MiaominalOutput([string]$path,[int]$limit){{ ",
            "$bytes=[System.IO.File]::ReadAllBytes($path); ",
            "$length=$bytes.Length; ",
            "if($limit -le 0 -or $length -eq 0){{ ",
            "$slice=[byte[]]::new(0) ",
            "}}elseif($length -gt $limit){{ ",
            "$slice=$bytes[0..($limit-1)] ",
            "}}else{{ ",
            "$slice=$bytes ",
            "}}; ",
            "[pscustomobject]@{{Text=[System.Text.Encoding]::UTF8.GetString($slice); ",
            "Truncated=$length -gt $limit}} ",
            "}}; ",
            "try{{ ",
            "$psi=[System.Diagnostics.ProcessStartInfo]::new(); ",
            "$psi.FileName={program}; ",
            "$psi.Arguments={arguments}; ",
            "$psi.UseShellExecute=$false; ",
            "$psi.RedirectStandardOutput=$true; ",
            "$psi.RedirectStandardError=$true; ",
            "$psi.WorkingDirectory=(Get-Location).Path; ",
            "$process=[System.Diagnostics.Process]::new(); ",
            "$process.StartInfo=$psi; ",
            "$outStream=[System.IO.File]::Open($out,[System.IO.FileMode]::Create,[System.IO.FileAccess]::Write,[System.IO.FileShare]::Read); ",
            "$errStream=[System.IO.File]::Open($err,[System.IO.FileMode]::Create,[System.IO.FileAccess]::Write,[System.IO.FileShare]::Read); ",
            "[void]$process.Start(); ",
            "$stdoutTask=$process.StandardOutput.BaseStream.CopyToAsync($outStream); ",
            "$stderrTask=$process.StandardError.BaseStream.CopyToAsync($errStream); ",
            "if($process.WaitForExit({timeout_ms})){{ ",
            "$process.WaitForExit(); ",
            "$stdoutTask.Wait(); ",
            "$stderrTask.Wait(); ",
            "$ec=$process.ExitCode ",
            "}}else{{ ",
            "taskkill /t /f /pid $process.Id *> $null; ",
            "$process.WaitForExit(); ",
            "try{{ $stdoutTask.Wait(1000) *> $null }}catch{{}}; ",
            "try{{ $stderrTask.Wait(1000) *> $null }}catch{{}}; ",
            "$ec=124 ",
            "}} ",
            "}}catch{{ ",
            "$caughtError=$_ | Out-String; ",
            "$ec=1 ",
            "}}finally{{ ",
            "if($null -ne $outStream){{ $outStream.Dispose() }}; ",
            "if($null -ne $errStream){{ $errStream.Dispose() }}; ",
            "if($null -ne $process){{ $process.Dispose() }} ",
            "}}; ",
            "if($null -ne $caughtError){{ Set-Content -Path $err -Value $caughtError -Encoding utf8 }}; ",
            "$stdout=Read-MiaominalOutput $out {max_bytes}; ",
            "$stderr=Read-MiaominalOutput $err {max_bytes}; ",
            "if($null -eq $ec){{ $ec=1 }}; ",
            "Write-Output \"MIAOMINAL_STATUS=$ec\"; ",
            "Write-Output MIAOMINAL_STDOUT_BEGIN; ",
            "Write-Output $stdout.Text; ",
            "Write-Output MIAOMINAL_STDOUT_END; ",
            "Write-Output MIAOMINAL_STDERR_BEGIN; ",
            "Write-Output $stderr.Text; ",
            "Write-Output MIAOMINAL_STDERR_END; ",
            "if($stdout.Truncated -or $stderr.Truncated){{ ",
            "Write-Output \"MIAOMINAL_TRUNCATED=1\" ",
            "}}else{{ ",
            "Write-Output \"MIAOMINAL_TRUNCATED=0\" ",
            "}}; ",
            "Remove-Item $out,$err -ErrorAction SilentlyContinue"
        ),
        cd = cd,
        env = env,
        program = quoted_program,
        arguments = quoted_arguments,
        timeout_ms = timeout_ms,
        max_bytes = max_bytes,
    );

    format!(
        "powershell.exe -NoProfile -EncodedCommand {}",
        super::windows::powershell_encoded_payload(&ps_script)
    )
}

fn windows_command_line_args(arguments: &[&str]) -> String {
    arguments
        .iter()
        .map(|argument| windows_command_line_arg(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

fn windows_command_line_arg(argument: &str) -> String {
    if argument.is_empty()
        || argument
            .chars()
            .any(|character| matches!(character, ' ' | '\t' | '"'))
    {
        let mut quoted = String::from("\"");
        let mut backslashes = 0;
        for character in argument.chars() {
            match character {
                '\\' => backslashes += 1,
                '"' => {
                    quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                    quoted.push('"');
                    backslashes = 0;
                }
                _ => {
                    quoted.push_str(&"\\".repeat(backslashes));
                    backslashes = 0;
                    quoted.push(character);
                }
            }
        }
        quoted.push_str(&"\\".repeat(backslashes * 2));
        quoted.push('"');
        quoted
    } else {
        argument.to_string()
    }
}

/// Build a CMD terminal-mode wrapper (WinkTerm sentinel style).
///
/// Uses `setlocal enabledelayedexpansion` so that `!ERRORLEVEL!` and `!CD!`
/// reflect the values **after** `{user_command}` executes.  Without delayed
/// expansion, `%ERRORLEVEL%` and `%CD%` are expanded at parse time — before
/// any command on the line runs — which would report the pre-command state.
fn build_cmd_terminal(
    user_command: &str,
    cwd: &str,
    sentinel: &str,
    shell_type: ShellType,
) -> String {
    debug_assert!(cmd_terminal_cwd_is_safe(cwd));
    let cd = format!("cd /d \"%USERPROFILE%\" && cd /d \"{cwd}\"");
    let env = env_setup(shell_type);
    format!(
        "setlocal enabledelayedexpansion & {cd} & {env} & {user_command} & echo( & echo {sentinel}!ERRORLEVEL!:!CD! & endlocal",
        cd = cd,
        env = env,
        user_command = user_command,
        sentinel = sentinel,
    )
}

fn cmd_terminal_cwd_is_safe(cwd: &str) -> bool {
    !cwd.chars()
        .any(|character| matches!(character, '%' | '!' | '^' | '"' | '\r' | '\n' | '\0'))
}

pub fn parse_shell_result(output: &str) -> AgentResult<ShellCommandResult> {
    let cleaned = sanitize_shell_output(output);
    let exit_status = extract_status(&cleaned)?;
    let stdout = extract_section(&cleaned, "MIAOMINAL_STDOUT_BEGIN", "MIAOMINAL_STDOUT_END")
        .unwrap_or_default();
    let stderr = extract_section(&cleaned, "MIAOMINAL_STDERR_BEGIN", "MIAOMINAL_STDERR_END")
        .unwrap_or_default();
    let truncated = extract_truncated(&cleaned);

    Ok(ShellCommandResult {
        stdout,
        stderr,
        exit_status,
        timed_out: exit_status == 124,
        truncated,
    })
}

/// Strip ANSI escape sequences and carriage returns from raw PTY output.
///
/// The terminal output tap collects raw PTY bytes which include ANSI control
/// sequences (colors, cursor movement, bracketed-paste markers) and `\r` from
/// PTY onlcr line-ending conversion. These would break sentinel parsing.
fn sanitize_shell_output(raw: &str) -> String {
    let mut result = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.peek() {
                Some('[') => {
                    chars.next();
                    for nc in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&nc) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    while let Some(nc) = chars.next() {
                        if nc == '\x07' {
                            break;
                        }
                        if nc == '\x1b' {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            '\r' => {}
            _ => result.push(c),
        }
    }
    result
}

/// Extract the exit status by searching for the last `MIAOMINAL_STATUS=` marker.
///
/// Uses `rfind` (last occurrence) so that wrapper command text displayed by the
/// line editor in the terminal tap does not shadow the real sentinel output.
///
/// Returns 0 when the status value is missing or whitespace-only (e.g. when
/// PowerShell's `$ec` evaluated to `$null`), so that the agent can still
/// consume stdout/stderr content.
fn extract_status(output: &str) -> AgentResult<i32> {
    const MARKER: &str = "MIAOMINAL_STATUS=";
    let search_area = output
        .rfind("MIAOMINAL_STDOUT_BEGIN")
        .map(|pos| &output[..pos])
        .unwrap_or(output);
    let pos = search_area
        .rfind(MARKER)
        .ok_or_else(|| AgentError::Backend(anyhow!("missing shell exit status")))?;
    let after = &search_area[pos + MARKER.len()..];
    let after_trimmed = after.trim_start_matches(|c: char| c.is_whitespace());
    let digits: String = after_trimmed
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return Ok(0);
    }
    digits
        .parse::<i32>()
        .map_err(|_| AgentError::Backend(anyhow!("invalid shell exit status: {digits}")))
}

/// Extract content between the last pair of begin/end sentinels.
///
/// Uses `rfind` so that wrapper text in the tap does not produce a false match.
fn extract_section(output: &str, begin: &str, end: &str) -> Option<String> {
    let begin_pos = output.rfind(begin)?;
    let after_begin = &output[begin_pos + begin.len()..];
    let after_begin = after_begin.strip_prefix('\n').unwrap_or(after_begin);
    let end_pos = after_begin.find(end)?;
    let section = &after_begin[..end_pos];
    let section = section.strip_suffix('\n').unwrap_or(section);
    Some(section.to_string())
}

/// Extract the truncation flag from the last `MIAOMINAL_TRUNCATED=` marker.
fn extract_truncated(output: &str) -> bool {
    const MARKER: &str = "MIAOMINAL_TRUNCATED=";
    output
        .rfind(MARKER)
        .map(|pos| output[pos + MARKER.len()..].starts_with('1'))
        .unwrap_or(false)
}

/// Parse output from the terminal PTY path (WinkTerm-style unique sentinel).
pub fn parse_terminal_shell_result(
    output: &str,
    sentinel: &str,
) -> AgentResult<ShellCommandResult> {
    let cleaned = sanitize_shell_output(output);
    let pos = cleaned
        .rfind(sentinel)
        .ok_or_else(|| AgentError::Backend(anyhow!("missing sentinel marker")))?;
    let after = &cleaned[pos + sentinel.len()..];
    let colon = after
        .find(':')
        .ok_or_else(|| AgentError::Backend(anyhow!("missing colon in sentinel")))?;
    let exit_status: i32 = after[..colon]
        .parse()
        .map_err(|_| AgentError::Backend(anyhow!("invalid exit code in sentinel")))?;
    let nl = after.find('\n').unwrap_or(after.len());
    let _pwd = after[colon + 1..nl].to_string();
    let mut stdout = cleaned[..pos]
        .strip_suffix('\n')
        .unwrap_or(&cleaned[..pos])
        .to_string();
    let truncated = stdout.len() > DEFAULT_MAX_OUTPUT_BYTES;
    if truncated {
        stdout.truncate(stdout.floor_char_boundary(DEFAULT_MAX_OUTPUT_BYTES));
    }
    Ok(ShellCommandResult {
        stdout,
        stderr: String::new(),
        exit_status,
        timed_out: exit_status == 124,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use miaominal_core::profile::ShellType;
    use miaominal_secrets::SecretStore;
    use miaominal_storage::known_hosts_store::KnownHostsStore;

    fn profile(shell_type: ShellType) -> miaominal_core::profile::SessionProfile {
        let mut profile = miaominal_core::profile::SessionProfile::blank("session-1", 1);
        profile.host = "example.com".into();
        profile.username = "akko".into();
        profile.shell_type = shell_type;
        profile
    }

    fn decode_encoded_powershell_command(command: &str) -> String {
        let encoded = command
            .strip_prefix("powershell.exe -NoProfile -EncodedCommand ")
            .expect("encoded PowerShell command prefix");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("encoded command decodes");
        let units = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        String::from_utf16(&units).expect("encoded command is UTF-16LE")
    }

    #[test]
    fn shell_result_parser_extracts_status_streams_and_truncation() {
        let output = concat!(
            "MIAOMINAL_STATUS=7\n",
            "MIAOMINAL_STDOUT_BEGIN\n",
            "hello\n",
            "MIAOMINAL_STDOUT_END\n",
            "MIAOMINAL_STDERR_BEGIN\n",
            "oops\n",
            "MIAOMINAL_STDERR_END\n",
            "MIAOMINAL_TRUNCATED=1\n"
        );
        let result = parse_shell_result(output).unwrap();
        assert_eq!(result.exit_status, 7);
        assert_eq!(result.stdout, "hello");
        assert_eq!(result.stderr, "oops");
        assert!(result.truncated);
    }

    #[test]
    fn shell_result_parser_marks_timeout_exit_code() {
        let output = concat!(
            "MIAOMINAL_STATUS=124\n",
            "MIAOMINAL_STDOUT_BEGIN\n",
            "\nMIAOMINAL_STDOUT_END\n",
            "MIAOMINAL_STDERR_BEGIN\n",
            "\nMIAOMINAL_STDERR_END\n",
            "MIAOMINAL_TRUNCATED=0\n"
        );
        let result = parse_shell_result(output).unwrap();
        assert!(result.timed_out);
        assert!(!result.truncated);
    }

    #[test]
    fn shell_result_parser_strips_ansi_and_carriage_returns() {
        let output = concat!(
            "stty -echo\r\n",
            "\x1b[?2004l",
            "user@host:~$ \x1b[0m",
            "MIAOMINAL_STATUS=42\r\n",
            "MIAOMINAL_STDOUT_BEGIN\r\n",
            "line1\r\n",
            "line2\r\n",
            "MIAOMINAL_STDOUT_END\r\n",
            "MIAOMINAL_STDERR_BEGIN\r\n",
            "boom\r\n",
            "MIAOMINAL_STDERR_END\r\n",
            "MIAOMINAL_TRUNCATED=0\r\n",
            "\x1b[?2004h",
            "user@host:~$ ",
        );
        let result = parse_shell_result(output).unwrap();
        assert_eq!(result.exit_status, 42);
        assert_eq!(result.stdout, "line1\nline2");
        assert_eq!(result.stderr, "boom");
        assert!(!result.truncated);
    }

    #[test]
    fn shell_result_parser_ignores_sentinel_literals_in_wrapper_text() {
        let output = concat!(
            "user@host:~$ printf 'MIAOMINAL_STATUS=%s\\n' \"$_mm_rc\"; ",
            "printf 'MIAOMINAL_STDOUT_BEGIN\\n'; head -c 65536 \"$out\"; ",
            "printf '\\nMIAOMINAL_STDOUT_END\\n'; ",
            "printf 'MIAOMINAL_TRUNCATED=1\\n'; ",
            "printf 'MIAOMINAL_TRUNCATED=0\\n'; fi\n",
            "MIAOMINAL_STATUS=0\n",
            "MIAOMINAL_STDOUT_BEGIN\n",
            "hello\n",
            "MIAOMINAL_STDOUT_END\n",
            "MIAOMINAL_STDERR_BEGIN\n",
            "\nMIAOMINAL_STDERR_END\n",
            "MIAOMINAL_TRUNCATED=0\n",
        );
        let result = parse_shell_result(output).unwrap();
        assert_eq!(result.exit_status, 0);
        assert_eq!(result.stdout, "hello");
        assert_eq!(result.stderr, "");
        assert!(!result.truncated);
    }

    #[test]
    fn shell_result_parser_handles_prompt_on_same_line_as_sentinel() {
        let output = concat!(
            "user@host:~$ MIAOMINAL_STATUS=7\n",
            "MIAOMINAL_STDOUT_BEGIN\n",
            "hello\n",
            "MIAOMINAL_STDOUT_END\n",
            "MIAOMINAL_STDERR_BEGIN\n",
            "oops\n",
            "MIAOMINAL_STDERR_END\n",
            "MIAOMINAL_TRUNCATED=1\n"
        );
        let result = parse_shell_result(output).unwrap();
        assert_eq!(result.exit_status, 7);
        assert_eq!(result.stdout, "hello");
        assert_eq!(result.stderr, "oops");
        assert!(result.truncated);
    }

    #[test]
    fn shell_result_parser_does_not_pick_status_from_stdout_content() {
        let output = concat!(
            "MIAOMINAL_STATUS=5\n",
            "MIAOMINAL_STDOUT_BEGIN\n",
            "some output\n",
            "MIAOMINAL_STATUS=999\n",
            "more output\n",
            "MIAOMINAL_STDOUT_END\n",
            "MIAOMINAL_STDERR_BEGIN\n",
            "\nMIAOMINAL_STDERR_END\n",
            "MIAOMINAL_TRUNCATED=0\n"
        );
        let result = parse_shell_result(output).unwrap();
        assert_eq!(result.exit_status, 5);
        assert_eq!(
            result.stdout,
            "some output\nMIAOMINAL_STATUS=999\nmore output"
        );
        assert_eq!(result.stderr, "");
        assert!(!result.truncated);
    }

    #[test]
    fn terminal_parser_extracts_exit_code_and_output() {
        let sentinel = "MIAOMINAL_abc123def4567890_";
        let output = concat!(
            "cd ...; df -h; printf '\\n",
            "MIAOMINAL_abc123def4567890_%d:%s\\n' \"$?\" \"$PWD\"\n",
            "Filesystem      Size  Used Avail\n",
            "/dev/sda1       100G   50G   50G\n",
            "MIAOMINAL_abc123def4567890_0:/home/user\n",
            "user@host:~$ ",
        );
        let result = parse_terminal_shell_result(output, sentinel).unwrap();
        assert_eq!(result.exit_status, 0);
        assert_eq!(
            result.stdout,
            concat!(
                "cd ...; df -h; printf '\\n",
                "MIAOMINAL_abc123def4567890_%d:%s\\n' \"$?\" \"$PWD\"\n",
                "Filesystem      Size  Used Avail\n",
                "/dev/sda1       100G   50G   50G",
            )
        );
        assert_eq!(result.stderr, "");
        assert!(!result.truncated);
    }

    #[test]
    fn terminal_parser_strips_ansi_and_cr() {
        let sentinel = "MIAOMINAL_deadbeef00000000_";
        let output = concat!(
            "\x1b[31mcd ...\x1b[0m; ls; printf '\n",
            "MIAOMINAL_deadbeef00000000_%d:%s\n' \"$?\" \"$PWD\"\r\n",
            "\x1b[32mfile.txt\x1b[0m\r\n",
            "MIAOMINAL_deadbeef00000000_0:/home/user\r\n",
            "\x1b[?2004huser@host:~$ ",
        );
        let result = parse_terminal_shell_result(output, sentinel).unwrap();
        assert_eq!(result.exit_status, 0);
        assert_eq!(
            result.stdout,
            "cd ...; ls; printf '\nMIAOMINAL_deadbeef00000000_%d:%s\n' \"$?\" \"$PWD\"\nfile.txt"
        );
        assert_eq!(result.stderr, "");
    }

    #[test]
    fn terminal_parser_ignores_echoed_sentinel_in_wrapper_text() {
        let sentinel = "MIAOMINAL_1111222233334444_";
        let output = concat!(
            "cd \"$HOME\" && cd . && export ...; ls; printf '\\n",
            "MIAOMINAL_1111222233334444_%d:%s\\n' \"$?\" \"$PWD\"\r\n",
            "README.md\ntarget\n",
            "MIAOMINAL_1111222233334444_0:/home/project\n",
            "user@host:~$ ",
        );
        let result = parse_terminal_shell_result(output, sentinel).unwrap();
        assert_eq!(result.exit_status, 0);
        assert_eq!(
            result.stdout,
            concat!(
                "cd \"$HOME\" && cd . && export ...; ls; printf '\\n",
                "MIAOMINAL_1111222233334444_%d:%s\\n' \"$?\" \"$PWD\"\n",
                "README.md\ntarget",
            )
        );
        assert_eq!(result.stderr, "");
    }

    #[test]
    fn default_shell_matches_profile_shell_type() {
        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Posix),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-default-shell-posix")),
        );
        assert_eq!(channel.shell_label(), "posix-sh");

        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Fish),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-default-shell-fish")),
        );
        assert_eq!(channel.shell_label(), "fish");

        let channel = AgentExecChannel::for_profile(
            profile(ShellType::PowerShell),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-default-shell-powershell")),
        );
        assert_eq!(channel.shell_label(), "powershell");

        let channel = AgentExecChannel::for_profile(
            profile(ShellType::Cmd),
            Vec::new(),
            SecretStore::new_locked_vault(),
            KnownHostsStore::with_path(std::env::temp_dir().join("agent-default-shell-cmd")),
        );
        assert_eq!(channel.shell_label(), "cmd");
    }

    #[test]
    fn shell_type_from_label_accepts_supported_labels() {
        assert_eq!(shell_type_from_label("posix-sh"), Some(ShellType::Posix));
        assert_eq!(shell_type_from_label("sh"), Some(ShellType::Posix));
        assert_eq!(shell_type_from_label("fish"), Some(ShellType::Fish));
        assert_eq!(
            shell_type_from_label("powershell"),
            Some(ShellType::PowerShell)
        );
        assert_eq!(shell_type_from_label("cmd"), Some(ShellType::Cmd));
        assert_eq!(shell_type_from_label("pwsh"), None);
    }

    #[test]
    fn explicit_shell_forces_raw_exec_path() {
        assert_eq!(select_exec_path(true, true, true), ShellExecPath::Exec);
        assert_eq!(select_exec_path(false, true, true), ShellExecPath::Exec);
        assert_eq!(select_exec_path(true, true, false), ShellExecPath::Terminal);
        assert_eq!(select_exec_path(false, true, false), ShellExecPath::Pty);
        assert_eq!(select_exec_path(false, false, false), ShellExecPath::Exec);
    }

    #[test]
    fn cmd_unsafe_terminal_cwd_falls_back_before_wrapper_construction() {
        for cwd in [r#"C:\work\100%"#, r#"C:\work\wow!"#, r#"C:\work\a^b"#] {
            let exec_path = select_exec_path_for_shell(ShellType::Cmd, true, true, false, cwd);

            assert_eq!(exec_path, ShellExecPath::Exec);
            assert!(
                build_terminal_command(
                    exec_path,
                    ShellType::Cmd,
                    "echo ok",
                    cwd,
                    "MIAOMINAL_TEST_",
                )
                .is_none()
            );
        }

        let safe_cwd = r#"C:\work\x & whoami & rem"#;
        let exec_path = select_exec_path_for_shell(ShellType::Cmd, true, true, false, safe_cwd);
        assert_eq!(exec_path, ShellExecPath::Terminal);
        assert!(
            build_terminal_command(
                exec_path,
                ShellType::Cmd,
                "echo ok",
                safe_cwd,
                "MIAOMINAL_TEST_",
            )
            .is_some()
        );
    }

    // ── PowerShell / CMD wrapper tests ──

    #[test]
    fn powershell_run_shell_wrapper_uses_new_temporary_file() {
        let wrapper = build_powershell_non_terminal(
            "echo hello",
            "C:\\Users\\test",
            30,
            65536,
            ShellType::PowerShell,
        );
        let script = decode_encoded_powershell_command(&wrapper);

        assert!(script.contains("New-TemporaryFile"));
        assert!(!wrapper.contains("mktemp"));
        assert!(script.contains("$psi.FileName='powershell.exe'"));
        assert!(script.contains("$psi.Arguments='-NoProfile -Command \"echo hello\"'"));
        assert!(script.contains("[System.Diagnostics.ProcessStartInfo]::new()"));
        assert!(script.contains("WaitForExit(30000)"));
        assert!(script.contains("taskkill /t /f /pid $process.Id"));
        assert!(script.contains("$ec=$process.ExitCode"));
        assert!(script.contains("MIAOMINAL_STATUS="));
        assert!(script.contains("MIAOMINAL_STDOUT_BEGIN"));
        assert!(script.contains("MIAOMINAL_STDERR_BEGIN"));
        assert!(wrapper.starts_with("powershell.exe -NoProfile -EncodedCommand "));
        assert!(!wrapper.contains("echo hello"));
    }

    #[test]
    fn powershell_run_shell_wrapper_uses_byte_limited_replay() {
        let wrapper = build_powershell_non_terminal("dir", ".", 30, 4096, ShellType::PowerShell);
        let script = decode_encoded_powershell_command(&wrapper);
        assert!(script.contains("ReadAllBytes"));
        assert!(script.contains("4096"));
    }

    #[test]
    fn cmd_run_shell_wrapper_uses_timeout_and_byte_limit() {
        let wrapper = build_cmd_non_terminal("dir", ".", 30, 4096, ShellType::Cmd);
        let script = decode_encoded_powershell_command(&wrapper);

        assert!(wrapper.starts_with("powershell.exe -NoProfile -EncodedCommand "));
        assert!(!wrapper.contains("cmd.exe"));
        assert!(!wrapper.contains("dir"));
        assert!(script.contains("$psi.FileName='cmd.exe'"));
        assert!(script.contains("$psi.Arguments='/d /c dir'"));
        assert!(script.contains("WaitForExit(30000)"));
        assert!(script.contains("ReadAllBytes"));
        assert!(script.contains("MIAOMINAL_TRUNCATED=1"));
    }

    #[test]
    fn windows_command_line_arg_quotes_nested_quotes() {
        let args = windows_command_line_args(&[
            "/d",
            "/c",
            "Remove-Item -Path \"C:\\nope\" -Force -ErrorAction Stop",
        ]);

        assert_eq!(
            args,
            "/d /c \"Remove-Item -Path \\\"C:\\nope\\\" -Force -ErrorAction Stop\""
        );
    }

    #[test]
    fn cmd_terminal_cwd_quotes_control_operators_and_rejects_expansion() {
        let wrapper = build_cmd_terminal(
            "echo ok",
            r#"C:\work\x & whoami & rem"#,
            "MIAOMINAL_TEST_",
            ShellType::Cmd,
        );
        assert!(wrapper.contains(r#"cd /d "C:\work\x & whoami & rem""#));
        assert!(cmd_terminal_cwd_is_safe(r#"C:\work\x & whoami"#));
        for unsafe_path in [r#"C:\work\100%"#, r#"C:\work\wow!"#, r#"C:\work\a^b"#] {
            assert!(!cmd_terminal_cwd_is_safe(unsafe_path));
        }
    }

    #[test]
    fn windows_command_line_arg_only_quotes_ascii_space_tab_and_quotes() {
        assert_eq!(windows_command_line_arg("hello world"), "\"hello world\"");
        assert_eq!(windows_command_line_arg("hello\tworld"), "\"hello\tworld\"");
        assert_eq!(
            windows_command_line_arg("hello\"world"),
            "\"hello\\\"world\""
        );
        assert_eq!(
            windows_command_line_arg("hello\u{00a0}world"),
            "hello\u{00a0}world"
        );
    }

    #[test]
    fn crlf_output_parsing_handles_miaominal_markers_with_crlf() {
        let output = "MIAOMINAL_STATUS=0\r\n\
                      MIAOMINAL_STDOUT_BEGIN\r\n\
                      hello world\r\n\
                      MIAOMINAL_STDOUT_END\r\n\
                      MIAOMINAL_STDERR_BEGIN\r\n\
                      \r\nMIAOMINAL_STDERR_END\r\n\
                      MIAOMINAL_TRUNCATED=0\r\n";
        let result = parse_shell_result(output).unwrap();
        assert_eq!(result.exit_status, 0);
        assert_eq!(result.stdout, "hello world");
        assert_eq!(result.stderr, "");
        assert!(!result.truncated);
    }

    #[test]
    fn posix_wrapper_unchanged_structure() {
        let wrapper =
            build_posix_non_terminal("ls -la", "/home/user/project", 30, 65536, ShellType::Posix);
        assert!(wrapper.contains("mktemp"));
        assert!(wrapper.contains("head -c 65536"));
        assert!(wrapper.contains("wc -c"));
        assert!(wrapper.contains("timeout 30"));
    }

    #[test]
    fn terminal_parser_truncates_multibyte_output_on_char_boundary() {
        let sentinel = "MIAOMINAL_utf8boundary_";
        let long_stdout = format!("你{}", "🚀".repeat(DEFAULT_MAX_OUTPUT_BYTES / 4));

        assert!(long_stdout.len() > DEFAULT_MAX_OUTPUT_BYTES);
        assert!(!long_stdout.is_char_boundary(DEFAULT_MAX_OUTPUT_BYTES));

        let output = format!("{long_stdout}\n{sentinel}0:/home/user\nuser@host:~$ ");
        let result = parse_terminal_shell_result(&output, sentinel).unwrap();

        assert!(result.truncated);
        assert!(result.stdout.len() <= DEFAULT_MAX_OUTPUT_BYTES);
        assert!(result.stdout.is_char_boundary(result.stdout.len()));
        assert!(long_stdout.starts_with(&result.stdout));
        assert!(
            result
                .stdout
                .chars()
                .all(|character| character == '你' || character == '🚀')
        );
    }
}
