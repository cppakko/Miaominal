use crate::ui::i18n;
use miaominal_core::profile::ImportSourceKind;
use miaominal_core::ssh_bridge_security::BridgeSecurityLevel;
use miaominal_settings::{
    LastTabCloseBehavior, LocalVaultAutoLockDuration, MonitorHistoryDuration, ThemeId,
    WindowCloseBehavior,
};

pub(in crate::ui) fn window_close_behavior_label(behavior: WindowCloseBehavior) -> String {
    i18n::string(match behavior {
        WindowCloseBehavior::ExitApplication => "enum.window_close_behavior.exit_application",
        WindowCloseBehavior::MinimizeToTray => "enum.window_close_behavior.minimize_to_tray",
    })
}

pub(in crate::ui) fn last_tab_close_behavior_label(behavior: LastTabCloseBehavior) -> String {
    i18n::string(match behavior {
        LastTabCloseBehavior::ExitApplication => "enum.last_tab_close_behavior.exit_application",
        LastTabCloseBehavior::OpenNewHomeTab => "enum.last_tab_close_behavior.open_new_home_tab",
    })
}

pub(in crate::ui) fn monitor_history_duration_label(duration: MonitorHistoryDuration) -> String {
    i18n::string(match duration {
        MonitorHistoryDuration::OneMinute => "enum.monitor_history.one_minute",
        MonitorHistoryDuration::FiveMinutes => "enum.monitor_history.five_minutes",
        MonitorHistoryDuration::TenMinutes => "enum.monitor_history.ten_minutes",
        MonitorHistoryDuration::ThirtyMinutes => "enum.monitor_history.thirty_minutes",
    })
}

pub(in crate::ui) fn local_vault_auto_lock_duration_label(
    duration: LocalVaultAutoLockDuration,
) -> String {
    i18n::string(match duration {
        LocalVaultAutoLockDuration::Off => "enum.local_vault_auto_lock.off",
        LocalVaultAutoLockDuration::FiveMinutes => "enum.local_vault_auto_lock.five_minutes",
        LocalVaultAutoLockDuration::FifteenMinutes => "enum.local_vault_auto_lock.fifteen_minutes",
        LocalVaultAutoLockDuration::OneHour => "enum.local_vault_auto_lock.one_hour",
        LocalVaultAutoLockDuration::OneDay => "enum.local_vault_auto_lock.one_day",
    })
}

pub(in crate::ui) fn theme_id_label(theme_id: ThemeId) -> String {
    i18n::string(match theme_id {
        ThemeId::Light => "enum.theme.light",
        ThemeId::Dark => "enum.theme.dark",
    })
}

pub(in crate::ui) fn bridge_security_level_label(level: BridgeSecurityLevel) -> String {
    i18n::string(match level {
        BridgeSecurityLevel::Standard => "settings.openssh_integration.security.levels.standard",
        BridgeSecurityLevel::RequireApproval { .. } => {
            "settings.openssh_integration.security.levels.approval"
        }
        BridgeSecurityLevel::RequireSystemAuth => {
            "settings.openssh_integration.security.levels.system_auth"
        }
    })
}

pub(in crate::ui) fn localized_profile_import_source_label(source: ImportSourceKind) -> String {
    i18n::string(match source {
        ImportSourceKind::OpenSshConfig => "settings.connections.import_sources.openssh",
        ImportSourceKind::PuttyRegistry => "settings.connections.import_sources.putty",
        ImportSourceKind::SecureCrtXml => "settings.connections.import_sources.securecrt",
        ImportSourceKind::FinalShellJson => "settings.connections.import_sources.finalshell",
    })
}
