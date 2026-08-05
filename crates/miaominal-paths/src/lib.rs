use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io::{Read, Write};
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
const DATA_LOCATION_FILE: &str = "data_location.toml";
pub const APP_INSTANCE_LOCK_FILE: &str = ".miaominal-instance.lock";
const MIGRATION_MARKER_FILE: &str = ".miaominal-migration.toml";
const PORTABLE_FLAG_FILE: &str = "portable.flag";
const PORTABLE_DATA_DIR: &str = "data";
const STALE_ATOMIC_TEMP_AGE: Duration = Duration::from_secs(24 * 60 * 60);
static RUNTIME_CONTEXT: OnceLock<Result<RuntimeContext, String>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeMode {
    Standard,
    Portable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialPolicy {
    SystemKeyring,
    LocalVaultRequired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeWarningKind {
    DataMigrationBeforeSwitchFailed,
    DataMigrationCleanupPending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataDirectoryChangeErrorKind {
    PortableMode,
    RuntimeUnavailable,
    TargetUnavailable,
    TargetUnsafeLink,
    TargetNotDirectory,
    TargetIsFilesystemRoot,
    TargetOverlapsSource,
    TargetNotEmpty,
    TargetNotWritable,
    BootstrapUnavailable,
    Unknown,
}

#[derive(Debug)]
struct ClassifiedDataDirectoryError {
    kind: DataDirectoryChangeErrorKind,
    detail: String,
}

impl fmt::Display for ClassifiedDataDirectoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for ClassifiedDataDirectoryError {}

fn classified_data_directory_error(
    kind: DataDirectoryChangeErrorKind,
    detail: impl Into<String>,
) -> anyhow::Error {
    ClassifiedDataDirectoryError {
        kind,
        detail: detail.into(),
    }
    .into()
}

pub fn data_directory_change_error_kind(error: &anyhow::Error) -> DataDirectoryChangeErrorKind {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<ClassifiedDataDirectoryError>())
        .map(|error| error.kind)
        .unwrap_or(DataDirectoryChangeErrorKind::Unknown)
}

#[derive(Clone, Debug)]
pub struct RuntimeContext {
    mode: RuntimeMode,
    active_data_dir: PathBuf,
    default_data_dir: PathBuf,
    bootstrap_dir: PathBuf,
    config_initialization: ConfigDirInitialization,
    warning: Option<String>,
    warning_kind: Option<RuntimeWarningKind>,
}

impl RuntimeContext {
    pub fn mode(&self) -> RuntimeMode {
        self.mode
    }

    pub fn credential_policy(&self) -> CredentialPolicy {
        match self.mode {
            RuntimeMode::Standard => CredentialPolicy::SystemKeyring,
            RuntimeMode::Portable => CredentialPolicy::LocalVaultRequired,
        }
    }

    pub fn active_data_dir(&self) -> &Path {
        &self.active_data_dir
    }

    pub fn default_data_dir(&self) -> &Path {
        &self.default_data_dir
    }

    pub fn bootstrap_dir(&self) -> &Path {
        &self.bootstrap_dir
    }

    pub fn config_initialization(&self) -> &ConfigDirInitialization {
        &self.config_initialization
    }

    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }

    pub fn warning_kind(&self) -> Option<RuntimeWarningKind> {
        self.warning_kind
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DataLocationConfig {
    #[serde(default = "data_location_version")]
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    migration: Option<DataMigration>,
}

impl Default for DataLocationConfig {
    fn default() -> Self {
        Self {
            version: data_location_version(),
            active_dir: None,
            migration: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum MigrationPhase {
    Copy,
    Cleanup,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DataMigration {
    id: String,
    source: PathBuf,
    target: PathBuf,
    phase: MigrationPhase,
}

#[derive(Serialize, Deserialize)]
struct MigrationMarker {
    id: String,
}

const fn data_location_version() -> u32 {
    1
}

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

pub fn initialize_runtime() -> Result<RuntimeContext> {
    match RUNTIME_CONTEXT
        .get_or_init(|| initialize_runtime_from_environment().map_err(|error| format!("{error:#}")))
    {
        Ok(context) => Ok(context.clone()),
        Err(error) => Err(anyhow!(error.clone())),
    }
}

fn initialize_runtime_from_environment() -> Result<RuntimeContext> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let executable = std::env::current_exe().context("failed to locate current executable")?;
    initialize_runtime_with(&arguments, &executable)
}

fn initialize_runtime_with(
    arguments: &[std::ffi::OsString],
    executable: &Path,
) -> Result<RuntimeContext> {
    let portable_root = portable_root_for_executable(executable)?;
    let portable_requested = arguments.iter().any(|argument| argument == "--portable")
        || portable_root.join(PORTABLE_FLAG_FILE).is_file();
    if portable_requested {
        let data_dir = portable_root.join(PORTABLE_DATA_DIR);
        validate_or_create_portable_data_dir(&portable_root, &data_dir)?;
        return Ok(RuntimeContext {
            mode: RuntimeMode::Portable,
            active_data_dir: data_dir.clone(),
            default_data_dir: data_dir.clone(),
            bootstrap_dir: portable_root,
            config_initialization: ConfigDirInitialization::Current { path: data_dir },
            warning: None,
            warning_kind: None,
        });
    }

    let current = project_dirs()?.config_dir().to_path_buf();
    let legacy = legacy_project_dirs()?.config_dir().to_path_buf();
    initialize_standard_runtime(current, legacy)
}

fn initialize_standard_runtime(current: PathBuf, legacy: PathBuf) -> Result<RuntimeContext> {
    let initialization = initialize_config_dir_paths(current.clone(), legacy)?;
    let bootstrap_dir = initialization.active_dir().to_path_buf();
    fs::create_dir_all(&bootstrap_dir)
        .with_context(|| format!("failed to create {}", bootstrap_dir.display()))?;
    let location_file = bootstrap_dir.join(DATA_LOCATION_FILE);
    let mut location = read_data_location(&location_file)?;
    let mut active = location
        .active_dir
        .clone()
        .unwrap_or_else(|| initialization.active_dir().to_path_buf());
    let mut warning = None;
    let mut warning_kind = None;

    if let Some(migration) = location.migration.clone() {
        match resume_data_migration(
            &location_file,
            &bootstrap_dir,
            &current,
            &mut location,
            migration,
        ) {
            Ok(path) => active = path,
            Err(error) => {
                warning = Some(format!("data directory migration failed: {error:#}"));
                let failed_before_switch = location
                    .migration
                    .as_ref()
                    .is_some_and(|migration| migration.phase == MigrationPhase::Copy);
                warning_kind = Some(if failed_before_switch {
                    RuntimeWarningKind::DataMigrationBeforeSwitchFailed
                } else {
                    RuntimeWarningKind::DataMigrationCleanupPending
                });
                if failed_before_switch
                    && let Err(rollback_error) = rollback_failed_copy_migration(
                        &bootstrap_dir,
                        &location_file,
                        &mut location,
                    )
                {
                    log::warn!("failed to roll back data migration request: {rollback_error:#}");
                }
                active = location
                    .active_dir
                    .clone()
                    .unwrap_or_else(|| initialization.active_dir().to_path_buf());
            }
        }
    }

    fs::create_dir_all(&active)
        .with_context(|| format!("failed to create data directory {}", active.display()))?;
    Ok(RuntimeContext {
        mode: RuntimeMode::Standard,
        active_data_dir: active,
        default_data_dir: current,
        bootstrap_dir,
        config_initialization: initialization,
        warning,
        warning_kind,
    })
}

pub fn runtime_mode() -> Result<RuntimeMode> {
    Ok(initialize_runtime()?.mode())
}

pub fn credential_policy() -> Result<CredentialPolicy> {
    Ok(initialize_runtime()?.credential_policy())
}

pub fn active_data_dir() -> Result<PathBuf> {
    Ok(initialize_runtime()?.active_data_dir().to_path_buf())
}

pub fn default_data_dir() -> Result<PathBuf> {
    Ok(initialize_runtime()?.default_data_dir().to_path_buf())
}

pub fn initialization_outcome() -> Result<RuntimeContext> {
    initialize_runtime()
}

pub fn initialize_config_dir() -> Result<ConfigDirInitialization> {
    Ok(initialize_runtime()?.config_initialization().clone())
}

pub fn config_dir() -> Result<PathBuf> {
    active_data_dir()
}

pub fn config_file(file_name: &str) -> Result<PathBuf> {
    Ok(active_data_dir()?.join(file_name))
}

pub fn schedule_data_dir_change(target: impl AsRef<Path>) -> Result<()> {
    let context = initialize_runtime().map_err(|error| {
        classified_data_directory_error(
            DataDirectoryChangeErrorKind::RuntimeUnavailable,
            format!("runtime data directory is unavailable: {error:#}"),
        )
    })?;
    if context.mode() == RuntimeMode::Portable {
        return Err(classified_data_directory_error(
            DataDirectoryChangeErrorKind::PortableMode,
            "the data directory is fixed in portable mode",
        ));
    }
    let target = target.as_ref();
    validate_migration_target(context.active_data_dir(), target, context.bootstrap_dir())?;
    let target = fs::canonicalize(target).map_err(|error| {
        classified_data_directory_error(
            DataDirectoryChangeErrorKind::TargetUnavailable,
            format!("failed to resolve {}: {error}", target.display()),
        )
    })?;
    let location_file = context.bootstrap_dir().join(DATA_LOCATION_FILE);
    let mut location = read_data_location(&location_file).map_err(|error| {
        classified_data_directory_error(
            DataDirectoryChangeErrorKind::BootstrapUnavailable,
            format!("failed to read data location bootstrap: {error:#}"),
        )
    })?;
    location.active_dir =
        (!paths_refer_to_same_location(context.active_data_dir(), context.default_data_dir()))
            .then(|| context.active_data_dir().to_path_buf());
    location.migration = Some(DataMigration {
        id: uuid::Uuid::new_v4().to_string(),
        source: context.active_data_dir().to_path_buf(),
        target,
        phase: MigrationPhase::Copy,
    });
    write_data_location(&location_file, &location).map_err(|error| {
        classified_data_directory_error(
            DataDirectoryChangeErrorKind::BootstrapUnavailable,
            format!("failed to write data location bootstrap: {error:#}"),
        )
    })
}

pub fn clear_active_data_dir() -> Result<()> {
    let context = initialize_runtime()?;
    clear_active_data_dir_with(&context)
}

fn clear_active_data_dir_with(context: &RuntimeContext) -> Result<()> {
    let location_file = context.bootstrap_dir().join(DATA_LOCATION_FILE);
    let instance_lock_file = context.active_data_dir().join(APP_INSTANCE_LOCK_FILE);
    let mut retained = vec![instance_lock_file.as_path()];
    if context.mode() == RuntimeMode::Standard
        && paths_refer_to_same_location(context.active_data_dir(), context.bootstrap_dir())
    {
        retained.push(location_file.as_path());
    }
    clear_directory_except(context.active_data_dir(), &retained)?;
    fs::create_dir_all(context.active_data_dir()).with_context(|| {
        format!(
            "failed to recreate data directory {}",
            context.active_data_dir().display()
        )
    })
}

fn portable_root_for_executable(executable: &Path) -> Result<PathBuf> {
    let executable = executable
        .canonicalize()
        .unwrap_or_else(|_| executable.to_path_buf());
    let parent = executable
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", executable.display()))?;
    #[cfg(target_os = "macos")]
    {
        if parent.file_name().is_some_and(|name| name == "MacOS")
            && parent
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == "Contents")
            && let Some(bundle_parent) = parent
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
        {
            return Ok(bundle_parent.to_path_buf());
        }
    }
    Ok(parent.to_path_buf())
}

fn validate_or_create_portable_data_dir(root: &Path, data_dir: &Path) -> Result<()> {
    let root_metadata = fs::symlink_metadata(root)
        .with_context(|| format!("failed to inspect portable root {}", root.display()))?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(anyhow!(
            "portable root {} is not a real directory",
            root.display()
        ));
    }
    verify_directory_writable(root)?;
    match fs::symlink_metadata(data_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(anyhow!(
                "portable data path {} is not a real directory",
                data_dir.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(data_dir).with_context(|| {
                format!(
                    "failed to create portable data directory {}",
                    data_dir.display()
                )
            })?;
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", data_dir.display()));
        }
    }
    verify_directory_writable(data_dir)
}

fn verify_directory_writable(directory: &Path) -> Result<()> {
    let temporary = Builder::new()
        .prefix(".miaominal-write-test-")
        .tempfile_in(directory)
        .with_context(|| format!("directory {} is not writable", directory.display()))?;
    temporary
        .close()
        .with_context(|| format!("failed to remove write test in {}", directory.display()))
}

fn paths_refer_to_same_location(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn read_data_location(path: &Path) -> Result<DataLocationConfig> {
    match fs::read_to_string(path) {
        Ok(content) if content.trim().is_empty() => Ok(DataLocationConfig::default()),
        Ok(content) => {
            let config: DataLocationConfig = toml::from_str(&content)
                .with_context(|| format!("failed to parse {}", path.display()))?;
            if config.version != data_location_version() {
                return Err(anyhow!(
                    "unsupported data location version {}",
                    config.version
                ));
            }
            Ok(config)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(DataLocationConfig::default())
        }
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn write_data_location(path: &Path, config: &DataLocationConfig) -> Result<()> {
    let serialized = toml::to_string_pretty(config).context("failed to serialize data location")?;
    atomic_write(path, serialized)
}

fn validate_migration_target(source: &Path, target: &Path, bootstrap_dir: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(target).map_err(|error| {
        classified_data_directory_error(
            DataDirectoryChangeErrorKind::TargetUnavailable,
            format!(
                "failed to inspect target directory {}: {error}",
                target.display()
            ),
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(classified_data_directory_error(
            DataDirectoryChangeErrorKind::TargetUnsafeLink,
            format!("target {} is a symbolic link", target.display()),
        ));
    }
    if !metadata.is_dir() {
        return Err(classified_data_directory_error(
            DataDirectoryChangeErrorKind::TargetNotDirectory,
            format!("target {} is not a real directory", target.display()),
        ));
    }
    let target = fs::canonicalize(target).map_err(|error| {
        classified_data_directory_error(
            DataDirectoryChangeErrorKind::TargetUnavailable,
            format!("failed to resolve {}: {error}", target.display()),
        )
    })?;
    let source = fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    if target.parent().is_none() {
        return Err(classified_data_directory_error(
            DataDirectoryChangeErrorKind::TargetIsFilesystemRoot,
            "filesystem root cannot be used as a data directory",
        ));
    }
    if target == source || target.starts_with(&source) || source.starts_with(&target) {
        return Err(classified_data_directory_error(
            DataDirectoryChangeErrorKind::TargetOverlapsSource,
            "source and target data directories cannot overlap",
        ));
    }
    let target_is_bootstrap = paths_refer_to_same_location(&target, bootstrap_dir);
    let allowed_file = target.join(DATA_LOCATION_FILE);
    for entry in fs::read_dir(&target).map_err(|error| {
        classified_data_directory_error(
            DataDirectoryChangeErrorKind::TargetUnavailable,
            format!("failed to read {}: {error}", target.display()),
        )
    })? {
        let path = entry
            .map_err(|error| {
                classified_data_directory_error(
                    DataDirectoryChangeErrorKind::TargetUnavailable,
                    format!(
                        "failed to inspect an entry in {}: {error}",
                        target.display()
                    ),
                )
            })?
            .path();
        if target_is_bootstrap && paths_refer_to_same_location(&path, &allowed_file) {
            continue;
        }
        return Err(classified_data_directory_error(
            DataDirectoryChangeErrorKind::TargetNotEmpty,
            format!("target directory {} must be empty", target.display()),
        ));
    }
    verify_directory_writable(&target).map_err(|error| {
        classified_data_directory_error(
            DataDirectoryChangeErrorKind::TargetNotWritable,
            format!(
                "target directory {} is not writable: {error:#}",
                target.display()
            ),
        )
    })
}

fn resume_data_migration(
    location_file: &Path,
    bootstrap_dir: &Path,
    default_dir: &Path,
    location: &mut DataLocationConfig,
    mut migration: DataMigration,
) -> Result<PathBuf> {
    if migration.phase == MigrationPhase::Cleanup {
        cleanup_migration_source(&migration.source, bootstrap_dir, location_file)?;
        remove_migration_marker(&migration.target);
        location.migration = None;
        write_data_location(location_file, location)?;
        return Ok(migration.target);
    }

    validate_source_directory(&migration.source)?;
    prepare_or_recover_target(&migration, bootstrap_dir, location_file)?;
    let target_ready = migration_marker_matches(&migration.target, &migration.id)
        && verify_tree_contents(&migration.source, &migration.target, bootstrap_dir).is_ok();
    if !target_ready {
        let stage = migration_stage_path(&migration)?;
        if stage.exists() {
            fs::remove_dir_all(&stage).with_context(|| {
                format!("failed to clear stale migration stage {}", stage.display())
            })?;
        }
        fs::create_dir(&stage)
            .with_context(|| format!("failed to create migration stage {}", stage.display()))?;
        copy_tree(&migration.source, &stage, bootstrap_dir)?;
        verify_tree_contents(&migration.source, &stage, bootstrap_dir)?;
        write_migration_marker(&stage, &migration.id)?;
        materialize_migration_target(&stage, &migration.target, bootstrap_dir, location_file)?;
        verify_tree_contents(&migration.source, &migration.target, bootstrap_dir)?;
    }

    location.active_dir = (!paths_refer_to_same_location(&migration.target, default_dir))
        .then(|| migration.target.clone());
    migration.phase = MigrationPhase::Cleanup;
    location.migration = Some(migration.clone());
    write_data_location(location_file, location)?;

    if let Err(error) = cleanup_migration_source(&migration.source, bootstrap_dir, location_file) {
        return Err(error
            .context("data directory switched, but the previous directory could not be cleaned"));
    }
    remove_migration_marker(&migration.target);
    location.migration = None;
    write_data_location(location_file, location)?;
    Ok(migration.target)
}

fn validate_source_directory(source: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source).with_context(|| {
        format!(
            "failed to inspect source data directory {}",
            source.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(anyhow!(
            "source data path {} is not a real directory",
            source.display()
        ));
    }
    Ok(())
}

fn prepare_or_recover_target(
    migration: &DataMigration,
    bootstrap_dir: &Path,
    location_file: &Path,
) -> Result<()> {
    if migration_marker_matches(&migration.target, &migration.id) {
        return Ok(());
    }
    if paths_refer_to_same_location(&migration.target, bootstrap_dir) {
        clear_directory_except(bootstrap_dir, &[location_file])?;
        return Ok(());
    }
    validate_migration_target(&migration.source, &migration.target, bootstrap_dir)
}

fn rollback_failed_copy_migration(
    bootstrap_dir: &Path,
    location_file: &Path,
    location: &mut DataLocationConfig,
) -> Result<()> {
    let Some(migration) = location.migration.clone() else {
        return Ok(());
    };
    if migration.phase != MigrationPhase::Copy {
        return Ok(());
    }
    let stage = migration_stage_path(&migration)?;
    if stage.exists() {
        fs::remove_dir_all(&stage)
            .with_context(|| format!("failed to remove migration stage {}", stage.display()))?;
    }
    if migration_marker_matches(&migration.target, &migration.id) {
        if paths_refer_to_same_location(&migration.target, bootstrap_dir) {
            clear_directory_except(bootstrap_dir, &[location_file])?;
        } else {
            fs::remove_dir_all(&migration.target).with_context(|| {
                format!(
                    "failed to remove incomplete target {}",
                    migration.target.display()
                )
            })?;
            fs::create_dir(&migration.target).with_context(|| {
                format!(
                    "failed to recreate empty target {}",
                    migration.target.display()
                )
            })?;
        }
    }
    location.migration = None;
    write_data_location(location_file, location)
}

fn migration_stage_path(migration: &DataMigration) -> Result<PathBuf> {
    let parent = migration
        .target
        .parent()
        .ok_or_else(|| anyhow!("migration target has no parent directory"))?;
    let name = migration
        .target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("data");
    Ok(parent.join(format!(".{name}.miaominal-migration-{}", migration.id)))
}

fn materialize_migration_target(
    stage: &Path,
    target: &Path,
    bootstrap_dir: &Path,
    location_file: &Path,
) -> Result<()> {
    if paths_refer_to_same_location(target, bootstrap_dir) {
        clear_directory_except(target, &[location_file])?;
        for entry in fs::read_dir(stage)? {
            let entry = entry?;
            fs::rename(entry.path(), target.join(entry.file_name()))
                .with_context(|| format!("failed to move staged data into {}", target.display()))?;
        }
        fs::remove_dir(stage)?;
        return Ok(());
    }
    fs::remove_dir(target)
        .with_context(|| format!("failed to remove empty target {}", target.display()))?;
    fs::rename(stage, target)
        .with_context(|| format!("failed to activate migrated directory {}", target.display()))
}

fn copy_tree(source: &Path, target: &Path, bootstrap_dir: &Path) -> Result<()> {
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry = entry?;
        let source_path = entry.path();
        if paths_refer_to_same_location(source, bootstrap_dir)
            && entry.file_name() == DATA_LOCATION_FILE
        {
            continue;
        }
        if entry.file_name() == MIGRATION_MARKER_FILE {
            continue;
        }
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            return Err(anyhow!(
                "symbolic links inside the data directory are not supported: {}",
                source_path.display()
            ));
        }
        let target_path = target.join(entry.file_name());
        if metadata.is_dir() {
            fs::create_dir(&target_path)?;
            copy_tree(&source_path, &target_path, bootstrap_dir)?;
            fs::set_permissions(&target_path, metadata.permissions())?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &target_path)?;
            fs::set_permissions(&target_path, metadata.permissions())?;
            verify_file_hash(&source_path, &target_path)?;
        }
    }
    Ok(())
}

fn verify_tree_contents(source: &Path, target: &Path, bootstrap_dir: &Path) -> Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if paths_refer_to_same_location(source, bootstrap_dir)
            && entry.file_name() == DATA_LOCATION_FILE
        {
            continue;
        }
        if entry.file_name() == MIGRATION_MARKER_FILE {
            continue;
        }
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.is_dir() {
            if !target_path.is_dir() {
                return Err(anyhow!(
                    "missing migrated directory {}",
                    target_path.display()
                ));
            }
            verify_tree_contents(&source_path, &target_path, bootstrap_dir)?;
        } else if metadata.is_file() {
            verify_file_hash(&source_path, &target_path)?;
        }
    }
    Ok(())
}

fn verify_file_hash(source: &Path, target: &Path) -> Result<()> {
    if hash_file(source)? != hash_file(target)? {
        return Err(anyhow!(
            "copied file verification failed for {}",
            target.display()
        ));
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<[u8; 32]> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().into())
}

fn write_migration_marker(directory: &Path, id: &str) -> Result<()> {
    let marker = toml::to_string(&MigrationMarker { id: id.to_string() })?;
    atomic_write(directory.join(MIGRATION_MARKER_FILE), marker)
}

fn migration_marker_matches(directory: &Path, id: &str) -> bool {
    fs::read_to_string(directory.join(MIGRATION_MARKER_FILE))
        .ok()
        .and_then(|content| toml::from_str::<MigrationMarker>(&content).ok())
        .is_some_and(|marker| marker.id == id)
}

fn remove_migration_marker(directory: &Path) {
    if let Err(error) = fs::remove_file(directory.join(MIGRATION_MARKER_FILE))
        && error.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("failed to remove data migration marker: {error}");
    }
}

fn cleanup_migration_source(
    source: &Path,
    bootstrap_dir: &Path,
    location_file: &Path,
) -> Result<()> {
    if paths_refer_to_same_location(source, bootstrap_dir) {
        clear_directory_except(source, &[location_file])
    } else {
        match fs::remove_dir_all(source) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("failed to remove {}", source.display()))
            }
        }
    }
}

fn clear_directory_except(directory: &Path, retained: &[&Path]) -> Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if retained
            .iter()
            .any(|retained| paths_refer_to_same_location(retained, &path))
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
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
    atomic_write_with_protection(
        path.as_ref(),
        contents.as_ref(),
        AtomicFileProtection::Default,
    )
}

/// Durably replace a file and restrict it to the current user and SYSTEM on Windows.
///
/// This is intended for files consumed by Win32-OpenSSH, which rejects files
/// inheriting access entries for unrelated local users. Unix atomic writes are
/// already restricted to mode `0600`.
pub fn atomic_write_user_only(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Result<()> {
    atomic_write_with_protection(
        path.as_ref(),
        contents.as_ref(),
        AtomicFileProtection::CurrentUserOnly,
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AtomicFileProtection {
    Default,
    CurrentUserOnly,
}

fn atomic_write_with_protection(
    path: &Path,
    contents: &[u8],
    protection: AtomicFileProtection,
) -> Result<()> {
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
    protect_atomic_path(temporary.path(), path, protection)?;
    protect_existing_destination(path, protection)?;

    temporary
        .write_all(contents)
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
    protect_atomic_path(path, path, protection)?;

    sync_parent_directory(parent)?;
    Ok(())
}

/// Atomically replace `path` and return the exact bytes that occupied the path
/// at the replacement linearization point.
///
/// `None` means the destination did not exist and was created without
/// overwriting another file. Callers can compare the returned value with the
/// version used to produce `contents` and retry from the returned version when
/// another process changed the file concurrently.
pub fn atomic_replace_and_read_previous(
    path: impl AsRef<Path>,
    contents: impl AsRef<[u8]>,
) -> Result<Option<Vec<u8>>> {
    atomic_replace_with_protection(
        path.as_ref(),
        contents.as_ref(),
        AtomicFileProtection::Default,
    )
}

/// Atomically replace a file, return its previous contents, and restrict the
/// resulting file to the current user and SYSTEM on Windows.
pub fn atomic_replace_user_only_and_read_previous(
    path: impl AsRef<Path>,
    contents: impl AsRef<[u8]>,
) -> Result<Option<Vec<u8>>> {
    atomic_replace_with_protection(
        path.as_ref(),
        contents.as_ref(),
        AtomicFileProtection::CurrentUserOnly,
    )
}

fn atomic_replace_with_protection(
    path: &Path,
    contents: &[u8],
    protection: AtomicFileProtection,
) -> Result<Option<Vec<u8>>> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;

    let temporary = prepare_atomic_replacement(path, contents, protection)?;
    protect_existing_destination(path, protection)?;
    let previous = match temporary.persist_noclobber(path) {
        Ok(file) => {
            file.sync_all()
                .with_context(|| format!("failed to sync {}", path.display()))?;
            sync_parent_directory(parent)?;
            None
        }
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            let (file, replacement) = error
                .file
                .keep()
                .map_err(|error| error.error)
                .with_context(|| format!("failed to retain replacement for {}", path.display()))?;
            drop(file);
            exchange_existing_file(path, &replacement)?
        }
        Err(error) => {
            return Err(error.error).with_context(|| {
                format!("failed to create {} without overwriting it", path.display())
            });
        }
    };
    protect_atomic_path(path, path, protection)?;
    Ok(previous)
}

fn prepare_atomic_replacement(
    path: &Path,
    contents: &[u8],
    protection: AtomicFileProtection,
) -> Result<tempfile::NamedTempFile> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
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
    protect_atomic_path(temporary.path(), path, protection)?;
    temporary
        .write_all(contents)
        .with_context(|| format!("failed to write temporary file for {}", path.display()))?;
    temporary
        .flush()
        .with_context(|| format!("failed to flush temporary file for {}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .with_context(|| format!("failed to sync temporary file for {}", path.display()))?;
    Ok(temporary)
}

fn protect_existing_destination(path: &Path, protection: AtomicFileProtection) -> Result<()> {
    if protection == AtomicFileProtection::CurrentUserOnly && path.exists() {
        restrict_path_to_current_user(path)?;
    }
    Ok(())
}

fn protect_atomic_path(
    actual_path: &Path,
    destination: &Path,
    protection: AtomicFileProtection,
) -> Result<()> {
    if protection == AtomicFileProtection::CurrentUserOnly {
        restrict_path_to_current_user(actual_path).with_context(|| {
            format!(
                "failed to restrict atomic file for {}",
                destination.display()
            )
        })?;
    }
    Ok(())
}

#[cfg(windows)]
fn exchange_existing_file(path: &Path, replacement: &Path) -> Result<Option<Vec<u8>>> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_WRITE_THROUGH, ReplaceFileW};

    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
    let backup = Builder::new()
        .prefix(".miaominal-previous-")
        .suffix(".backup")
        .tempfile_in(parent)
        .with_context(|| format!("failed to reserve a backup path in {}", parent.display()))?;
    let (backup_file, backup_path) = backup
        .keep()
        .map_err(|error| error.error)
        .context("failed to retain OpenSSH config backup path")?;
    drop(backup_file);
    fs::remove_file(&backup_path)
        .with_context(|| format!("failed to prepare backup path {}", backup_path.display()))?;

    let path_wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replacement_wide = replacement
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let backup_wide = backup_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replaced = unsafe {
        ReplaceFileW(
            path_wide.as_ptr(),
            replacement_wide.as_ptr(),
            backup_wide.as_ptr(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if replaced == 0 {
        let error = std::io::Error::last_os_error();
        let _ = fs::remove_file(replacement);
        return Err(error).with_context(|| {
            if backup_path.exists() {
                format!(
                    "failed to exchange {}; a recovery copy was retained at {}",
                    path.display(),
                    backup_path.display()
                )
            } else {
                format!("failed to exchange {}", path.display())
            }
        });
    }

    let previous = fs::read(&backup_path)
        .with_context(|| format!("failed to read replaced file {}", backup_path.display()))?;
    fs::remove_file(&backup_path)
        .with_context(|| format!("failed to remove backup {}", backup_path.display()))?;
    sync_parent_directory(parent)?;
    Ok(Some(previous))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn exchange_existing_file(path: &Path, replacement: &Path) -> Result<Option<Vec<u8>>> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path_c = CString::new(path.as_os_str().as_bytes())
        .with_context(|| format!("{} contains a NUL byte", path.display()))?;
    let replacement_c = CString::new(replacement.as_os_str().as_bytes())
        .with_context(|| format!("{} contains a NUL byte", replacement.display()))?;
    let exchanged = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            replacement_c.as_ptr(),
            libc::AT_FDCWD,
            path_c.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if exchanged != 0 {
        let error = std::io::Error::last_os_error();
        let _ = fs::remove_file(replacement);
        return Err(error).with_context(|| format!("failed to exchange {}", path.display()));
    }
    finish_exchanged_file(path, replacement)
}

#[cfg(target_os = "macos")]
fn exchange_existing_file(path: &Path, replacement: &Path) -> Result<Option<Vec<u8>>> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path_c = CString::new(path.as_os_str().as_bytes())
        .with_context(|| format!("{} contains a NUL byte", path.display()))?;
    let replacement_c = CString::new(replacement.as_os_str().as_bytes())
        .with_context(|| format!("{} contains a NUL byte", replacement.display()))?;
    let exchanged =
        unsafe { libc::renamex_np(replacement_c.as_ptr(), path_c.as_ptr(), libc::RENAME_SWAP) };
    if exchanged != 0 {
        let error = std::io::Error::last_os_error();
        let _ = fs::remove_file(replacement);
        return Err(error).with_context(|| format!("failed to exchange {}", path.display()));
    }
    finish_exchanged_file(path, replacement)
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn exchange_existing_file(path: &Path, replacement: &Path) -> Result<Option<Vec<u8>>> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
    let backup = Builder::new()
        .prefix(".miaominal-previous-")
        .suffix(".backup")
        .tempfile_in(parent)
        .with_context(|| format!("failed to reserve a backup path in {}", parent.display()))?;
    let (backup_file, backup_path) = backup
        .keep()
        .map_err(|error| error.error)
        .context("failed to retain OpenSSH config backup path")?;
    drop(backup_file);
    fs::remove_file(&backup_path)
        .with_context(|| format!("failed to prepare backup path {}", backup_path.display()))?;

    fs::rename(path, &backup_path).with_context(|| {
        format!(
            "failed to move {} to recovery backup {}",
            path.display(),
            backup_path.display()
        )
    })?;
    if let Err(error) = fs::hard_link(replacement, path) {
        let _ = fs::remove_file(replacement);
        if !path.exists() && fs::hard_link(&backup_path, path).is_ok() {
            let _ = fs::remove_file(&backup_path);
        }
        return Err(error).with_context(|| {
            if backup_path.exists() {
                format!(
                    "failed to install {}; the previous file was retained at {}",
                    path.display(),
                    backup_path.display()
                )
            } else {
                format!(
                    "failed to install {}; the previous file was restored",
                    path.display()
                )
            }
        });
    }
    fs::remove_file(replacement).with_context(|| {
        format!(
            "failed to remove replacement link {}",
            replacement.display()
        )
    })?;
    let previous = fs::read(&backup_path)
        .with_context(|| format!("failed to read replaced file {}", backup_path.display()))?;
    fs::remove_file(&backup_path)
        .with_context(|| format!("failed to remove backup {}", backup_path.display()))?;
    sync_parent_directory(parent)?;
    Ok(Some(previous))
}

#[cfg(unix)]
fn finish_exchanged_file(path: &Path, replacement: &Path) -> Result<Option<Vec<u8>>> {
    let previous = fs::read(replacement)
        .with_context(|| format!("failed to read replaced file {}", replacement.display()))?;
    fs::remove_file(replacement)
        .with_context(|| format!("failed to remove replaced file {}", replacement.display()))?;
    if let Some(parent) = path.parent() {
        sync_parent_directory(parent)?;
    }
    Ok(Some(previous))
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

#[cfg(unix)]
fn restrict_path_to_current_user(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to restrict permissions for {}", path.display()))
}

#[cfg(windows)]
fn restrict_path_to_current_user(path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, GetTokenInformation, PROTECTED_DACL_SECURITY_INFORMATION,
        SetFileSecurityW, TOKEN_QUERY, TOKEN_USER, TokenUser,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to open the current process token");
    }

    let mut size = 0_u32;
    unsafe {
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
    }
    if size == 0 {
        unsafe {
            CloseHandle(token);
        }
        return Err(std::io::Error::last_os_error())
            .context("failed to size the current process user token");
    }

    let mut buffer = vec![0_u8; size as usize];
    let loaded = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    };
    unsafe {
        CloseHandle(token);
    }
    if loaded == 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to read the current process user token");
    }

    let token_user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
    let mut string_sid = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut string_sid) } == 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to encode the current process user SID");
    }
    let mut sid_length = 0;
    while unsafe { *string_sid.add(sid_length) } != 0 {
        sid_length += 1;
    }
    let sid =
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(string_sid, sid_length) });
    unsafe {
        LocalFree(string_sid.cast());
    }

    let sddl = format!("D:P(A;;FA;;;{sid})(A;;FA;;;SY)")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut descriptor = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!("failed to build security descriptor for {}", path.display())
        });
    }

    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let applied = unsafe {
        SetFileSecurityW(
            wide_path.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    unsafe {
        LocalFree(descriptor);
    }
    if applied == 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("failed to restrict permissions for {}", path.display()));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn restrict_path_to_current_user(_path: &Path) -> Result<()> {
    Ok(())
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
    fn atomic_replace_returns_the_exact_previous_contents() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("config");

        let created = atomic_replace_and_read_previous(&path, b"first")
            .expect("missing destination should be created");
        assert_eq!(created, None);
        assert_eq!(fs::read(&path).unwrap(), b"first");

        let previous = atomic_replace_and_read_previous(&path, b"second")
            .expect("existing destination should be exchanged");
        assert_eq!(previous, Some(b"first".to_vec()));
        assert_eq!(fs::read(&path).unwrap(), b"second");
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

    #[test]
    fn portable_flag_selects_sibling_data_directory() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let executable = root.path().join(if cfg!(windows) {
            "miaominal.exe"
        } else {
            "miaominal"
        });
        fs::write(&executable, b"test executable").expect("fake executable should be written");
        fs::write(root.path().join(PORTABLE_FLAG_FILE), b"")
            .expect("portable flag should be written");

        let context =
            initialize_runtime_with(&[], &executable).expect("portable runtime should initialize");

        assert_eq!(context.mode(), RuntimeMode::Portable);
        assert_eq!(
            context.active_data_dir(),
            root.path().canonicalize().unwrap().join(PORTABLE_DATA_DIR)
        );
        assert_eq!(
            context.credential_policy(),
            CredentialPolicy::LocalVaultRequired
        );
    }

    #[test]
    fn portable_argument_enables_mode_without_flag_file() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let executable = root.path().join(if cfg!(windows) {
            "miaominal.exe"
        } else {
            "miaominal"
        });
        fs::write(&executable, b"test executable").expect("fake executable should be written");

        let context =
            initialize_runtime_with(&[std::ffi::OsString::from("--portable")], &executable)
                .expect("portable runtime should initialize from argument");

        assert_eq!(context.mode(), RuntimeMode::Portable);
        assert!(context.active_data_dir().is_dir());
        assert!(!root.path().join(PORTABLE_FLAG_FILE).exists());
    }

    #[test]
    fn data_migration_copies_files_switches_location_and_cleans_source() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let bootstrap = root.path().join("default");
        let target = root.path().join("custom");
        fs::create_dir_all(&bootstrap).expect("bootstrap directory should be created");
        fs::create_dir(&target).expect("target directory should be created");
        fs::write(bootstrap.join("settings.toml"), "font_size = 14\n")
            .expect("settings should be written");
        fs::create_dir(bootstrap.join("nested")).expect("nested directory should be created");
        fs::write(bootstrap.join("nested").join("chat.bin"), [1, 2, 3, 4])
            .expect("nested file should be written");
        let location_file = bootstrap.join(DATA_LOCATION_FILE);
        let migration = DataMigration {
            id: "test-migration".to_string(),
            source: bootstrap.clone(),
            target: target.clone(),
            phase: MigrationPhase::Copy,
        };
        let mut location = DataLocationConfig {
            migration: Some(migration.clone()),
            ..Default::default()
        };
        write_data_location(&location_file, &location).expect("location should be written");

        let active = resume_data_migration(
            &location_file,
            &bootstrap,
            &bootstrap,
            &mut location,
            migration,
        )
        .expect("migration should succeed");

        assert_eq!(active, target);
        assert_eq!(
            fs::read_to_string(active.join("settings.toml")).unwrap(),
            "font_size = 14\n"
        );
        assert_eq!(
            fs::read(active.join("nested").join("chat.bin")).unwrap(),
            [1, 2, 3, 4]
        );
        assert!(location_file.exists());
        assert!(!bootstrap.join("settings.toml").exists());
        assert!(location.migration.is_none());
        assert_eq!(location.active_dir, Some(active));
    }

    #[test]
    fn migration_from_custom_directory_can_restore_default_with_bootstrap_file() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let bootstrap = root.path().join("default");
        let custom = root.path().join("custom");
        fs::create_dir(&bootstrap).expect("default directory should be created");
        fs::create_dir(&custom).expect("custom directory should be created");
        fs::write(custom.join("settings.toml"), "font_size = 15\n")
            .expect("custom data should be written");

        let canonical_bootstrap = fs::canonicalize(&bootstrap).unwrap();
        let canonical_custom = fs::canonicalize(&custom).unwrap();
        let location_file = bootstrap.join(DATA_LOCATION_FILE);
        let migration = DataMigration {
            id: "restore-default-test".to_string(),
            source: canonical_custom.clone(),
            target: canonical_bootstrap.clone(),
            phase: MigrationPhase::Copy,
        };
        let mut location = DataLocationConfig {
            active_dir: Some(canonical_custom),
            migration: Some(migration.clone()),
            ..Default::default()
        };
        write_data_location(&location_file, &location).expect("location should be written");

        validate_migration_target(&migration.source, &bootstrap, &bootstrap)
            .expect("bootstrap file should be allowed when restoring the default directory");
        let active = resume_data_migration(
            &location_file,
            &bootstrap,
            &bootstrap,
            &mut location,
            migration,
        )
        .expect("restoring the default directory should succeed");

        assert!(paths_refer_to_same_location(&active, &bootstrap));
        assert_eq!(
            fs::read_to_string(bootstrap.join("settings.toml")).unwrap(),
            "font_size = 15\n"
        );
        assert!(location_file.is_file());
        assert!(!custom.exists());
        assert!(location.active_dir.is_none());
        assert!(location.migration.is_none());
    }

    #[test]
    fn migration_target_must_be_empty() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("unrelated.txt"), "keep").unwrap();

        let error = validate_migration_target(&source, &target, &source)
            .expect_err("non-empty target should be rejected");

        assert_eq!(
            data_directory_change_error_kind(&error),
            DataDirectoryChangeErrorKind::TargetNotEmpty
        );
        assert!(error.to_string().contains("must be empty"));
        assert_eq!(
            fs::read_to_string(target.join("unrelated.txt")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn migration_target_must_not_overlap_source() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let source = root.path().join("source");
        let target = source.join("nested-target");
        fs::create_dir_all(&target).unwrap();

        let error = validate_migration_target(&source, &target, root.path())
            .expect_err("source descendants should be rejected");

        assert_eq!(
            data_directory_change_error_kind(&error),
            DataDirectoryChangeErrorKind::TargetOverlapsSource
        );
        assert!(error.to_string().contains("cannot overlap"));
    }

    #[test]
    fn portable_data_path_must_not_be_a_regular_file() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let data = root.path().join(PORTABLE_DATA_DIR);
        fs::write(&data, b"not a directory").unwrap();

        let error = validate_or_create_portable_data_dir(root.path(), &data)
            .expect_err("a regular file must not be accepted as portable data");

        assert!(error.to_string().contains("not a real directory"));
    }

    #[test]
    fn failed_copy_migration_rolls_back_owned_stage_and_target() {
        let root = tempfile::tempdir().expect("temporary directory should be created");
        let bootstrap = root.path().join("bootstrap");
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::create_dir(&bootstrap).unwrap();
        fs::create_dir(&source).unwrap();
        fs::create_dir(&target).unwrap();
        let location_file = bootstrap.join(DATA_LOCATION_FILE);
        let migration = DataMigration {
            id: "rollback-test".to_string(),
            source,
            target: target.clone(),
            phase: MigrationPhase::Copy,
        };
        let stage = migration_stage_path(&migration).unwrap();
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("partial.txt"), b"partial").unwrap();
        write_migration_marker(&target, &migration.id).unwrap();
        fs::write(target.join("partial.txt"), b"partial").unwrap();
        let mut location = DataLocationConfig {
            migration: Some(migration),
            ..DataLocationConfig::default()
        };

        rollback_failed_copy_migration(&bootstrap, &location_file, &mut location).unwrap();

        assert!(!stage.exists());
        assert!(target.is_dir());
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        assert!(location.migration.is_none());
        assert!(location_file.is_file());
    }

    #[test]
    fn clearing_default_data_keeps_bootstrap_location_file() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let location_file = directory.path().join(DATA_LOCATION_FILE);
        fs::write(&location_file, "version = 1\n").unwrap();
        fs::write(directory.path().join("settings.toml"), "font_size = 14\n").unwrap();
        fs::create_dir(directory.path().join("nested")).unwrap();
        fs::write(
            directory.path().join("nested").join("secret.bin"),
            b"secret",
        )
        .unwrap();

        clear_directory_except(directory.path(), &[location_file.as_path()]).unwrap();

        assert!(location_file.is_file());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn clearing_active_data_keeps_the_live_instance_lock() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let active_data_dir = directory.path().to_path_buf();
        let lock_path = active_data_dir.join(APP_INSTANCE_LOCK_FILE);
        let owner = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .expect("instance lock should open");
        owner.try_lock().expect("instance lock should be acquired");
        fs::write(active_data_dir.join("settings.toml"), "font_size = 14\n")
            .expect("settings should be written");
        let context = RuntimeContext {
            mode: RuntimeMode::Portable,
            active_data_dir: active_data_dir.clone(),
            default_data_dir: active_data_dir.clone(),
            bootstrap_dir: active_data_dir.clone(),
            config_initialization: ConfigDirInitialization::Current {
                path: active_data_dir.clone(),
            },
            warning: None,
            warning_kind: None,
        };

        clear_active_data_dir_with(&context).expect("active data should be cleared");

        assert!(lock_path.is_file());
        assert!(!active_data_dir.join("settings.toml").exists());
        let competitor = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .expect("retained instance lock should open");
        assert!(matches!(
            competitor.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
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
