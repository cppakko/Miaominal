//! Shell names, executable lookup and local profile validation shared by the editor and runtime.

use crate::ui::i18n;
use std::path::{Path, PathBuf};

/// Whether the configured shell can be located on this machine.
pub(crate) fn local_shell_is_available(program: &str) -> bool {
    let program = program.trim();
    !program.is_empty() && find_program(program).is_some()
}

/// Whether the configured working directory can be used. An empty value means
/// the shell decides its own starting directory.
pub(crate) fn local_working_directory_is_available(directory: &str) -> bool {
    let directory = directory.trim();
    directory.is_empty() || Path::new(directory).is_dir()
}

/// Placeholder for the required shell executable path field.
pub(crate) fn local_shell_placeholder() -> String {
    if cfg!(windows) {
        i18n::string("placeholders.host_editor.local_shell_windows")
    } else {
        i18n::string("placeholders.host_editor.local_shell_posix")
    }
}

/// Label used when a local terminal profile has no user supplied name.
pub(crate) fn local_terminal_label(shell: &str) -> String {
    let shell = shell.trim();
    if shell.is_empty() {
        i18n::string("tabs.initial.local_terminal_title")
    } else {
        shell_display(shell)
    }
}

pub(crate) fn shell_display(program: &str) -> String {
    Path::new(program)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or(program)
        .to_string()
}

/// Locates a shell executable. A path has to point at an executable file; a
/// bare name is resolved through `PATH`, including `PATHEXT` on Windows.
pub(crate) fn find_program(program: &str) -> Option<PathBuf> {
    let program = program.trim();
    let is_path = program.contains('/') || program.contains('\\');
    if is_path && is_executable_file(Path::new(program)) {
        return Some(PathBuf::from(program));
    }
    let mut candidates = if is_path {
        Vec::new()
    } else {
        vec![PathBuf::from(program)]
    };
    if Path::new(program).extension().is_none() {
        candidates.extend(path_candidates_with_extensions(program));
    }
    candidates
        .into_iter()
        .find_map(|candidate| find_in_path(&candidate))
}

fn find_in_path(program: &Path) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(program))
        .find(|candidate| is_executable_file(candidate))
}

/// Whether `path` points at a file the platform can execute directly.
#[cfg(not(windows))]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.is_file()
        && path
            .metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

/// Whether `path` points at a file the platform can execute directly. Windows
/// executable resolution is extension based, so the file needs an extension
/// listed in `PATHEXT`.
#[cfg(windows)]
fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };
    let extension = format!(".{}", extension.to_ascii_lowercase());
    path_extensions()
        .iter()
        .any(|known_extension| known_extension == &extension)
}

#[cfg(windows)]
fn path_candidates_with_extensions(program: &str) -> Vec<PathBuf> {
    path_extensions()
        .into_iter()
        .map(|extension| PathBuf::from(format!("{program}{extension}")))
        .collect()
}

#[cfg(not(windows))]
fn path_candidates_with_extensions(_program: &str) -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn path_extensions() -> Vec<String> {
    let raw = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    let extensions: Vec<String> = raw
        .split(';')
        .map(str::trim)
        .filter(|extension| !extension.is_empty())
        .map(|extension| extension.to_ascii_lowercase())
        .collect();
    if extensions.is_empty() {
        vec![".exe".to_string()]
    } else {
        extensions
    }
}
