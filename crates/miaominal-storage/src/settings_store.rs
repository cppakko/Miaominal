use anyhow::{Context, Result};
use fs2::FileExt as _;
use miaominal_paths::{self as paths, atomic_write};
use miaominal_settings::{AppSettings, CURRENT_ONBOARDING_VERSION, changed, install};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SettingsStore {
    settings_file: PathBuf,
    settings: AppSettings,
}

impl SettingsStore {
    pub fn load() -> Result<Self> {
        let mut store = Self::load_with_path(paths::config_file("settings.toml")?)?;
        if paths::credential_policy()? == paths::CredentialPolicy::LocalVaultRequired
            && !store.settings.local_vault_enabled
        {
            let mut settings = store.settings.clone();
            settings.local_vault_enabled = true;
            store.replace(settings)?;
        }
        Ok(store)
    }

    #[doc(hidden)]
    pub fn load_with_path(settings_file: PathBuf) -> Result<Self> {
        let settings_file_exists = settings_file.exists();
        let existing_app_data = has_existing_app_data(&settings_file)?;

        let (
            mut settings,
            has_onboarding_field,
            migrated_terminal_font_family,
            migrated_open_ssh_integration,
        ) = if settings_file_exists {
            read_settings_file(&settings_file)?
        } else {
            (AppSettings::default_for_system(), false, false, false)
        };

        let migrated_legacy_onboarding = if settings_file_exists {
            !has_onboarding_field
        } else {
            existing_app_data
        };

        if migrated_legacy_onboarding {
            settings.completed_onboarding_version = CURRENT_ONBOARDING_VERSION;
        }

        settings.sanitize();
        let repaired_invalid_bridge_policy =
            repair_invalid_bridge_policy(&mut settings, &settings_file);
        install(settings.clone());

        let store = Self {
            settings_file,
            settings,
        };

        if repaired_invalid_bridge_policy {
            let repair_result = SettingsFileLock::acquire(&store.settings_file).and_then(|_lock| {
                persist_settings_document_unlocked(&store.settings_file, &store.settings)
            });
            if let Err(error) = repair_result {
                log::warn!("failed to persist repaired SSH Bridge policy: {error:?}");
            }
        } else if (migrated_legacy_onboarding
            || migrated_terminal_font_family
            || migrated_open_ssh_integration)
            && let Err(error) = store.persist()
        {
            log::warn!("failed to persist settings migration: {error:?}");
        }

        Ok(store)
    }

    pub fn fallback() -> Self {
        let mut settings = AppSettings::default_for_system();
        if paths::credential_policy().ok() == Some(paths::CredentialPolicy::LocalVaultRequired) {
            settings.local_vault_enabled = true;
        }
        install(settings.clone());
        Self {
            settings_file: std::env::temp_dir().join("miaominal_settings.toml"),
            settings,
        }
    }

    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub fn update<F: FnOnce(&mut AppSettings)>(&mut self, f: F) -> bool {
        let mut next = self.settings.clone();
        f(&mut next);
        match self.replace(next) {
            Ok(changed) => changed,
            Err(error) => {
                log::warn!("failed to persist settings: {error:?}");
                false
            }
        }
    }

    pub fn replace(&mut self, mut settings: AppSettings) -> Result<bool> {
        let before = self.settings.clone();
        settings.sanitize();
        validate_settings(&settings)?;
        if changed(&before, &settings) {
            let _lock = SettingsFileLock::acquire(&self.settings_file)?;
            if self.settings_file.exists() {
                let latest = load_settings_document_unlocked(&self.settings_file)?;
                preserve_newer_bridge_policy(&mut settings, &latest);
            }
            persist_settings_document_unlocked(&self.settings_file, &settings)?;
            self.settings = settings;
            install(self.settings.clone());
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn persist(&self) -> Result<()> {
        self.persist_settings(&self.settings)
    }

    fn persist_settings(&self, settings: &AppSettings) -> Result<()> {
        validate_settings(settings)?;
        let _lock = SettingsFileLock::acquire(&self.settings_file)?;
        let mut settings = settings.clone();
        if self.settings_file.exists() {
            let latest = load_settings_document_unlocked(&self.settings_file)?;
            preserve_newer_bridge_policy(&mut settings, &latest);
        }
        persist_settings_document_unlocked(&self.settings_file, &settings)
    }
}

pub(crate) struct SettingsFileLock {
    file: File,
}

impl SettingsFileLock {
    pub(crate) fn acquire(settings_file: &Path) -> Result<Self> {
        if let Some(parent) = settings_file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let lock_path = settings_lock_path(settings_file);
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("failed to open {}", lock_path.display()))?;
        file.lock_exclusive()
            .with_context(|| format!("failed to lock {}", lock_path.display()))?;
        Ok(Self { file })
    }
}

impl Drop for SettingsFileLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

pub(crate) fn load_settings_document_unlocked(settings_file: &Path) -> Result<AppSettings> {
    let mut settings = if settings_file.exists() {
        read_settings_file(settings_file)?.0
    } else {
        AppSettings::default_for_system()
    };
    settings.sanitize();
    validate_settings(&settings)?;
    Ok(settings)
}

pub(crate) fn persist_settings_document_unlocked(
    settings_file: &Path,
    settings: &AppSettings,
) -> Result<()> {
    validate_settings(settings)?;
    if let Some(parent) = settings_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let serialized = toml::to_string_pretty(settings).context("failed to serialize settings")?;
    atomic_write(settings_file, serialized)
}

fn validate_settings(settings: &AppSettings) -> Result<()> {
    settings
        .ssh_bridge
        .validate()
        .map_err(anyhow::Error::msg)
        .context("invalid SSH Bridge settings")
}

fn repair_invalid_bridge_policy(settings: &mut AppSettings, settings_file: &Path) -> bool {
    let Err(error) = settings.ssh_bridge.validate() else {
        return false;
    };
    log::warn!(
        "invalid SSH Bridge policy in {}; using the default policy: {error}",
        settings_file.display()
    );
    settings.ssh_bridge.security_policy = Default::default();
    true
}

fn preserve_newer_bridge_policy(settings: &mut AppSettings, latest: &AppSettings) {
    let latest_policy = &latest.ssh_bridge.security_policy;
    let requested_policy = &settings.ssh_bridge.security_policy;
    if latest_policy.generation > requested_policy.generation
        || (latest_policy.generation == requested_policy.generation
            && latest_policy != requested_policy)
    {
        settings.ssh_bridge.security_policy = latest_policy.clone();
    }
}

fn settings_lock_path(settings_file: &Path) -> PathBuf {
    let mut path = settings_file.as_os_str().to_os_string();
    path.push(".lock");
    path.into()
}

fn read_settings_file(settings_file: &Path) -> Result<(AppSettings, bool, bool, bool)> {
    let content = fs::read_to_string(settings_file)
        .with_context(|| format!("failed to read {}", settings_file.display()))?;

    if content.trim().is_empty() {
        return Ok((AppSettings::default_for_system(), false, false, false));
    }

    let raw: toml::Value = toml::from_str(&content)
        .with_context(|| format!("failed to parse {}", settings_file.display()))?;
    let table = raw.as_table();
    let has_onboarding_field =
        table.is_some_and(|table| table.contains_key("completed_onboarding_version"));
    let has_terminal_font_family =
        table.is_some_and(|table| table.contains_key("terminal_font_family"));
    let has_open_ssh_integration_mode =
        table.is_some_and(|table| table.contains_key("open_ssh_integration_mode"));
    let mut settings: AppSettings = toml::from_str(&content)
        .with_context(|| format!("failed to parse {}", settings_file.display()))?;
    let migrated_terminal_font_family = !has_terminal_font_family;
    if migrated_terminal_font_family {
        settings.terminal_font_family = settings.font_family.clone();
    }
    let migrated_open_ssh_integration =
        !has_open_ssh_integration_mode && settings.managed_open_ssh_integration_enabled;
    if migrated_open_ssh_integration {
        settings.open_ssh_integration_mode = miaominal_settings::OpenSshIntegrationMode::Direct;
    }
    settings.managed_open_ssh_integration_enabled = false;

    Ok((
        settings,
        has_onboarding_field,
        migrated_terminal_font_family,
        migrated_open_ssh_integration,
    ))
}

fn has_existing_app_data(settings_file: &Path) -> Result<bool> {
    let Some(config_dir) = settings_file.parent() else {
        return Ok(false);
    };
    if !config_dir.exists() {
        return Ok(false);
    }

    let settings_lock_file = settings_lock_path(settings_file);
    for entry in fs::read_dir(config_dir)
        .with_context(|| format!("failed to read {}", config_dir.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read {}", config_dir.display()))?;
        if entry.path() == settings_file || entry.path() == settings_lock_file {
            continue;
        }
        return Ok(true);
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BridgeSecuritySettingsStore;
    use miaominal_core::ssh_bridge_security::BridgeSecurityLevel;
    use uuid::Uuid;

    struct TestSettingsPath {
        root: PathBuf,
        settings_file: PathBuf,
    }

    impl TestSettingsPath {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("miaominal-settings-test-{}", Uuid::new_v4()));
            let settings_file = root.join("settings.toml");
            Self {
                root,
                settings_file,
            }
        }

        fn create_dir(&self) {
            fs::create_dir_all(&self.root).expect("test config dir should be created");
        }
    }

    impl Drop for TestSettingsPath {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn fresh_install_keeps_onboarding_incomplete() {
        let paths = TestSettingsPath::new();

        let store = SettingsStore::load_with_path(paths.settings_file.clone())
            .expect("fresh install settings should load");

        assert!(store.settings().should_show_onboarding());
        assert_eq!(store.settings().completed_onboarding_version, 0);
        assert!(!paths.settings_file.exists());
    }

    #[test]
    fn settings_lock_file_does_not_count_as_existing_app_data() {
        let paths = TestSettingsPath::new();
        let policy_store = BridgeSecuritySettingsStore::open(&paths.settings_file)
            .expect("policy store should open");
        assert_eq!(
            policy_store.policy().unwrap(),
            miaominal_core::ssh_bridge_security::BridgeSecurityPolicy::default()
        );

        let store = SettingsStore::load_with_path(paths.settings_file.clone())
            .expect("fresh settings should load");

        assert!(store.settings().should_show_onboarding());
        assert!(!paths.settings_file.exists());
    }

    #[test]
    fn legacy_settings_without_onboarding_field_are_migrated() {
        let paths = TestSettingsPath::new();
        paths.create_dir();
        fs::write(&paths.settings_file, "font_size = 14.0\n")
            .expect("legacy settings file should be written");

        let store = SettingsStore::load_with_path(paths.settings_file.clone())
            .expect("legacy settings should load");

        assert!(!store.settings().should_show_onboarding());
        assert_eq!(
            store.settings().completed_onboarding_version,
            CURRENT_ONBOARDING_VERSION
        );

        let persisted = fs::read_to_string(&paths.settings_file)
            .expect("migrated settings file should be readable");
        assert!(persisted.contains("completed_onboarding_version = 1"));
    }

    #[test]
    fn legacy_font_family_is_migrated_to_terminal_font_family() {
        let paths = TestSettingsPath::new();
        paths.create_dir();
        fs::write(
            &paths.settings_file,
            "completed_onboarding_version = 1\nfont_family = \"Fira Code\"\n",
        )
        .expect("legacy settings file should be written");

        let store = SettingsStore::load_with_path(paths.settings_file.clone())
            .expect("legacy settings should load");

        assert_eq!(store.settings().font_family, "Fira Code");
        assert_eq!(store.settings().terminal_font_family, "Fira Code");

        let persisted = fs::read_to_string(&paths.settings_file)
            .expect("migrated settings file should be readable");
        assert!(persisted.contains("terminal_font_family = \"Fira Code\""));
    }

    #[test]
    fn explicit_terminal_font_family_is_preserved() {
        let paths = TestSettingsPath::new();
        paths.create_dir();
        fs::write(
            &paths.settings_file,
            concat!(
                "completed_onboarding_version = 1\n",
                "font_family = \"Segoe UI\"\n",
                "terminal_font_family = \"JetBrains Mono\"\n",
            ),
        )
        .expect("settings file should be written");

        let store = SettingsStore::load_with_path(paths.settings_file.clone())
            .expect("settings should load");

        assert_eq!(store.settings().font_family, "Segoe UI");
        assert_eq!(store.settings().terminal_font_family, "JetBrains Mono");
    }

    #[test]
    fn legacy_managed_openssh_boolean_migrates_once_to_direct_mode() {
        let paths = TestSettingsPath::new();
        paths.create_dir();
        fs::write(
            &paths.settings_file,
            concat!(
                "completed_onboarding_version = 1\n",
                "managed_open_ssh_integration_enabled = true\n",
            ),
        )
        .expect("legacy settings file should be written");

        let store = SettingsStore::load_with_path(paths.settings_file.clone())
            .expect("legacy OpenSSH settings should load");

        assert_eq!(
            store.settings().open_ssh_integration_mode,
            miaominal_settings::OpenSshIntegrationMode::Direct
        );
        assert!(!store.settings().managed_open_ssh_integration_enabled);
        let persisted = fs::read_to_string(&paths.settings_file)
            .expect("migrated settings file should be readable");
        assert!(persisted.contains("open_ssh_integration_mode = \"direct\""));
        assert!(!persisted.contains("managed_open_ssh_integration_enabled"));
    }

    #[test]
    fn explicit_openssh_mode_takes_precedence_over_legacy_boolean() {
        let paths = TestSettingsPath::new();
        paths.create_dir();
        fs::write(
            &paths.settings_file,
            concat!(
                "completed_onboarding_version = 1\n",
                "open_ssh_integration_mode = \"disabled\"\n",
                "managed_open_ssh_integration_enabled = true\n",
            ),
        )
        .expect("settings file should be written");

        let store = SettingsStore::load_with_path(paths.settings_file.clone())
            .expect("OpenSSH settings should load");

        assert_eq!(
            store.settings().open_ssh_integration_mode,
            miaominal_settings::OpenSshIntegrationMode::Disabled
        );
        assert!(!store.settings().managed_open_ssh_integration_enabled);
    }

    #[test]
    fn existing_app_data_without_settings_skips_initial_onboarding() {
        let paths = TestSettingsPath::new();
        paths.create_dir();
        fs::write(paths.root.join("sessions.toml"), "sessions = []\n")
            .expect("legacy session data should be written");

        let store = SettingsStore::load_with_path(paths.settings_file.clone())
            .expect("legacy app data should load settings");

        assert!(!store.settings().should_show_onboarding());
        assert_eq!(
            store.settings().completed_onboarding_version,
            CURRENT_ONBOARDING_VERSION
        );
        assert!(paths.settings_file.exists());
    }

    #[test]
    fn explicit_incomplete_onboarding_version_is_preserved() {
        let paths = TestSettingsPath::new();
        paths.create_dir();
        fs::write(
            &paths.settings_file,
            "completed_onboarding_version = 0\nfont_size = 14.0\n",
        )
        .expect("settings file should be written");

        let store = SettingsStore::load_with_path(paths.settings_file.clone())
            .expect("settings should load");

        assert!(store.settings().should_show_onboarding());
        assert_eq!(store.settings().completed_onboarding_version, 0);
    }

    #[test]
    fn ordinary_settings_updates_preserve_newer_bridge_policy() {
        let paths = TestSettingsPath::new();
        let mut settings_store = SettingsStore::load_with_path(paths.settings_file.clone())
            .expect("settings should load");
        let policy_store = BridgeSecuritySettingsStore::open(&paths.settings_file)
            .expect("policy store should open");
        let policy = policy_store
            .set_policy(
                BridgeSecurityLevel::RequireApproval { timeout_secs: 30 },
                42,
            )
            .expect("policy should persist");

        assert!(settings_store.update(|settings| settings.font_size = 18.0));

        assert_eq!(policy_store.policy().unwrap(), policy);
        assert_eq!(settings_store.settings().ssh_bridge.security_policy, policy);
    }

    #[test]
    fn invalid_bridge_policy_is_repaired_without_discarding_other_settings() {
        let paths = TestSettingsPath::new();
        paths.create_dir();
        fs::write(
            &paths.settings_file,
            concat!(
                "completed_onboarding_version = 1\n",
                "font_size = 18.0\n",
                "[ssh_bridge.security_policy]\n",
                "updated_at = 1\n",
                "generation = 1\n",
                "[ssh_bridge.security_policy.level]\n",
                "kind = \"require_approval\"\n",
                "timeout_secs = 4\n",
            ),
        )
        .expect("invalid settings should be written");

        let mut store = SettingsStore::load_with_path(paths.settings_file.clone())
            .expect("invalid bridge policy should not discard all settings");
        assert_eq!(store.settings().font_size, 18.0);
        assert_eq!(
            store.settings().ssh_bridge.security_policy,
            miaominal_core::ssh_bridge_security::BridgeSecurityPolicy::default()
        );

        let policy_store = BridgeSecuritySettingsStore::open(&paths.settings_file)
            .expect("policy store should open");
        assert_eq!(
            policy_store
                .policy()
                .expect("repaired policy should be valid on disk"),
            miaominal_core::ssh_bridge_security::BridgeSecurityPolicy::default()
        );

        assert!(store.update(|settings| settings.font_size = 19.0));
        let repaired = load_settings_document_unlocked(&paths.settings_file)
            .expect("repaired settings should remain valid on disk");
        assert_eq!(repaired.font_size, 19.0);
        assert_eq!(
            repaired.ssh_bridge.security_policy,
            miaominal_core::ssh_bridge_security::BridgeSecurityPolicy::default()
        );
    }
}
