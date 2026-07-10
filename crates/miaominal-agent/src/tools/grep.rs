use crate::channel::{AgentExecChannel, DEFAULT_MAX_OUTPUT_BYTES, ToolOutput};
use crate::error::{AgentError, AgentResult};
#[cfg(any())]
use crate::path_guard::cd_prefix;
#[cfg(test)]
use crate::path_guard::resolve_workspace_path;
use crate::path_guard::{RemotePathKind, shell_quote};
use crate::policy::{AgentPathAccess, posix_find_sensitive_predicate};
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

    let root = channel
        .authorize_existing_path(&args.root, AgentPathAccess::Read, RemotePathKind::Directory)
        .await?;
    let root = root.as_str();

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
        ShellType::PowerShell | ShellType::Cmd => build_windows_grep(
            shell_type,
            &args,
            root,
            max_results,
            max_bytes,
            !channel.policy_bypass_enabled(),
        ),
        _ => {
            // Posix (bash/zsh) and Fish — keep existing rg/grep/find fallback chain
            build_posix_grep(
                &args,
                root,
                max_results,
                max_bytes,
                shell_type,
                !channel.policy_bypass_enabled(),
            )
        }
    };

    Ok(ToolOutput::Text {
        content: channel.exec(command).await?,
        truncated: false,
    })
}

// ── PowerShell: Select-String via Get-ChildItem ──

fn build_windows_grep(
    shell_type: ShellType,
    args: &GrepArgs,
    root: &str,
    max_results: usize,
    _max_bytes: usize,
    guard_sensitive: bool,
) -> String {
    let root_q = shell_quote(root, ShellType::PowerShell);
    let pattern_q = shell_quote(&args.pattern, ShellType::PowerShell);
    let case_flag = if args.case_insensitive {
        ""
    } else {
        " -CaseSensitive"
    };
    let patterns = args
        .include
        .iter()
        .map(|include| shell_quote(include, ShellType::PowerShell))
        .collect::<Vec<_>>()
        .join(", ");
    let guard_sensitive = if guard_sensitive { "true" } else { "false" };

    let ps_script = format!(
        "{sensitive_function}; $root={root_q}; $pattern={pattern_q}; $includes=@({patterns}); $guardSensitive=${guard_sensitive}; $max={max_results}; $count=0; $stack=[Collections.Generic.Stack[string]]::new(); $stack.Push($root); while($stack.Count -gt 0 -and $count -lt $max){{ $dir=$stack.Pop(); foreach($item in @(Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue)){{ if(($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0){{continue}}; if($guardSensitive -and (Test-MiaominalSensitivePath $item.FullName)){{continue}}; if($item.PSIsContainer){{$stack.Push($item.FullName); continue}}; $includeOk=$includes.Count -eq 0; foreach($include in $includes){{if($item.Name -like $include){{$includeOk=$true; break}}}}; if(-not $includeOk){{continue}}; foreach($match in @(Select-String -LiteralPath $item.FullName -Pattern $pattern{case_flag} -ErrorAction SilentlyContinue)){{ $item.Name + ':' + $match.LineNumber + ':' + $match.Line; $count++; if($count -ge $max){{break}} }}; if($count -ge $max){{break}} }} }}",
        sensitive_function = super::windows::powershell_sensitive_path_function(),
    );
    super::windows::powershell_command_for_shell(shell_type, &ps_script)
}

#[cfg(any())]
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
#[cfg(any())]
fn pattern_q_for_assign(value: &str) -> String {
    shell_quote(value, ShellType::PowerShell)
}

// ── CMD: findstr fallback ──

#[cfg(any())]
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
    guard_sensitive: bool,
) -> String {
    let case_flag = if args.case_insensitive { "-i " } else { "" };
    let find_name_filter = if args.include.is_empty() {
        String::new()
    } else {
        let filters = args
            .include
            .iter()
            .map(|include| format!("-name {}", shell_quote(include, ShellType::Posix)))
            .collect::<Vec<_>>()
            .join(" -o ");
        format!(" \\( {filters} \\)")
    };
    let sensitive_guard = if guard_sensitive {
        format!("{} -prune -o -type f", posix_find_sensitive_predicate())
    } else {
        "-type f".to_string()
    };
    let script = format!(
        "cd \"$HOME\" && find -P {root} {sensitive_guard}{find_name_filter} -exec grep -nH {case_flag}-E -- {pattern} {{}} + | head -n {max_results} | head -c {max_bytes}",
        case_flag = case_flag,
        max_results = max_results,
        find_name_filter = find_name_filter,
        pattern = shell_quote(&args.pattern, ShellType::Posix),
        root = shell_quote(root, ShellType::Posix),
        max_bytes = max_bytes,
    );
    if matches!(shell_type, ShellType::Fish) {
        format!("sh -lc {}", shell_quote(&script, ShellType::Fish))
    } else {
        script
    }
}

fn default_dot() -> String {
    ".".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    fn decode_powershell_command(command: &str) -> String {
        let payload = command
            .strip_prefix("powershell.exe -NoProfile -EncodedCommand ")
            .expect("encoded PowerShell command");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .expect("valid base64");
        let units = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        String::from_utf16(&units).expect("valid UTF-16LE")
    }

    #[test]
    fn encoded_windows_grep_hides_untrusted_arguments_and_skips_links() {
        let args = GrepArgs {
            pattern: "error & whoami".into(),
            root: String::new(),
            include: vec!["*.log".into()],
            max_results: Some(100),
            max_bytes: Some(65536),
            case_insensitive: false,
        };
        let powershell = build_windows_grep(
            ShellType::PowerShell,
            &args,
            "C:/x & whoami & rem",
            100,
            65536,
            true,
        );
        assert!(!powershell.contains("whoami"));
        let script = decode_powershell_command(&powershell);
        assert!(script.contains("ReparsePoint"));
        assert!(script.contains("Test-MiaominalSensitivePath"));
        assert!(script.contains("Select-String -LiteralPath"));

        let cmd = build_windows_grep(
            ShellType::Cmd,
            &args,
            "C:/x & whoami & rem",
            100,
            65536,
            true,
        );
        assert!(cmd.starts_with("set MIAOMINAL_AGENT_PS_GZIP="));
        assert!(!cmd.contains("whoami"));
    }

    #[cfg(windows)]
    #[test]
    fn cmd_grep_prunes_sensitive_tree_and_does_not_inject() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("x & echo MIAOMINAL_INJECTED & rem");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("safe.log"), "MIAOMINAL_MATCH").unwrap();
        std::fs::create_dir(root.join(".ssh")).unwrap();
        std::fs::write(root.join(".ssh").join("secret.log"), "MIAOMINAL_MATCH").unwrap();
        let args = GrepArgs {
            pattern: "MIAOMINAL_MATCH".into(),
            root: String::new(),
            include: vec!["*.log".into()],
            max_results: Some(100),
            max_bytes: Some(65536),
            case_insensitive: false,
        };

        let command = build_windows_grep(
            ShellType::Cmd,
            &args,
            root.to_string_lossy().as_ref(),
            100,
            65536,
            true,
        );
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", &command])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert!(stdout.contains("safe.log"));
        assert!(!stdout.contains("secret.log"));
        assert!(!stdout.contains("MIAOMINAL_INJECTED"));
    }

    // ── PowerShell command generation ──

    #[test]
    #[cfg(any())]
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
    #[cfg(any())]
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
    #[cfg(any())]
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
    #[cfg(any())]
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
    #[cfg(any())]
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
    #[cfg(any())]
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
    #[cfg(any())]
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
    fn posix_grep_command_uses_no_follow_find() {
        let args = GrepArgs {
            pattern: "fn main".into(),
            root: ".".into(),
            include: vec![],
            max_results: Some(100),
            max_bytes: Some(65536),
            case_insensitive: false,
        };
        let root = resolve_workspace_path(&args.root).unwrap();
        let cmd = build_posix_grep(&args, &root, 100, 65536, ShellType::Posix, true);

        assert!(
            cmd.contains("find -P"),
            "Posix command should not follow links, got: {cmd}"
        );
        assert!(cmd.contains("-iname '.ssh'"));
        assert!(cmd.contains("-ipath '/etc/shadow'"));
        assert!(cmd.contains("-ipath '/etc/sudoers'"));
        assert!(cmd.contains("-iname '*.env.*'"));
        assert!(cmd.contains("-iname '*.rdp'"));
        assert!(cmd.contains("-iname '*.kdbx'"));
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
        let cmd = build_posix_grep(&args, &root, 50, 65536, ShellType::Posix, true);

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
