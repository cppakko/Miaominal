use crate::app::paths;
use crate::settings::{AppSettings, CURRENT_ONBOARDING_VERSION, changed, install};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SettingsStore {
    settings_file: PathBuf,
    settings: AppSettings,
}

impl SettingsStore {
    pub fn load() -> Result<Self> {
        Self::load_with_path(paths::config_file("settings.toml")?)
    }

    fn load_with_path(settings_file: PathBuf) -> Result<Self> {
        let settings_file_exists = settings_file.exists();
        let existing_app_data = has_existing_app_data(&settings_file)?;

        let (mut settings, has_onboarding_field) = if settings_file_exists {
            read_settings_file(&settings_file)?
        } else {
            (AppSettings::default_for_system(), false)
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
        install(settings.clone());

        let store = Self {
            settings_file,
            settings,
        };

        if migrated_legacy_onboarding && let Err(error) = store.persist() {
            log::warn!("failed to persist legacy onboarding migration: {error:?}");
        }

        Ok(store)
    }

    pub fn fallback() -> Self {
        let settings = AppSettings::default_for_system();
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
        if changed(&before, &settings) {
            self.settings = settings;
            install(self.settings.clone());
            self.persist()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn persist(&self) -> Result<()> {
        if let Some(parent) = self.settings_file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let serialized =
            toml::to_string_pretty(&self.settings).context("failed to serialize settings")?;
        fs::write(&self.settings_file, serialized)
            .with_context(|| format!("failed to write {}", self.settings_file.display()))?;
        Ok(())
    }
}

fn read_settings_file(settings_file: &Path) -> Result<(AppSettings, bool)> {
    let content = fs::read_to_string(settings_file)
        .with_context(|| format!("failed to read {}", settings_file.display()))?;

    if content.trim().is_empty() {
        return Ok((AppSettings::default_for_system(), false));
    }

    let raw: toml::Value = toml::from_str(&content)
        .with_context(|| format!("failed to parse {}", settings_file.display()))?;
    let has_onboarding_field = raw
        .as_table()
        .is_some_and(|table| table.contains_key("completed_onboarding_version"));
    let settings: AppSettings = toml::from_str(&content)
        .with_context(|| format!("failed to parse {}", settings_file.display()))?;

    Ok((settings, has_onboarding_field))
}

fn has_existing_app_data(settings_file: &Path) -> Result<bool> {
    let Some(config_dir) = settings_file.parent() else {
        return Ok(false);
    };
    if !config_dir.exists() {
        return Ok(false);
    }

    for entry in fs::read_dir(config_dir)
        .with_context(|| format!("failed to read {}", config_dir.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read {}", config_dir.display()))?;
        if entry.path() == settings_file {
            continue;
        }
        return Ok(true);
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
