use crate::channel::{AgentExecChannel, DEFAULT_MAX_OUTPUT_BYTES, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::path_guard::{cd_prefix, resolve_workspace_path, shell_quote};
use miaominal_core::profile::ShellType;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct GrepArgs {
    pub pattern: String,
    #[serde(default = "default_dot")]
    pub root: String,
    #[serde(default)]
    pub include: Vec<String>,
    pub max_results: Option<usize>,
    pub max_bytes: Option<usize>,
    #[serde(default)]
    pub case_insensitive: bool,
}

pub async fn grep(channel: &AgentExecChannel, args: GrepArgs) -> AgentResult<ToolOutput> {
    if matches!(channel.shell_type(), ShellType::PowerShell | ShellType::Cmd) {
        super::workspace_info::ensure_exec_shell_detected(channel).await;
    }

    let root = resolve_workspace_path(&args.root)?;
    if !channel.policy_bypass_enabled() {
        channel
            .policy()
            .enforce_path(crate::policy::AgentPathAccess::Read, &root, false)?;
    }

    // Sensitive pattern policy runs before command generation unless policy is bypassed.
    if !channel.policy_bypass_enabled() && crate::policy::is_sensitive_grep_pattern(&args.pattern) {
        return Err(crate::error::AgentError::Denied {
            tool_name: "grep".into(),
            reason: "grep pattern targets sensitive secret material".into(),
        });
    }

    if root == "/" || root == "/root" || root == "/home" {
        return Err(AgentError::InvalidPath(
            "grep requires a narrowed root".into(),
        ));
    }

    let max_results = args.max_results.unwrap_or(100);
    let max_bytes = args.max_bytes.unwrap_or(DEFAULT_MAX_OUTPUT_BYTES);
    let shell_type = channel.shell_type();

    let command = match shell_type {
        ShellType::PowerShell => build_powershell_grep(&args, &root, max_results, max_bytes),
        ShellType::Cmd => build_cmd_grep(&args, &root, max_results),
        _ => {
            // Posix (bash/zsh) and Fish — keep existing rg/grep/find fallback chain
            build_posix_grep(&args, &root, max_results, max_bytes, shell_type)
        }
    };

    Ok(ToolOutput::Text {
        content: channel.exec(command).await?,
        truncated: false,
    })
}

// ── PowerShell: Select-String via Get-ChildItem ──

fn build_powershell_grep(
    args: &GrepArgs,
    root: &str,
    max_results: usize,
    _max_bytes: usize,
) -> String {
    let cd = cd_prefix(ShellType::PowerShell, root);
    let pattern_q = shell_quote(&args.pattern, ShellType::PowerShell);
    let case_flag = if args.case_insensitive {
        ""
    } else {
        " -CaseSensitive"
    };

    let (include_var, include_param) = if args.include.is_empty() {
        (String::new(), String::new())
    } else {
        let patterns: Vec<String> = args
            .include
            .iter()
            .map(|inc| shell_quote(inc, ShellType::PowerShell))
            .collect();
        (
            format!("$include=@({}); ", patterns.join(", ")),
            " -Include $include".to_string(),
        )
    };

    let ps_script = format!(
        "{cd}; {include_var}$root={root_q}; $pattern={pattern_q}; \
         Get-ChildItem -LiteralPath $root -Recurse -File{include_param} \
         -ErrorAction SilentlyContinue | \
         Select-String -Pattern $pattern{case_flag} | \
         ForEach-Object {{ $_.Filename + ':' + $_.LineNumber + ':' + $_.Line }} | \
         Select-Object -First {max_results}",
        cd = cd,
        include_var = include_var,
        root_q = pattern_q_for_assign(root),
        pattern_q = pattern_q,
        include_param = include_param,
        case_flag = case_flag,
        max_results = max_results,
    );
    format!("powershell.exe -NoProfile -Command \"{ps_script}\"")
}

/// PowerShell variable assignment uses single-quoted strings (same as shell_quote output).
fn pattern_q_for_assign(value: &str) -> String {
    shell_quote(value, ShellType::PowerShell)
}

// ── CMD: findstr fallback ──

fn build_cmd_grep(args: &GrepArgs, root: &str, _max_results: usize) -> String {
    let cd = cd_prefix(ShellType::Cmd, root);
    let pattern_q = shell_quote(&args.pattern, ShellType::Cmd);
    // findstr expects the search string in double quotes
    let pattern_dq = format!("\"{}\"", pattern_q);
    let case_flag = if args.case_insensitive { " /i" } else { "" };

    // findstr /s (recurse) /n (line numbers) — output format matches filename:line:content
    format!(
        "{cd} && findstr /s /n{case_flag} {pattern} *",
        cd = cd,
        case_flag = case_flag,
        pattern = pattern_dq,
    )
}

// ── Posix / Fish: existing rg → grep → find -exec grep chain ──

fn build_posix_grep(
    args: &GrepArgs,
    root: &str,
    max_results: usize,
    max_bytes: usize,
    shell_type: ShellType,
) -> String {
    let case_flag = if args.case_insensitive { "-i " } else { "" };
    let include_args = args
        .include
        .iter()
        .map(|include| format!(" --glob {}", shell_quote(include, shell_type)))
        .collect::<String>();
    let find_name_filter = args
        .include
        .first()
        .map(|include| format!(" -name {}", shell_quote(include, shell_type)))
        .unwrap_or_default();

    format!(
        "cd \"$HOME\" && if command -v rg >/dev/null 2>&1; then \
         rg -n {case_flag}--max-count {max_results} --max-columns 300{include_args} -- {pattern} {root}; \
         else find {root} -type f{find_name_filter} -exec grep -n {case_flag}-E -- {pattern} {{}} \\; \
         | head -n {max_results}; fi | head -c {max_bytes}",
        case_flag = case_flag,
        max_results = max_results,
        include_args = include_args,
        find_name_filter = find_name_filter,
        pattern = shell_quote(&args.pattern, shell_type),
        root = shell_quote(root, shell_type),
        max_bytes = max_bytes,
    )
}

fn default_dot() -> String {
    ".".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── PowerShell command generation ──

    #[test]
    fn powershell_grep_command_basic() {
        let args = GrepArgs {
            pattern: "fn main".into(),
            root: ".".into(),
            include: vec![],
            max_results: Some(100),
            max_bytes: Some(65536),
            case_insensitive: false,
        };
        let root = resolve_workspace_path(&args.root).unwrap();
        let cmd = build_powershell_grep(&args, &root, 100, 65536);

        assert!(
            cmd.contains("Select-String"),
            "PowerShell command should use Select-String, got: {cmd}"
        );
        assert!(
            cmd.contains("-CaseSensitive"),
            "Case-sensitive flag should be present, got: {cmd}"
        );
        assert!(
            cmd.contains("Get-ChildItem"),
            "Should use Get-ChildItem for file enumeration, got: {cmd}"
        );
        assert!(
            cmd.contains("ForEach-Object"),
            "Should format output with ForEach-Object, got: {cmd}"
        );
        assert!(
            cmd.contains("Select-Object -First 100"),
            "Should limit results with Select-Object, got: {cmd}"
        );
        assert!(
            !cmd.contains("$include"),
            "Should not have $include when no include patterns, got: {cmd}"
        );
    }

    #[test]
    fn powershell_grep_command_case_insensitive() {
        let args = GrepArgs {
            pattern: "hello".into(),
            root: ".".into(),
            include: vec![],
            max_results: Some(50),
            max_bytes: Some(65536),
            case_insensitive: true,
        };
        let root = resolve_workspace_path(&args.root).unwrap();
        let cmd = build_powershell_grep(&args, &root, 50, 65536);

        assert!(
            !cmd.contains("-CaseSensitive"),
            "Case-insensitive search should omit -CaseSensitive, got: {cmd}"
        );
    }

    #[test]
    fn powershell_grep_command_with_includes() {
        let args = GrepArgs {
            pattern: "struct".into(),
            root: ".".into(),
            include: vec!["*.rs".into(), "*.toml".into()],
            max_results: Some(100),
            max_bytes: Some(65536),
            case_insensitive: false,
        };
        let root = resolve_workspace_path(&args.root).unwrap();
        let cmd = build_powershell_grep(&args, &root, 100, 65536);

        assert!(
            cmd.contains("$include=@("),
            "Should declare $include when include patterns present, got: {cmd}"
        );
        assert!(
            cmd.contains("'*.rs'"),
            "Should quote include pattern, got: {cmd}"
        );
        assert!(
            cmd.contains("'*.toml'"),
            "Should quote second include pattern, got: {cmd}"
        );
        assert!(
            cmd.contains("-Include $include"),
            "Should pass -Include $include to Get-ChildItem, got: {cmd}"
        );
    }

    #[test]
    fn powershell_grep_command_output_format() {
        let args = GrepArgs {
            pattern: "test".into(),
            root: ".".into(),
            include: vec![],
            max_results: Some(10),
            max_bytes: Some(65536),
            case_insensitive: false,
        };
        let root = resolve_workspace_path(&args.root).unwrap();
        let cmd = build_powershell_grep(&args, &root, 10, 65536);

        // Output format: filename:line:content (matching rg's default -n output)
        assert!(
            cmd.contains("$_.Filename") && cmd.contains("$_.LineNumber") && cmd.contains("$_.Line"),
            "Should format as filename:line:content, got: {cmd}"
        );
    }

    // ── CMD command generation ──

    #[test]
    fn cmd_grep_command_basic() {
        let args = GrepArgs {
            pattern: "fn main".into(),
            root: ".".into(),
            include: vec![],
            max_results: Some(100),
            max_bytes: Some(65536),
            case_insensitive: false,
        };
        let root = resolve_workspace_path(&args.root).unwrap();
        let cmd = build_cmd_grep(&args, &root, 100);

        assert!(
            cmd.contains("findstr"),
            "CMD command should use findstr, got: {cmd}"
        );
        assert!(cmd.contains("/s"), "Should recurse with /s, got: {cmd}");
        assert!(
            cmd.contains("/n"),
            "Should show line numbers with /n, got: {cmd}"
        );
        assert!(
            !cmd.contains("/i"),
            "Case-sensitive should omit /i, got: {cmd}"
        );
        assert!(
            cmd.ends_with(" *"),
            "Should search all files with *, got: {cmd}"
        );
    }

    #[test]
    fn cmd_grep_command_case_insensitive() {
        let args = GrepArgs {
            pattern: "hello".into(),
            root: ".".into(),
            include: vec![],
            max_results: Some(50),
            max_bytes: Some(65536),
            case_insensitive: true,
        };
        let root = resolve_workspace_path(&args.root).unwrap();
        let cmd = build_cmd_grep(&args, &root, 50);

        assert!(
            cmd.contains(" /i"),
            "Case-insensitive should add /i flag, got: {cmd}"
        );
    }

    #[test]
    fn cmd_grep_command_cd_prefix() {
        let args = GrepArgs {
            pattern: "test".into(),
            root: "C:\\Users\\demo\\project".into(),
            include: vec![],
            max_results: Some(100),
            max_bytes: Some(65536),
            case_insensitive: false,
        };
        let root = resolve_workspace_path(&args.root).unwrap();
        let cmd = build_cmd_grep(&args, &root, 100);

        assert!(
            cmd.contains("cd /d %USERPROFILE%"),
            "Should prefix with cd to user profile, got: {cmd}"
        );
    }

    // ── Sensitive pattern rejection (applies to all shell types) ──

    #[test]
    fn sensitive_pattern_rejects_password() {
        assert!(
            crate::policy::is_sensitive_grep_pattern("password"),
            "Should reject 'password'"
        );
        assert!(
            crate::policy::is_sensitive_grep_pattern("PASSWORD"),
            "Should reject 'PASSWORD' (case-insensitive)"
        );
    }

    #[test]
    fn sensitive_pattern_rejects_private_key() {
        assert!(
            crate::policy::is_sensitive_grep_pattern("private key"),
            "Should reject 'private key'"
        );
        assert!(
            crate::policy::is_sensitive_grep_pattern("PRIVATE KEY"),
            "Should reject 'PRIVATE KEY'"
        );
    }

    #[test]
    fn sensitive_pattern_rejects_token_and_secret() {
        assert!(crate::policy::is_sensitive_grep_pattern("token"));
        assert!(crate::policy::is_sensitive_grep_pattern("api_secret"));
        assert!(crate::policy::is_sensitive_grep_pattern("my-secret-key"));
    }

    #[test]
    fn sensitive_pattern_rejects_ssh_keys() {
        assert!(crate::policy::is_sensitive_grep_pattern("id_rsa"));
        assert!(crate::policy::is_sensitive_grep_pattern("id_ed25519"));
    }

    #[test]
    fn sensitive_pattern_allows_safe_patterns() {
        assert!(!crate::policy::is_sensitive_grep_pattern("fn main"));
        assert!(!crate::policy::is_sensitive_grep_pattern("struct User"));
        assert!(!crate::policy::is_sensitive_grep_pattern("import React"));
        assert!(!crate::policy::is_sensitive_grep_pattern("TODO"));
    }

    // ── Posix command generation (existing path — sanity checks) ──

    #[test]
    fn posix_grep_command_uses_rg_first() {
        let args = GrepArgs {
            pattern: "fn main".into(),
            root: ".".into(),
            include: vec![],
            max_results: Some(100),
            max_bytes: Some(65536),
            case_insensitive: false,
        };
        let root = resolve_workspace_path(&args.root).unwrap();
        let cmd = build_posix_grep(&args, &root, 100, 65536, ShellType::Posix);

        assert!(
            cmd.contains("rg -n"),
            "Posix command should prefer rg, got: {cmd}"
        );
        assert!(
            cmd.contains("head -c"),
            "Posix command should limit bytes with head -c, got: {cmd}"
        );
    }

    #[test]
    fn posix_grep_command_case_flag() {
        let args = GrepArgs {
            pattern: "hello".into(),
            root: ".".into(),
            include: vec![],
            max_results: Some(50),
            max_bytes: Some(65536),
            case_insensitive: true,
        };
        let root = resolve_workspace_path(&args.root).unwrap();
        let cmd = build_posix_grep(&args, &root, 50, 65536, ShellType::Posix);

        assert!(
            cmd.contains("-i "),
            "Posix command should include -i for case-insensitive, got: {cmd}"
        );
    }

    // ── default_dot ──

    #[test]
    fn default_dot_returns_dot() {
        assert_eq!(default_dot(), ".");
    }
}
