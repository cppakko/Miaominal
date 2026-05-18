use super::super::*;
use crate::domain::sync::{SyncProvider, SyncStatus};
use crate::services::{SyncService, SyncTaskResult};
use crate::{settings, ui::i18n};
use gpui_component::{WindowExt as _, notification::Notification};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ManualSyncAction {
    Push,
    ForcePush,
    Pull,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManualSyncGate {
    InProgress,
    ProviderRequired,
    VaultUnlockRequired,
    Ready,
}

const SYNC_STATUS_ERROR_SUMMARY_MAX_CHARS: usize = 96;

fn sync_provider_label(provider: SyncProvider) -> String {
    match provider {
        SyncProvider::None => i18n::string("settings.sync.providers.none"),
        SyncProvider::GithubGist => i18n::string("settings.sync.providers.gist"),
        SyncProvider::WebDav => i18n::string("settings.sync.providers.webdav"),
    }
}

fn summarize_sync_error(error: &str) -> String {
    let normalized = error.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_with_ellipsis(&normalized, SYNC_STATUS_ERROR_SUMMARY_MAX_CHARS)
}

pub(in crate::ui::shell) fn sync_status_summary(status: &SyncStatus) -> String {
    match status {
        SyncStatus::Idle => i18n::string("settings.sync.status.state.idle"),
        SyncStatus::Syncing => i18n::string("settings.sync.status.state.syncing"),
        SyncStatus::RemoteBindingRequired { provider } => match provider {
            SyncProvider::GithubGist => {
                i18n::string("settings.sync.status.state.github_gist_binding_required")
            }
            _ => i18n::string_args(
                "settings.sync.status.state.remote_binding_required",
                &[("provider", &sync_provider_label(*provider))],
            ),
        },
        SyncStatus::Pulled { at } => {
            let timestamp = format_local_timestamp(Some(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(*at),
            ))
            .to_string();
            i18n::string_args(
                "settings.sync.status.state.pulled_at",
                &[("time", &timestamp)],
            )
        }
        SyncStatus::Pushed { at } => {
            let timestamp = format_local_timestamp(Some(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(*at),
            ))
            .to_string();
            i18n::string_args(
                "settings.sync.status.state.pushed_at",
                &[("time", &timestamp)],
            )
        }
        SyncStatus::PullRequired { remote_at } => {
            let timestamp = format_local_timestamp(Some(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(*remote_at),
            ))
            .to_string();
            i18n::string_args(
                "settings.sync.status.state.pull_required_at",
                &[("time", &timestamp)],
            )
        }
        SyncStatus::UpToDate { .. } => i18n::string("settings.sync.status.state.up_to_date"),
        SyncStatus::Error(error) => {
            let message = summarize_sync_error(error);
            i18n::string_args("settings.sync.status.state.error", &[("message", &message)])
        }
    }
}

fn show_sync_result_notification(window: &mut Window, status: &SyncStatus, cx: &mut App) -> String {
    let message = sync_status_summary(status);
    let notification = match status {
        SyncStatus::Error(_) => Notification::error(message.clone()).title(i18n::string(
            "settings.sync.status.notifications.failed_title",
        )),
        SyncStatus::PullRequired { .. } | SyncStatus::RemoteBindingRequired { .. } => {
            Notification::error(message.clone()).title(i18n::string(
                "settings.sync.status.notifications.action_required_title",
            ))
        }
        _ => Notification::success(message.clone()).title(i18n::string(
            "settings.sync.status.notifications.succeeded_title",
        )),
    };

    window.push_notification(notification, cx);
    message
}

fn sync_github_gist_id_input_value(
    input: &Entity<InputState>,
    gist_id: Option<String>,
    window: &mut Window,
    cx: &mut App,
) {
    set_input_value(input, gist_id.unwrap_or_default(), window, cx);
}

impl AppView {
    pub(in crate::ui::shell) fn sync_in_progress(&self) -> bool {
        matches!(self.sync.sync_status, SyncStatus::Syncing)
    }

    fn manual_push_pull_confirm_reason(
        action: ManualSyncAction,
        status: &SyncStatus,
    ) -> Option<SyncPullConfirmReason> {
        if action == ManualSyncAction::Push && matches!(status, SyncStatus::PullRequired { .. }) {
            Some(SyncPullConfirmReason::RemoteNewer)
        } else {
            None
        }
    }

    fn manual_sync_gate(
        sync_in_progress: bool,
        sync_enabled_for_provider: bool,
        sync_requires_local_vault_unlock: bool,
    ) -> ManualSyncGate {
        if sync_in_progress {
            ManualSyncGate::InProgress
        } else if !sync_enabled_for_provider {
            ManualSyncGate::ProviderRequired
        } else if sync_requires_local_vault_unlock {
            ManualSyncGate::VaultUnlockRequired
        } else {
            ManualSyncGate::Ready
        }
    }

    fn manual_sync_unlock_follow_up(gate: ManualSyncGate) -> Option<PendingLocalVaultUnlockAction> {
        match gate {
            ManualSyncGate::VaultUnlockRequired => Some(PendingLocalVaultUnlockAction::OpenSyncNow),
            _ => None,
        }
    }

    pub(in crate::ui::shell) fn sync_requires_local_vault_unlock(&self) -> bool {
        self.local_vault_status == LocalVaultStatus::Locked
            && self.sync.sync_engine.sync_enabled_for_provider()
    }

    fn show_sync_provider_required_error(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let notification = Self::error_notification(
            i18n::string("settings.sync.provider_required_error.title"),
            i18n::string("settings.sync.provider_required_error.message"),
        );
        let message = i18n::string("settings.sync.provider_required_error.message");

        self.sync.sync_status = SyncStatus::Error(message.clone());
        self.status_message = message;
        window.push_notification(notification, cx);
        cx.notify();
    }

    fn execute_manual_sync(
        &mut self,
        action: ManualSyncAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let manual_sync_gate = Self::manual_sync_gate(
            self.sync_in_progress(),
            self.sync.sync_engine.sync_enabled_for_provider(),
            self.sync_requires_local_vault_unlock(),
        );

        if manual_sync_gate == ManualSyncGate::InProgress {
            self.status_message = sync_status_summary(&self.sync.sync_status);
            cx.notify();
            return;
        }

        let notification_window = cx.active_window();

        if manual_sync_gate == ManualSyncGate::ProviderRequired {
            self.show_sync_provider_required_error(window, cx);
            return;
        }

        if let Some(follow_up) = Self::manual_sync_unlock_follow_up(manual_sync_gate) {
            self.prompt_local_vault_unlock_for_action(follow_up, window, cx);
            return;
        }

        let service = match SyncService::new(
            self.services.runtime.clone(),
            self.services.session_store.clone(),
            self.services.snippet_store.clone(),
            self.services.keychain_store.clone(),
            self.services.secrets.clone(),
        ) {
            Ok(service) => service,
            Err(error) => {
                let status = SyncStatus::Error(error.to_string());
                self.sync.sync_status = status.clone();
                self.status_message = show_sync_result_notification(window, &status, cx);
                cx.notify();
                return;
            }
        };
        let settings_store = self.settings_store.clone();
        let engine = self.sync.sync_engine.clone();
        let runtime = service.runtime().clone();

        self.sync.sync_status = SyncStatus::Syncing;
        cx.notify();

        let (tx, rx) = std::sync::mpsc::sync_channel::<anyhow::Result<SyncTaskResult>>(1);
        runtime.spawn(async move {
            let result = match action {
                ManualSyncAction::Push => service.push(engine, settings_store).await,
                ManualSyncAction::ForcePush => service.push_force(engine, settings_store).await,
                ManualSyncAction::Pull => service.pull(engine, settings_store).await,
            };
            let _ = tx.send(result);
        });

        let task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rx.recv()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("sync task cancelled")))
                })
                .await;

            this.update(cx, move |view, cx| {
                match result {
                    Ok(r) => {
                        let status = r.status.clone();
                        let pulled = matches!(status, SyncStatus::Pulled { .. });
                        let pull_confirm_reason =
                            Self::manual_push_pull_confirm_reason(action, &status);

                        view.sync.sync_engine.config_store.config = r.updated_config;
                        let gist_id_input =
                            view.panel_forms.settings.sync_github_gist_id_input.clone();
                        let gist_id = view.sync.sync_engine.config_store.config.gist_id.clone();
                        view.sync.sync_status = status.clone();
                        view.status_message = sync_status_summary(&status);

                        if let Some(reason) = pull_confirm_reason {
                            view.dialogs.pending_sync_pull_confirm =
                                Some(PendingSyncPullConfirmState { reason });
                        }

                        if pulled {
                            view.reload_data_after_sync(cx);
                        }

                        if let Some(window_handle) = notification_window {
                            let _ = window_handle.update(cx, move |_, window, cx| {
                                sync_github_gist_id_input_value(
                                    &gist_id_input,
                                    gist_id,
                                    window,
                                    cx,
                                );
                                show_sync_result_notification(window, &status, cx);
                            });
                        }
                    }
                    Err(e) => {
                        let status = SyncStatus::Error(e.to_string());
                        view.sync.sync_status = status.clone();
                        view.status_message = sync_status_summary(&status);

                        if let Some(window_handle) = notification_window {
                            let _ = window_handle.update(cx, move |_, window, cx| {
                                show_sync_result_notification(window, &status, cx);
                            });
                        }
                    }
                }
                view.sync.active_sync_task = None;
                cx.notify();
            })
            .ok();
        });

        self.sync.active_sync_task = Some(task);
    }

    pub(in crate::ui::shell) fn trigger_sync_now(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let manual_sync_gate = Self::manual_sync_gate(
            self.sync_in_progress(),
            self.sync.sync_engine.sync_enabled_for_provider(),
            self.sync_requires_local_vault_unlock(),
        );

        if manual_sync_gate == ManualSyncGate::InProgress {
            self.status_message = sync_status_summary(&self.sync.sync_status);
            cx.notify();
            return;
        }
        if manual_sync_gate == ManualSyncGate::ProviderRequired {
            self.show_sync_provider_required_error(window, cx);
            return;
        }
        self.dialogs.pending_sync_pull_confirm = None;

        if let Some(follow_up) = Self::manual_sync_unlock_follow_up(manual_sync_gate) {
            self.prompt_local_vault_unlock_for_action(follow_up, window, cx);
            return;
        }

        self.dialogs.pending_sync_direction = Some(PendingSyncDirectionState);
        cx.notify();
    }

    pub(in crate::ui::shell) fn cancel_sync_direction_prompt(&mut self, cx: &mut Context<Self>) {
        if let Some(prompt) = self.dialogs.pending_sync_direction.take() {
            self.start_dialog_exit(DialogOverlaySnapshot::SyncDirection(prompt), cx);
        }
    }

    pub(in crate::ui::shell) fn select_sync_now_push(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.dialogs.pending_sync_direction.take() else {
            return;
        };

        self.start_dialog_exit(DialogOverlaySnapshot::SyncDirection(prompt), cx);
        self.execute_manual_sync(ManualSyncAction::Push, window, cx);
    }

    pub(in crate::ui::shell) fn select_sync_now_pull(&mut self, cx: &mut Context<Self>) {
        let Some(prompt) = self.dialogs.pending_sync_direction.take() else {
            return;
        };

        self.start_dialog_exit(DialogOverlaySnapshot::SyncDirection(prompt), cx);
        self.dialogs.pending_sync_pull_confirm = Some(PendingSyncPullConfirmState {
            reason: SyncPullConfirmReason::Manual,
        });
        cx.notify();
    }

    pub(in crate::ui::shell) fn cancel_sync_pull_confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(prompt) = self.dialogs.pending_sync_pull_confirm.take() {
            self.start_dialog_exit(DialogOverlaySnapshot::SyncPullConfirm(prompt), cx);
        }
    }

    pub(in crate::ui::shell) fn confirm_sync_pull(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.dialogs.pending_sync_pull_confirm.take() else {
            return;
        };

        self.start_dialog_exit(DialogOverlaySnapshot::SyncPullConfirm(prompt), cx);
        self.execute_manual_sync(ManualSyncAction::Pull, window, cx);
    }

    pub(in crate::ui::shell) fn confirm_sync_force_push(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.dialogs.pending_sync_pull_confirm.take() else {
            return;
        };

        self.start_dialog_exit(DialogOverlaySnapshot::SyncPullConfirm(prompt), cx);
        self.execute_manual_sync(ManualSyncAction::ForcePush, window, cx);
    }

    pub(in crate::ui::shell) fn reload_data_after_sync(&mut self, cx: &mut Context<Self>) {
        let mut any_reload_failed = false;

        match SettingsStore::load() {
            Ok(store) => {
                self.settings_store = store;
                settings::sync_component_theme(cx);
            }
            Err(e) => {
                log::warn!("failed to reload settings after sync: {e:?}");
                any_reload_failed = true;
            }
        }
        if let Ok(service) = SyncService::new(
            self.services.runtime.clone(),
            self.services.session_store.clone(),
            self.services.snippet_store.clone(),
            self.services.keychain_store.clone(),
            self.services.secrets.clone(),
        ) {
            match service.reload_sessions() {
                Ok(sessions) => self.data.sessions = sessions,
                Err(e) => {
                    log::warn!("failed to reload sessions after sync: {e:?}");
                    any_reload_failed = true;
                }
            }
            match service.reload_snippets() {
                Ok(snippets) => self.data.snippets = snippets,
                Err(e) => {
                    log::warn!("failed to reload snippets after sync: {e:?}");
                    any_reload_failed = true;
                }
            }
            match service.reload_managed_keys() {
                Ok(keys) => self.data.managed_keys = keys,
                Err(e) => {
                    log::warn!("failed to reload keys after sync: {e:?}");
                    any_reload_failed = true;
                }
            }
        }

        if any_reload_failed {
            let notification = Self::error_notification(
                i18n::string("settings.sync.status.notifications.reload_failed_title"),
                i18n::string("settings.sync.status.notifications.reload_failed_message"),
            );
            if let Some(window_handle) = cx.active_window() {
                let _ = window_handle.update(cx, move |_, window, cx| {
                    window.push_notification(notification, cx);
                });
            }
        }

        self.sync_managed_key_select_in_active_window(None, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_push_routes_pull_required_into_pull_confirm() {
        assert_eq!(
            AppView::manual_push_pull_confirm_reason(
                ManualSyncAction::Push,
                &SyncStatus::PullRequired { remote_at: 42 },
            ),
            Some(SyncPullConfirmReason::RemoteNewer)
        );
        assert_eq!(
            AppView::manual_push_pull_confirm_reason(
                ManualSyncAction::Pull,
                &SyncStatus::PullRequired { remote_at: 42 },
            ),
            None
        );
        assert_eq!(
            AppView::manual_push_pull_confirm_reason(
                ManualSyncAction::ForcePush,
                &SyncStatus::PullRequired { remote_at: 42 },
            ),
            None
        );
    }

    #[test]
    fn manual_sync_gate_routes_locked_state_to_unlock_popup_follow_up() {
        let gate = AppView::manual_sync_gate(false, true, true);

        assert_eq!(gate, ManualSyncGate::VaultUnlockRequired);
        assert!(matches!(
            AppView::manual_sync_unlock_follow_up(gate),
            Some(PendingLocalVaultUnlockAction::OpenSyncNow)
        ));
    }

    #[test]
    fn manual_sync_gate_preserves_provider_error_before_unlock_prompt() {
        let gate = AppView::manual_sync_gate(false, false, true);

        assert_eq!(gate, ManualSyncGate::ProviderRequired);
        assert!(AppView::manual_sync_unlock_follow_up(gate).is_none());
    }
}
