use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tempfile::Builder;

const QUALIFIER: &str = "dev";
const ORGANIZATION: &str = "";
const APPLICATION: &str = "Miaominal";
const ATOMIC_TEMP_PREFIX: &str = ".miaominal-";
const ATOMIC_TEMP_SUFFIX: &str = ".tmp";
const STALE_ATOMIC_TEMP_AGE: Duration = Duration::from_secs(24 * 60 * 60);

pub fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .ok_or_else(|| anyhow!("failed to locate user config directory"))
}

pub fn config_file(file_name: &str) -> Result<PathBuf> {
    Ok(project_dirs()?.config_dir().join(file_name))
}

/// Remove abandoned atomic-write files left by a terminated process.
///
/// Recent files are retained so another running Miaominal instance cannot lose
/// an in-progress write.
pub fn cleanup_stale_atomic_write_files() -> Result<usize> {
    let project_dirs = project_dirs()?;
    cleanup_stale_atomic_write_files_in(
        project_dirs.config_dir(),
        SystemTime::now(),
        STALE_ATOMIC_TEMP_AGE,
    )
}

fn cleanup_stale_atomic_write_files_in(
    directory: &Path,
    now: SystemTime,
    stale_age: Duration,
) -> Result<usize> {
    if !directory.exists() {
        return Ok(0);
    }

    let entries = fs::read_dir(directory)
        .with_context(|| format!("failed to read {}", directory.display()))?;
    let mut removed = 0;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                log::warn!("failed to inspect atomic-write temporary file: {error}");
                continue;
            }
        };
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if !file_name.starts_with(ATOMIC_TEMP_PREFIX) || !file_name.ends_with(ATOMIC_TEMP_SUFFIX) {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => continue,
            Err(error) => {
                log::warn!(
                    "failed to read metadata for stale temporary file {}: {error}",
                    entry.path().display()
                );
                continue;
            }
        };
        let is_stale = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= stale_age);
        if !is_stale {
            continue;
        }

        match fs::remove_file(entry.path()) {
            Ok(()) => removed += 1,
            Err(error) => log::warn!(
                "failed to remove stale temporary file {}: {error}",
                entry.path().display()
            ),
        }
    }

    Ok(removed)
}

/// Durably replace a file without exposing readers to a partially-written value.
///
/// The temporary file is created in the destination directory so the final
/// persist operation stays on the same filesystem and can be atomic.
pub fn atomic_write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Result<()> {
    let path = path.as_ref();
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;

    let mut temporary = Builder::new()
        .prefix(ATOMIC_TEMP_PREFIX)
        .suffix(ATOMIC_TEMP_SUFFIX)
        .tempfile_in(parent)
        .with_context(|| format!("failed to create temporary file in {}", parent.display()))?;

    if let Ok(metadata) = fs::metadata(path) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .with_context(|| format!("failed to copy permissions for {}", path.display()))?;
    }
    restrict_temporary_file_permissions(temporary.as_file(), path)?;

    temporary
        .write_all(contents.as_ref())
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .flush()
        .with_context(|| format!("failed to flush temporary file for {}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .with_context(|| format!("failed to sync temporary file for {}", path.display()))?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to atomically replace {}", path.display()))?;

    sync_parent_directory(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> Result<()> {
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("failed to sync directory {}", parent.display()))
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_temporary_file_permissions(file: &fs::File, path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to restrict permissions for {}", path.display()))
}

#[cfg(not(unix))]
fn restrict_temporary_file_permissions(file: &fs::File, path: &Path) -> Result<()> {
    let mut permissions = file
        .metadata()
        .with_context(|| format!("failed to read permissions for {}", path.display()))?
        .permissions();
    permissions.set_readonly(false);
    file.set_permissions(permissions)
        .with_context(|| format!("failed to set permissions for {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::FileTimes;

    #[test]
    fn atomic_write_creates_and_replaces_file() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("settings.toml");

        atomic_write(&path, b"first").expect("file should be created");
        atomic_write(&path, b"second").expect("file should be replaced");

        assert_eq!(fs::read(&path).expect("file should be readable"), b"second");
        assert_eq!(
            fs::read_dir(directory.path())
                .expect("directory should be readable")
                .count(),
            1
        );
    }

    #[test]
    fn cleanup_only_removes_expired_atomic_write_files() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let old_temporary = directory.path().join(".miaominal-old.tmp");
        let recent_temporary = directory.path().join(".miaominal-recent.tmp");
        let unrelated = directory.path().join("notes.tmp");
        fs::write(&old_temporary, b"old").expect("old temporary file should be written");
        fs::write(&recent_temporary, b"recent").expect("recent file should be written");
        fs::write(&unrelated, b"unrelated").expect("unrelated file should be written");

        let now = SystemTime::now();
        let old_modified = now - STALE_ATOMIC_TEMP_AGE - Duration::from_secs(1);
        fs::File::options()
            .write(true)
            .open(&old_temporary)
            .expect("old temporary file should open")
            .set_times(FileTimes::new().set_modified(old_modified))
            .expect("old modification time should be set");

        let removed =
            cleanup_stale_atomic_write_files_in(directory.path(), now, STALE_ATOMIC_TEMP_AGE)
                .expect("cleanup should succeed");

        assert_eq!(removed, 1);
        assert!(!old_temporary.exists());
        assert!(recent_temporary.exists());
        assert!(unrelated.exists());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_restricts_file_to_current_user() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("settings.toml");
        atomic_write(&path, b"settings").expect("file should be written");

        let mode = fs::metadata(path)
            .expect("file metadata should be readable")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
