use crate::channel::{AgentExecChannel, ToolOutput};
use crate::error::{AgentError, AgentResult};
use crate::path_guard::{RemotePathKind, cd_prefix, resolve_workspace_path, shell_quote};
use crate::policy::AgentPathAccess;
use base64::Engine as _;
use miaominal_core::profile::ShellType;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct ApplyPatchArgs {
    pub patch: String,
    #[serde(default = "default_dot")]
    pub base_dir: String,
    pub validator: Option<PatchValidator>,
}

#[derive(Debug, Deserialize)]
pub struct PatchValidator {
    pub command: String,
}

pub async fn apply_patch(
    channel: &AgentExecChannel,
    args: ApplyPatchArgs,
) -> AgentResult<ToolOutput> {
    if matches!(channel.shell_type(), ShellType::PowerShell | ShellType::Cmd) {
        super::workspace_info::ensure_exec_shell_detected(channel).await;
    }

    let base_dir = channel
        .authorize_existing_path(
            &args.base_dir,
            AgentPathAccess::Edit,
            RemotePathKind::Directory,
        )
        .await?;
    let base_dir = base_dir.as_str().to_string();
    let patch_paths = extract_patch_target_paths(&args.patch)?;

    if !channel.policy_bypass_enabled() {
        for path in &patch_paths {
            let full_path = format!("{}/{}", base_dir.trim_end_matches(['/', '\\']), path);
            channel
                .policy()
                .enforce_path(AgentPathAccess::Edit, &full_path, true)?;
        }
    }

    let original_shell = channel.shell_type();
    #[cfg(debug_assertions)]
    log::info!(
        "[apply_patch] original_shell={:?} base_dir={:?} patch_len={}",
        original_shell,
        base_dir,
        args.patch.len(),
    );

    let is_windows = matches!(original_shell, ShellType::PowerShell | ShellType::Cmd);

    if !is_windows && !channel.policy_bypass_enabled() {
        ensure_posix_patch_targets_do_not_follow_links(
            channel,
            original_shell,
            &base_dir,
            &patch_paths,
        )
        .await?;
    }

    let patch_output = if is_windows {
        apply_patch_windows(channel, original_shell, &base_dir, &args.patch).await?
    } else {
        apply_patch_posix(channel, original_shell, &base_dir, &args.patch).await?
    };

    if let Some(validator) = args.validator {
        if !channel.policy_bypass_enabled() {
            channel.policy().enforce_command(&validator.command, true)?;
        }
        let validation = super::run_shell::run_shell(
            channel,
            super::run_shell::RunShellArgs {
                command: validator.command,
                cwd: Some(base_dir),
                timeout_seconds: Some(60),
                max_bytes: None,
                shell: None,
            },
        )
        .await?;
        Ok(ToolOutput::Patch {
            summary: patch_output,
            validation: Some(Box::new(validation)),
        })
    } else {
        Ok(ToolOutput::Patch {
            summary: patch_output,
            validation: None,
        })
    }
}

/// POSIX / Fish: delegate to system `patch` command.
async fn apply_patch_posix(
    channel: &AgentExecChannel,
    shell: ShellType,
    base_dir: &str,
    patch: &str,
) -> AgentResult<String> {
    let command = build_patch_command(shell, base_dir, patch)?;
    #[cfg(debug_assertions)]
    log::info!(
        "[apply_patch] posix cmd for {:?} ({} bytes)",
        shell,
        command.len(),
    );
    channel.exec(command).await.map_err(|e| {
        #[cfg(debug_assertions)]
        log::info!("[apply_patch] posix exec failed: {:?}", e);
        e
    })
}

/// Windows (PowerShell / CMD): try external `patch.exe` first;
/// if it is unavailable, fall back to the built-in Rust diff engine.
///
/// CMD sessions cannot use `patch.exe` at all (even if it were installed,
/// there's no heredoc support), so they go straight to the engine.
async fn apply_patch_windows(
    channel: &AgentExecChannel,
    shell: ShellType,
    base_dir: &str,
    patch: &str,
) -> AgentResult<String> {
    if matches!(shell, ShellType::Cmd) {
        #[cfg(debug_assertions)]
        log::info!("[apply_patch] CMD shell - using built-in engine directly");
        return apply_patch_via_engine(channel, base_dir, patch).await;
    }

    let command = build_patch_command(shell, base_dir, patch)?;
    #[cfg(debug_assertions)]
    log::info!(
        "[apply_patch] win cmd for {:?} ({} bytes)",
        shell,
        command.len(),
    );

    match channel.exec(command).await {
        Ok(output) => {
            #[cfg(debug_assertions)]
            log::info!("[apply_patch] external patch OK ({} bytes)", output.len());
            Ok(output)
        }
        Err(exec_err) => {
            #[cfg(debug_assertions)]
            log::info!(
                "[apply_patch] external patch failed ({:?}), trying built-in engine",
                exec_err,
            );
            apply_patch_via_engine(channel, base_dir, patch).await
        }
    }
}

/// Apply a unified diff using the built-in Rust engine.
///
/// Windows fallback reads each target file as bytes, applies hunks locally
/// with context validation, then writes the new bytes back through an encoded
/// PowerShell command. This avoids relying on `patch.exe` or remote shell
/// quoting rules for the diff content.
async fn apply_patch_via_engine(
    channel: &AgentExecChannel,
    base_dir: &str,
    patch: &str,
) -> AgentResult<String> {
    let files = super::patch_engine::parse_unified_diff(patch)
        .map_err(|e| AgentError::InvalidArguments(e.to_string()))?;

    #[cfg(debug_assertions)]
    log::info!("[apply_patch] engine: {} file(s) to patch", files.len(),);

    let mut results: HashMap<String, super::patch_engine::PatchResult<Vec<u8>>> = HashMap::new();

    for file in &files {
        let target_key = super::patch_engine::extract_target_path(file).to_string();
        let target_path = match resolve_patch_target_path(&target_key) {
            Ok(path) => path,
            Err(err) => {
                results.insert(target_key, Err(err));
                continue;
            }
        };

        #[cfg(debug_assertions)]
        log::info!("[apply_patch] engine: processing {target_path:?}");

        let result = apply_windows_file_patch(channel, base_dir, &target_path, file).await;
        results.insert(target_key, result.map(|_| Vec::new()));
    }

    let summary = super::patch_engine::build_summary(&files, &results);
    #[cfg(debug_assertions)]
    log::info!("[apply_patch] engine done:\n{summary}");
    Ok(summary)
}

async fn apply_windows_file_patch(
    channel: &AgentExecChannel,
    base_dir: &str,
    target_path: &str,
    file: &super::patch_engine::FilePatch,
) -> super::patch_engine::PatchResult<()> {
    if file.is_new_file {
        let patched = super::patch_engine::apply_file_patch("", &file.hunks)?;
        write_windows_file(channel, base_dir, target_path, &patched, true).await
    } else if file.is_deleted {
        let original = read_windows_file(channel, base_dir, target_path).await?;
        let patched = super::patch_engine::apply_file_patch(&original, &file.hunks)?;
        if !patched.is_empty() {
            return Err(super::patch_engine::PatchError::Apply(format!(
                "delete patch for {target_path} did not remove all content"
            )));
        }
        delete_windows_file(channel, base_dir, target_path).await
    } else {
        let original = read_windows_file(channel, base_dir, target_path).await?;
        let patched = super::patch_engine::apply_file_patch(&original, &file.hunks)?;
        write_windows_file(channel, base_dir, target_path, &patched, false).await
    }
}

async fn read_windows_file(
    channel: &AgentExecChannel,
    base_dir: &str,
    target_path: &str,
) -> super::patch_engine::PatchResult<String> {
    let command = build_windows_read_file_command(base_dir, target_path);
    let output = channel.exec(command).await.map_err(|err| {
        let msg = err.to_string();
        if msg.contains("file not found:") {
            super::patch_engine::PatchError::NotFound(target_path.to_string())
        } else {
            super::patch_engine::PatchError::Apply(msg)
        }
    })?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(output.trim())
        .map_err(|err| {
            super::patch_engine::PatchError::Apply(format!(
                "invalid base64 read response for {target_path}: {err}"
            ))
        })?;
    String::from_utf8(bytes).map_err(|err| {
        super::patch_engine::PatchError::Apply(format!("{target_path} is not valid UTF-8: {err}"))
    })
}

async fn write_windows_file(
    channel: &AgentExecChannel,
    base_dir: &str,
    target_path: &str,
    content: &str,
    fail_if_exists: bool,
) -> super::patch_engine::PatchResult<()> {
    let content_base64 = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
    let command =
        build_windows_write_file_command(base_dir, target_path, &content_base64, fail_if_exists);
    channel
        .exec(command)
        .await
        .map(|_| ())
        .map_err(|err| super::patch_engine::PatchError::Apply(err.to_string()))
}

async fn delete_windows_file(
    channel: &AgentExecChannel,
    base_dir: &str,
    target_path: &str,
) -> super::patch_engine::PatchResult<()> {
    let command = build_windows_delete_file_command(base_dir, target_path);
    channel
        .exec(command)
        .await
        .map(|_| ())
        .map_err(|err| super::patch_engine::PatchError::Apply(err.to_string()))
}

fn resolve_patch_target_path(path: &str) -> super::patch_engine::PatchResult<String> {
    let normalized = resolve_workspace_path(path)
        .map_err(|err| super::patch_engine::PatchError::Apply(err.to_string()))?;
    if normalized == "." || normalized.starts_with('/') || has_windows_drive_prefix(&normalized) {
        return Err(super::patch_engine::PatchError::Apply(format!(
            "patch target must be relative to the workspace: {path}"
        )));
    }
    Ok(normalized)
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

fn build_windows_read_file_command(base_dir: &str, target_path: &str) -> String {
    let script = format!(
        "{path_setup}; \
         if (-not (Test-Path -LiteralPath $full -PathType Leaf)) {{ \
             Write-Error ('file not found: ' + $path); exit 2 \
         }}; \
         [Convert]::ToBase64String([IO.File]::ReadAllBytes($full))",
        path_setup = ps_path_setup(base_dir, target_path),
    );
    powershell_wrapper(&script)
}

fn build_windows_write_file_command(
    base_dir: &str,
    target_path: &str,
    content_base64: &str,
    fail_if_exists: bool,
) -> String {
    let exists_guard = if fail_if_exists {
        "if (Test-Path -LiteralPath $full) { Write-Error ('file already exists: ' + $path); exit 3 }; "
    } else {
        ""
    };
    let script = format!(
        "{path_setup}; \
         {exists_guard}\
         $parent = Split-Path -Parent $full; \
         if ($parent -and -not (Test-Path -LiteralPath $parent)) {{ \
             New-Item -ItemType Directory -Path $parent -Force | Out-Null \
         }}; \
         $bytes = [Convert]::FromBase64String('{content_base64}'); \
         [IO.File]::WriteAllBytes($full, $bytes); \
         Write-Output ('written: ' + $path)",
        path_setup = ps_path_setup(base_dir, target_path),
        exists_guard = exists_guard,
        content_base64 = content_base64,
    );
    powershell_wrapper(&script)
}

fn build_windows_delete_file_command(base_dir: &str, target_path: &str) -> String {
    let script = format!(
        "{path_setup}; \
         if (-not (Test-Path -LiteralPath $full -PathType Leaf)) {{ \
             Write-Error ('file not found: ' + $path); exit 2 \
         }}; \
         Remove-Item -LiteralPath $full -Force -ErrorAction Stop; \
         Write-Output ('deleted: ' + $path)",
        path_setup = ps_path_setup(base_dir, target_path),
    );
    powershell_wrapper(&script)
}

fn ps_path_setup(base_dir: &str, target_path: &str) -> String {
    format!(
        "{cd}; $path = '{path}'; $base=(Get-Location).Path; $cursor=$base; foreach($segment in $path.Replace('\\','/').Split('/')) {{ if([string]::IsNullOrWhiteSpace($segment)){{continue}}; $cursor=Join-Path $cursor $segment; if(Test-Path -LiteralPath $cursor){{ $item=Get-Item -LiteralPath $cursor -Force; if(($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0){{ throw ('patch target traverses a symbolic link or reparse point: ' + $path) }} }} }}; $full = if ([IO.Path]::IsPathRooted($path)) {{ $path }} else {{ Join-Path $base $path }}",
        cd = cd_prefix(ShellType::PowerShell, base_dir),
        path = ps_escape_single_quoted(target_path),
    )
}

async fn ensure_posix_patch_targets_do_not_follow_links(
    channel: &AgentExecChannel,
    shell_type: ShellType,
    base_dir: &str,
    paths: &[String],
) -> AgentResult<()> {
    let mut checks = String::new();
    for path in paths {
        let mut prefix = String::new();
        for component in path.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            let quoted = shell_quote(&prefix, ShellType::Posix);
            checks.push_str(&format!(
                "if [ -L {quoted} ]; then printf 'patch target traverses a symbolic link: %s\\n' {quoted} >&2; exit 4; fi; "
            ));
        }
    }

    let script = format!(
        "cd \"$HOME\" && cd {base_dir} && {checks}",
        base_dir = shell_quote(base_dir, ShellType::Posix),
    );
    let command = if matches!(shell_type, ShellType::Fish) {
        format!("sh -lc {}", shell_quote(&script, ShellType::Fish))
    } else {
        script
    };
    channel.exec(command).await.map(|_| ())
}

fn ps_escape_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}

/// Wrap a PowerShell scriptlet in an encoded command so embedded quotes cannot
/// be reinterpreted by CMD or PowerShell's command-line parser.
fn powershell_wrapper(script: &str) -> String {
    super::windows::powershell_encoded_command(script)
}

fn build_patch_command(shell: ShellType, base_dir: &str, patch: &str) -> AgentResult<String> {
    match shell {
        ShellType::Posix => Ok(format!(
            "cd \"$HOME\" && cd {base_dir} && patch -p0 <<'MIAOMINAL_AGENT_PATCH'\n{patch}\nMIAOMINAL_AGENT_PATCH",
            base_dir = shell_quote(base_dir, ShellType::Posix),
            patch = patch,
        )),
        ShellType::Fish => Ok(build_fish_patch_command(base_dir, patch)),
        ShellType::PowerShell => {
            let patch_base64 = base64::engine::general_purpose::STANDARD.encode(patch.as_bytes());
            let ps_script = format!(
                "{cd_prefix}\n$patch = [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String('{patch_base64}'))\n$patch | & patch -p0",
                cd_prefix = cd_prefix(shell, base_dir),
                patch_base64 = patch_base64,
            );
            Ok(super::windows::powershell_encoded_command(&ps_script))
        }
        ShellType::Cmd => Err(AgentError::PosixOnly(
            "apply_patch is not supported in CMD sessions. \
                 Use a PowerShell session or install Git for Windows."
                .into(),
        )),
    }
}

fn build_fish_patch_command(base_dir: &str, patch: &str) -> String {
    let patch_args = patch.lines().map(fish_double_quote_arg).collect::<Vec<_>>();
    let patch_args = if patch_args.is_empty() {
        fish_double_quote_arg("")
    } else {
        patch_args.join(" ")
    };
    format!(
        "cd \"$HOME\"; and cd {base_dir}; and printf '%s\\n' {patch_args} | patch -p0",
        base_dir = fish_double_quote_arg(base_dir),
        patch_args = patch_args,
    )
}

fn fish_double_quote_arg(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
    )
}

fn extract_patch_target_paths(patch: &str) -> AgentResult<Vec<String>> {
    let mut paths = Vec::new();
    for line in patch.lines() {
        let Some(rest) = line
            .strip_prefix("--- ")
            .or_else(|| line.strip_prefix("+++ "))
        else {
            continue;
        };
        let raw_path = rest.split('\t').next().unwrap_or("");
        if raw_path.is_empty() || raw_path == "/dev/null" {
            continue;
        }
        let raw_target = resolve_patch_target_path(raw_path)
            .map_err(|error| AgentError::InvalidPath(error.to_string()))?;
        if !paths.contains(&raw_target) {
            paths.push(raw_target);
        }
        let engine_path = raw_path
            .strip_prefix("a/")
            .or_else(|| raw_path.strip_prefix("b/"))
            .unwrap_or(raw_path);
        let engine_target = resolve_patch_target_path(engine_path)
            .map_err(|error| AgentError::InvalidPath(error.to_string()))?;
        if !paths.contains(&engine_target) {
            paths.push(engine_target);
        }
    }
    if paths.is_empty() {
        return Err(AgentError::InvalidArguments(
            "patch does not contain a target path".into(),
        ));
    }
    Ok(paths)
}

fn default_dot() -> String {
    ".".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── build_patch_command tests ──

    #[test]
    fn posix_apply_patch_unchanged() {
        let cmd = build_patch_command(
            ShellType::Posix,
            "/home/user/project",
            "--- a/file.txt\n+++ b/file.txt\n@@ -1 +1 @@\n-old\n+new",
        )
        .unwrap();

        assert!(
            cmd.contains("<<'MIAOMINAL_AGENT_PATCH'"),
            "POSIX should use heredoc"
        );
        assert!(cmd.contains("patch -p0"), "POSIX should call patch -p0");
        assert!(
            cmd.contains("cd \"$HOME\""),
            "POSIX should cd to HOME first"
        );
        assert!(
            cmd.contains("MIAOMINAL_AGENT_PATCH"),
            "POSIX should use MIAOMINAL_AGENT_PATCH sentinel"
        );
        // Verify heredoc content is embedded
        assert!(cmd.contains("--- a/file.txt"));
        assert!(cmd.contains("+++ b/file.txt"));
    }

    #[test]
    fn fish_apply_patch_uses_heredoc() {
        let cmd = build_patch_command(
            ShellType::Fish,
            "/home/user/project",
            "--- a/file.txt\n+++ b/file.txt\n@@ -1 +1 @@\n-old\n+new",
        )
        .unwrap();

        assert!(
            !cmd.contains("<<'MIAOMINAL_AGENT_PATCH'"),
            "Fish should not use POSIX heredoc"
        );
        assert!(cmd.contains("printf '%s\\n'"), "Fish should pipe printf");
        assert!(cmd.contains("patch -p0"), "Fish should call patch -p0");
        assert!(
            cmd.contains("cd \"$HOME\"; and cd"),
            "Fish should use fish command chaining"
        );
    }

    #[test]
    fn powershell_patch_unavailable_error() {
        // Test that PowerShell command uses EncodedCommand and patch invocation.
        let cmd = build_patch_command(
            ShellType::PowerShell,
            "C:\\Users\\user\\project",
            "--- a/file.txt\n+++ b/file.txt\n@@ -1 +1 @@\n-\"old\"\n+\"new\"",
        )
        .unwrap();

        assert!(
            cmd.starts_with("powershell.exe -NoProfile -EncodedCommand "),
            "PowerShell should use EncodedCommand"
        );
        assert!(
            !cmd.contains("\"old\""),
            "raw patch content should not be embedded in the outer command"
        );
        assert!(
            !cmd.contains("<<'MIAOMINAL_AGENT_PATCH'"),
            "PowerShell should NOT use POSIX heredoc"
        );
    }

    #[test]
    fn cmd_returns_posix_only_error() {
        let result = build_patch_command(ShellType::Cmd, "C:\\Users\\user\\project", "some diff");

        match result {
            Err(AgentError::PosixOnly(msg)) => {
                assert!(msg.contains("CMD"), "Error should mention CMD: {msg}");
                assert!(
                    msg.contains("PowerShell"),
                    "Error should suggest PowerShell: {msg}"
                );
                assert!(
                    msg.contains("Git for Windows"),
                    "Error should suggest Git for Windows: {msg}"
                );
            }
            other => panic!("Expected PosixOnly error for CMD, got: {other:?}"),
        }
    }

    #[test]
    fn powershell_here_string_contains_patch_content() {
        let patch = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@\n-'@\n+\"bar\"";
        let cmd = build_patch_command(ShellType::PowerShell, "C:\\project", patch).unwrap();

        assert!(
            !cmd.contains(patch),
            "PowerShell outer command should not contain raw patch content"
        );
    }

    #[test]
    fn posix_quotes_path_with_spaces() {
        let cmd = build_patch_command(ShellType::Posix, "/home/user/my project", "diff").unwrap();

        assert!(
            cmd.contains("'/home/user/my project'"),
            "POSIX should single-quote paths with spaces"
        );
    }

    #[test]
    fn powershell_quotes_path_with_spaces() {
        let cmd = build_patch_command(ShellType::PowerShell, "C:\\Users\\user\\my project", "diff")
            .unwrap();

        assert!(cmd.starts_with("powershell.exe -NoProfile -EncodedCommand "));
    }

    #[test]
    fn powershell_encoded_command_round_trips_utf16le() {
        let script = "Set-Location 'C:\\Users\\user\\my project'\n$patch = '\"quoted\"'";
        let encoded = crate::tools::windows::powershell_encoded_payload(script);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("encoded command decodes");
        let units = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();

        assert_eq!(String::from_utf16(&units).unwrap(), script);
    }

    #[test]
    fn fish_patch_command_escapes_expansions_in_patch_lines() {
        let cmd = build_patch_command(
            ShellType::Fish,
            "/home/user/$project",
            "--- a/file\n+++ b/file\n@@ -1 +1 @@\n-$old \"value\" \\ path\n+$new",
        )
        .unwrap();

        assert!(cmd.contains("\"/home/user/\\$project\""));
        assert!(cmd.contains("\"-\\$old \\\"value\\\" \\\\ path\""));
        assert!(cmd.contains("\"+\\$new\""));
    }

    // -- Windows engine command builder tests --

    #[test]
    fn windows_read_file_command_uses_encoded_command() {
        let command = build_windows_read_file_command("C:\\project", "src/lib.rs");

        assert!(command.starts_with("powershell.exe -NoProfile -EncodedCommand "));
        assert!(!command.contains("src/lib.rs"));
        assert!(!command.contains("ReadAllLines"));
    }

    #[test]
    fn windows_write_file_command_hides_quotes_in_encoded_payload() {
        let content_base64 =
            base64::engine::general_purpose::STANDARD.encode(b"let value = \"quoted\";\n");
        let command =
            build_windows_write_file_command(".", "src/quoted file.rs", &content_base64, true);

        assert!(command.starts_with("powershell.exe -NoProfile -EncodedCommand "));
        assert!(!command.contains("quoted file"));
        assert!(!command.contains("\"quoted\""));
    }

    #[test]
    fn windows_delete_file_command_uses_literal_path() {
        let command = build_windows_delete_file_command(".", "src/[literal].rs");
        let encoded = command
            .strip_prefix("powershell.exe -NoProfile -EncodedCommand ")
            .expect("encoded command prefix");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("encoded command decodes");
        let units = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        let script = String::from_utf16(&units).unwrap();

        assert!(script.contains("Remove-Item -LiteralPath $full"));
        assert!(script.contains("src/[literal].rs"));
        assert!(script.contains("ReparsePoint"));
    }

    #[test]
    fn patch_target_path_rejects_parent_segments() {
        let err = resolve_patch_target_path("../outside.txt").unwrap_err();
        assert!(err.to_string().contains(".."));
    }

    #[test]
    fn patch_target_path_rejects_absolute_windows_drive() {
        let err = resolve_patch_target_path("C:/outside.txt").unwrap_err();
        assert!(err.to_string().contains("relative"));
    }

    #[test]
    fn patch_target_extraction_strips_diff_prefixes() {
        let paths = extract_patch_target_paths(
            "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new",
        )
        .unwrap();
        assert_eq!(paths, vec!["a/src/lib.rs", "src/lib.rs", "b/src/lib.rs"]);
    }
}
