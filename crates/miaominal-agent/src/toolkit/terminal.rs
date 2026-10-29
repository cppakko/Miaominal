//! Read-only access to the user's terminal (design §7.3). Implemented by
//! the desktop UI; the agent crate never depends on terminal internals.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalSnapshot {
    /// Execution target the terminal is connected to.
    pub target: String,
    pub title: String,
    pub selection: Option<String>,
    /// Recent lines, ANSI removed.
    pub tail: String,
    pub cwd: Option<String>,
    /// A foreground program is running (PTY execution is unavailable).
    pub busy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalReadRequest {
    pub target: Option<String>,
    pub lines: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalReadResult {
    pub target: String,
    pub text: String,
    pub truncated: bool,
}

pub trait TerminalContextSource: Send + Sync {
    fn snapshot(&self, target: Option<&str>, tail_lines: usize) -> Option<TerminalSnapshot>;
    fn read(&self, request: TerminalReadRequest) -> Option<TerminalReadResult>;
}

/// Render the `<context>` block attached to a user message.
pub fn render_context(snapshot: &TerminalSnapshot, include_tail: bool) -> String {
    let mut block = format!("<context>\nterminal: @{}", snapshot.target);
    if let Some(cwd) = &snapshot.cwd {
        block.push_str(&format!(" cwd={cwd}"));
    }
    block.push('\n');
    if let Some(selection) = snapshot
        .selection
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        block.push_str("<selection>\n");
        block.push_str(selection.trim_end());
        block.push_str("\n</selection>\n");
    }
    if include_tail && !snapshot.tail.trim().is_empty() {
        block.push_str("<terminal_tail>\n");
        block.push_str(snapshot.tail.trim_end());
        block.push_str("\n</terminal_tail>\n");
    }
    block.push_str("</context>");
    block
}

/// Whether a question refers to what is on the screen ("这个报错", "why").
pub fn refers_to_terminal(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "这个",
        "上面",
        "报错",
        "错误",
        "输出",
        "屏幕",
        "终端",
        "怎么回事",
        "error",
        "why",
        "above",
        "this output",
        "failed",
        "what happened",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_block_contains_selection_and_tail() {
        let snapshot = TerminalSnapshot {
            target: "web".into(),
            title: "web".into(),
            selection: Some("Permission denied".into()),
            tail: "$ ls\nfoo\n".into(),
            cwd: Some("/srv".into()),
            busy: false,
        };
        let block = render_context(&snapshot, true);
        assert!(block.starts_with("<context>\nterminal: @web cwd=/srv\n"));
        assert!(block.contains("<selection>\nPermission denied\n</selection>"));
        assert!(block.contains("<terminal_tail>\n$ ls\nfoo\n</terminal_tail>"));
        assert!(!render_context(&snapshot, false).contains("terminal_tail"));
        assert!(refers_to_terminal("这个报错是什么意思"));
        assert!(!refers_to_terminal("帮我写个脚本"));
    }
}
