//! A small, conservative POSIX-ish shell lexer used for risk assessment.
//!
//! It splits a command line into simple-command segments (on `|`, `&&`,
//! `||`, `;`, `&`, newlines and subshell parentheses), separates
//! redirections from words, removes quoting, and collects command
//! substitutions for recursive analysis. Anything it cannot make sense of
//! is flagged as `unparsable` so callers can fall back to a cautious level.

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Segment {
    pub words: Vec<String>,
    pub redirects: Vec<Redirect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Redirect {
    /// `>`, `>>`, `>|`, `&>`, `&>>`, `>&`, `<`, `<>`, `<<`, `<<<`, `<&`.
    pub op: String,
    /// File name, here-doc body (for `<<`), or fd for duplications.
    pub target: String,
    pub fd_dup: bool,
}

impl Redirect {
    /// True when the redirection may write to its target. `<>` opens the
    /// file read-write and therefore counts as output.
    pub fn is_output(&self) -> bool {
        self.op.contains('>')
    }
}

/// Quoting/escaping rules of the target shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellDialect {
    /// sh/bash/zsh: `\` escapes, backticks are command substitution.
    #[default]
    Posix,
    /// PowerShell: `` ` `` escapes, `\` is a literal path separator.
    PowerShell,
    /// cmd.exe: `^` escapes, `\` is a literal path separator.
    Cmd,
}

#[derive(Debug, Default)]
pub(crate) struct Parsed {
    pub segments: Vec<Segment>,
    pub substitutions: Vec<String>,
    pub unparsable: bool,
    pub ifs: bool,
}

#[cfg(test)]
pub(crate) fn parse(command: &str) -> Parsed {
    parse_dialect(command, ShellDialect::Posix)
}

pub(crate) fn parse_dialect(command: &str, dialect: ShellDialect) -> Parsed {
    let mut lexer = Lexer {
        dialect,
        chars: command.chars().collect(),
        i: 0,
        out: Parsed::default(),
        seg: Segment::default(),
        word: String::new(),
        in_word: false,
        pending_redirect: None,
        heredocs: Vec::new(),
    };
    lexer.run();
    lexer.out
}

struct PendingHeredoc {
    delimiter: String,
    strip_tabs: bool,
    segment: usize,
    redirect: usize,
}

struct Lexer {
    dialect: ShellDialect,
    chars: Vec<char>,
    i: usize,
    out: Parsed,
    seg: Segment,
    word: String,
    in_word: bool,
    pending_redirect: Option<String>,
    heredocs: Vec<PendingHeredoc>,
}

impl Lexer {
    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.i + offset).copied()
    }

    fn run(&mut self) {
        while self.i < self.chars.len() {
            let c = self.chars[self.i];
            if self.is_escape(c) {
                match self.peek(1) {
                    Some('\n') => self.i += 2,
                    Some('\r') if self.peek(2) == Some('\n') => self.i += 3,
                    Some(next) => {
                        self.word.push(next);
                        self.in_word = true;
                        self.i += 2;
                    }
                    None => self.i += 1,
                }
                continue;
            }
            match c {
                ' ' | '\t' => {
                    self.end_word();
                    self.i += 1;
                }
                '\'' if self.dialect != ShellDialect::Cmd => {
                    self.in_word = true;
                    self.i += 1;
                    match self.chars[self.i..].iter().position(|&ch| ch == '\'') {
                        Some(end) => {
                            let text: String = self.chars[self.i..self.i + end].iter().collect();
                            self.word.push_str(&text);
                            self.i += end + 1;
                        }
                        None => {
                            self.out.unparsable = true;
                            let text: String = self.chars[self.i..].iter().collect();
                            self.word.push_str(&text);
                            self.i = self.chars.len();
                        }
                    }
                }
                '"' => self.double_quoted(),
                '`' if self.dialect == ShellDialect::Posix => self.backtick(),
                '$' if self.dialect != ShellDialect::Cmd => self.dollar(),
                '#' if !self.in_word && self.dialect != ShellDialect::Cmd => {
                    while self.i < self.chars.len() && self.chars[self.i] != '\n' {
                        self.i += 1;
                    }
                }
                '|' => {
                    self.separator();
                    self.i += if matches!(self.peek(1), Some('|' | '&')) {
                        2
                    } else {
                        1
                    };
                }
                '&' => match self.peek(1) {
                    Some('&') => {
                        self.separator();
                        self.i += 2;
                    }
                    Some('>') => {
                        self.end_word();
                        if self.peek(2) == Some('>') {
                            self.start_redirect("&>>", 3);
                        } else {
                            self.start_redirect("&>", 2);
                        }
                    }
                    _ => {
                        self.separator();
                        self.i += 1;
                    }
                },
                ';' => {
                    self.separator();
                    self.i += if self.peek(1) == Some(';') { 2 } else { 1 };
                }
                '\n' | '\r' => {
                    self.separator();
                    self.i += 1;
                    if c == '\n' {
                        self.read_heredoc_bodies();
                    }
                }
                '(' if !self.in_word => {
                    self.separator();
                    self.i += 1;
                }
                ')' => {
                    self.separator();
                    self.i += 1;
                }
                '>' | '<' => self.redirect(c),
                _ => {
                    self.word.push(c);
                    self.in_word = true;
                    self.i += 1;
                }
            }
        }
        self.separator();
        if !self.heredocs.is_empty() {
            // Here-doc without body on a single line: harmless, nothing to read.
            self.heredocs.clear();
        }
    }

    fn is_escape(&self, c: char) -> bool {
        match self.dialect {
            ShellDialect::Posix => c == '\\',
            ShellDialect::PowerShell => c == '`',
            ShellDialect::Cmd => c == '^',
        }
    }

    fn double_quoted(&mut self) {
        self.in_word = true;
        self.i += 1;
        while self.i < self.chars.len() {
            let c = self.chars[self.i];
            match c {
                '"' => {
                    self.i += 1;
                    return;
                }
                '\\' if self.dialect == ShellDialect::Posix => match self.peek(1) {
                    Some('\n') => self.i += 2,
                    Some(next @ ('$' | '`' | '"' | '\\')) => {
                        self.word.push(next);
                        self.i += 2;
                    }
                    _ => {
                        self.word.push('\\');
                        self.i += 1;
                    }
                },
                '`' if self.dialect == ShellDialect::PowerShell => {
                    if let Some(next) = self.peek(1) {
                        self.word.push(next);
                    }
                    self.i += 2;
                }
                '`' if self.dialect == ShellDialect::Posix => self.backtick(),
                '$' if self.dialect != ShellDialect::Cmd => self.dollar(),
                _ => {
                    self.word.push(c);
                    self.i += 1;
                }
            }
        }
        self.out.unparsable = true;
    }

    fn backtick(&mut self) {
        self.in_word = true;
        let start = self.i + 1;
        let mut j = start;
        let mut inner = String::new();
        while j < self.chars.len() && self.chars[j] != '`' {
            if self.chars[j] == '\\' && j + 1 < self.chars.len() {
                inner.push(self.chars[j + 1]);
                j += 2;
                continue;
            }
            inner.push(self.chars[j]);
            j += 1;
        }
        if j >= self.chars.len() {
            self.out.unparsable = true;
        }
        self.word.push_str("`…`");
        self.out.substitutions.push(inner);
        self.i = (j + 1).min(self.chars.len());
    }

    fn dollar(&mut self) {
        self.in_word = true;
        match self.peek(1) {
            Some('(') if self.peek(2) == Some('(') => {
                // Arithmetic expansion: no commands run.
                let (_, end) = self.capture_balanced(self.i + 1, '(', ')');
                self.word.push_str("$((…))");
                self.i = end;
            }
            Some('(') => {
                let (inner, end) = self.capture_balanced(self.i + 1, '(', ')');
                self.word.push_str("$(…)");
                self.out.substitutions.push(inner);
                self.i = end;
            }
            Some('{') => {
                let (inner, end) = self.capture_balanced(self.i + 1, '{', '}');
                if inner.contains("IFS") {
                    self.out.ifs = true;
                }
                // `${x:-$(cmd)}` style nested substitutions.
                if inner.contains("$(") || inner.contains('`') {
                    self.out.substitutions.push(inner.clone());
                }
                self.word.push_str("${");
                self.word.push_str(&inner);
                self.word.push('}');
                self.i = end;
            }
            _ => {
                let rest: String = self.chars[self.i + 1..].iter().take(3).collect();
                if rest == "IFS" {
                    self.out.ifs = true;
                }
                self.word.push('$');
                self.i += 1;
            }
        }
    }

    /// `open_index` points at the opening delimiter. Returns the inner text
    /// and the index just after the closing delimiter.
    fn capture_balanced(&mut self, open_index: usize, open: char, close: char) -> (String, usize) {
        let mut depth = 0usize;
        let mut j = open_index;
        let mut quote: Option<char> = None;
        while j < self.chars.len() {
            let c = self.chars[j];
            if let Some(q) = quote {
                if c == '\\' && q == '"' {
                    j += 2;
                    continue;
                }
                if c == q {
                    quote = None;
                }
                j += 1;
                continue;
            }
            match c {
                '\\' => {
                    j += 2;
                    continue;
                }
                '\'' | '"' => quote = Some(c),
                c if c == open => depth += 1,
                c if c == close => {
                    depth -= 1;
                    if depth == 0 {
                        let inner: String = self.chars[open_index + 1..j].iter().collect();
                        return (inner, j + 1);
                    }
                }
                _ => {}
            }
            j += 1;
        }
        self.out.unparsable = true;
        let inner: String = self.chars[(open_index + 1).min(self.chars.len())..]
            .iter()
            .collect();
        (inner, self.chars.len())
    }

    fn redirect(&mut self, c: char) {
        // Process substitution `<(cmd)` / `>(cmd)` is an argument.
        if self.peek(1) == Some('(') {
            self.end_word();
            let (inner, end) = self.capture_balanced(self.i + 1, '(', ')');
            self.out.substitutions.push(inner);
            self.word.push_str(if c == '<' { "<(…)" } else { ">(…)" });
            self.in_word = true;
            self.i = end;
            return;
        }
        // A word made only of digits right before the operator is an fd.
        if self.in_word && !self.word.is_empty() && self.word.chars().all(|ch| ch.is_ascii_digit())
        {
            self.word.clear();
            self.in_word = false;
        } else {
            self.end_word();
        }
        let next = self.peek(1);
        let next2 = self.peek(2);
        let (op, len) = match (c, next, next2) {
            ('>', Some('>'), _) => (">>", 2),
            ('>', Some('|'), _) => (">|", 2),
            ('>', Some('&'), _) => (">&", 2),
            ('>', _, _) => (">", 1),
            ('<', Some('<'), Some('<')) => ("<<<", 3),
            ('<', Some('<'), Some('-')) => ("<<-", 3),
            ('<', Some('<'), _) => ("<<", 2),
            ('<', Some('>'), _) => ("<>", 2),
            ('<', Some('&'), _) => ("<&", 2),
            _ => ("<", 1),
        };
        self.start_redirect(op, len);
    }

    fn start_redirect(&mut self, op: &str, len: usize) {
        if self.pending_redirect.is_some() {
            self.out.unparsable = true;
        }
        self.pending_redirect = Some(op.to_string());
        self.i += len;
    }

    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        let word = std::mem::take(&mut self.word);
        self.in_word = false;
        match self.pending_redirect.take() {
            Some(op) if op == "<<" || op == "<<-" => {
                self.seg.redirects.push(Redirect {
                    op: "<<".into(),
                    target: String::new(),
                    fd_dup: false,
                });
                self.heredocs.push(PendingHeredoc {
                    delimiter: word,
                    strip_tabs: op == "<<-",
                    segment: self.out.segments.len(),
                    redirect: self.seg.redirects.len() - 1,
                });
            }
            Some(op) => {
                let fd_dup = (op == ">&" || op == "<&")
                    && (word == "-" || word.chars().all(|ch| ch.is_ascii_digit()));
                self.seg.redirects.push(Redirect {
                    op,
                    target: word,
                    fd_dup,
                });
            }
            None => self.seg.words.push(word),
        }
    }

    fn separator(&mut self) {
        self.end_word();
        if self.pending_redirect.take().is_some() {
            self.out.unparsable = true;
        }
        if !self.seg.words.is_empty() || !self.seg.redirects.is_empty() {
            self.out.segments.push(std::mem::take(&mut self.seg));
        }
    }

    fn read_heredoc_bodies(&mut self) {
        let pending = std::mem::take(&mut self.heredocs);
        for heredoc in pending {
            let mut body = String::new();
            loop {
                if self.i >= self.chars.len() {
                    break;
                }
                let end = self.chars[self.i..]
                    .iter()
                    .position(|&ch| ch == '\n')
                    .map(|offset| self.i + offset)
                    .unwrap_or(self.chars.len());
                let line: String = self.chars[self.i..end].iter().collect();
                self.i = (end + 1).min(self.chars.len());
                let compare = if heredoc.strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line.as_str()
                };
                if compare.trim_end_matches('\r') == heredoc.delimiter {
                    break;
                }
                body.push_str(&line);
                body.push('\n');
            }
            if let Some(redirect) = self
                .out
                .segments
                .get_mut(heredoc.segment)
                .and_then(|segment| segment.redirects.get_mut(heredoc.redirect))
            {
                redirect.target = body;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(command: &str) -> Vec<Vec<String>> {
        parse(command)
            .segments
            .into_iter()
            .map(|segment| segment.words)
            .collect()
    }

    #[test]
    fn splits_on_operators_and_strips_quotes() {
        assert_eq!(
            words("ps aux | grep 'nginx;x' && echo \"a b\"; ls\nr\\m -rf x"),
            vec![
                vec!["ps", "aux"],
                vec!["grep", "nginx;x"],
                vec!["echo", "a b"],
                vec!["ls"],
                vec!["rm", "-rf", "x"],
            ]
        );
    }

    #[test]
    fn redirects_are_separated() {
        let parsed = parse("cmd 2>/dev/null >out.txt 2>&1 < in");
        let segment = &parsed.segments[0];
        assert_eq!(segment.words, vec!["cmd"]);
        let ops: Vec<_> = segment
            .redirects
            .iter()
            .map(|r| (r.op.as_str(), r.target.as_str(), r.fd_dup))
            .collect();
        assert_eq!(
            ops,
            vec![
                (">", "/dev/null", false),
                (">", "out.txt", false),
                (">&", "1", true),
                ("<", "in", false)
            ]
        );
    }

    #[test]
    fn substitutions_are_collected() {
        let parsed = parse("echo \"$(rm -rf x)\" `id` $((1+2)) <(ls)");
        assert_eq!(parsed.substitutions, vec!["rm -rf x", "id", "ls"]);
        assert!(!parsed.unparsable);
    }

    #[test]
    fn heredoc_body_is_attached() {
        let parsed = parse("cat <<'EOF' > /etc/x\nline1\nEOF\necho done");
        assert_eq!(parsed.segments.len(), 2);
        let redirects = &parsed.segments[0].redirects;
        assert_eq!(redirects[0].target, "line1\n");
        assert_eq!(redirects[1].target, "/etc/x");
        assert_eq!(parsed.segments[1].words, vec!["echo", "done"]);
    }

    #[test]
    fn unbalanced_input_is_flagged() {
        assert!(parse("echo 'abc").unparsable);
        assert!(parse("echo $(ls").unparsable);
        assert!(parse("echo >").unparsable);
        assert!(parse("echo ${IFS}x").ifs);
        assert!(parse("cat$IFS/etc/passwd").ifs);
    }
}
