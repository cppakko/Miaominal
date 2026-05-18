use crate::settings::{
    LastTabCloseBehavior, LocalVaultAutoLockDuration, MonitorHistoryDuration, ThemeId,
};
use crate::ui::i18n;

pub(in crate::ui::shell) fn last_tab_close_behavior_label(
    behavior: LastTabCloseBehavior,
) -> String {
    i18n::string(match behavior {
        LastTabCloseBehavior::ExitApplication => "enum.last_tab_close_behavior.exit_application",
        LastTabCloseBehavior::OpenNewHomeTab => "enum.last_tab_close_behavior.open_new_home_tab",
    })
}

pub(in crate::ui::shell) fn monitor_history_duration_label(
    duration: MonitorHistoryDuration,
) -> String {
    i18n::string(match duration {
        MonitorHistoryDuration::OneMinute => "enum.monitor_history.one_minute",
        MonitorHistoryDuration::FiveMinutes => "enum.monitor_history.five_minutes",
        MonitorHistoryDuration::TenMinutes => "enum.monitor_history.ten_minutes",
        MonitorHistoryDuration::ThirtyMinutes => "enum.monitor_history.thirty_minutes",
    })
}

pub(in crate::ui::shell) fn local_vault_auto_lock_duration_label(
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

pub(in crate::ui::shell) fn theme_id_label(theme_id: ThemeId) -> String {
    i18n::string(match theme_id {
        ThemeId::Light => "enum.theme.light",
        ThemeId::Dark => "enum.theme.dark",
    })
}
