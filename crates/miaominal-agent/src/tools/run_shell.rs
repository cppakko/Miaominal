use crate::backend::{BackendRoute, ExecMode};
use crate::channel::{AgentExecChannel, DEFAULT_MAX_OUTPUT_BYTES, ShellCommandResult, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::path_guard::{cd_prefix, env_setup, resolve_workspace_path, shell_quote};
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
    let cwd = resolve_workspace_path(args.cwd.as_deref().unwrap_or("."))?;
    channel
        .policy()
        .enforce_path(crate::policy::AgentPathAccess::Read, &cwd, true)?;
    if matches!(
        channel.policy().decide_command(&args.command, true),
        crate::policy::AgentPolicyDecision::Deny { .. }
    ) {
        channel.policy().enforce_command(&args.command, true)?;
    }
    let timeout_secs = args.timeout_seconds.unwrap_or(20).max(1);
    let max_bytes = args.max_bytes.unwrap_or(DEFAULT_MAX_OUTPUT_BYTES);
    let shell = args.shell.as_deref().unwrap_or(channel.shell_label());
    let is_fish = shell == "fish" || channel.is_fish_shell();
    let st = channel.shell_type();
    if shell != "posix-sh"
        && shell != "sh"
        && shell != "fish"
        && shell != "powershell"
        && shell != "cmd"
    {
        return Err(AgentError::PosixOnly(
            "run_shell v1 supports posix-sh, sh, fish, powershell, or cmd".into(),
        ));
    }

    let sentinel = format!(
        "MIAOMINAL_{:016x}_",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0),
    );

    let (command, terminal_command) = match st {
        ShellType::PowerShell => (
            build_powershell_non_terminal(&args.command, &cwd, timeout_secs, max_bytes, st),
            build_powershell_terminal(&args.command, &cwd, &sentinel, st),
        ),
        ShellType::Cmd => (
            build_cmd_non_terminal(&args.command, &cwd, timeout_secs, max_bytes, st),
            build_cmd_terminal(&args.command, &cwd, &sentinel, st),
        ),
        _ => {
            let cmd = build_posix_non_terminal(&args.command, &cwd, timeout_secs, max_bytes, st);
            let tc = if is_fish {
                format!(
                    "cd \"$HOME\"; and cd {cwd}; and set -x PAGER cat; set -x SYSTEMD_PAGER \"\"; set -x GIT_PAGER cat; set -x LESS \"\"; set -x LANG C.UTF-8; set -x NO_COLOR 1; set -x CLICOLOR 0; set -x TERM xterm-256color; {user_command}; printf '\\n{sentinel}%d:%s\\n' $status $PWD",
                    cwd = shell_quote(&cwd, st),
                    user_command = args.command,
                    sentinel = sentinel,
                )
            } else {
                format!(
                    "cd \"$HOME\" && cd {cwd} && export PAGER=cat SYSTEMD_PAGER= GIT_PAGER=cat LESS= LANG=C.UTF-8 NO_COLOR=1 CLICOLOR=0 TERM=xterm-256color; {user_command}; printf '\\n{sentinel}%d:%s\\n' \"$?\" \"$PWD\"",
                    cwd = shell_quote(&cwd, st),
                    user_command = args.command,
                    sentinel = sentinel,
                )
            };
            (cmd, tc)
        }
    };

    let output = if channel.terminal_exec().is_some() {
        channel
            .exec_via_terminal(terminal_command, &sentinel, timeout_secs + 5)
            .await?
    } else if channel.uses_pty() {
        channel
            .exec_with_mode(BackendRoute::Pty, command, ExecMode::pty_default())
            .await?
    } else {
        channel.exec(command).await?
    };
    let result = if channel.terminal_exec().is_some() {
        parse_terminal_shell_result(&output, &sentinel)?
    } else {
        parse_shell_result(&output)?
    };
    Ok(ToolOutput::Shell { result })
}

// ── Command wrapper builders ──

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
/// Uses `New-TemporaryFile` for temp files, `$LASTEXITCODE` for exit code,
/// `Get-Content -Raw` with `.Substring()` for byte truncation.  Wrapped with
/// `powershell.exe -NoProfile -Command "..."` so it works over SSH exec even
/// when the remote default shell is cmd.exe.
fn build_powershell_non_terminal(
    user_command: &str,
    cwd: &str,
    _timeout_secs: u64,
    max_bytes: usize,
    shell_type: ShellType,
) -> String {
    let cd = cd_prefix(shell_type, cwd);
    let env = env_setup(shell_type);
    let quoted_cmd = shell_quote(user_command, shell_type);

    // PowerShell wrapper avoids double-quotes so it nests cleanly inside
    // the outer `powershell.exe -NoProfile -Command "..."` call.
    let ps_script = format!(
        "{cd}; {env}; \
         $out=(New-TemporaryFile).FullName; $err=(New-TemporaryFile).FullName; \
         try{{& {{{quoted_cmd}}} 1>$out 2>$err}}catch{{}}; \
         $ec=if($LASTEXITCODE -ne $null){{$LASTEXITCODE}}else{{0}}; \
         Write-Output MIAOMINAL_STATUS=$ec; \
         Write-Output MIAOMINAL_STDOUT_BEGIN; \
         $s=Get-Content -Path $out -Raw; \
         if($s.Length -gt {max_bytes}){{$s=$s.Substring(0,{max_bytes})}}; \
         Write-Output $s; \
         Write-Output MIAOMINAL_STDOUT_END; \
         Write-Output MIAOMINAL_STDERR_BEGIN; \
         $e=Get-Content -Path $err -Raw; \
         if($e.Length -gt {max_bytes}){{$e=$e.Substring(0,{max_bytes})}}; \
         Write-Output $e; \
         Write-Output MIAOMINAL_STDERR_END; \
         $so=(Get-Item $out).Length; $se=(Get-Item $err).Length; \
         if($so -gt {max_bytes} -or $se -gt {max_bytes}){{ \
           Write-Output MIAOMINAL_TRUNCATED=1 \
         }}else{{ \
           Write-Output MIAOMINAL_TRUNCATED=0 \
         }}; \
         Remove-Item $out,$err",
        cd = cd,
        env = env,
        quoted_cmd = quoted_cmd,
        max_bytes = max_bytes,
    );

    // Outer wrapping for SSH exec (remote may start as cmd.exe).
    format!("powershell.exe -NoProfile -Command \"{ps_script}\"")
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

/// Build a CMD non-terminal wrapper with %TEMP%-based temp files.
///
/// CMD has no built-in byte truncation and cannot separate stdout/stderr natively.
/// Both streams are merged; truncation is handled by the Rust parsing layer.
fn build_cmd_non_terminal(
    user_command: &str,
    cwd: &str,
    _timeout_secs: u64,
    max_bytes: usize,
    shell_type: ShellType,
) -> String {
    let cd = cd_prefix(shell_type, cwd);
    let env = env_setup(shell_type);
    let quoted_cmd = shell_quote(user_command, shell_type);
    // CMD cannot truncate at byte level; max_bytes unused here.
    let _ = max_bytes;
    format!(
        "{cd} & {env} & \
         set \"_mo=%TEMP%\\miaominal-%RANDOM%.tmp\" & \
         {quoted_cmd} > \"%_mo%\" 2>&1 & \
         echo MIAOMINAL_STATUS=%ERRORLEVEL% & \
         echo MIAOMINAL_STDOUT_BEGIN & \
         type \"%_mo%\" & \
         echo MIAOMINAL_STDOUT_END & \
         echo MIAOMINAL_STDERR_BEGIN & \
         echo MIAOMINAL_STDERR_END & \
         echo MIAOMINAL_TRUNCATED=0 & \
         del \"%_mo%\"",
        cd = cd,
        env = env,
        quoted_cmd = quoted_cmd,
    )
}

/// Build a CMD terminal-mode wrapper (WinkTerm sentinel style).
fn build_cmd_terminal(
    user_command: &str,
    cwd: &str,
    sentinel: &str,
    shell_type: ShellType,
) -> String {
    let cd = cd_prefix(shell_type, cwd);
    let env = env_setup(shell_type);
    format!(
        "{cd} & {env} & {user_command} & echo( & echo {sentinel}%ERRORLEVEL%:%CD%",
        cd = cd,
        env = env,
        user_command = user_command,
        sentinel = sentinel,
    )
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
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
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
        assert!(wrapper.contains("New-TemporaryFile"));
        assert!(!wrapper.contains("mktemp"));
        assert!(wrapper.contains("$LASTEXITCODE"));
        assert!(wrapper.contains("MIAOMINAL_STATUS="));
        assert!(wrapper.contains("MIAOMINAL_STDOUT_BEGIN"));
        assert!(wrapper.contains("MIAOMINAL_STDERR_BEGIN"));
        assert!(wrapper.starts_with("powershell.exe -NoProfile -Command"));
    }

    #[test]
    fn powershell_run_shell_wrapper_truncation_uses_substring() {
        let wrapper = build_powershell_non_terminal("dir", ".", 30, 4096, ShellType::PowerShell);
        assert!(wrapper.contains(".Substring"));
        assert!(wrapper.contains("4096"));
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
        assert!(result
            .stdout
            .chars()
            .all(|character| character == '你' || character == '🚀'));
    }
}
