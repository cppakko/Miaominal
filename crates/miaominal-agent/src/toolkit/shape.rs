//! Result shaping (design §9 "结果整形"): what the model sees is short,
//! clean and predictable; the UI keeps the full structured data.

/// Maximum bytes of command output shown to the model.
pub const MODEL_OUTPUT_LIMIT: usize = 16 * 1024;
const HEAD_BYTES: usize = 4 * 1024;

/// Remove ANSI/VT escape sequences (CSI, OSC, single-character escapes).
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.peek() {
                Some('[') => {
                    chars.next();
                    // CSI: parameters/intermediates until a final byte @..~
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    // OSC: until BEL or ST (ESC \)
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some('P' | 'X' | '^' | '_') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(' '..='/') => {
                    // nF escape: intermediates then one final byte.
                    while chars.peek().is_some_and(|c| (' '..='/').contains(c)) {
                        chars.next();
                    }
                    chars.next();
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            '\u{9b}' => {
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            '\u{7}' | '\u{8}' => {}
            _ => out.push(c),
        }
    }
    out
}

/// Collapse carriage-return progress output to the final frame of each line
/// (`10%\r20%\r100%\n` → `100%\n`). CRLF line endings become LF.
pub fn collapse_carriage_returns(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        let frame = line
            .rsplit('\r')
            .find(|frame| !frame.is_empty())
            .unwrap_or("");
        out.push_str(frame);
    }
    out
}

/// Clean terminal-ish output for the model.
pub fn clean_output(text: &str) -> String {
    collapse_carriage_returns(&strip_ansi(text))
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index.min(text.len())
}

/// Keep the first ~`limit/4` and last ~`3*limit/4` bytes with an omission
/// marker in between. Cuts are moved to line starts when one is near.
pub fn head_tail(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_string(), false);
    }
    let head_budget = if limit == MODEL_OUTPUT_LIMIT {
        HEAD_BYTES
    } else {
        limit / 4
    };
    let tail_budget = limit - head_budget;
    let mut head_end = floor_char_boundary(text, head_budget);
    if let Some(newline) = text[..head_end].rfind('\n')
        && newline > head_end / 2
    {
        head_end = newline + 1;
    }
    let mut tail_start = ceil_char_boundary(text, text.len() - tail_budget);
    if let Some(offset) = text[tail_start..].find('\n')
        && offset < tail_budget / 4
    {
        tail_start += offset + 1;
    }
    let omitted = tail_start.saturating_sub(head_end);
    (
        format!(
            "{}[... {omitted} bytes omitted ...]\n{}",
            &text[..head_end],
            &text[tail_start..]
        ),
        true,
    )
}

/// Model text for a finished command.
pub fn shell_model_text(
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
    timed_out: bool,
    remote_truncated: bool,
) -> String {
    let stdout = clean_output(stdout);
    let stderr = clean_output(stderr);
    let mut text = String::new();
    match (timed_out, exit_code) {
        (true, _) => text.push_str(
            "timed out: the command was stopped. For long-running work use job_start and job_poll.\n",
        ),
        (false, Some(code)) => text.push_str(&format!("exit_code: {code}\n")),
        (false, None) => {}
    }
    let stdout_budget = if stderr.trim().is_empty() {
        MODEL_OUTPUT_LIMIT
    } else {
        MODEL_OUTPUT_LIMIT * 3 / 4
    };
    if !stdout.trim().is_empty() {
        let (shaped, _) = head_tail(stdout.trim_end(), stdout_budget);
        text.push_str("stdout:\n");
        text.push_str(&shaped);
        text.push('\n');
    }
    if !stderr.trim().is_empty() {
        let (shaped, _) = head_tail(stderr.trim_end(), MODEL_OUTPUT_LIMIT / 4);
        text.push_str("stderr:\n");
        text.push_str(&shaped);
        text.push('\n');
    }
    if stdout.trim().is_empty() && stderr.trim().is_empty() {
        text.push_str("(no output)\n");
    }
    if remote_truncated {
        text.push_str(
            "[output was truncated at the source; narrow it with grep/tail/head if you need more]\n",
        );
    }
    text.trim_end().to_string()
}

/// Number lines like `  12│text`, starting at `first_line`.
pub fn number_lines(text: &str, first_line: usize) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let lines = if lines.last() == Some(&"") && lines.len() > 1 {
        &lines[..lines.len() - 1]
    } else {
        &lines[..]
    };
    let width = (first_line + lines.len()).to_string().len().max(4);
    let mut out = String::with_capacity(text.len() + lines.len() * (width + 2));
    for (offset, line) in lines.iter().enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        out.push_str(&format!("{:>width$}│{line}\n", first_line + offset));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_sequences_are_removed() {
        assert_eq!(
            strip_ansi("\u{1b}[1;31merror\u{1b}[0m \u{1b}]0;title\u{7}done\u{1b}(B"),
            "error done"
        );
    }

    #[test]
    fn carriage_return_progress_keeps_final_frame() {
        assert_eq!(
            collapse_carriage_returns("start\r\n10%\r50%\r100%\nend\r\n"),
            "start\n100%\nend\n"
        );
    }

    #[test]
    fn head_tail_keeps_both_ends() {
        let text: String = (0..5000).map(|i| format!("line {i}\n")).collect();
        let (shaped, truncated) = head_tail(&text, MODEL_OUTPUT_LIMIT);
        assert!(truncated);
        assert!(shaped.len() <= MODEL_OUTPUT_LIMIT + 64);
        assert!(shaped.starts_with("line 0\n"));
        assert!(shaped.trim_end().ends_with("line 4999"));
        assert!(shaped.contains("bytes omitted"));
        let (short, truncated) = head_tail("abc", MODEL_OUTPUT_LIMIT);
        assert_eq!((short.as_str(), truncated), ("abc", false));
        // Multi-byte text never splits a character.
        let cjk = "错误".repeat(10_000);
        let (shaped, _) = head_tail(&cjk, 1000);
        assert!(shaped.contains("omitted"));
    }

    #[test]
    fn shell_text_format() {
        assert_eq!(
            shell_model_text(Some(1), "", "boom\n", false, false),
            "exit_code: 1\nstderr:\nboom"
        );
        assert_eq!(
            shell_model_text(Some(0), "", "", false, false),
            "exit_code: 0\n(no output)"
        );
        assert!(shell_model_text(None, "x", "", true, false).starts_with("timed out"));
    }

    #[test]
    fn lines_are_numbered() {
        assert_eq!(number_lines("a\nb\n", 9), "   9│a\n  10│b\n");
    }
}
