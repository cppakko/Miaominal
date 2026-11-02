//! Non-interactive command execution typed into an interactive POSIX shell.
//!
//! Terminal-only targets (Telnet) have no exec channel and no SFTP, so every
//! agent command is typed into the user's visible terminal. To get the same
//! result an SSH exec channel would return, the command is shipped as a
//! base64 encoded script and run by a fresh `sh`:
//!
//! - the script text never appears in the echoed input, so markers inside it
//!   are unambiguous, and history expansion, aliases or line length limits of
//!   the interactive shell cannot alter it;
//! - stdout and stderr are captured to temporary files and replayed as
//!   base64, so terminal line discipline (CRLF, control characters) cannot
//!   corrupt them and binary output never garbles the user's screen;
//! - stdin is `/dev/null`, like an exec channel without input.
//!
//! The only text that must survive the interactive shell is the short outer
//! line, which needs a POSIX shell with `base64` and `mktemp` on the target.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::error::{AgentError, AgentResult};
use anyhow::anyhow;

/// Base64 line width used for the typed script, like `base64` output.
const SCRIPT_LINE_WIDTH: usize = 76;
/// Exit status the script uses when it cannot create its temporary files.
const SCRIPT_SETUP_FAILED: i32 = 125;

/// Markers for one script run. `token` is unique per command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TerminalScriptMarkers {
    token: String,
}

impl TerminalScriptMarkers {
    pub(crate) fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }

    fn stdout(&self) -> String {
        format!("{}_O", self.token)
    }

    fn stderr(&self) -> String {
        format!("{}_R", self.token)
    }

    /// Printed after the script with its exit status; the sentinel the
    /// terminal lease waits for (followed by a digit).
    pub(crate) fn end(&self) -> String {
        format!("{}_E", self.token)
    }

    fn heredoc(&self) -> String {
        format!("{}_X", self.token)
    }
}

/// Bytes to type into the interactive shell (ends with Enter).
pub(crate) fn terminal_script_input(command: &str, markers: &TerminalScriptMarkers) -> Vec<u8> {
    let script = format!(
        concat!(
            "o=$(mktemp) && e=$(mktemp) || exit {setup_failed}\n",
            "( {command}\n",
            ") </dev/null >\"$o\" 2>\"$e\"\n",
            "s=$?\n",
            "printf '%s\\n' '{stdout}'\n",
            "base64 <\"$o\"\n",
            "printf '%s\\n' '{stderr}'\n",
            "base64 <\"$e\"\n",
            "rm -f \"$o\" \"$e\"\n",
            "exit $s\n",
        ),
        setup_failed = SCRIPT_SETUP_FAILED,
        command = command,
        stdout = markers.stdout(),
        stderr = markers.stderr(),
    );
    let encoded = STANDARD.encode(script.as_bytes());
    let mut input = String::with_capacity(encoded.len() + 256);
    // The leading space keeps the line out of shell history where
    // HISTCONTROL ignores space-prefixed commands. Markers are split into
    // separate words so the echoed input never contains them.
    input.push_str(&format!(
        " sh -c \"$(base64 -d <<'{heredoc}'\r",
        heredoc = markers.heredoc()
    ));
    for chunk in encoded.as_bytes().chunks(SCRIPT_LINE_WIDTH) {
        input.push_str(std::str::from_utf8(chunk).expect("base64 is ASCII"));
        input.push('\r');
    }
    let end = markers.end();
    let (token, suffix) = end.split_at(markers.token.len());
    input.push_str(&format!(
        "{heredoc}\r)\"; printf '\\n%s%s%d\\n' '{token}' '{suffix}' \"$?\"\r",
        heredoc = markers.heredoc(),
    ));
    input.into_bytes()
}

/// Result of one script run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TerminalScriptOutput {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) status: i32,
}

/// Exit status after the end marker, once the whole number has arrived.
pub(crate) fn terminal_script_status(
    collected: &str,
    markers: &TerminalScriptMarkers,
) -> Option<i32> {
    let end = markers.end();
    let position = collected.rfind(&end)?;
    let after = &collected[position + end.len()..];
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || after.len() == digits.len() {
        // No terminating newline yet: more digits may follow.
        return None;
    }
    digits.parse().ok()
}

/// Decodes the replayed output. `collected` is everything the terminal
/// printed while the script ran, including the echoed input and prompts.
pub(crate) fn parse_terminal_script_output(
    collected: &str,
    markers: &TerminalScriptMarkers,
) -> AgentResult<TerminalScriptOutput> {
    let status = terminal_script_status(collected, markers).ok_or_else(|| {
        AgentError::Backend(anyhow!("terminal command finished without an exit status"))
    })?;
    let stdout_marker = markers.stdout();
    let stderr_marker = markers.stderr();
    let end_position = collected.rfind(&markers.end()).unwrap_or(collected.len());
    let Some(stdout_start) = collected[..end_position].find(&stdout_marker) else {
        if status == SCRIPT_SETUP_FAILED {
            return Err(AgentError::Backend(anyhow!(
                "the terminal target could not create temporary files (mktemp failed)"
            )));
        }
        return Err(AgentError::Backend(anyhow!(
            "the terminal target did not run the agent script (exit status {status}); \
             it needs a POSIX shell with base64 and mktemp"
        )));
    };
    let stdout_body_start = stdout_start + stdout_marker.len();
    let stderr_start = collected[stdout_body_start..end_position]
        .find(&stderr_marker)
        .map(|offset| stdout_body_start + offset)
        .ok_or_else(|| {
            AgentError::Backend(anyhow!("terminal command output was cut off before stderr"))
        })?;
    let stdout = decode_block(&collected[stdout_body_start..stderr_start])?;
    let stderr = decode_block(&collected[stderr_start + stderr_marker.len()..end_position])?;
    Ok(TerminalScriptOutput {
        stdout,
        stderr,
        status,
    })
}

fn decode_block(block: &str) -> AgentResult<String> {
    let compact: String = block.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let bytes = STANDARD.decode(compact.as_bytes()).map_err(|error| {
        AgentError::Backend(anyhow!("terminal command output was garbled: {error}"))
    })?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Shell probe that resolves one path relative to the login home, like the
/// SFTP resolver: prints `kind` (`d`, `f`, `o`), `symlink` (`0`/`1`) and the
/// canonical path on three lines; exits 2 when the path does not exist.
pub(crate) fn path_probe_command(path_literal: &str) -> String {
    format!(
        concat!(
            "cd \"$HOME\" || exit 3\n",
            "p={path}\n",
            "[ -e \"$p\" ] || [ -L \"$p\" ] || exit 2\n",
            "if [ -d \"$p\" ]; then k=d; elif [ -f \"$p\" ]; then k=f; else k=o; fi\n",
            "if [ -L \"$p\" ]; then l=1; else l=0; fi\n",
            "c=$(readlink -f -- \"$p\" 2>/dev/null || realpath -- \"$p\" 2>/dev/null)\n",
            "if [ -z \"$c\" ]; then\n",
            "  if [ \"$k\" = d ]; then c=$(cd -P -- \"$p\" && pwd -P)\n",
            "  else c=$(cd -P -- \"$(dirname -- \"$p\")\" && pwd -P)/$(basename -- \"$p\"); fi\n",
            "fi\n",
            "printf '%s\\n%s\\n%s\\n' \"$k\" \"$l\" \"$c\"",
        ),
        path = path_literal,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProbedPath {
    pub(crate) kind: miaominal_core::sftp::SftpEntryKind,
    pub(crate) is_symlink: bool,
    pub(crate) canonical_path: String,
}

pub(crate) fn parse_path_probe(stdout: &str) -> AgentResult<ProbedPath> {
    let mut lines = stdout.lines();
    let kind = match lines.next().map(str::trim) {
        Some("d") => miaominal_core::sftp::SftpEntryKind::Directory,
        Some("f") => miaominal_core::sftp::SftpEntryKind::File,
        Some("o") => miaominal_core::sftp::SftpEntryKind::Other,
        _ => {
            return Err(AgentError::Backend(anyhow!(
                "remote path probe returned no kind"
            )));
        }
    };
    let is_symlink = lines.next().map(str::trim) == Some("1");
    let canonical_path = lines.next().unwrap_or_default().to_string();
    if !canonical_path.starts_with('/') {
        return Err(AgentError::Backend(anyhow!(
            "remote path probe returned no absolute path"
        )));
    }
    Ok(ProbedPath {
        kind,
        is_symlink,
        canonical_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    fn markers() -> TerminalScriptMarkers {
        TerminalScriptMarkers::new("MIAOMINAL_00000000000000ab")
    }

    fn encode(text: &str) -> String {
        STANDARD.encode(text)
    }

    #[test]
    fn typed_input_hides_every_marker() {
        let input = String::from_utf8(terminal_script_input("echo hi", &markers())).unwrap();
        assert!(input.starts_with(" sh -c \"$(base64 -d <<'MIAOMINAL_00000000000000ab_X'\r"));
        assert!(input.ends_with("\"$?\"\r"));
        for marker in ["_O", "_R", "_E"] {
            assert!(
                !input.contains(&format!("MIAOMINAL_00000000000000ab{marker}")),
                "{marker} leaked into the echoed input"
            );
        }
        assert!(!input.contains('\n'));
        assert!(input.split('\r').all(|line| line.len() <= 120));
    }

    #[test]
    fn typed_script_decodes_to_the_wrapped_command() {
        let input = String::from_utf8(terminal_script_input("ls -la /tmp", &markers())).unwrap();
        let encoded: String = input
            .split('\r')
            .skip(1)
            .take_while(|line| !line.starts_with("MIAOMINAL"))
            .collect();
        let script = String::from_utf8(STANDARD.decode(encoded).unwrap()).unwrap();
        assert!(script.contains("( ls -la /tmp\n) </dev/null"));
        assert!(script.contains("printf '%s\\n' 'MIAOMINAL_00000000000000ab_O'"));
        assert!(script.ends_with("exit $s\n"));
    }

    #[test]
    fn output_is_decoded_from_the_noisy_terminal_stream() {
        let markers = markers();
        let collected = format!(
            "$ sh -c \"$(base64 -d <<'MIAOMINAL_00000000000000ab_X'\r\n> abc\r\n> )\"; printf '\\n%s%s%d\\n' 'MIAOMINAL_00000000000000ab' '_E' \"$?\"\r\n\
             MIAOMINAL_00000000000000ab_O\r\n{}\r\nMIAOMINAL_00000000000000ab_R\r\n{}\r\n\r\nMIAOMINAL_00000000000000ab_E1\r\n$ ",
            encode("line one\r\nline two\n\u{1b}[31m"),
            encode("warning\n"),
        );
        assert_eq!(terminal_script_status(&collected, &markers), Some(1));
        let output = parse_terminal_script_output(&collected, &markers).unwrap();
        assert_eq!(output.stdout, "line one\r\nline two\n\u{1b}[31m");
        assert_eq!(output.stderr, "warning\n");
        assert_eq!(output.status, 1);
    }

    #[test]
    fn status_waits_for_the_complete_number() {
        let markers = markers();
        assert_eq!(
            terminal_script_status("MIAOMINAL_00000000000000ab_E12", &markers),
            None
        );
        assert_eq!(
            terminal_script_status("MIAOMINAL_00000000000000ab_E127\r\n", &markers),
            Some(127)
        );
        assert_eq!(
            terminal_script_status("'MIAOMINAL_00000000000000ab' '_E'", &markers),
            None
        );
    }

    #[test]
    fn missing_tools_are_reported() {
        let markers = markers();
        let error = parse_terminal_script_output(
            "sh: base64: not found\r\nMIAOMINAL_00000000000000ab_E0\r\n",
            &markers,
        )
        .unwrap_err();
        assert!(error.to_string().contains("POSIX shell with base64"));
        let error = parse_terminal_script_output("MIAOMINAL_00000000000000ab_E125\r\n", &markers)
            .unwrap_err();
        assert!(error.to_string().contains("mktemp"));
    }

    #[test]
    fn path_probe_output_is_parsed() {
        let probed = parse_path_probe("d\n0\n/home/akko/project\n").unwrap();
        assert_eq!(probed.kind, miaominal_core::sftp::SftpEntryKind::Directory);
        assert!(!probed.is_symlink);
        assert_eq!(probed.canonical_path, "/home/akko/project");
        assert!(parse_path_probe("").is_err());
        assert!(parse_path_probe("f\n1\nrelative\n").is_err());
    }

    /// Runs the typed input through a real interactive-style `sh` when one
    /// is available, proving the outer line and script work end to end.
    #[test]
    fn script_round_trips_through_a_real_shell() {
        let Ok(mut child) = Command::new("sh")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        else {
            return;
        };
        let has_tools = Command::new("sh")
            .args(["-c", "command -v base64 && command -v mktemp"])
            .output()
            .is_ok_and(|output| output.status.success());
        if !has_tools {
            let _ = child.kill();
            return;
        }
        let markers = markers();
        let command = "printf 'out\\n'; printf 'err\\n' >&2; cat; exit 3";
        let input = terminal_script_input(command, &markers);
        // A pipe has no terminal line discipline translating CR to LF.
        let input: Vec<u8> = input
            .into_iter()
            .map(|byte| if byte == b'\r' { b'\n' } else { byte })
            .collect();
        child.stdin.take().unwrap().write_all(&input).unwrap();
        let output = child.wait_with_output().unwrap();
        let collected = String::from_utf8_lossy(&output.stdout);
        let parsed = parse_terminal_script_output(&collected, &markers).unwrap();
        assert_eq!(parsed.stdout, "out\n");
        assert_eq!(parsed.stderr, "err\n");
        assert_eq!(parsed.status, 3);

        let probe = Command::new("sh")
            .args(["-c", &path_probe_command("'.'")])
            .output()
            .unwrap();
        let probed = parse_path_probe(&String::from_utf8_lossy(&probe.stdout)).unwrap();
        assert_eq!(probed.kind, miaominal_core::sftp::SftpEntryKind::Directory);
    }
}
