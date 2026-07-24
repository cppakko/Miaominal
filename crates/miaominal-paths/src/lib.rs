use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};
use tempfile::Builder;

const QUALIFIER: &str = "dev";
const ORGANIZATION: &str = "";
const APPLICATION: &str = "Miaominal";
const LEGACY_ORGANIZATION: &str = "akko";
const LEGACY_APPLICATION: &str = "miaominal";
const ATOMIC_TEMP_PREFIX: &str = ".miaominal-";
const ATOMIC_TEMP_SUFFIX: &str = ".tmp";
const STALE_ATOMIC_TEMP_AGE: Duration = Duration::from_secs(24 * 60 * 60);
static CONFIG_DIR_INITIALIZATION: OnceLock<Result<ConfigDirInitialization, String>> =
    OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigDirInitialization {
    Current {
        path: PathBuf,
    },
    Migrated {
        from: PathBuf,
        to: PathBuf,
    },
    LegacyFallback {
        path: PathBuf,
        intended: PathBuf,
        error: String,
    },
}

impl ConfigDirInitialization {
    pub fn active_dir(&self) -> &Path {
        match self {
            Self::Current { path }
            | Self::LegacyFallback { path, .. }
            | Self::Migrated { to: path, .. } => path,
        }
    }
}

fn project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .ok_or_else(|| anyhow!("failed to locate user config directory"))
}

fn legacy_project_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from(QUALIFIER, LEGACY_ORGANIZATION, LEGACY_APPLICATION)
        .ok_or_else(|| anyhow!("failed to locate legacy user config directory"))
}

pub fn initialize_config_dir() -> Result<ConfigDirInitialization> {
    match CONFIG_DIR_INITIALIZATION.get_or_init(|| {
        let current = project_dirs()
            .map(|dirs| dirs.config_dir().to_path_buf())
            .map_err(|error| format!("{error:#}"))?;
        let legacy = legacy_project_dirs()
            .map(|dirs| dirs.config_dir().to_path_buf())
            .map_err(|error| format!("{error:#}"))?;
        initialize_config_dir_paths(current, legacy).map_err(|error| format!("{error:#}"))
    }) {
        Ok(initialization) => Ok(initialization.clone()),
        Err(error) => Err(anyhow!(error.clone())),
    }
}

pub fn config_dir() -> Result<PathBuf> {
    Ok(initialize_config_dir()?.active_dir().to_path_buf())
}

pub fn config_file(file_name: &str) -> Result<PathBuf> {
    Ok(config_dir()?.join(file_name))
}

fn initialize_config_dir_paths(
    current: PathBuf,
    legacy: PathBuf,
) -> Result<ConfigDirInitialization> {
    if current == legacy {
        return Ok(ConfigDirInitialization::Current { path: current });
    }

    match fs::metadata(&current) {
        Ok(metadata) if metadata.is_dir() => {
            return Ok(ConfigDirInitialization::Current { path: current });
        }
        Ok(_) => {
            return fallback_to_legacy_or_error(
                legacy,
                current.clone(),
                format!("{} exists but is not a directory", current.display()),
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return fallback_to_legacy_or_error(
                legacy,
                current.clone(),
                format!("failed to inspect {}: {error}", current.display()),
            );
        }
    }

    let legacy_link_metadata = match fs::symlink_metadata(&legacy) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ConfigDirInitialization::Current { path: current });
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to inspect legacy config directory {}",
                    legacy.display()
                )
            });
        }
    };

    if legacy_link_metadata.file_type().is_symlink() {
        return fallback_to_legacy_or_error(
            legacy,
            current,
            "legacy config directory is a symbolic link and was not moved".to_string(),
        );
    }
    if !legacy_link_metadata.is_dir() {
        return Err(anyhow!(
            "legacy config path {} exists but is not a directory",
            legacy.display()
        ));
    }

    let Some(parent) = current.parent().map(Path::to_path_buf) else {
        return fallback_to_legacy_or_error(
            legacy,
            current.clone(),
            format!("{} has no parent directory", current.display()),
        );
    };
    if let Err(error) = fs::create_dir_all(&parent) {
        return Ok(ConfigDirInitialization::LegacyFallback {
            path: legacy,
            intended: current,
            error: format!("failed to create {}: {error}", parent.display()),
        });
    }

    match fs::rename(&legacy, &current) {
        Ok(()) => Ok(ConfigDirInitialization::Migrated {
            from: legacy,
            to: current,
        }),
        Err(error) => Ok(ConfigDirInitialization::LegacyFallback {
            path: legacy,
            intended: current.clone(),
            error: format!(
                "failed to move legacy config directory to {}: {error}",
                current.display()
            ),
        }),
    }
}

fn fallback_to_legacy_or_error(
    legacy: PathBuf,
    intended: PathBuf,
    error: String,
) -> Result<ConfigDirInitialization> {
    match fs::metadata(&legacy) {
        Ok(metadata) if metadata.is_dir() => Ok(ConfigDirInitialization::LegacyFallback {
            path: legacy,
            intended,
            error,
        }),
        Ok(_) => Err(anyhow!(
            "{error}; legacy config path {} is not a directory",
            legacy.display()
        )),
        Err(legacy_error) => Err(anyhow!(
            "{error}; legacy config directory {} is unavailable: {legacy_error}",
            legacy.display()
        )),
    }
}

/// Remove abandoned atomic-write files left by a terminated process.
///
/// Recent files are retained so another running Miaominal instance cannot lose
/// an in-progress write.
pub fn cleanup_stale_atomic_write_files() -> Result<usize> {
    cleanup_stale_atomic_write_files_in(&config_dir()?, SystemTime::now(), STALE_ATOMIC_TEMP_AGE)
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
#[allow(
    clippy::permissions_set_readonly_false,
    reason = "this branch only targets platforms where readonly is a file attribute, not Unix mode bits"
)]
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
    fn current_directory_wins_without_touching_legacy_directory() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let current = root.path().join("current");
        let legacy = root.path().join("legacy");
        fs::create_dir_all(&current).expect("current directory should be created");
        fs::create_dir_all(&legacy).expect("legacy directory should be created");
        fs::write(current.join("settings.toml"), "current")
            .expect("current settings should be written");
        fs::write(legacy.join("settings.toml"), "legacy")
            .expect("legacy settings should be written");

        let result = initialize_config_dir_paths(current.clone(), legacy.clone())
            .expect("current directory should be selected");

        assert_eq!(result, ConfigDirInitialization::Current { path: current });
        assert_eq!(
            fs::read_to_string(legacy.join("settings.toml"))
                .expect("legacy settings should remain readable"),
            "legacy"
        );
    }

    #[test]
    fn identical_current_and_legacy_paths_do_not_migrate() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let path = root.path().join("miaominal");

        let result = initialize_config_dir_paths(path.clone(), path.clone())
            .expect("identical paths should be accepted");

        assert_eq!(result, ConfigDirInitialization::Current { path });
    }

    #[test]
    fn legacy_directory_is_moved_as_one_unit() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let current = root.path().join("new-parent").join("config");
        let legacy = root.path().join("old-parent").join("config");
        fs::create_dir_all(&legacy).expect("legacy directory should be created");
        let fixtures: [(&str, &[u8]); 7] = [
            ("settings.toml", b"local_vault_enabled = true\n"),
            ("secret_vault.json", b"encrypted-vault"),
            ("sessions.toml", b"sessions = []\n"),
            ("snippets.toml", b"snippets = []\n"),
            ("known_hosts", b"example.test ssh-ed25519 AAAA\n"),
            ("managed_keys.toml", b"keys = []\n"),
            ("sync_config.toml", b"provider = 'github_gist'\n"),
        ];
        for (file_name, contents) in fixtures {
            fs::write(legacy.join(file_name), contents).expect("legacy fixture should be written");
        }

        let result = initialize_config_dir_paths(current.clone(), legacy.clone())
            .expect("legacy directory should be migrated");

        assert_eq!(
            result,
            ConfigDirInitialization::Migrated {
                from: legacy.clone(),
                to: current.clone(),
            }
        );
        assert!(!legacy.exists());
        for (file_name, contents) in fixtures {
            assert_eq!(
                fs::read(current.join(file_name)).expect("migrated fixture should be readable"),
                contents
            );
        }
    }

    #[test]
    fn failed_parent_creation_falls_back_to_legacy_directory() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let blocking_parent = root.path().join("blocked");
        let current = blocking_parent.join("config");
        let legacy = root.path().join("legacy");
        fs::write(&blocking_parent, "not a directory").expect("blocking file should be written");
        fs::create_dir_all(&legacy).expect("legacy directory should be created");

        let result = initialize_config_dir_paths(current.clone(), legacy.clone())
            .expect("migration failure should use legacy directory");

        assert!(matches!(
            result,
            ConfigDirInitialization::LegacyFallback {
                path,
                intended,
                ..
            } if path == legacy && intended == current
        ));
    }

    #[test]
    fn a_later_process_can_retry_a_failed_migration() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let blocking_parent = root.path().join("blocked");
        let current = blocking_parent.join("config");
        let legacy = root.path().join("legacy");
        fs::write(&blocking_parent, "not a directory").expect("blocking file should be written");
        fs::create_dir_all(&legacy).expect("legacy directory should be created");

        let first = initialize_config_dir_paths(current.clone(), legacy.clone())
            .expect("first attempt should fall back");
        assert!(matches!(
            first,
            ConfigDirInitialization::LegacyFallback { .. }
        ));

        fs::remove_file(&blocking_parent).expect("blocking file should be removed");
        let second = initialize_config_dir_paths(current.clone(), legacy.clone())
            .expect("second process should retry migration");
        assert_eq!(
            second,
            ConfigDirInitialization::Migrated {
                from: legacy,
                to: current,
            }
        );
    }

    #[test]
    fn current_file_falls_back_to_legacy_directory() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let current = root.path().join("current");
        let legacy = root.path().join("legacy");
        fs::write(&current, "not a directory").expect("current file should be written");
        fs::create_dir_all(&legacy).expect("legacy directory should be created");

        let result = initialize_config_dir_paths(current.clone(), legacy.clone())
            .expect("legacy directory should be used");

        assert!(matches!(
            result,
            ConfigDirInitialization::LegacyFallback {
                path,
                intended,
                ..
            } if path == legacy && intended == current
        ));
    }

    #[test]
    fn current_file_without_legacy_directory_is_rejected() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let current = root.path().join("current");
        let legacy = root.path().join("legacy");
        fs::write(&current, "not a directory").expect("current file should be written");

        let error = initialize_config_dir_paths(current, legacy)
            .expect_err("an obstructed current path without fallback should fail");

        assert!(error.to_string().contains("exists but is not a directory"));
        assert!(error.to_string().contains("is unavailable"));
    }

    #[test]
    fn legacy_file_is_rejected() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let current = root.path().join("current");
        let legacy = root.path().join("legacy");
        fs::write(&legacy, "not a directory").expect("legacy file should be written");

        let error = initialize_config_dir_paths(current, legacy)
            .expect_err("legacy file should not be migrated");

        assert!(error.to_string().contains("is not a directory"));
    }

    #[cfg(unix)]
    #[test]
    fn legacy_symlink_is_used_without_being_moved() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temporary directory should be created");
        let current = root.path().join("current");
        let target = root.path().join("target");
        let legacy = root.path().join("legacy");
        fs::create_dir_all(&target).expect("symlink target should be created");
        symlink(&target, &legacy).expect("legacy symlink should be created");

        let result = initialize_config_dir_paths(current.clone(), legacy.clone())
            .expect("legacy symlink should be used as fallback");

        assert!(matches!(
            result,
            ConfigDirInitialization::LegacyFallback {
                path,
                intended,
                ..
            } if path == legacy && intended == current
        ));
        assert!(
            fs::symlink_metadata(&legacy)
                .expect("legacy symlink metadata should be readable")
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_project_paths_match_released_layouts() {
        assert_eq!(
            project_dirs().unwrap().project_path(),
            Path::new("Miaominal")
        );
        assert_eq!(
            legacy_project_dirs().unwrap().project_path(),
            Path::new("akko").join("miaominal")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_project_paths_match_released_layouts() {
        assert_eq!(
            project_dirs().unwrap().project_path(),
            Path::new("dev.Miaominal")
        );
        assert_eq!(
            legacy_project_dirs().unwrap().project_path(),
            Path::new("dev.akko.miaominal")
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_project_paths_are_unchanged() {
        assert_eq!(
            project_dirs().unwrap().project_path(),
            Path::new("miaominal")
        );
        assert_eq!(
            legacy_project_dirs().unwrap().project_path(),
            Path::new("miaominal")
        );
    }

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
