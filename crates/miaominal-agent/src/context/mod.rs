//! Prompt and context construction (design doc §7, §8).

use crate::policy::ApprovalMode;
use crate::transcript::{Item, ItemKind};

const SYSTEM_PROMPT: &str = include_str!("prompts/system.md");

/// Facts about one execution target, rendered into `<environment>`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TargetInfo {
    /// Name the model uses in the `target` argument (without `@`).
    pub name: String,
    pub is_default: bool,
    pub host: String,
    pub user: String,
    /// e.g. "linux", "macos", "windows"; empty when unknown.
    pub os: String,
    /// Shell that runs tool commands, e.g. "posix-sh", "fish", "powershell", "cmd".
    pub exec_shell: String,
    pub cwd: String,
    /// True when commands run in the user's visible terminal (PTY mode).
    pub pty: bool,
    /// Extra facts such as "tools: rg=yes jq=no".
    pub notes: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptOptions {
    pub mode: ApprovalMode,
    pub web_search: bool,
    pub terminal_available: bool,
}

pub fn system_prompt(targets: &[TargetInfo], options: PromptOptions) -> String {
    let mut prompt = String::with_capacity(SYSTEM_PROMPT.len() + 1024);
    prompt.push_str(SYSTEM_PROMPT.trim_end());
    prompt.push_str("\n\n");
    prompt.push_str(&environment_block(targets));
    prompt.push_str("\n\n");
    prompt.push_str(mode_note(options));
    prompt
}

pub fn environment_block(targets: &[TargetInfo]) -> String {
    if targets.is_empty() {
        return "<environment>\nNo execution target is connected. You cannot run commands or read remote files; \
answer from knowledge and ask the user to open a session if tools are needed.\n</environment>"
            .to_string();
    }
    let mut block = String::from("<environment>\n");
    for target in targets {
        block.push_str(&format!(
            "  <target name=\"@{}\"{}>\n",
            target.name,
            if target.is_default {
                " default=\"true\""
            } else {
                ""
            }
        ));
        let mut line = Vec::new();
        if !target.host.is_empty() {
            line.push(format!("host={}", target.host));
        }
        if !target.user.is_empty() {
            line.push(format!("user={}", target.user));
        }
        if !target.os.is_empty() {
            line.push(format!("os={}", target.os));
        }
        if !line.is_empty() {
            block.push_str(&format!("    {}\n", line.join(" ")));
        }
        let mut line = Vec::new();
        if !target.exec_shell.is_empty() {
            line.push(format!("exec_shell={}", target.exec_shell));
        }
        if !target.cwd.is_empty() {
            line.push(format!("cwd={}", target.cwd));
        }
        line.push(format!(
            "exec_mode={}",
            if target.pty {
                "pty (commands run in the user's visible terminal)"
            } else {
                "exec-channel"
            }
        ));
        block.push_str(&format!("    {}\n", line.join(" ")));
        for note in &target.notes {
            block.push_str(&format!("    {note}\n"));
        }
        block.push_str("  </target>\n");
    }
    block.push_str("</environment>");
    block
}

fn mode_note(options: PromptOptions) -> &'static str {
    match options.mode {
        ApprovalMode::ReadOnly => {
            "<mode>read-only: you can inspect but not modify. Only read-only commands are allowed; \
propose changes as commands or patches for the user to apply instead of attempting them.</mode>"
        }
        ApprovalMode::Cautious => {
            "<mode>cautious: read-only actions run immediately; every modifying action waits for the user's approval. \
Group related changes so the user approves fewer, clearer steps.</mode>"
        }
        ApprovalMode::Standard => {
            "<mode>standard: read-only and low-impact actions run immediately; service, package, system-config \
and destructive actions wait for the user's approval.</mode>"
        }
        ApprovalMode::FullAuto => {
            "<mode>full-auto: most actions run without confirmation, so be especially careful: back up before \
editing, verify after changing, and never take destructive actions the user did not ask for.</mode>"
        }
    }
}

/// Rough token estimate: CJK characters count one token each, other text
/// about 3.2 characters per token.
pub fn estimate_tokens(text: &str) -> usize {
    let mut cjk = 0usize;
    let mut other = 0usize;
    for ch in text.chars() {
        if is_cjk(ch) {
            cjk += 1;
        } else {
            other += 1;
        }
    }
    cjk + (other as f64 / 3.2).ceil() as usize
}

fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
}

/// Estimated prompt tokens contributed by an item (images ≈ 1000 tokens).
pub fn estimate_item_tokens(item: &Item) -> usize {
    match &item.kind {
        ItemKind::UserMessage {
            text,
            attachments,
            context,
            ..
        } => {
            let images = attachments.iter().filter(|a| a.is_image()).count();
            let files: usize = attachments
                .iter()
                .filter_map(|a| a.as_text_file())
                .map(|f| estimate_tokens(&f.text))
                .sum();
            estimate_tokens(text)
                + context.as_deref().map(estimate_tokens).unwrap_or(0)
                + files
                + images * 1000
                + 4
        }
        ItemKind::AssistantText { text, .. } => estimate_tokens(text) + 4,
        ItemKind::Reasoning { text, .. } => estimate_tokens(text),
        ItemKind::ToolCall {
            name, arguments, ..
        } => estimate_tokens(name) + estimate_tokens(arguments) + 8,
        ItemKind::ToolResult {
            model_text, elided, ..
        } => {
            if *elided {
                20
            } else {
                estimate_tokens(model_text) + 8
            }
        }
        ItemKind::Summary { text, .. } => estimate_tokens(text) + 16,
        ItemKind::Notice { .. } | ItemKind::Usage { .. } => 0,
    }
}

pub const SUMMARY_INSTRUCTIONS: &str = "Summarize the conversation so far so that work can continue without it. \
Keep: the user's goals; confirmed facts (hosts, paths, config values, versions, error messages); \
changes already made (files edited, services restarted, packages installed) and how they were verified; \
open problems and remaining steps. Omit pleasantries and raw command output. \
Write compact bullet points in the user's language.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_contains_environment_and_mode() {
        let targets = vec![TargetInfo {
            name: "prod-web".into(),
            is_default: true,
            host: "10.0.3.12".into(),
            user: "deploy".into(),
            os: "linux".into(),
            exec_shell: "posix-sh".into(),
            cwd: "/home/deploy".into(),
            pty: false,
            notes: vec![],
        }];
        let prompt = system_prompt(
            &targets,
            PromptOptions {
                mode: ApprovalMode::ReadOnly,
                web_search: false,
                terminal_available: false,
            },
        );
        assert!(prompt.starts_with("You are Miaominal Assistant"));
        assert!(prompt.contains("<target name=\"@prod-web\" default=\"true\">"));
        assert!(prompt.contains("host=10.0.3.12 user=deploy os=linux"));
        assert!(prompt.contains("exec_shell=posix-sh cwd=/home/deploy exec_mode=exec-channel"));
        assert!(prompt.contains("<mode>read-only"));
    }

    #[test]
    fn token_estimate_counts_cjk() {
        assert_eq!(estimate_tokens("你好世界"), 4);
        assert_eq!(estimate_tokens("abcdefghijklmnop"), 5);
    }
}
