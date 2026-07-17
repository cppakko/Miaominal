mod empty_state;
mod forward;
mod hosts;
mod keychain;
mod onboarding;
mod settings;
mod sftp;
mod snippets;
mod trusted;

pub(in crate::ui::shell) use empty_state::{shell_compact_empty_state, shell_empty_state};
pub(in crate::ui::shell) use onboarding::render_onboarding_page;
pub(in crate::ui::shell) use settings::render_settings_page;
