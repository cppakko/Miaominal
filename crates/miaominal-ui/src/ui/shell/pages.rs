pub(in crate::ui::shell) mod empty_state;
mod keychain;
mod onboarding;
mod settings;
mod sftp;

pub(in crate::ui::shell) use empty_state::{shell_compact_empty_state, shell_empty_state};
pub(in crate::ui::shell) use onboarding::render_onboarding_page;
pub(in crate::ui::shell) use settings::{SettingsDestination, render_settings_page};
