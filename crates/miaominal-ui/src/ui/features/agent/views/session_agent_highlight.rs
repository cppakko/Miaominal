//! Lightweight highlighting for tool cards: shell commands and command
//! output. Pure tokenizers (tested) plus a mapping onto MD3 theme roles.
//!
//! A full grammar is unnecessary here: commands may span lines and output is
//! free text, so a small lexer gives stable, readable colors without the
//! markdown code-block chrome that does not fit the narrow panel.

use std::ops::Range;

use gpui_kit::{FontStyle, FontWeight, HighlightStyle, rgb};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::features::agent) enum Token {
    Prompt,
    Command,
    Flag,
    String,
    Variable,
    Operator,
    Comment,
    Error,
    Warning,
    Success,
    Meta,
}

fn is_operator_char(c: char) -> bool {
    matches!(c, '|' | '&' | ';' | '>' | '<' | '(' | ')')
}

fn shell_escape_end(chars: &[(usize, char)], start: usize) -> usize {
    let end = (start + 2).min(chars.len());
    if chars.get(start + 1).is_some_and(|(_, c)| *c == '\r')
        && chars.get(end).is_some_and(|(_, c)| *c == '\n')
    {
        end + 1
    } else {
        end
    }
}

// Return a character index, never a byte offset. Incomplete expansions extend
// to the end of the available input, as they commonly do while streaming.
fn shell_variable_end(chars: &[(usize, char)], start: usize) -> usize {
    let Some(&(_, next)) = chars.get(start + 1) else {
        return start + 1;
    };
    if matches!(next, '{' | '(') {
        let close = if next == '{' { '}' } else { ')' };
        let mut depth = 1;
        let mut quote = None;
        let mut i = start + 2;
        while i < chars.len() {
            let c = chars[i].1;
            if c == '\\' && quote != Some('\'') {
                i = shell_escape_end(chars, i);
                continue;
            }
            if let Some(delimiter) = quote {
                if c == delimiter {
                    quote = None;
                }
            } else if matches!(c, '\'' | '"') {
                quote = Some(c);
            } else if c == next {
                depth += 1;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            i += 1;
        }
        return i;
    }
    if next.is_ascii_digit() || matches!(next, '?' | '@' | '*' | '#' | '$' | '!' | '-') {
        return start + 2;
    }
    let mut i = start + 1;
    if next.is_alphabetic() || next == '_' {
        while i < chars.len() && (chars[i].1.is_alphanumeric() || chars[i].1 == '_') {
            i += 1;
        }
    }
    i
}

/// Tokenize shell words, preserving quoted/escaped newlines and word boundaries.
/// This is deliberately a small POSIX-ish lexer, not a complete shell grammar.
pub(in crate::ui::features::agent) fn shell_tokens(text: &str) -> Vec<(Range<usize>, Token)> {
    let mut tokens = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let end_of = |index: usize| chars.get(index).map_or(text.len(), |(offset, _)| *offset);
    let mut i = 0;
    let mut expect_command = true;
    let mut redirect_target = false;
    if text.starts_with("$ ") {
        tokens.push((0..1, Token::Prompt));
        i = 2;
    }
    while i < chars.len() {
        let (start, c) = chars[i];
        if c.is_whitespace() {
            if c == '\n' {
                expect_command = true;
                redirect_target = false;
            }
            i += 1;
            continue;
        }
        // A continuation by itself is not a word (and cannot consume a command).
        if c == '\\'
            && (chars.get(i + 1).is_some_and(|(_, c)| *c == '\n')
                || (chars.get(i + 1).is_some_and(|(_, c)| *c == '\r')
                    && chars.get(i + 2).is_some_and(|(_, c)| *c == '\n')))
        {
            i = shell_escape_end(&chars, i);
            continue;
        }
        if c == '#' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].1 != '\n' {
                j += 1;
            }
            tokens.push((start..end_of(j), Token::Comment));
            i = j;
            continue;
        }
        if c.is_ascii_digit() {
            let mut j = i + 1;
            while j < chars.len() && chars[j].1.is_ascii_digit() {
                j += 1;
            }
            // A redirection's descriptor is not the command (`2> log cmd`).
            if chars.get(j).is_some_and(|(_, c)| matches!(c, '>' | '<')) {
                i = j;
                continue;
            }
        }
        if is_operator_char(c) {
            let mut j = i + 1;
            let redirection = matches!(c, '>' | '<')
                || (c == '&' && chars.get(j).is_some_and(|(_, c)| *c == '>'));
            // Do not merge unrelated adjacent operators such as `;(`.
            if c == '&' && redirection {
                j += 1;
                if chars.get(j).is_some_and(|(_, c)| *c == '>') {
                    j += 1;
                }
            } else {
                while j < chars.len() && chars[j].1 == c && matches!(c, '|' | '&' | '>' | '<') {
                    j += 1;
                }
                if redirection && chars.get(j).is_some_and(|(_, c)| *c == '&') {
                    j += 1;
                }
            }
            tokens.push((start..end_of(j), Token::Operator));
            if redirection {
                redirect_target = true;
            } else {
                expect_command = c != ')';
                redirect_target = false;
            }
            i = j;
            continue;
        }

        let word_start = i;
        let mut name_end = i;
        if c.is_alphabetic() || c == '_' {
            while name_end < chars.len()
                && (chars[name_end].1.is_alphanumeric() || chars[name_end].1 == '_')
            {
                name_end += 1;
            }
        }
        let is_assignment =
            !redirect_target && name_end > i && chars.get(name_end).is_some_and(|(_, c)| *c == '=');
        let word_token = if redirect_target || is_assignment {
            None
        } else if c == '-' {
            Some(Token::Flag)
        } else if expect_command {
            Some(Token::Command)
        } else {
            None
        };
        if is_assignment {
            tokens.push((start..end_of(name_end), Token::Variable));
            i = name_end + 1;
        }
        let mut plain_start = i;
        while i < chars.len() && !chars[i].1.is_whitespace() && !is_operator_char(chars[i].1) {
            let c = chars[i].1;
            if c == '\\' {
                // Escaped quotes, spaces, hashes and operators stay in this word.
                i = shell_escape_end(&chars, i);
                continue;
            }
            let variable_end = if c == '$' {
                shell_variable_end(&chars, i)
            } else {
                i
            };
            if matches!(c, '\'' | '"') || variable_end > i + 1 {
                if let Some(token) = word_token.filter(|_| plain_start < i) {
                    tokens.push((end_of(plain_start)..end_of(i), token));
                }
                if variable_end > i + 1 {
                    tokens.push((end_of(i)..end_of(variable_end), Token::Variable));
                    i = variable_end;
                } else {
                    let quote = c;
                    let mut string_start = i;
                    i += 1;
                    while i < chars.len() {
                        let c = chars[i].1;
                        if c == '\\' && quote == '"' {
                            i = shell_escape_end(&chars, i);
                            continue;
                        }
                        if c == quote {
                            i += 1;
                            break;
                        }
                        if c == '$' && quote == '"' {
                            let end = shell_variable_end(&chars, i);
                            if end > i + 1 {
                                if string_start < i {
                                    tokens.push((end_of(string_start)..end_of(i), Token::String));
                                }
                                tokens.push((end_of(i)..end_of(end), Token::Variable));
                                i = end;
                                string_start = i;
                                continue;
                            }
                        }
                        i += 1;
                    }
                    if string_start < i {
                        tokens.push((end_of(string_start)..end_of(i), Token::String));
                    }
                }
                plain_start = i;
            } else {
                i += 1;
            }
        }
        if let Some(token) = word_token.filter(|_| plain_start < i) {
            tokens.push((end_of(plain_start)..end_of(i), token));
        }
        if redirect_target {
            // A leading `> file` still leaves the actual command to be read.
            redirect_target = false;
        } else if !is_assignment && word_token != Some(Token::Flag) {
            // Wrapper handling is intentionally limited to argument-free forms.
            expect_command = word_token == Some(Token::Command)
                && matches!(
                    &text[end_of(word_start)..end_of(i)],
                    "sudo" | "doas" | "env" | "time" | "nohup" | "exec" | "xargs" | "watch"
                );
        }
    }
    tokens
}

const ERROR_WORDS: &[&str] = &[
    "error",
    "errors",
    "failed",
    "failure",
    "failures",
    "fatal",
    "denied",
    "not found",
    "no such file",
    "cannot",
    "can't",
    "unable",
    "refused",
    "timed out",
    "panic",
    "exception",
    "错误",
    "失败",
];
const WARNING_WORDS: &[&str] = &["warning", "warnings", "warn", "deprecated", "警告"];
const SUCCESS_WORDS: &[&str] = &["active (running)", "ok", "success", "succeeded", "done"];

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_zero_count(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|c| c == b'0')
}

fn has_output_marker(lower: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| {
        lower.match_indices(marker).any(|(start, matched)| {
            let before = &lower[..start];
            let after = &lower[start + matched.len()..];
            // English markers require whole words; CJK messages need not use spaces.
            if marker.is_ascii()
                && (before.chars().next_back().is_some_and(is_word_char)
                    || after.chars().next().is_some_and(is_word_char))
            {
                return false;
            }
            let previous = before.split_whitespace().next_back().unwrap_or("");
            if matches!(previous, "no" | "not" | "without")
                || is_zero_count(previous.trim_start_matches(['(', '[']))
            {
                return false;
            }
            let after = after.trim_start();
            if let Some(value) = after.strip_prefix('=').or_else(|| after.strip_prefix(':')) {
                let value = value
                    .trim_start()
                    .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ')' | ']' | '}'))
                    .next()
                    .unwrap_or("")
                    .trim_end_matches('.');
                if is_zero_count(value) || matches!(value, "false" | "none") {
                    return false;
                }
            }
            true
        })
    })
}

/// Highlight command output line by line: bounded severity markers, nonzero
/// counters and the `exit_code:` header. Unknown/partial exit values are metadata.
pub(in crate::ui::features::agent) fn output_tokens(text: &str) -> Vec<(Range<usize>, Token)> {
    let mut tokens = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        let range = offset..offset + content.len();
        offset += line.len();
        if content.trim().is_empty() {
            continue;
        }
        let lower = content.to_lowercase();
        if let Some(value) = lower.trim_start().strip_prefix("exit_code:") {
            let value = value.trim();
            let value = value
                .strip_prefix('+')
                .or_else(|| value.strip_prefix('-'))
                .unwrap_or(value);
            let numeric = !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit());
            let failed = numeric && !is_zero_count(value);
            tokens.push((range, if failed { Token::Error } else { Token::Meta }));
        } else if has_output_marker(&lower, ERROR_WORDS) {
            tokens.push((range, Token::Error));
        } else if has_output_marker(&lower, WARNING_WORDS) {
            tokens.push((range, Token::Warning));
        } else if has_output_marker(&lower, SUCCESS_WORDS) {
            tokens.push((range, Token::Success));
        }
    }
    tokens
}

fn diff_hunk_counts(header: &str) -> Option<(usize, usize)> {
    let mut fields = header.strip_prefix("@@ ")?.split_ascii_whitespace();
    let count = |field: &str, prefix| {
        let field = field.strip_prefix(prefix)?;
        let (start, count) = field.split_once(',').unwrap_or((field, "1"));
        start.parse::<usize>().ok()?;
        count.parse::<usize>().ok()
    };
    let old = count(fields.next()?, '-')?;
    let new = count(fields.next()?, '+')?;
    (fields.next()? == "@@").then_some((old, new))
}

/// Unified diff: additions, removals and hunk headers. Hunk counts distinguish
/// literal added/removed `+++`/`---` text from file-header metadata.
pub(in crate::ui::features::agent) fn diff_tokens(text: &str) -> Vec<(Range<usize>, Token)> {
    let mut tokens = Vec::new();
    let mut offset = 0;
    let mut remaining = None;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        let range = offset..offset + content.len();
        offset += line.len();
        let token = if content.starts_with("diff ") {
            remaining = None;
            None
        } else if content.starts_with("@@") {
            remaining = diff_hunk_counts(content);
            Some(Token::Flag)
        } else if let Some((old, new)) = remaining.as_mut() {
            match content.as_bytes().first() {
                Some(b'+') => {
                    *new = new.saturating_sub(1);
                    Some(Token::Success)
                }
                Some(b'-') => {
                    *old = old.saturating_sub(1);
                    Some(Token::Error)
                }
                Some(b' ') => {
                    *old = old.saturating_sub(1);
                    *new = new.saturating_sub(1);
                    None
                }
                _ => None,
            }
        } else if content == "+++"
            || content == "---"
            || content.starts_with("+++ ")
            || content.starts_with("--- ")
            || content.starts_with("+++\t")
            || content.starts_with("---\t")
        {
            Some(Token::Meta)
        } else if content.starts_with('+') {
            Some(Token::Success)
        } else if content.starts_with('-') {
            Some(Token::Error)
        } else {
            None
        };
        if remaining == Some((0, 0)) {
            remaining = None;
        }
        if let Some(token) = token {
            tokens.push((range, token));
        }
    }
    tokens
}

/// Map tokens onto MD3 theme roles.
pub(in crate::ui::features::agent) fn highlight_styles(
    tokens: Vec<(Range<usize>, Token)>,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let material = miaominal_settings::current_theme().material;
    let roles = material.roles;
    let muted = crate::ui::theme::palette_tone_rgb(
        material.palettes.neutral_variant,
        if material.dark { 65 } else { 50 },
    );
    tokens
        .into_iter()
        .filter(|(range, _)| !range.is_empty())
        .map(|(range, token)| {
            let color = |value: u32| Some(rgb(value).into());
            let style = match token {
                Token::Prompt | Token::Meta => HighlightStyle {
                    color: color(muted),
                    ..Default::default()
                },
                Token::Command => HighlightStyle {
                    color: color(roles.primary),
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..Default::default()
                },
                Token::Flag => HighlightStyle {
                    color: color(roles.tertiary),
                    ..Default::default()
                },
                Token::String => HighlightStyle {
                    color: color(roles.secondary),
                    ..Default::default()
                },
                Token::Variable => HighlightStyle {
                    color: color(roles.tertiary),
                    font_weight: Some(FontWeight::MEDIUM),
                    ..Default::default()
                },
                Token::Operator => HighlightStyle {
                    color: color(roles.on_surface_variant),
                    font_weight: Some(FontWeight::BOLD),
                    ..Default::default()
                },
                Token::Comment => HighlightStyle {
                    color: color(muted),
                    font_style: Some(FontStyle::Italic),
                    ..Default::default()
                },
                Token::Error => HighlightStyle {
                    color: color(roles.error),
                    ..Default::default()
                },
                Token::Warning => HighlightStyle {
                    color: color(roles.tertiary),
                    ..Default::default()
                },
                Token::Success => HighlightStyle {
                    color: color(roles.primary),
                    ..Default::default()
                },
            };
            (range, style)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slices(text: &str, tokens: Vec<(Range<usize>, Token)>) -> Vec<(&str, Token)> {
        tokens
            .into_iter()
            .map(|(range, token)| (&text[range], token))
            .collect()
    }

    fn kinds(text: &str) -> Vec<(&str, Token)> {
        slices(text, shell_tokens(text))
    }

    fn assert_all_ranges(text: &str) {
        type Tokenizer = fn(&str) -> Vec<(Range<usize>, Token)>;
        let tokenizers: [Tokenizer; 3] = [shell_tokens, output_tokens, diff_tokens];
        for tokenize in tokenizers {
            let mut previous_end = 0;
            for (range, _) in tokenize(text) {
                assert!(
                    range.start < range.end,
                    "empty/reversed {range:?} in {text:?}"
                );
                assert!(
                    range.end <= text.len(),
                    "out of bounds {range:?} in {text:?}"
                );
                assert!(text.is_char_boundary(range.start), "{range:?} in {text:?}");
                assert!(text.is_char_boundary(range.end), "{range:?} in {text:?}");
                assert!(
                    previous_end <= range.start,
                    "overlapping {range:?} in {text:?}"
                );
                previous_end = range.end;
            }
        }
    }

    #[test]
    fn shell_tokens_cover_common_shapes() {
        assert_eq!(
            kinds("$ sudo systemctl status nginx --no-pager | grep -i \"error\" > /tmp/x # note"),
            vec![
                ("$", Token::Prompt),
                ("sudo", Token::Command),
                ("systemctl", Token::Command),
                ("--no-pager", Token::Flag),
                ("|", Token::Operator),
                ("grep", Token::Command),
                ("-i", Token::Flag),
                ("\"error\"", Token::String),
                (">", Token::Operator),
                ("# note", Token::Comment),
            ]
        );
        assert_eq!(
            kinds("LANG=C ls $HOME && echo ${PATH}"),
            vec![
                ("LANG", Token::Variable),
                ("ls", Token::Command),
                ("$HOME", Token::Variable),
                ("&&", Token::Operator),
                ("echo", Token::Command),
                ("${PATH}", Token::Variable),
            ]
        );
        // Unterminated quotes and multi-byte text never panic.
        assert_eq!(
            kinds("echo '未完"),
            vec![("echo", Token::Command), ("'未完", Token::String)]
        );
    }

    #[test]
    fn output_tokens_mark_errors_and_exit_codes() {
        let text = "exit_code: 1\nstdout:\nok line\nE: Unable to locate package\nwarning: x\n";
        let found: Vec<(&str, Token)> = output_tokens(text)
            .into_iter()
            .map(|(range, token)| (&text[range], token))
            .collect();
        assert_eq!(found[0], ("exit_code: 1", Token::Error));
        assert!(found.contains(&("warning: x", Token::Warning)));
        let ok = output_tokens("exit_code: 0\n");
        assert_eq!(ok[0].1, Token::Meta);
    }

    #[test]
    fn shell_escaped_delimiters_and_hashes_stay_in_words() {
        assert_eq!(
            kinds(r##"echo foo\"bar \#literal x\ #still-word a\|b "a\"#b" # real"##),
            vec![
                ("echo", Token::Command),
                (r##""a\"#b""##, Token::String),
                ("# real", Token::Comment),
            ]
        );
        assert_eq!(
            kinds("echo;# comment\necho ''#literal"),
            vec![
                ("echo", Token::Command),
                (";", Token::Operator),
                ("# comment", Token::Comment),
                ("echo", Token::Command),
                ("''", Token::String),
            ]
        );
    }

    #[test]
    fn shell_double_quotes_expand_variables_but_single_quotes_and_escapes_do_not() {
        assert_eq!(
            kinds(r#"echo "$HOME/${USER}" '$HOME' "\$NO \"quoted\"" pre$PATH"#),
            vec![
                ("echo", Token::Command),
                ("\"", Token::String),
                ("$HOME", Token::Variable),
                ("/", Token::String),
                ("${USER}", Token::Variable),
                ("\"", Token::String),
                ("'$HOME'", Token::String),
                (r#""\$NO \"quoted\"""#, Token::String),
                ("$PATH", Token::Variable),
            ]
        );
    }

    #[test]
    fn shell_multiline_arguments_do_not_start_new_commands() {
        assert_eq!(
            kinds("echo first\\\nsecond \\\nthird \"fourth\nfifth\" sixth\nnext"),
            vec![
                ("echo", Token::Command),
                ("\"fourth\nfifth\"", Token::String),
                ("next", Token::Command),
            ]
        );
        assert_eq!(
            kinds("\\\r\n echo a\\\r\nb \\\r\n c\nnext"),
            vec![("echo", Token::Command), ("next", Token::Command)]
        );
        assert_eq!(
            kinds("LANG=\"a\n$HOME\" cmd arg"),
            vec![
                ("LANG", Token::Variable),
                ("\"a\n", Token::String),
                ("$HOME", Token::Variable),
                ("\"", Token::String),
                ("cmd", Token::Command),
            ]
        );
    }

    #[test]
    fn shell_assignments_and_redirections_preserve_command_position() {
        assert_eq!(
            kinds(r#"LANG="$HOME" >"日志" 2>&1 cmd arg &> log more"#),
            vec![
                ("LANG", Token::Variable),
                ("\"", Token::String),
                ("$HOME", Token::Variable),
                ("\"", Token::String),
                (">", Token::Operator),
                ("\"日志\"", Token::String),
                (">&", Token::Operator),
                ("cmd", Token::Command),
                ("&>", Token::Operator),
            ]
        );
        assert_eq!(kinds("9bad=x"), vec![("9bad=x", Token::Command)]);
        assert_eq!(kinds("=value"), vec![("=value", Token::Command)]);
        assert_eq!(
            kinds("--opt=$HOME"),
            vec![("--opt=", Token::Flag), ("$HOME", Token::Variable)]
        );
    }

    #[test]
    fn shell_expansions_are_balanced_and_special_variables_are_bounded() {
        assert_eq!(
            kinds(r#"echo $(printf '%s' "$(whoami)") ${NAME:-${OTHER}} $$ $? $1 $@ $- $# $12 $"#),
            vec![
                ("echo", Token::Command),
                (r#"$(printf '%s' "$(whoami)")"#, Token::Variable),
                ("${NAME:-${OTHER}}", Token::Variable),
                ("$$", Token::Variable),
                ("$?", Token::Variable),
                ("$1", Token::Variable),
                ("$@", Token::Variable),
                ("$-", Token::Variable),
                ("$#", Token::Variable),
                ("$1", Token::Variable),
            ]
        );
    }

    #[test]
    fn output_markers_do_not_match_substrings_or_zero_counters() {
        for text in [
            "failed=0",
            "0 errors",
            "errors: 000",
            "0 warnings",
            "warnings=0",
            "no errors",
            "without error",
            "not failed",
            "terror",
            "error_count=3",
            "successful_backup",
            "unsuccessful",
            "undone",
            "abandoned",
            "book",
            "success=false",
            "success=0",
            "timed outlandish",
        ] {
            assert!(output_tokens(text).is_empty(), "false marker in {text:?}");
        }
    }

    #[test]
    fn output_markers_keep_nonzero_counts_and_severity_precedence() {
        for (text, expected) in [
            ("failed=01", Token::Error),
            ("1 error", Token::Error),
            ("errors=2", Token::Error),
            ("failed=0 error: real failure", Token::Error),
            ("0 errors, 1 warning", Token::Warning),
            ("failed=0 success=1", Token::Success),
            ("warning: operation failed", Token::Error),
            ("timed out waiting", Token::Error),
            ("OK", Token::Success),
            ("done.", Token::Success),
            ("发生错误：🙂", Token::Error),
            ("警告：磁盘", Token::Warning),
            ("no such file", Token::Error),
            ("not found", Token::Error),
        ] {
            assert_eq!(
                output_tokens(text),
                vec![(0..text.len(), expected)],
                "{text:?}"
            );
        }
    }

    #[test]
    fn exit_codes_require_a_complete_decimal_value_not_a_zero_prefix() {
        for (value, expected) in [
            ("0", Token::Meta),
            ("00", Token::Meta),
            ("-0", Token::Meta),
            ("+000", Token::Meta),
            ("01", Token::Error),
            ("-01", Token::Error),
            ("+1", Token::Error),
            ("999999999999999999999999999999", Token::Error),
            ("", Token::Meta),
            ("-", Token::Meta),
            ("+", Token::Meta),
            ("--1", Token::Meta),
            ("+-1", Token::Meta),
            ("0pending", Token::Meta),
            ("0.1", Token::Meta),
            ("null", Token::Meta),
        ] {
            let text = format!("  EXIT_CODE: {value}\r\n");
            assert_eq!(
                output_tokens(&text),
                vec![(0..text.len() - 2, expected)],
                "{value:?}"
            );
        }
    }

    #[test]
    fn output_ranges_use_original_utf8_offsets_and_exclude_crlf() {
        let text = "\r\n🙂 warning: x\r\n错误：路径\r\nİ warning\r\nOK";
        assert_eq!(
            slices(text, output_tokens(text)),
            vec![
                ("🙂 warning: x", Token::Warning),
                ("错误：路径", Token::Error),
                ("İ warning", Token::Warning),
                ("OK", Token::Success),
            ]
        );
        assert_all_ranges(text);
    }

    #[test]
    fn diff_statuses_distinguish_headers_from_literal_hunk_content() {
        let text = "--- a/路径\r\n+++ b/路径\r\n@@ -1,2 +1,2 @@\r\n--- removed\r\n+++ added\r\n context\r\n\\ No newline at end of file\r\n--- a/next\r\n+++ b/next\r\n@@ -0,0 +1 @@\r\n+new";
        assert_eq!(
            slices(text, diff_tokens(text)),
            vec![
                ("--- a/路径", Token::Meta),
                ("+++ b/路径", Token::Meta),
                ("@@ -1,2 +1,2 @@", Token::Flag),
                ("--- removed", Token::Error),
                ("+++ added", Token::Success),
                ("--- a/next", Token::Meta),
                ("+++ b/next", Token::Meta),
                ("@@ -0,0 +1 @@", Token::Flag),
                ("+new", Token::Success),
            ]
        );
        assert_all_ranges(text);
        let text = "+++value\n---value\n+added\n-removed\n@@ incomplete";
        assert_eq!(
            slices(text, diff_tokens(text)),
            vec![
                ("+++value", Token::Success),
                ("---value", Token::Error),
                ("+added", Token::Success),
                ("-removed", Token::Error),
                ("@@ incomplete", Token::Flag),
            ]
        );
    }

    #[test]
    fn diff_hunk_counts_handle_omitted_zero_and_incomplete_ranges() {
        assert_eq!(diff_hunk_counts("@@ -1 +2 @@ context"), Some((1, 1)));
        assert_eq!(diff_hunk_counts("@@ -0,0 +1,2 @@"), Some((0, 2)));
        assert_eq!(diff_hunk_counts("@@ -1,2 +"), None);
        assert_eq!(diff_hunk_counts("@@ -x +1 @@"), None);
        assert_eq!(
            diff_hunk_counts("@@ -1,99999999999999999999999 +1 @@"),
            None
        );
        assert_eq!(diff_hunk_counts("@@@ -1 -1 +1 @@@"), None);
        let text = "@@ -1,9 +1,9 @@\n-old\n+new\ndiff --git a/x b/x\n--- a/x\n+++ b/x";
        let found = slices(text, diff_tokens(text));
        assert_eq!(
            found[found.len() - 2..],
            [("--- a/x", Token::Meta), ("+++ b/x", Token::Meta)]
        );
    }

    #[test]
    fn every_streaming_utf8_prefix_has_sorted_nonoverlapping_valid_ranges() {
        for text in [
            "",
            "$ ",
            "echo \\",
            "'未完🦀",
            "${",
            "$(",
            "echo \"unterminated ${USER",
            r#"echo "路径$HOME \"quoted\" \${literal}" --opt=$USER"#,
            "LANG=\"$HOME/目录\" echo a\\ b \\#hash \\\n --flag\nprintf '尾'",
            r#"echo $(printf '%s' "$(whoami)") ${NAME:-${OTHER}} $?"#,
            "exit_code: -01\r\n0 errors, 1 warning\r\n错误：🙂\nİ warning\n",
            "--- a/目录\r\n+++ b/目录\r\n@@ -1 +1 @@\r\n--- old\r\n+++ new\r\n",
        ] {
            for end in text
                .char_indices()
                .map(|(i, _)| i)
                .chain(std::iter::once(text.len()))
            {
                assert_all_ranges(&text[..end]);
            }
        }
    }

    #[test]
    fn short_unicode_and_shell_punctuation_inputs_have_valid_ranges() {
        let alphabet = [
            'λ', '🦀', '\\', '"', '\'', '$', '{', '}', '(', ')', '#', ' ', '\n', '>', '=',
        ];
        for a in alphabet {
            for b in alphabet {
                for c in alphabet {
                    assert_all_ranges(&format!("{a}{b}{c}"));
                }
            }
        }
    }
}
