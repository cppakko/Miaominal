use super::*;

impl AppView {
    pub(in crate::ui::shell) fn set_terminal_right_click_behavior(
        &mut self,
        behavior: TerminalRightClickBehavior,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .settings_store
            .update(|settings| settings.terminal_right_click_behavior = behavior);
        if changed {
            self.status_message = match behavior {
                TerminalRightClickBehavior::ContextMenu => {
                    i18n::string("status.right_click_context_menu")
                }
                TerminalRightClickBehavior::CopySelectionOrPaste => {
                    i18n::string("status.right_click_copy_paste")
                }
            };
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn set_terminal_shift_right_click_context_menu(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .settings_store
            .update(|settings| settings.terminal_shift_right_click_context_menu = enabled);
        if changed {
            self.status_message = if enabled {
                i18n::string("status.shift_right_click_enabled")
            } else {
                i18n::string("status.shift_right_click_disabled")
            };
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn set_monitor_history_duration(
        &mut self,
        duration: MonitorHistoryDuration,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .settings_store
            .update(|settings| settings.monitor_history_duration = duration);
        if changed {
            self.status_message = i18n::string("status.monitor_history_duration_changed");
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn set_local_vault_auto_lock_duration(
        &mut self,
        duration: LocalVaultAutoLockDuration,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .settings_store
            .update(|settings| settings.local_vault_auto_lock_duration = duration);
        if changed {
            self.sync_local_vault_auto_lock_task(cx);
            self.status_message = i18n::string("status.local_vault_auto_lock_duration_changed");
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn set_last_tab_close_behavior(
        &mut self,
        behavior: LastTabCloseBehavior,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .settings_store
            .update(|settings| settings.last_tab_close_behavior = behavior);
        if changed {
            self.status_message = match behavior {
                LastTabCloseBehavior::ExitApplication => {
                    i18n::string("status.last_tab_close_behavior_exit")
                }
                LastTabCloseBehavior::OpenNewHomeTab => {
                    i18n::string("status.last_tab_close_behavior_open_home")
                }
            };
            cx.notify();
        }
    }
    pub(in crate::ui::shell) fn persist_sftp_browser_hidden_columns(
        &mut self,
        side: SftpBrowserSide,
        hidden_columns: Vec<usize>,
        _cx: &mut Context<Self>,
    ) {
        let changed = match side {
            SftpBrowserSide::Local => self
                .settings_store
                .update(|settings| settings.local_sftp_hidden_columns = hidden_columns),
            SftpBrowserSide::Remote => self
                .settings_store
                .update(|settings| settings.remote_sftp_hidden_columns = hidden_columns),
        };

        let _ = changed;
    }
}
